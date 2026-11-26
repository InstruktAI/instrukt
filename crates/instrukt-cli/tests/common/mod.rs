//! The test harness for instrukt's command-line tests.
//!
//! Every test runs the real `instrukt` binary against temporary application roots
//! and real git repositories inside a temporary directory
//! (`docs/project/policy/testing.md`). [`TestEnv`] owns that directory:
//!
//! - subprocesses (instrukt and git) run with a cleared environment: `PATH`,
//!   the inherited `HOME`, explicit native roots, an isolated git configuration and whatever the
//!   test sets with [`TestEnv::set_env`] — no live session variable leaks in;
//! - `INSTRUKT_LOG_DIR` selects a per-test log directory independently of the
//!   fixture directory; it is created by the logger, never by setup;
//! - every subprocess carries the write fence
//!   (`instrukt_core::fsx::FENCE_VARIABLE`) over the live user's home — the
//!   whole home when the inherited temporary directory lies outside it,
//!   otherwise the locations instrukt is known to write there, each around
//!   the way down to that directory (`live_fence`): an
//!   instrukt process whose own file operation would land there, directly or
//!   through a symbolic link, refuses it before it happens and records the
//!   refusal in a record this test owns and names in the fence; on drop the
//!   test fails when a refusal was recorded, or when the record cannot be
//!   read. What other processes on the computer do to the live home is not
//!   the test's concern and is not looked at. The fence is not a sandbox:
//!   the programs instrukt starts and the test's own fixtures stay inside the
//!   fixture directory by construction.
//!
//! [`github::FakeGitHub`] (or [`TestEnv::with_fake_github`]) stands in for
//! GitHub's API at the network line for the work, workflow and runner
//! tests.
#![allow(
    dead_code,
    reason = "each test crate uses the part of the harness it needs"
)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test harness: a failed setup step fails the test"
)]

#[path = "../../../../tests/support/wait.rs"]
mod polling;
pub use polling::{wait_until, wait_until_with_interval};

pub mod github;
pub mod redis;
pub mod redis_proxy;
pub mod t3_readiness;

/// `text` with every published commit a source line names
/// (`published commit <sha>`) shown as `[SHA]`: commits made in a test
/// differ from run to run.
pub fn mask_published_commits(text: &str) -> String {
    const MARK: &str = "published commit ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(MARK) {
        let after = at + MARK.len();
        out.push_str(&rest[..after]);
        let hex = rest[after..]
            .bytes()
            .take_while(u8::is_ascii_hexdigit)
            .count();
        if hex == 40 {
            out.push_str("[SHA]");
            rest = &rest[after + hex..];
        } else {
            rest = &rest[after..];
        }
    }
    out.push_str(rest);
    out
}

/// The repository root of the published position `domain` addresses:
/// `docs/general` for `general`, `docs/domain/<domain>` for any other.
pub fn position_dir(domain: &str) -> String {
    if domain == "general" {
        "docs/general".to_owned()
    } else {
        format!("docs/domain/{domain}")
    }
}

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::SystemTime;

use assert_cmd::Command;
use assert_cmd::assert::Assert;
use tempfile::TempDir;

/// The git identity every test's git configuration carries.
const DEVELOPER_NAME: &str = "Test Developer";
const DEVELOPER_EMAIL: &str = "developer@example.test";

/// A temporary machine: a user home, a place for repositories, and the
/// environment every subprocess of the test runs with.
pub struct TestEnv {
    root: TempDir,
    _snapshot_settings: insta::internals::SettingsBindDropGuard,
    home: PathBuf,
    vars: BTreeMap<String, String>,
    redis: std::sync::Mutex<Option<redis::RedisServer>>,
    readiness: std::sync::Mutex<Option<t3_readiness::Server>>,
}

/// What a test inherits from the session that runs it: the live home its
/// children run with and the fence protects, the temporary directory its
/// fixture is created in and the fence is computed around, and the `TMPDIR`
/// value its children are given, verbatim, when there is one.
struct Session {
    home: String,
    temp_dir: PathBuf,
    tmpdir: Option<String>,
}

