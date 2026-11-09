//! Hook runs: what `instrukt hooks run <hook>` starts, the detached worker
//! that waits for the sync it started, and the per-hook record of the last
//! run's outcome that `hooks status` shows.
//!
//! Checkout rendering completes before return. Other hook work is detached. For a post-hook it writes the
//! run's record (sync `running`), starts the detached worker and returns;
//! the worker starts `instrukt sync`, waits for it within a bound, and
//! completes the record with sync's exit status — unless a later run of the
//! same hook has replaced the record meanwhile.

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::{self, Env};
use crate::fsx::{LockGuard, atomic_write};
use crate::git;
use crate::home::Home;
use crate::ids::{CommitSha, ProjectSlug};
use crate::interfaces::{CoreWiring, GenerationRequest, MainAdvance};

use super::_error::RepositoryError;
use super::_git;
use super::_hook::Hook;

/// The arguments, after the executable, that start the detached hook
/// worker. The CLI routes them to [`run_hook_worker`].
pub const HOOK_WORKER_COMMAND: [&str; 2] = ["hooks", "worker"];

/// How long the worker waits for a sync before ending it.
const SYNC_TIMEOUT: Duration = Duration::from_secs(600);
/// How often the worker looks whether sync ended.
/// How long a record writer waits for another writer of the same record.
const RECORD_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest sync message a record keeps.
const MESSAGE_LIMIT: usize = 2000;

/// What one hook run did, recorded per hook under
/// `state/hooks/<project>/<hook>.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookRunRecord {
    #[serde(default = "hook_record_version")]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout: Option<CheckoutOutcome>,
    pub hook: Hook,
    /// Identifies the run, so a worker completes only its own run's record.
    pub run: String,
    /// When the run started, in milliseconds since the Unix epoch.
    pub started_at_ms: u64,
    /// The sync the run started; `None` for a hook that starts none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncOutcome>,
    /// The revision handed to graph generation; `None` when the run was no
    /// trigger for it or blueprint is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_generation: Option<GenerationOutcome>,
}

/// The first versioned hook-record shape. Unversioned records convert on read
/// with no checkout outcome, and the next recorded run persists this shape.
fn hook_record_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CheckoutOutcome {
    Rendered,
    Bootstrapped,
    Failed { error: String },
}

/// The outcome of the sync a hook run started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SyncOutcome {
    /// Started; the worker has not seen it end yet.
    Running,
    /// The worker or the sync could not be started.
    NotStarted { error: String },
    /// Sync ended. `exit_code` is `None` when a signal ended it; a non-zero
    /// exit (a refused or failed sync) keeps the last line sync reported.
    Finished {
        finished_at_ms: u64,
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Sync outlived the worker's bound and was ended.
    TimedOut {
        finished_at_ms: u64,
        after_secs: u64,
    },
}

/// The outcome of handing a revision of main to graph generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GenerationOutcome {
    Started { sha: CommitSha },
    Failed { sha: CommitSha, error: String },
}

/// What git handed a hook run.
#[derive(Debug, Clone, Copy)]
pub struct HookInvocation<'a> {
    pub hook: Hook,
    /// The hook's arguments, as git passed them.
    pub args: &'a [String],
    /// The hook's standard input (pre-push's ref lines), empty for others.
    pub stdin: &'a str,
    /// The environment git ran the hook in.
    pub env: &'a Env,
    /// The running instrukt executable, which the worker and sync run as.
    pub exe: &'a Path,
}

