//! Repository commands: `instrukt init`, `instrukt hooks …`, `instrukt repo commit|stamp`.
//!
//! One hidden entry serves git rather than people: `instrukt hooks worker`,
//! the detached process a hook run starts.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::config::Env;
use instrukt_core::ids::{ProjectSlug, TodoSlug};
use instrukt_core::publication::{
    ClaimStatus, ClaimsPreview, EditorChange, EditorChangeKind, GlobalFiles, SCHEMAS_DIR,
};
use instrukt_core::repository::{
    self, CommitRequest, GENERATED_FILE, GenerationOutcome, Hook, HookChange, HookInvocation,
    HookRunRecord, HooksDir, HooksReport, InitPart, KEPT_INSTRUCTIONS, KeptInstructions,
    MASTER_FILE, NotWritten, NotesPush, PROJECT_DOCS, PartOutcome, PendingReason, SectionChange,
    SectionStatus, Stamp, StampTime, SyncOutcome,
};

use crate::context::Ctx;
use crate::wiring::core_wiring;

#[derive(Debug, clap::Args)]
pub struct InitArgs {
    /// The project slug, instead of the one derived from the primary
    /// worktree's folder name.
    #[arg(long)]
    slug: Option<String>,
}

pub fn run_init(args: InitArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let slug = args.slug.as_deref().map(ProjectSlug::parse).transpose()?;
    let report = repository::init(&ctx.home, &ctx.cwd, slug.as_ref())?;
    for (part, outcome) in &report.parts {
        ctx.out.line(&format!(
            "{}: {}",
            part_label(*part),
            part_outcome(*part, *outcome)
        ))?;
        let details = match part {
            InitPart::EditorSchemas => editor_lines(&report.editor),
            InitPart::HookSections => hooks_report_lines(&report.hooks),
            InitPart::KeptInstructions => report.kept.map(kept_lines).unwrap_or_default(),
            _ => Vec::new(),
        };
        for line in details {
            ctx.out.line(&format!("  {line}"))?;
        }
    }
    if let Some(claims) = &report.claims {
        for line in claims_lines(claims) {
            ctx.out.line(&line)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn part_label(part: InitPart) -> String {
    match part {
        InitPart::ProjectLayer => "project layer (instrukt.yaml)".to_owned(),
        InitPart::ProjectDocs => format!("project docs ({PROJECT_DOCS}/)"),
        InitPart::ProjectIndex => format!("project index ({PROJECT_DOCS}/index.yaml)"),
        InitPart::ProjectSchema => format!("project schema ({PROJECT_DOCS}/schema.yaml)"),
        InitPart::Master => format!("master ({MASTER_FILE})"),
        InitPart::KeptInstructions => format!("kept instructions ({KEPT_INSTRUCTIONS})"),
        InitPart::EditorSchemas => format!("editor schemas ({SCHEMAS_DIR}/)"),
        InitPart::Ignores => format!("ignores of {GENERATED_FILE} and .instrukt/ (.gitignore)"),
        InitPart::HookSections => "hook sections".to_owned(),
    }
}

fn part_outcome(part: InitPart, outcome: PartOutcome) -> String {
    match (part, outcome) {
        (InitPart::ProjectSchema, PartOutcome::Created) => {
            "created: given the sdlc schema; change it as you see fit".to_owned()
        }
        (InitPart::KeptInstructions, PartOutcome::Created) => format!(
            "created: the hand-written {GENERATED_FILE}, byte for byte, kept until its instructions are placed in {PROJECT_DOCS}/"
        ),
        (_, PartOutcome::Created) => "created".to_owned(),
        (_, PartOutcome::Completed) => "completed".to_owned(),
        (_, PartOutcome::AlreadyPresent) => "already present".to_owned(),
        (_, PartOutcome::NotWritten(NotWritten::SchemaDomainNotPublished)) => {
            "not written: sdlc is not published on this computer, so the project follows the root rules only".to_owned()
        }
        (_, PartOutcome::NotWritten(NotWritten::SchemaDomainHasNoSchema)) => {
            "not written: the published sdlc has no schema.yaml, so the project follows the root rules only".to_owned()
        }
    }
}

/// What a refresh of the editor schemas changed, one line per file.
pub(crate) fn editor_lines(changes: &[EditorChange]) -> Vec<String> {
    changes
        .iter()
        .map(|change| {
            let what = match change.kind {
                EditorChangeKind::SchemaWritten => "schema written",
                EditorChangeKind::ModelineInserted => "modeline inserted",
                EditorChangeKind::ModelineReplaced => "modeline updated",
                EditorChangeKind::Linked => "not written: a symbolic link",
            };
            format!("{}: {what}", change.path.display())
        })
        .collect()
}

fn kept_lines(kept: KeptInstructions) -> Vec<String> {
    let mut lines = vec![format!(
        "{GENERATED_FILE} is generated from now on: the first sync renders it from {MASTER_FILE} and {PROJECT_DOCS}/index.yaml"
    )];
    if kept.tracked {
        lines.push(format!(
            "stop tracking it in the commit that adds the master: git rm --cached {GENERATED_FILE}"
        ));
    }
    lines
}

/// The claims report: what the repository's first sync will claim.
fn claims_lines(claims: &ClaimsPreview) -> Vec<String> {
    let mut lines =
        vec!["claims of the first sync (nothing is published until it runs):".to_owned()];
    for (position, status) in &claims.positions {
        let what = match status {
            ClaimStatus::Free => "free; the sync claims it".to_owned(),
            ClaimStatus::Ours => "already owned by this project".to_owned(),
            ClaimStatus::OwnedBy(owner) => {
                format!("owned by project `{owner}`; the sync would be refused")
            }
        };
        lines.push(format!("  {position}: {what}"));
    }
    for dir in &claims.reserved {
        lines.push(format!(
            "  {dir}: refused: a reserved name; no domain may be called `general`, `project` or `third-party`"
        ));
    }
    match &claims.global_files {
        GlobalFiles::NotHeld => lines.push(
            "  global agent files: not written; the repository holds no docs/general/".to_owned(),
        ),
        GlobalFiles::OwnedElsewhere(owner) => lines.push(format!(
            "  global agent files: not written; docs/general/ is owned by project `{owner}`"
        )),
        GlobalFiles::Rendered { agents, foreign } => {
            if agents.is_empty() {
                lines.push(
                    "  global agent files: none; no supported agent CLI is installed on this computer"
                        .to_owned(),
                );
            }
            for (agent, path) in agents {
                lines.push(format!(
                    "  global agent file of {agent}: the sync writes {}",
                    path.display()
                ));
            }
            for path in foreign {
                lines.push(format!(
                    "  {}: not written by instrukt; it stops the sync until its content moves into docs or the file moves out of the way",
                    path.display()
                ));
            }
        }
    }
    lines
}

#[derive(Debug, clap::Args)]
pub struct HooksArgs {
    #[command(subcommand)]
    command: HooksCommand,
}

#[derive(Debug, clap::Subcommand)]
enum HooksCommand {
    /// Write instrukt's section into each hook file it uses, in the hooks
    /// directory `core.hooksPath` names or the repository's own. Touches no
    /// line outside the section.
    Install,
    /// Remove instrukt's section from every hook file; every other line and
    /// the files themselves remain.
    Remove,
    /// Per hook: the hooks directory, whether the section is current,
    /// outdated or absent (with the line to give a hook manager), and the
    /// last run's outcome.
    Status,
    /// What a section calls: start what acts on the hook point and return
    /// without waiting. Never fails the git operation.
    Run(RunArgs),
    /// The detached process a hook run starts.
    #[command(hide = true)]
    Worker(WorkerArgs),
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    /// The hook point: post-commit, post-merge, post-rewrite, post-checkout
    /// or pre-push.
    hook: String,
    /// The arguments git passed the hook.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Debug, clap::Args)]
struct WorkerArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    job: Vec<String>,
}

pub fn run_hooks(args: HooksArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        HooksCommand::Install => {
            let report = repository::install_hooks(&ctx.home, &ctx.cwd)?;
            for line in hooks_report_lines(&report) {
                ctx.out.line(&line)?;
            }
        }
        HooksCommand::Remove => {
            let report = repository::remove_hooks(&ctx.cwd)?;
            ctx.out.line(&hooks_dir_line(&report.dir))?;
            for hook in &report.hooks {
                let what = match hook.change {
                    SectionChange::Removed => "removed",
                    _ => "absent",
                };
                ctx.out.line(&format!("{}: {what}", hook.hook))?;
            }
        }
        HooksCommand::Status => status(ctx)?,
        HooksCommand::Run(args) => return run_hook(&args, ctx),
        HooksCommand::Worker(args) => {
            let exe = std::env::current_exe().context("cannot locate the instrukt executable")?;
            repository::run_hook_worker(&ctx.home, &ctx.cwd, &exe, &args.job)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// A hook run never fails git: once the hook is known, every failure is
/// reported on stderr and the run still exits zero.
fn run_hook(args: &RunArgs, ctx: &Ctx) -> anyhow::Result<ExitCode> {
    let hook = Hook::parse(&args.hook).with_context(|| {
        let known: Vec<&str> = Hook::ALL.iter().map(|hook| hook.name()).collect();
        format!(
            "`{}` is not a hook instrukt uses ({})",
            args.hook,
            known.join(", ")
        )
    })?;
    if let Err(err) = start_hook_run(hook, args, ctx) {
        tracing::warn!(event = "hook_run_failed", hook = %hook, error = %format!("{err:#}"));
    }
    Ok(ExitCode::SUCCESS)
}

fn start_hook_run(hook: Hook, args: &RunArgs, ctx: &Ctx) -> anyhow::Result<()> {
    let mut stdin = String::new();
    if hook == Hook::PrePush {
        std::io::stdin()
            .read_to_string(&mut stdin)
            .context("cannot read the pushed refs from standard input")?;
    }
    let exe = std::env::current_exe().context("cannot locate the instrukt executable")?;
    let env = Env::from_process();
    let invocation = HookInvocation {
        hook,
        args: &args.args,
        stdin: &stdin,
        env: &env,
        exe: &exe,
    };
    repository::run_hook(&ctx.home, &ctx.cwd, &core_wiring(ctx)?, &invocation)?;
    Ok(())
}

fn status(ctx: &mut Ctx) -> anyhow::Result<()> {
    let status = repository::hooks_status(&ctx.home, &ctx.cwd)?;
    ctx.out.line(&hooks_dir_line(&status.dir))?;
    for hook in &status.hooks {
        let section = match &hook.section {
            SectionStatus::Current => "current".to_owned(),
            SectionStatus::Outdated => "outdated; `instrukt hooks install` updates it".to_owned(),
            SectionStatus::Absent => "absent".to_owned(),
            SectionStatus::Damaged { reason } => format!("damaged: {reason}"),
        };
        ctx.out.line(&format!("{}: {section}", hook.hook))?;
        if hook.section == SectionStatus::Absent {
            ctx.out.line(&format!(
                "  add this line to the tool that manages {}: {}",
                hook.file.display(),
                repository::hook_run_line(hook.hook)
            ))?;
        }
        if hook.not_executable {
            ctx.out
                .line("  the file is not executable, so git does not run it")?;
        }
        let last_run = match (&status.project, &hook.last_run) {
            (None, _) => "not recorded: the project layer declares no project slug".to_owned(),
            (Some(_), None) => "none".to_owned(),
            (Some(_), Some(record)) => last_run_text(record),
        };
        ctx.out.line(&format!("  last run: {last_run}"))?;
    }
    Ok(())
}

fn last_run_text(record: &HookRunRecord) -> String {
    let mut parts = Vec::new();
    match &record.sync {
        None => {}
        Some(SyncOutcome::Running) => parts.push("sync running".to_owned()),
        Some(SyncOutcome::NotStarted { error }) => parts.push(format!("sync not started: {error}")),
        Some(SyncOutcome::Finished {
            exit_code: Some(0), ..
        }) => parts.push("sync succeeded".to_owned()),
        Some(SyncOutcome::Finished {
            exit_code: Some(code),
            message,
            ..
        }) => parts.push(match message {
            Some(message) => format!("sync exited {code}: {message}"),
            None => format!("sync exited {code}"),
        }),
        Some(SyncOutcome::Finished {
            exit_code: None, ..
        }) => parts.push("sync was ended by a signal".to_owned()),
        Some(SyncOutcome::TimedOut { after_secs, .. }) => {
            parts.push(format!("sync ended after running {after_secs}s"));
        }
    }
    match &record.graph_generation {
        None => {}
        Some(GenerationOutcome::Started { sha }) => {
            parts.push(format!("graph generation started for {sha}"));
        }
        Some(GenerationOutcome::Failed { sha, error }) => {
            parts.push(format!("graph generation for {sha} failed: {error}"));
        }
    }
    if parts.is_empty() {
        "nothing started".to_owned()
    } else {
        parts.join("; ")
    }
}

fn hooks_dir_line(dir: &HooksDir) -> String {
    if dir.from_hooks_path {
        format!(
            "hooks directory: {} (set by core.hooksPath)",
            dir.path.display()
        )
    } else {
        format!("hooks directory: {}", dir.path.display())
    }
}

fn hooks_report_lines(report: &HooksReport) -> Vec<String> {
    let mut lines = vec![hooks_dir_line(&report.dir)];
    lines.extend(report.hooks.iter().map(hook_change_line));
    lines
}

fn hook_change_line(change: &HookChange) -> String {
    let mut text = match (&change.change, change.before_exec_line) {
        (SectionChange::Written, Some(line)) => {
            format!("written before the line running exec (line {line})")
        }
        (SectionChange::Written, None) => "written".to_owned(),
        (SectionChange::Updated, _) => "updated".to_owned(),
        (SectionChange::AlreadyCurrent, _) => "already current".to_owned(),
        (SectionChange::Removed, _) => "removed (blueprint is off)".to_owned(),
        (SectionChange::Absent, _) => "absent".to_owned(),
        (SectionChange::NotShell { interpreter }, _) => format!(
            "not written: {} runs with {interpreter}; add this line to it: {}",
            change.file.display(),
            repository::hook_run_line(change.hook)
        ),
    };
    if change.not_executable {
        text.push_str("; the file is not executable, so git does not run it (left as it is)");
    }
    format!("{}: {text}", change.hook)
}

#[derive(Debug, clap::Args)]
pub struct RepoArgs {
    #[command(subcommand)]
    command: RepoCommand,
}

#[derive(Debug, clap::Subcommand)]
enum RepoCommand {
    /// Commit as the acting agent: its configured git author, a `Session:`
    /// trailer when a runtime's session variable is set, `Human-Involved:`
    /// always, and `Todo:` or `Planned-Todo:` with --todo. Outside an agent
    /// session the commit is the git user's own. Prints the commit's id.
    Commit(CommitArgs),
    /// Record a commit-review verdict with the report's digest, or an
    /// exemption with its reason, as a git note on each named commit, and
    /// push the notes to the remote the default branch tracks.
    Stamp(StampArgs),
}

#[derive(Debug, clap::Args)]
struct CommitArgs {
    /// The message: a subject and an explanatory body. It may not hold the
    /// trailers instrukt writes.
    #[arg(short, long, required_unless_present = "file", conflicts_with = "file")]
    message: Option<String>,
    /// Read the message from this file.
    #[arg(short = 'F', long)]
    file: Option<PathBuf>,
    /// Whether a human was involved; `false` when omitted.
    #[arg(
        long,
        action = clap::ArgAction::Set,
        value_parser = clap::builder::BoolValueParser::new(),
        default_value_t = false,
        value_name = "true|false"
    )]
    human_involved: bool,
    /// The work item this commit belongs to.
    #[arg(long, value_name = "SLUG")]
    todo: Option<String>,
    /// Commit exactly these paths as they are in the working tree, new
    /// files included; everything else in the index stays as it was.
    #[arg(long, num_args = 1.., value_name = "PATH")]
    only: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Verdict {
    Approve,
    #[value(name = "needs_work")]
    NeedsWork,
    Exempt,
}

#[derive(Debug, clap::Args)]
struct StampArgs {
    /// The commits to stamp.
    #[arg(required = true, value_name = "SHA")]
    commits: Vec<String>,
    #[arg(long, value_enum)]
    verdict: Verdict,
    /// The complete review report (approve, needs_work).
    #[arg(long)]
    report: Option<PathBuf>,
    /// Why the commit needs no review (exempt).
    #[arg(long)]
    reason: Option<String>,
}

pub fn run_repo(args: RepoArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        RepoCommand::Commit(args) => commit(args, ctx),
        RepoCommand::Stamp(args) => stamp(args, ctx),
    }
}

fn commit(args: CommitArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let message = match (args.message, &args.file) {
        (Some(message), _) => message,
        (None, Some(file)) => read_text(file, "the message")?,
        (None, None) => anyhow::bail!("supply the message with --message or --file"),
    };
    let request = CommitRequest {
        message,
        human_involved: args.human_involved,
        todo: args.todo.as_deref().map(TodoSlug::parse).transpose()?,
        only: args.only,
    };
    let sha = repository::commit(
        &ctx.home,
        &ctx.cwd,
        &Env::from_process(),
        &core_wiring(ctx)?,
        &request,
    )?;
    ctx.out.line(sha.as_str())?;
    Ok(ExitCode::SUCCESS)
}

fn stamp(args: StampArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let stamp = match (args.verdict, &args.report, &args.reason) {
        (Verdict::Exempt, None, Some(reason)) => Stamp::exempt(reason)?,
        (Verdict::Exempt, _, _) => {
            anyhow::bail!("--verdict exempt takes --reason <text> and no --report")
        }
        (verdict, Some(report), None) => {
            let path = ctx.cwd.join(report);
            let bytes = std::fs::read(&path)
                .with_context(|| format!("cannot read the report {}", path.display()))?;
            Stamp::review(verdict == Verdict::Approve, &bytes)
        }
        (_, _, _) => {
            anyhow::bail!("--verdict approve|needs_work takes --report <file> and no --reason")
        }
    };
    // The clock is read here, at the composition root, and handed down.
    let at = StampTime::from_system(std::time::SystemTime::from(ctx.clock.now()))?;
    let outcome = repository::stamp(&ctx.cwd, &args.commits, &stamp, at)?;
    for commit in &outcome.commits {
        ctx.out
            .line(&format!("{commit}: {}", stamp.verdict().name()))?;
    }
    let push = match &outcome.push {
        NotesPush::Pushed { remote } => format!("review notes pushed to {remote}"),
        NotesPush::Pending { remote, reason } => {
            let why = match reason {
                PendingReason::Unreachable => "the remote could not be reached",
                PendingReason::StillRejected => "the remote kept rejecting it after merging",
            };
            format!(
                "review notes push to {remote} pending: {why}; the stamp is recorded locally and the next stamp that reaches {remote} delivers every note"
            )
        }
        NotesPush::NoRemote { branch } => {
            format!("review notes kept locally: {branch} tracks no remote")
        }
    };
    ctx.out.line(&push)?;
    Ok(ExitCode::SUCCESS)
}

fn read_text(path: &Path, what: &str) -> anyhow::Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {what} from {}", path.display()))
}