impl TestEnv {
    /// A test machine in the session that runs the test: children get the
    /// inherited `HOME`, and `TMPDIR` only when the host sets it, as set.
    pub fn new() -> Self {
        Self::in_session(
            &Session {
                home: std::env::var("HOME").unwrap(),
                temp_dir: std::env::temp_dir(),
                tmpdir: std::env::var("TMPDIR").ok(),
            },
            Vec::new(),
        )
    }

    /// A test machine in a session that inherited `home` and `temp_dir`,
    /// for a fence self-test that models one: the fixture is created in
    /// `temp_dir`, children get `home` as their `HOME` and `temp_dir` as
    /// their `TMPDIR`, and the fence covers `home` computed around
    /// `temp_dir` — and also the live home of the session running the test,
    /// so no child runs unfenced against it.
    pub fn in_stand_in_session(home: &Path, temp_dir: &Path) -> Self {
        let real = live_fence(
            Path::new(&std::env::var("HOME").unwrap()),
            &std::env::temp_dir(),
        );
        Self::in_session(
            &Session {
                home: home.display().to_string(),
                temp_dir: temp_dir.to_path_buf(),
                tmpdir: Some(temp_dir.display().to_string()),
            },
            real,
        )
    }

    /// The test machine of `session`; `also_fenced` is appended to the
    /// session's own live fence.
    fn in_session(session: &Session, also_fenced: Vec<PathBuf>) -> Self {
        let root = tempfile::Builder::new()
            .prefix("instrukt-test-")
            .tempdir_in(&session.temp_dir)
            .unwrap();
        // Canonical path: macOS's /var → /private/var would otherwise make
        // paths the binary reports differ from the ones the test built.
        let base = root.path().canonicalize().unwrap();
        let home = base.join("home");
        let tmp = base.join("tmp");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            format!(
                "[user]\n\tname = {DEVELOPER_NAME}\n\temail = {DEVELOPER_EMAIL}\n\
                 [init]\n\tdefaultBranch = main\n[commit]\n\tgpgsign = false\n\
                 [tag]\n\tgpgsign = false\n"
            ),
        )
        .unwrap();
        // Every isolated test shadows the live runtime, including tests that
        // only request model provenance for an otherwise unknown caller.
        let runtime_bin = base.join("runtime-bin");
        std::fs::create_dir_all(&runtime_bin).unwrap();
        let t3 = runtime_bin.join("t3");
        std::fs::write(
            &t3,
            "#!/bin/sh\necho 'test runtime is not configured' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&t3, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tools = base.join("system-tools");
        std::fs::create_dir_all(&tools).unwrap();
        for tool in [
            "git",
            "sh",
            "bash",
            "env",
            "python3",
            "cat",
            "rm",
            "mkdir",
            "mktemp",
            "dirname",
            "ps",
            "tr",
            "touch",
            "kill",
            "chmod",
            "cp",
            "mv",
            "ln",
            "uname",
            "head",
            "wc",
            "readlink",
            "sed",
            "tar",
            "gzip",
            "find",
            "sort",
            "grep",
            "id",
            "redis-server",
            "awk",
            "shasum",
        ] {
            std::os::unix::fs::symlink(tool_executable(tool), tools.join(tool)).unwrap();
        }
        let mut vars = BTreeMap::new();
        vars.insert(
            "PATH".into(),
            format!("{}:{}", base.join("runtime-bin").display(), tools.display()),
        );
        vars.insert("HOME".into(), session.home.clone());
        for (key, relative) in [
            ("INSTRUKT_HOME", ".instrukt"),
            ("T3CODE_HOME", ".t3"),
            ("CODEX_HOME", ".codex"),
            ("CLAUDE_CONFIG_DIR", ".claude"),
            ("OPENCODE_CONFIG_DIR", ".config/opencode"),
            ("FX_HOME", ".fx"),
            ("KIRO_HOME", ".kiro"),
            ("PI_CODING_AGENT_DIR", ".pi/agent"),
            ("SYSTEMD_UNIT_PATH", ".config/systemd/user"),
            ("INSTRUKT_LAUNCH_AGENTS_DIR", "Library/LaunchAgents"),
            ("INSTRUKT_CRONTAB_FILE", ".instrukt-crontab"),
        ] {
            vars.insert(key.into(), home.join(relative).display().to_string());
        }
        vars.insert(
            "INSTRUKT_LOG_DIR".into(),
            base.join("logs").display().to_string(),
        );

        if let Some(tmpdir) = &session.tmpdir {
            vars.insert("TMPDIR".into(), tmpdir.clone());
        }
        for key in ["XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"] {
            if let Ok(value) = std::env::var(key) {
                vars.insert(key.into(), value);
            }
        }
        vars.insert(
            "GIT_CONFIG_GLOBAL".into(),
            home.join(".gitconfig").display().to_string(),
        );
        vars.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        vars.insert("LANG".into(), "C.UTF-8".into());
        // The record lives in the test's own directory, where no command's
        // settings point; the fence names it, and instrukt only appends to it.
        let record = base.join("fence");
        std::fs::create_dir_all(&record).unwrap();
        std::fs::write(record.join(instrukt_core::fsx::FENCE_RECORD), "").unwrap();
        let mut fence = live_fence(Path::new(&session.home), &session.temp_dir);
        fence.extend(also_fenced);
        fence.push(PathBuf::from(format!(
            "{}{}",
            instrukt_core::fsx::FENCE_RECORD_ENTRY,
            record.display()
        )));
        vars.insert(
            instrukt_core::fsx::FENCE_VARIABLE.into(),
            std::env::join_paths(fence).unwrap().into_string().unwrap(),
        );
        let mut snapshots = insta::Settings::clone_current();
        snapshots.add_filter(
            r"(ai\.instrukt\.[a-zA-Z0-9_-]+)\.[0-9a-f]{8}\b",
            "$1.[HOME]",
        );
        snapshots.add_filter(r"(instrukt-[a-zA-Z0-9_-]+)-[0-9a-f]{8}(\b)", "$1-[HOME]$2");
        let inherited = session
            .home
            .chars()
            .map(|c| {
                if ".+*?()|[]{}^$\\".contains(c) {
                    format!("\\{c}")
                } else {
                    c.to_string()
                }
            })
            .collect::<String>();
        snapshots.add_filter(&inherited, "[INHERITED HOME]");
        let snapshot_settings = snapshots.bind_to_scope();
        Self {
            root,
            _snapshot_settings: snapshot_settings,
            home,
            vars,
            redis: std::sync::Mutex::new(None),
            readiness: std::sync::Mutex::new(None),
        }
    }