/// Runs one hook point in the repository containing `cwd`: hands a push
/// to main or a pull advancing main to graph generation when it is wired,
/// and for a post-hook starts the detached worker that runs `instrukt
/// sync`. Checkout first renders published entrypoints synchronously; other
/// work returns without waiting. Outside a working tree it
/// does nothing.
pub fn run_hook(
    home: &Home,
    cwd: &Path,
    wiring: &CoreWiring,
    invocation: &HookInvocation<'_>,
) -> Result<(), RepositoryError> {
    let started = Instant::now();
    let hook = invocation.hook;
    let Some(root) = git::worktree_root(cwd)? else {
        tracing::debug!(event = "hook_run_outside_worktree", hook = %hook, cwd = %cwd.display());
        return Ok(());
    };
    if hook == Hook::PostCheckout {
        let outcome = crate::publication::checkout_entrypoints(&crate::publication::SyncRequest {
            home,
            cwd: &root,
            force: false,
            wiring,
            now: jiff::Timestamp::now(),
        });
        let checkout = match &outcome {
            Ok(crate::publication::Checkout::Bootstrapped) => CheckoutOutcome::Bootstrapped,
            Ok(_) => CheckoutOutcome::Rendered,
            Err(error) => CheckoutOutcome::Failed {
                error: error.to_string(),
            },
        };
        if let Some(project) = config::project_slug(&root)? {
            save(
                home,
                &project,
                &HookRunRecord {
                    schema_version: hook_record_version(),
                    checkout: Some(checkout),
                    hook,
                    run: format!("{}-{}", now_ms(), std::process::id()),
                    started_at_ms: now_ms(),
                    sync: None,
                    graph_generation: None,
                },
            )?;
        }
        if !matches!(outcome?, crate::publication::Checkout::Primary) {
            return Ok(());
        }
    }
    if invocation.hook.starts_sync()
        && git::run(&root, ["branch", "--show-current"])? != crate::github::default_branch(&root)?
    {
        tracing::debug!(event="hook_unintegrated_worktree",root=%root.display());
        return Ok(());
    }
    let project = config::project_slug(&root)?;
    // A handoff that cannot be decided or made is reported and never keeps
    // the sync from starting.
    let graph_generation = match &wiring.graph_generation {
        Some(generation) => main_advance(&root, invocation)
            .and_then(|advance| {
                advance
                    .map(|(sha, advance)| {
                        hand_to_generation(
                            &root,
                            project.as_ref(),
                            generation.as_ref(),
                            sha,
                            advance,
                        )
                    })
                    .transpose()
            })
            .unwrap_or_else(|err| {
                tracing::warn!(event = "graph_generation_not_decided", hook = %hook, error = %err);
                None
            })
            .flatten(),
        None => None,
    };
    let run = format!("{}-{}", now_ms(), std::process::id());
    let mut record = HookRunRecord {
        schema_version: hook_record_version(),
        checkout: (hook == Hook::PostCheckout).then_some(CheckoutOutcome::Rendered),
        hook,
        run: run.clone(),
        started_at_ms: now_ms(),
        sync: hook.starts_sync().then_some(SyncOutcome::Running),
        graph_generation,
    };
    if let Some(project) = &project {
        save(home, project, &record)?;
    }
    if hook.starts_sync()
        && let Err(err) = start_worker(invocation.exe, &root, hook, &run, project.as_ref())
    {
        tracing::warn!(event = "hook_worker_not_started", hook = %hook, error = %err);
        record.sync = Some(SyncOutcome::NotStarted {
            error: err.to_string(),
        });
        if let Some(project) = &project {
            save(home, project, &record)?;
        }
    }
    tracing::debug!(
        event = "hook_run_started",
        hook = %hook,
        run = %run,
        repo_root = %root.display(),
        recorded = project.is_some(),
        duration_ms = started.elapsed().as_millis() as u64,
    );
    Ok(())
}

/// The detached worker a post-hook run started (`job` is what the run
/// passed after [`HOOK_WORKER_COMMAND`]): runs `instrukt sync` in `cwd`,
/// waits for it within its bound and completes the run's record.
pub fn run_hook_worker(
    home: &Home,
    cwd: &Path,
    exe: &Path,
    job: &[String],
) -> Result<(), RepositoryError> {
    let malformed = || RepositoryError::WorkerJob { job: job.join(" ") };
    let (hook, run, project) = match job {
        [hook, run] => (hook, run, None),
        [hook, run, project] => (hook, run, Some(ProjectSlug::parse(project)?)),
        _ => return Err(malformed()),
    };
    let hook = Hook::parse(hook).ok_or_else(malformed)?;
    let outcome = run_sync(exe, cwd);
    let exit_code = match &outcome {
        SyncOutcome::Finished { exit_code, .. } => *exit_code,
        _ => None,
    };
    tracing::debug!(
        event = "hook_sync_finished",
        hook = %hook,
        run = %run,
        exit_code = ?exit_code,
        recorded = project.is_some(),
    );
    if let Some(project) = project {
        complete(home, &project, hook, run, outcome)?;
    }
    Ok(())
}

/// The last recorded run of `hook` in `project`; `None` before the first.
pub fn last_run(
    home: &Home,
    project: &ProjectSlug,
    hook: Hook,
) -> Result<Option<HookRunRecord>, RepositoryError> {
    let path = record_path(home, project, hook);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(RepositoryError::Read { path, source }),
    };
    let record: HookRunRecord =
        serde_yaml_ng::from_str(&text).map_err(|source| RepositoryError::Record {
            path: path.clone(),
            source,
        })?;
    if record.schema_version != hook_record_version() {
        return Err(RepositoryError::HookRecordVersion {
            path,
            version: record.schema_version,
        });
    }
    Ok(Some(record))
}

