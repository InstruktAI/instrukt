//! Repository commands: `instrukt init`, `instrukt hooks …`, `instrukt repo commit|stamp`.
//!
//! Two hidden entries serve git rather than people: `instrukt hooks worker`,
//! the detached process a hook run starts, and `instrukt repo docs-filter`,
//! the docs reference filter git runs on checkout and commit.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::config::Env;
use instrukt_core::ids::{ProjectSlug, TodoSlug};
use instrukt_core::repository::{
    self, CommitRequest, FilterDirection, GENERATED_FILE, GenerationOutcome, Hook, HookChange,
    HookInvocation, HookRunRecord, HooksDir, HooksReport, InitPart, MASTER_FILE, PartOutcome,
    SectionChange, SectionStatus, Stamp, SyncOutcome,
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
        if *part == InitPart::HookSections {
            for line in hooks_report_lines(&report.hooks) {
                ctx.out.line(&format!("  {line}"))?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn part_label(part: InitPart) -> String {
    match part {
        InitPart::ProjectLayer => "project layer (instrukt.yaml)".to_owned(),
        InitPart::DocsFolders => "docs folders (docs/global/, docs/local/)".to_owned(),
        InitPart::Master => format!("master ({MASTER_FILE})"),
        InitPart::DocsFilter => format!(
            "docs reference filter (git config filter.{})",
            repository::FILTER_NAME
        ),
        InitPart::FilterPatterns => "filter patterns (.gitattributes)".to_owned(),
        InitPart::GeneratedIgnore => format!("generated {GENERATED_FILE} ignore (.gitignore)"),
        InitPart::HookSections => "hook sections".to_owned(),
    }
}

fn part_outcome(part: InitPart, outcome: PartOutcome) -> String {
    match outcome {
        PartOutcome::Created => "created".to_owned(),
        PartOutcome::Completed => "completed".to_owned(),
        PartOutcome::AlreadyPresent => "already present".to_owned(),
        PartOutcome::Withheld => match part {
            InitPart::Master => format!(
                "not seeded: {GENERATED_FILE} is hand-written; move its content into a docs/local snippet, remove {GENERATED_FILE} (it becomes generated) and run `instrukt init` again"
            ),
            _ => "not written".to_owned(),
        },
    }
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
    repository::run_hook(&ctx.home, &ctx.cwd, &core_wiring(ctx), &invocation)?;
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
    /// exemption with its reason, as a git note on each named commit.
    Stamp(StampArgs),
    /// The docs reference filter git runs on checkout and commit.
    #[command(hide = true)]
    DocsFilter(DocsFilterArgs),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Direction {
    Smudge,
    Clean,
}

#[derive(Debug, clap::Args)]
struct DocsFilterArgs {
    direction: Direction,
}

pub fn run_repo(args: RepoArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        RepoCommand::Commit(args) => commit(args, ctx),
        RepoCommand::Stamp(args) => stamp(args, ctx),
        RepoCommand::DocsFilter(args) => docs_filter(&args, ctx),
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
        &core_wiring(ctx),
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
    let commits = repository::stamp(&ctx.cwd, &args.commits, &stamp)?;
    for commit in commits {
        ctx.out.line(&format!("{commit}: {}", stamp.verdict()))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn docs_filter(args: &DocsFilterArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .context("cannot read the blob from standard input")?;
    let direction = match args.direction {
        Direction::Smudge => FilterDirection::Smudge,
        Direction::Clean => FilterDirection::Clean,
    };
    let output = repository::apply_docs_filter(&input, &ctx.home, direction);
    ctx.out.bytes(&output)?;
    Ok(ExitCode::SUCCESS)
}

fn read_text(path: &Path, what: &str) -> anyhow::Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {what} from {}", path.display()))
}