    pub fn readiness_server(&self) {
        let mut server = self.readiness.lock().unwrap();
        server.get_or_insert_with(|| t3_readiness::Server::new(self));
    }

    /// Stops T3's endpoint: its descriptor stays and names a port no one
    /// listens on, so T3 cannot be asked until it starts again.
    pub fn stop_readiness_server(&self) {
        drop(self.readiness.lock().unwrap().take());
    }

    /// Lazily start this temporary computer's Redis; it dies with the test.
    pub fn redis_url(&self) -> String {
        let mut server = self.redis.lock().unwrap();
        server
            .get_or_insert_with(|| redis::RedisServer::new(self))
            .url
            .clone()
    }

    /// The temporary fixture directory; subprocess HOME stays inherited.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The configured app log directory, including a case-specific override.
    pub fn log_dir(&self) -> PathBuf {
        PathBuf::from(self.injected("INSTRUKT_LOG_DIR").unwrap())
    }

    /// The explicitly selected Instrukt root of this fixture.
    pub fn instrukt_home(&self) -> PathBuf {
        PathBuf::from(
            self.vars
                .get("INSTRUKT_HOME")
                .expect("fixture selects an Instrukt root"),
        )
    }

    /// Where the last run of `hook` in the repository at `repo` is recorded:
    /// `state/hook-runs/<key>/<hook>.yaml`, the key computed as the product
    /// computes it from the repository's git common directory.
    pub fn hook_record_path(&self, repo: &Path, hook: instrukt_core::repository::Hook) -> PathBuf {
        let common = self.git(
            repo,
            ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        );
        let common = PathBuf::from(common.trim()).canonicalize().unwrap();
        self.instrukt_home()
            .join("state/hook-runs")
            .join(instrukt_core::repository::repository_key(&common))
            .join(format!("{}.yaml", hook.name()))
    }