fn start_worker(
    exe: &Path,
    root: &Path,
    hook: Hook,
    run: &str,
    project: Option<&ProjectSlug>,
) -> Result<(), RepositoryError> {
    let mut command = Command::new(exe);
    git::strip_repository_env(&mut command)
        .args(HOOK_WORKER_COMMAND)
        .arg(hook.name())
        .arg(run)
        .args(project.map(ProjectSlug::as_str))
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group: a terminal's signals to git's group do not
        // reach it.
        .process_group(0);
    #[allow(
        clippy::zombie_processes,
        reason = "detached by design: the hook run returns at once and the worker is reparented when it exits"
    )]
    command.spawn().map_err(|source| RepositoryError::Spawn {
        program: exe.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Runs `instrukt sync` in `cwd` and waits for it within [`SYNC_TIMEOUT`].
fn run_sync(exe: &Path, cwd: &Path) -> SyncOutcome {
    let mut command = Command::new(exe);
    git::strip_repository_env(&mut command)
        .arg("sync")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    match crate::process::output(&mut command, None, SYNC_TIMEOUT) {
        Ok(output) => SyncOutcome::Finished {
            finished_at_ms: now_ms(),
            exit_code: output.status.code(),
            message: (!output.status.success())
                .then(|| last_line(&String::from_utf8_lossy(&output.stderr)))
                .flatten(),
        },
        Err(crate::process::ProcessError::Timeout { .. }) => SyncOutcome::TimedOut {
            finished_at_ms: now_ms(),
            after_secs: SYNC_TIMEOUT.as_secs(),
        },
        Err(error) => SyncOutcome::NotStarted {
            error: error.to_string(),
        },
    }
}

/// The last non-empty line of `text`, within [`MESSAGE_LIMIT`].
fn last_line(text: &str) -> Option<String> {
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())?
        .trim();
    Some(line.chars().take(MESSAGE_LIMIT).collect())
}

/// The upstream revision of main brought by a push, pull or runner fast-forward.
/// An ordinary merge that differs from the fetched upstream is not a trigger.
fn main_advance(
    root: &Path,
    invocation: &HookInvocation<'_>,
) -> Result<Option<(CommitSha, MainAdvance)>, RepositoryError> {
    match invocation.hook {
        Hook::PrePush => {
            let Some(remote) = invocation.args.first() else {
                return Ok(None);
            };
            let branch = _git::default_branch(root, remote)?;
            Ok(pushed_to(invocation.stdin, &branch).map(|sha| (sha, MainAdvance::Push)))
        }
        Hook::PostMerge | Hook::PostRewrite => {
            let reflog_action = invocation.env.get("GIT_REFLOG_ACTION");
            if invocation.hook == Hook::PostRewrite
                && !is_pull(invocation.hook, invocation.args, reflog_action)
            {
                return Ok(None);
            }
            let Some(branch) = _git::current_branch(root)? else {
                return Ok(None);
            };
            let Some((remote, tracking)) = _git::upstream(root, &branch)? else {
                return Ok(None);
            };
            if branch != _git::default_branch(root, &remote)? {
                return Ok(None);
            }
            let sha = CommitSha::parse(&_git::resolve_commit(root, &tracking)?)?;
            if !is_pull(invocation.hook, invocation.args, reflog_action)
                && _git::resolve_commit(root, "HEAD")? != sha.as_str()
            {
                return Ok(None);
            }
            Ok(Some((sha, MainAdvance::Pull)))
        }
        Hook::PostCommit | Hook::PostCheckout => Ok(None),
    }
}

/// Whether git ran this post-merge or post-rewrite as part of a pull (a
/// pull through rebase rewrites with `rebase`).
fn is_pull(hook: Hook, args: &[String], reflog_action: Option<&str>) -> bool {
    let pulling =
        reflog_action.is_some_and(|action| action == "pull" || action.starts_with("pull "));
    match hook {
        Hook::PostMerge => pulling,
        Hook::PostRewrite => pulling && args.first().is_some_and(|kind| kind == "rebase"),
        _ => false,
    }
}

/// The commit pushed to `refs/heads/<branch>`, from pre-push's ref lines
/// (`<local ref> <local sha> <remote ref> <remote sha>`); a deletion pushes
/// nothing.
fn pushed_to(stdin: &str, branch: &str) -> Option<CommitSha> {
    let target = format!("refs/heads/{branch}");
    stdin.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            [_, local_sha, remote_ref, _] if *remote_ref == target => {
                if local_sha.chars().all(|c| c == '0') {
                    None
                } else {
                    CommitSha::parse(local_sha).ok()
                }
            }
            _ => None,
        }
    })
}