    /// The last run of `hook` in the repository at `repo` once its detached
    /// sync has ended: waits, failing the test after a bound.
    pub fn finished_hook_run(
        &self,
        repo: &Path,
        hook: instrukt_core::repository::Hook,
    ) -> instrukt_core::repository::HookRunRecord {
        use instrukt_core::repository::{HookRunRecord, SyncOutcome};
        let path = self.hook_record_path(repo, hook);
        let mut finished = None;
        wait_until(
            &format!("{hook} sync in {}", repo.display()),
            std::time::Duration::from_secs(60),
            || {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let record: HookRunRecord = serde_yaml_ng::from_str(&text).unwrap();
                    if record.sync != Some(SyncOutcome::Running) {
                        finished = Some(record);
                        return true;
                    }
                }
                false
            },
        );
        finished.unwrap()
    }

    /// A path inside the test's directory, outside `HOME`.
    pub fn service_name(&self, name: &str) -> String {
        use sha2::{Digest, Sha256};
        let root = self.instrukt_home();
        let root = root.canonicalize().unwrap_or(root);
        let digest = hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()));
        let (stem, extension) = name.rsplit_once('.').unwrap();
        let separator = if name.starts_with("ai.") { "." } else { "-" };
        format!("{stem}{separator}{}.{extension}", &digest[..8])
    }

    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    pub fn injected(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.base().join(relative)
    }

    /// Adds `location` to the write fence every later instrukt process
    /// carries.
    pub fn fence(&mut self, location: &Path) -> &mut Self {
        let current = self
            .vars
            .get(instrukt_core::fsx::FENCE_VARIABLE)
            .cloned()
            .unwrap_or_default();
        let mut fenced: Vec<PathBuf> = std::env::split_paths(&current).collect();
        fenced.push(location.to_path_buf());
        let joined = std::env::join_paths(fenced).unwrap().into_string().unwrap();
        self.vars
            .insert(instrukt_core::fsx::FENCE_VARIABLE.into(), joined);
        self
    }

    /// The writes instrukt processes of this test refused at the fence, one
    /// line each; taking them empties the record. A record that cannot be
    /// read fails the test: it is never taken for no refusal.
    pub fn refusals(&self) -> Vec<String> {
        let record = self.path("fence").join(instrukt_core::fsx::FENCE_RECORD);
        let text = std::fs::read_to_string(&record).unwrap_or_else(|error| {
            panic!(
                "the write fence's record {} cannot be read: {error}",
                record.display()
            )
        });
        std::fs::write(&record, "").unwrap();
        text.lines().map(str::to_owned).collect()
    }

    /// Sets an environment variable for every later subprocess, e.g. a
    /// runtime's session variable.
    pub fn set_env(&mut self, key: &str, value: &str) -> &mut Self {
        let value = if key == "PATH" {
            format!("{}:{value}", self.path("runtime-bin").display())
        } else if key == "T3CODE_THREAD_ID" {
            std::fs::read_to_string(self.path(&format!("t3-state/{value}.alias")))
                .unwrap_or_else(|_| value.to_owned())
        } else {
            value.to_owned()
        };
        self.vars.insert(key.to_owned(), value);
        self
    }

    pub fn remove_env(&mut self, key: &str) -> &mut Self {
        self.vars.remove(key);
        self
    }

    /// A new git repository at `<test dir>/<name>` on branch `main`, with
    /// no commits. A developer's repository on this computer is a project T3
    /// holds, so the fixture places its runtime project too.
    pub fn git_repo(&self, name: &str) -> Repo {
        let path = self.base().join(name);
        std::fs::create_dir_all(&path).unwrap();
        let repo = Repo {
            path,
            vars: self.vars.clone(),
        };
        repo.git(["init", "--quiet", "--initial-branch=main"]);
        self.place_runtime_project(&repo.path);
        repo
    }

    /// Places a live T3 project rooted exactly at `checkout`, as T3 holds
    /// one for a folder a person added.
    pub fn place_runtime_project(&self, checkout: &Path) {
        t3_readiness::place_project(
            &self.base(),
            checkout,
            &t3_readiness::default_project_id(checkout),
        );
    }

    /// Places the runtime project of company home `name`, as `instrukt
    /// install` leaves it for an onboarded company.
    pub fn place_company_project(&self, name: &str) {
        let home = instrukt_core::home::Home::at_user_home(self.home().into());
        let company = home.company_dir(&instrukt_core::ids::ProjectSlug::parse(name).unwrap());
        std::fs::create_dir_all(&company).unwrap();
        self.place_runtime_project(&company);
    }

    /// Removes the T3 project rooted exactly at `checkout`, as `t3 project
    /// remove` leaves it: deleted.
    pub fn withdraw_runtime_project(&self, checkout: &Path) {
        t3_readiness::withdraw_project(&self.base(), checkout);
    }

    /// A new bare repository at `<test dir>/<name>.git`, to push to and
    /// clone from.
    pub fn bare_remote(&self, name: &str) -> PathBuf {
        let path = self.base().join(format!("{name}.git"));
        std::fs::create_dir_all(&path).unwrap();
        run_git(
            &self.vars,
            &path,
            ["init", "--quiet", "--bare", "--initial-branch=main"],
        );
        path
    }

    /// Clones `remote` into `<test dir>/<name>`.
    pub fn clone_repo(&self, remote: &Path, name: &str) -> Repo {
        let path = self.base().join(name);
        run_git(
            &self.vars,
            &self.base(),
            [
                OsStr::new("clone"),
                OsStr::new("--quiet"),
                remote.as_os_str(),
                path.as_os_str(),
            ],
        );
        self.place_runtime_project(&path);
        Repo {
            path,
            vars: self.vars.clone(),
        }
    }

    pub fn command(&self, program: &Path, cwd: &Path) -> StdCommand {
        let mut command = StdCommand::new(program);
        command.env_clear().envs(&self.vars).current_dir(cwd);
        command
    }

    /// Run a command copied from an instrukt diagnostic using the test binary.
    pub fn recovery_shell(&self, cwd: &Path, command: &str) -> StdCommand {
        let binary = self.path("runtime-bin/instrukt");
        if !binary.exists() {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_instrukt"), binary).unwrap();
        }
        let mut shell = self.command(&tool_executable("sh"), cwd);
        shell.args(["-c", command]);
        shell
    }

    /// The built `instrukt` binary, ready to run in `cwd` with this test's
    /// environment.
    pub fn instrukt(&self, cwd: &Path) -> Command {
        github::sync_auth_clock(self.home(), self.injected("INSTRUKT_NOW"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_instrukt"));
        cmd.env_clear().envs(&self.vars).current_dir(cwd);
        cmd
    }

    /// The built `instrukt` binary as a plain process, for a test that
    /// runs it in the background and stops it part-way.
    pub fn instrukt_process(&self, cwd: &Path) -> StdCommand {
        github::sync_auth_clock(self.home(), self.injected("INSTRUKT_NOW"));
        let mut cmd = StdCommand::new(env!("CARGO_BIN_EXE_instrukt"));
        cmd.env_clear().envs(&self.vars).current_dir(cwd);
        cmd
    }

    /// `git` ready to run in `dir` with this test's environment, for a test
    /// that bounds how long git may take (`.timeout(…)`).
    pub fn git_command(&self, dir: &Path) -> Command {
        let mut cmd = Command::new(tool_executable("git"));
        cmd.env_clear().envs(&self.vars).current_dir(dir);
        cmd
    }

    /// Runs `instrukt <args>` in `cwd` and returns the assertion handle.
    pub fn run<I, S>(&self, cwd: &Path, args: I) -> Assert
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.instrukt(cwd).args(args).assert()
    }

    /// Runs `git <args>` in `dir` with this test's environment; returns
    /// trimmed stdout and fails the test on a non-zero exit.
    pub fn git<I, S>(&self, dir: &Path, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(&self.vars, dir, args)
    }

    fn base(&self) -> PathBuf {
        self.home.parent().unwrap().to_path_buf()
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let refused = self.refusals();
        assert!(
            refused.is_empty(),
            "the test tried to write outside its fixture directory ({}): {refused:#?}",
            self.root.path().display()
        );
    }
}

/// A git repository created by the harness.
pub struct Repo {
    path: PathBuf,
    vars: BTreeMap<String, String>,
}

impl Repo {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `contents` to `relative`, creating parent directories.
    pub fn write(&self, relative: &str, contents: &str) -> &Self {
        let target = self.path.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, contents).unwrap();
        self
    }

    pub fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path.join(relative)).unwrap()
    }

    /// Stages everything and commits it; returns the new commit's SHA.
    pub fn commit(&self, message: &str) -> String {
        self.git(["add", "--all"]);
        self.git(["commit", "--quiet", "--allow-empty", "-m", message]);
        self.head()
    }

    pub fn head(&self) -> String {
        self.git(["rev-parse", "HEAD"])
    }

    /// Runs `git <args>` in the repository; returns trimmed stdout.
    pub fn git<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(&self.vars, &self.path, args)
    }
}

fn run_git<I, S>(vars: &BTreeMap<String, String>, dir: &Path, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<_> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
    let output = StdCommand::new(tool_executable("git"))
        .env_clear()
        .envs(vars)
        .args(&args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

/// The live user's locations every instrukt process of a test is fenced
/// from, computed around the test's temporary directory: the whole live
/// home when that directory lies outside it, and in every case the files and
/// trees instrukt is known to write there (through a symbolic link, these may
/// resolve outside the home).
///
/// A named location that contains the temporary directory, such as a T3
/// session's under `.t3`, is replaced by its entries off the way down to
/// that directory, at each level: the record the test owns and instrukt's
/// own temporary files lie there, and the fence would refuse them. Not
/// fenced then are the directories on that way themselves, names created in
/// them after the test starts, and entries that link onto the way or into
/// the temporary directory.
fn live_fence(home: &Path, temp_dir: &Path) -> Vec<PathBuf> {
    let scratch = temp_dir.canonicalize().unwrap_or_else(|error| {
        panic!(
            "the temporary directory {} cannot be resolved: {error}",
            temp_dir.display()
        )
    });
    let named: Vec<PathBuf> = [
        ".instrukt",
        ".t3",
        ".gitconfig",
        ".config/git",
        ".claude",
        ".codex",
        ".gemini",
        ".dsh",
        "Library/LaunchAgents",
        ".config/systemd",
    ]
    .iter()
    .map(|rel| home.join(rel))
    .collect();
    let mut fenced = Vec::new();
    let real = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    if !scratch.starts_with(&real) {
        fenced.push(home.to_path_buf());
    }
    for entry in named {
        // A location that does not exist cannot contain the temporary
        // directory; it stays fenced as written.
        match entry.canonicalize() {
            Ok(location) if scratch.starts_with(&location) => {
                fenced.extend(off_the_way(&location, &scratch));
            }
            _ => fenced.push(entry),
        }
    }
    fenced
}

/// The entries of each directory from `location` down to the parent of
/// `scratch` that lead neither onto that way nor into `scratch`, each by its
/// own path. A directory on the way that cannot be listed fails setup: the
/// location is never left unfenced. An entry that vanishes or links nowhere
/// stays fenced: another program changing live data never fails a test.
fn off_the_way(location: &Path, scratch: &Path) -> Vec<PathBuf> {
    let mut fenced = Vec::new();
    let mut dir = location.to_path_buf();
    for step in scratch.strip_prefix(location).unwrap().components() {
        let unlisted = |error: std::io::Error| -> ! {
            panic!(
                "the fenced directory {} cannot be listed: {error}",
                dir.display()
            )
        };
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|error| unlisted(error)) {
            let path = entry.unwrap_or_else(|error| unlisted(error)).path();
            let onto_the_way = path
                .canonicalize()
                .is_ok_and(|leads| scratch.starts_with(&leads) || leads.starts_with(scratch));
            if !onto_the_way {
                entries.push(path);
            }
        }
        entries.sort();
        fenced.extend(entries);
        dir.push(step);
    }
    fenced
}

/// What a snapshot compares for one path: the path itself (a symlink's own
/// identity and where it points) and, followed, what it resolves to.
/// `None` parts do not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stat {
    link: Option<(u64, Option<SystemTime>, Option<PathBuf>)>,
    target: Option<(u64, Option<SystemTime>)>,
}