fn hand_to_generation(
    root: &Path,
    project: Option<&ProjectSlug>,
    generation: &dyn crate::interfaces::GraphGeneration,
    sha: CommitSha,
    advance: MainAdvance,
) -> Result<Option<GenerationOutcome>, RepositoryError> {
    let Some(project) = project else {
        tracing::warn!(
            event = "graph_generation_without_project",
            repo_root = %root.display(),
            sha = %sha,
            "the project layer declares no project slug; no graph is generated"
        );
        return Ok(None);
    };
    let request = GenerationRequest {
        project: project.clone(),
        project_root: _git::primary_worktree(root)?,
        sha: sha.clone(),
        advance,
    };
    Ok(Some(match generation.start_generation(&request) {
        Ok(()) => GenerationOutcome::Started { sha },
        Err(err) => {
            tracing::warn!(event = "graph_generation_not_started", sha = %sha, error = %err);
            GenerationOutcome::Failed {
                sha,
                error: err.to_string(),
            }
        }
    }))
}

fn record_path(home: &Home, project: &ProjectSlug, hook: Hook) -> PathBuf {
    home.hooks_state_dir(project)
        .join(format!("{}.yaml", hook.name()))
}

fn lock_path(home: &Home, project: &ProjectSlug, hook: Hook) -> PathBuf {
    home.hooks_state_dir(project)
        .join(format!("{}.lock", hook.name()))
}

/// Writes `record` as the hook's last run.
fn save(home: &Home, project: &ProjectSlug, record: &HookRunRecord) -> Result<(), RepositoryError> {
    let _lock = LockGuard::acquire(&lock_path(home, project, record.hook), RECORD_LOCK_TIMEOUT)?;
    write(home, project, record)
}

/// Completes run `run`'s record with the sync outcome, unless a later run
/// replaced it.
fn complete(
    home: &Home,
    project: &ProjectSlug,
    hook: Hook,
    run: &str,
    outcome: SyncOutcome,
) -> Result<(), RepositoryError> {
    let _lock = LockGuard::acquire(&lock_path(home, project, hook), RECORD_LOCK_TIMEOUT)?;
    match last_run(home, project, hook)? {
        Some(mut record) if record.run == run => {
            record.sync = Some(outcome);
            write(home, project, &record)
        }
        _ => {
            tracing::debug!(event = "hook_run_superseded", hook = %hook, run = %run);
            Ok(())
        }
    }
}

fn write(
    home: &Home,
    project: &ProjectSlug,
    record: &HookRunRecord,
) -> Result<(), RepositoryError> {
    let text = serde_yaml_ng::to_string(record).map_err(RepositoryError::RenderRecord)?;
    atomic_write(&record_path(home, project, record.hook), text.as_bytes())?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "1111111111111111111111111111111111111111";
    const ZERO: &str = "0000000000000000000000000000000000000000";

    #[test]
    fn unversioned_hook_records_read_as_current_without_inventing_a_checkout() {
        let legacy = "hook: post-checkout\nrun: old\nstarted_at_ms: 1\nsync: null\n";
        let record: HookRunRecord = serde_yaml_ng::from_str(legacy).unwrap();
        assert_eq!(record.schema_version, hook_record_version());
        assert_eq!(record.checkout, None);
    }

    #[test]
    fn a_push_to_main_names_the_pushed_commit_and_nothing_else_does() {
        let to_main = format!("refs/heads/main {SHA} refs/heads/main {ZERO}\n");
        assert_eq!(
            pushed_to(&to_main, "main"),
            Some(CommitSha::parse(SHA).unwrap())
        );
        assert_eq!(pushed_to(&to_main, "trunk"), None);
        let other = format!("refs/heads/f {SHA} refs/heads/f {ZERO}\n");
        assert_eq!(pushed_to(&other, "main"), None);
        let deletion = format!("(delete) {ZERO} refs/heads/main {SHA}\n");
        assert_eq!(pushed_to(&deletion, "main"), None);
    }

    #[test]
    fn only_a_pull_advances_main_through_merge_or_rebase() {
        let args = |kind: &str| vec![kind.to_owned()];
        assert!(is_pull(Hook::PostMerge, &args("0"), Some("pull")));
        assert!(is_pull(Hook::PostMerge, &args("0"), Some("pull -q")));
        assert!(is_pull(
            Hook::PostRewrite,
            &args("rebase"),
            Some("pull --rebase")
        ));
        assert!(!is_pull(
            Hook::PostRewrite,
            &args("amend"),
            Some("pull --rebase")
        ));
        assert!(!is_pull(Hook::PostMerge, &args("0"), Some("merge feature")));
        assert!(!is_pull(Hook::PostMerge, &args("0"), Some("pullish")));
        assert!(!is_pull(Hook::PostMerge, &args("0"), None));
    }
}