/// A snapshot of files and directory trees of a test's own directory,
/// compared later to show a command left them as they were. Symlinks are
/// compared both as links and through their targets; any inspection
/// failure other than a missing path fails the test.
pub struct Snapshot {
    entries: BTreeMap<PathBuf, Stat>,
    trees: Vec<PathBuf>,
}

impl Snapshot {
    /// Snapshots the given files and directory trees.
    pub fn over(files: Vec<PathBuf>, trees: Vec<PathBuf>) -> Self {
        let mut snapshot = Self {
            entries: BTreeMap::new(),
            trees,
        };
        for file in files {
            let state = stat(&file);
            snapshot.entries.insert(file, state);
        }
        for (path, state) in snapshot.walk_trees() {
            snapshot.entries.insert(path, state);
        }
        snapshot
    }

    /// Paths that appeared, disappeared or changed since the snapshot.
    pub fn changed(&self) -> Vec<PathBuf> {
        let mut now: BTreeMap<PathBuf, Stat> = self
            .entries
            .keys()
            .map(|path| (path.clone(), stat(path)))
            .collect();
        for (path, state) in self.walk_trees() {
            now.insert(path, state);
        }
        let absent = Stat {
            link: None,
            target: None,
        };
        now.into_iter()
            .filter(|(path, state)| self.entries.get(path).unwrap_or(&absent) != state)
            .map(|(path, _)| path)
            .collect()
    }

    fn walk_trees(&self) -> Vec<(PathBuf, Stat)> {
        let mut found = Vec::new();
        for tree in &self.trees {
            if stat(tree).link.is_none() {
                continue;
            }
            for entry in walkdir::WalkDir::new(tree).follow_links(true) {
                match entry {
                    Ok(entry) => found.push((entry.path().to_path_buf(), stat(entry.path()))),
                    Err(err) if is_not_found(err.io_error()) => {}
                    Err(err) => panic!("snapshot cannot inspect {}: {err}", tree.display()),
                }
            }
        }
        found
    }
}

fn stat(path: &Path) -> Stat {
    let link = match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            let points_to = if meta.file_type().is_symlink() {
                Some(std::fs::read_link(path).unwrap_or_else(|err| {
                    panic!("snapshot cannot read link {}: {err}", path.display())
                }))
            } else {
                None
            };
            Some((meta.len(), meta.modified().ok(), points_to))
        }
        Err(err) if is_not_found(Some(&err)) => None,
        Err(err) => panic!("snapshot cannot inspect {}: {err}", path.display()),
    };
    let target = match std::fs::metadata(path) {
        Ok(meta) => Some((meta.len(), meta.modified().ok())),
        Err(err) if is_not_found(Some(&err)) => None,
        Err(err) => panic!("snapshot cannot inspect {}: {err}", path.display()),
    };
    Stat { link, target }
}

fn is_not_found(err: Option<&std::io::Error>) -> bool {
    err.is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound)
}

/// Edit a complete current agents fixture, inheriting the shipped native
/// shapes. Every shipped role the patch does not name resolves to the
/// fixture's `default` entry; one it names takes its omitted class and grants
/// from its shipped entry and its omitted order and tier from `default`.
pub fn agent_profiles(env: &TestEnv, patch: &str) {
    fn merge(value: &mut serde_yaml_ng::Value, patch: serde_yaml_ng::Value) {
        if let (Some(target), Some(source)) = (value.as_mapping_mut(), patch.as_mapping()) {
            for (key, child) in source {
                merge(
                    target
                        .entry(key.clone())
                        .or_insert(serde_yaml_ng::Value::Null),
                    child.clone(),
                );
            }
        } else {
            *value = patch;
        }
    }
    let mut agents: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(instrukt_core::config::SHIPPED_AGENTS).unwrap();
    let shipped = std::mem::replace(
        &mut agents["roles"],
        serde_yaml_ng::Value::Mapping(Default::default()),
    );
    merge(&mut agents, serde_yaml_ng::from_str(patch).unwrap());
    let default = agents["default"].clone();
    for (name, shipped) in shipped.as_mapping().unwrap() {
        let named = agents["roles"].get(name).cloned();
        let capabilities = if named.is_some() { shipped } else { &default };
        let mut entry = named
            .and_then(|entry| entry.as_mapping().cloned())
            .unwrap_or_default();
        for (field, source) in [
            ("class", capabilities),
            ("can", capabilities),
            ("order", &default),
            ("tier", &default),
        ] {
            if !entry.contains_key(field) {
                entry.insert(field.into(), source[field].clone());
            }
        }
        entry
            .entry("github_app".into())
            .or_insert(serde_yaml_ng::Value::Null);
        agents["roles"][name] = serde_yaml_ng::Value::Mapping(entry);
    }
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    std::fs::write(
        env.instrukt_home().join("agents.yaml"),
        serde_yaml_ng::to_string(&agents).unwrap(),
    )
    .unwrap();
    let Ok(profiles) =
        instrukt_core::config::Agents::parse(&serde_yaml_ng::to_string(&agents).unwrap())
    else {
        return;
    };
    t3_readiness::update(env, |snapshot| {
        let providers = snapshot["providers"].as_array_mut().unwrap();
        for profile in profiles.profiles.values() {
            if providers
                .iter()
                .any(|provider| provider["instanceId"] == profile.instance.as_str())
            {
                continue;
            }
            let kind = serde_json::to_value(profile.kind).unwrap();
            let driver = if kind == "claude" {
                "claudeAgent"
            } else {
                kind.as_str().unwrap()
            };
            let mut provider = t3_readiness::provider(profile.instance.as_str(), driver, true);
            let models = [
                &profile.light,
                &profile.standard,
                &profile.strong,
                &profile.premium,
            ]
            .into_iter()
            .flatten()
            .filter_map(|entry| entry["model"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
            provider["models"] = serde_json::json!(
                models
                    .into_iter()
                    .map(|model| serde_json::json!({"slug":model}))
                    .collect::<Vec<_>>()
            );
            providers.push(provider);
        }
    });
}

/// Locate a declared prerequisite; subprocesses never search ambient PATH.
pub fn tool_executable(program: &str) -> PathBuf {
    let ambient = std::env::var_os("PATH").unwrap();
    let system = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];
    // Package-installed prerequisites may live outside the base system.
    let directories = if matches!(program, "git" | "python3" | "redis-server" | "uv") {
        std::env::split_paths(&ambient).collect::<Vec<_>>()
    } else {
        system.into_iter().map(PathBuf::from).collect::<Vec<_>>()
    };
    directories
        .into_iter()
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("required test tool {program} is not installed"))
}
