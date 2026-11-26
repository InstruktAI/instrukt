//! Repository commands: `instrukt init`, `instrukt hooks …`, `instrukt repo commit|reclaim`.
//!
//! One hidden entry serves git rather than people: `instrukt hooks worker`,
//! the detached process a hook run starts.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::ids::{ProjectSlug, TodoSlug};
use instrukt_core::publication::{
    ClaimStatus, ClaimsPreview, EditorChange, EditorChangeKind, GlobalFiles, SCHEMAS_DIR,
};
use instrukt_core::repository::{
    self, CommitRequest, GENERATED_FILE, GenerationOutcome, Hook, HookChange, HookInvocation,
    HookRunRecord, HooksDir, HooksReport, InitPart, KEPT_INSTRUCTIONS, KeptInstructions,
    NotWritten, PROJECT_DOCS, PartOutcome, RepositoryError, ReviewRouting, SectionChange,
    SectionStatus, SyncOutcome,
};

use crate::context::Ctx;
use crate::wiring::core_wiring;

#[derive(Debug, clap::Args)]
pub struct InitArgs {
    /// The project slug, instead of the one derived from the primary
    /// worktree's folder name.
    #[arg(long, conflicts_with = "company")]
    slug: Option<String>,
    /// Add the company of this key to the computer instead: its repository
    /// `company-<key>` in the organisation `github.org` names, created with
    /// the company skeleton when it does not exist, checked out as
    /// ~/.instrukt/companies/<key>/ and made ready. The working directory is
    /// not read.
    #[arg(long, value_name = "KEY")]
    company: Option<ProjectSlug>,
    /// The name people write the company by; needed only when its
    /// repository is created.
    #[arg(long, value_name = "NAME", requires = "company")]
    name: Option<instrukt_core::publication::CompanyName>,
    /// The company's GitHub administrator, a person's GitHub account; needed
    /// only when its repository is created.
    #[arg(long, value_name = "LOGIN", requires = "company")]
    admin: Option<instrukt_core::ids::GitHubLogin>,
    /// Whether a human was involved, declared for the root commit of a
    /// created repository; `false` when omitted.
    #[arg(long, value_name = "true|false", requires = "company")]
    human_involved: Option<bool>,
}

pub fn run_init(args: InitArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    if let Some(key) = args.company.clone() {
        return run_init_company(args, key, ctx);
    }
    let slug = args.slug.as_deref().map(ProjectSlug::parse).transpose()?;
    let wiring = crate::wiring::publication_wiring(ctx)?;
    let report = repository::init(
        &ctx.home,
        &ctx.cwd,
        slug.as_ref(),
        wiring.configured_runtimes.as_deref(),
    )?;
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

/// `instrukt init --company <key>`: adds the company to the computer. Its
/// wiring reads nothing of the working directory: the GitHub boundary and
/// the runtime adapter only.
fn run_init_company(args: InitArgs, key: ProjectSlug, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let wiring = instrukt_core::interfaces::CoreWiring {
        caller_identity: crate::wiring::caller_lookup(ctx)?,
        github_actors: Some(ctx.github.clone()),
        session_role: Some(std::sync::Arc::new(crate::wiring::RecordedRole(
            ctx.home.clone(),
        ))),
        flavor_schema: Some(std::sync::Arc::new(
            instrukt_todos::workflow::WorkflowSchema,
        )),
        configured_runtimes: crate::wiring::configured_runtimes(ctx)?,
        ..Default::default()
    };
    let env = crate::wiring::environment(ctx)?;
    let request = repository::CompanyRequest {
        key,
        name: args.name,
        admin: args.admin,
        human_involved: args.human_involved,
    };
    let ports = repository::CompanyPorts {
        accounts: ctx.github.as_ref(),
        repositories: ctx.github.as_ref(),
        env: &env,
        wiring: &wiring,
        now: ctx.clock.now(),
    };
    let report = repository::adopt_company(&ctx.home, &request, &ports)?;
    let how = match report.origin {
        repository::CompanyOrigin::Created if report.resumed => {
            "created, completing the creation an earlier run recorded"
        }
        repository::CompanyOrigin::Created => "created with the company skeleton",
        repository::CompanyOrigin::ByContract => {
            "checked out by contract: it already existed, and nothing on GitHub changed"
        }
    };
    ctx.out.line(&format!(
        "repository {}/{}: {how}",
        report.organisation, report.repository
    ))?;
    ctx.out.line(&format!("home: {}", report.home.display()))?;
    for flag in &report.unused {
        ctx.out.line(&format!("{flag}: not used"))?;
    }
    ctx.out.line(&format!(
        "settings document ({}): {}",
        instrukt_core::config::COMPANY_DOCUMENT,
        if report.settings_written {
            "created"
        } else {
            "already present"
        }
    ))?;
    ctx.out.line("hook sections:")?;
    for line in hooks_report_lines(&report.hooks) {
        ctx.out.line(&format!("  {line}"))?;
    }
    let published = match &report.sync {
        repository::CompanySync::Published => "published".to_owned(),
        repository::CompanySync::Unchanged => "already published".to_owned(),
        repository::CompanySync::NotPublished(why) => {
            format!("not published: {why}; the home stays, and `instrukt sync` in it publishes it")
        }
        repository::CompanySync::NoProjectLayer => format!(
            "does not apply: the repository holds no project layer ({}), so neither publication nor automatic review applies to the home; once its default branch holds one declaring the repository a company's, `instrukt sync` in the home publishes it",
            instrukt_core::config::CONFIG_FILE_NAME
        ),
    };
    ctx.out.line(&format!("publication: {published}"))?;
    if report.sync == repository::CompanySync::NoProjectLayer {
        return Ok(ExitCode::SUCCESS);
    }
    // What keeps automatic review from the home is reported, never failed.
    let layer = instrukt_core::config::Layer::Project(report.home.clone());
    let inspection = ctx.home.for_inspection();
    let mut review: Vec<instrukt_core::config::Finding> =
        instrukt_core::config::validate(&inspection, &layer)?
            .into_iter()
            .filter(|finding| finding.key == instrukt_core::config::REVIEW_AUTOMATIC_KEY)
            .collect();
    if review.is_empty()
        && let Some(repository) = instrukt_core::config::review_target(&inspection, &layer)?
    {
        review.extend(crate::commands::config::review_readiness_finding(
            ctx,
            &layer,
            &repository,
        )?);
    }
    for finding in &review {
        ctx.out
            .line(&format!("automatic review withheld here: {finding}"))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn part_label(part: InitPart) -> String {
    match part {
        InitPart::ProjectLayer => "project layer (instrukt.yaml)".to_owned(),
        InitPart::ProjectDocs => format!("project docs ({PROJECT_DOCS}/)"),
        InitPart::ProjectIndex => format!("project index ({PROJECT_DOCS}/index.yaml)"),
        InitPart::ProjectSchema => format!("project schema ({PROJECT_DOCS}/schema.yaml)"),
        InitPart::KeptInstructions => format!("kept instructions ({KEPT_INSTRUCTIONS})"),
        InitPart::EditorSchemas => format!("editor schemas ({SCHEMAS_DIR}/)"),
        InitPart::Ignores => "ignores of .instrukt/ (.gitignore)".to_owned(),
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
                EditorChangeKind::LinkReplaced => {
                    "a symbolic link, replaced; what it pointed to is left as it is"
                }
            };
            format!("{}: {what}", change.path.display())
        })
        .collect()
}

fn kept_lines(_kept: KeptInstructions) -> Vec<String> {
    vec![format!(
        "{GENERATED_FILE} is generated from now on: the next commit through instrukt's hook writes its rendering from {PROJECT_DOCS}/index.yaml over it"
    )]
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
        GlobalFiles::Rendered { files, targets } => {
            if files.is_empty() && targets.is_empty() {
                lines.push(
                    "  global agent files: none; no agent runtime is configured"
                        .to_owned(),
                );
            }
            for target in targets {
                if target.reason.is_some() || !target.assurance.is_empty() {
                    lines.push(format!("  {}", super::sync::instruction_line(target)));
                }
            }
            for file in files {
                lines.push(format!(
                    "  global agent file of {} ({}): the sync writes {}",
                    file.runtime,
                    file.instances.join(", "),
                    file.path.display()
                ));
            }
        }
        GlobalFiles::Unreachable { runtime, cause } => lines.push(format!(
            "  global agent files: not known; runtime `{runtime}` cannot be asked which agent runtimes are configured ({cause}); the sync writes them once it can"
        )),
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
    /// What a section calls: pre-commit stages the repository's generated
    /// root `AGENTS.md`, the removal of renderings left below it, and a
    /// project layer converted to the current schema
    /// into the commit; post-commit keeps the index in step with what was
    /// committed; checkout in the primary checkout starts publication
    /// detached; other publication triggers run detached. Never fails the git
    /// operation.
    Run(RunArgs),
    /// The detached process a hook run starts.
    #[command(hide = true)]
    Worker(WorkerArgs),
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    /// The hook point: pre-commit, post-commit, post-merge, post-rewrite,
    /// post-checkout or pre-push.
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
        if matches!(hook, Hook::PreCommit | Hook::PostCheckout) {
            eprintln!("instrukt {hook}: {}", crate::output::error_line(&err));
        }
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
    let env = crate::wiring::environment(ctx)?;
    let invocation = HookInvocation {
        hook,
        args: &args.args,
        stdin: &stdin,
        env: &env,
        exe: &exe,
    };
    // The pre-commit run reads no configuration: module wiring is built for
    // the post-hooks that use it. A configuration it cannot read leaves the
    // optional modules unwired, and the run is still recorded.
    let wiring = if hook == Hook::PreCommit {
        instrukt_core::interfaces::CoreWiring::default()
    } else {
        core_wiring(ctx).unwrap_or_else(|error| {
            tracing::warn!(event = "hook_wiring_unavailable", hook = %hook, error = %format!("{error:#}"));
            instrukt_core::interfaces::CoreWiring::default()
        })
    };
    repository::run_hook(&ctx.home, &ctx.cwd, &wiring, &invocation)?;
    Ok(())
}

fn status(ctx: &mut Ctx) -> anyhow::Result<()> {
    let status = repository::hooks_status(&ctx.home, &ctx.cwd)?;
    ctx.out.line(&hooks_dir_line(&status.dir))?;
    ctx.out.line(&format!(
        "runs recorded for the repository at {}",
        status.repository.display()
    ))?;
    if let Some(reason) = &status.blueprint_unknown {
        ctx.out.line(&format!(
            "pre-push: not shown; whether the blueprint module is on cannot be told: {reason}"
        ))?;
    }
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
        let last_run = hook
            .last_run
            .as_ref()
            .map_or_else(|| "none".to_owned(), last_run_text);
        ctx.out.line(&format!("  last run: {last_run}"))?;
        if let Some(
            repository::StagingOutcome::Staged { findings, .. }
            | repository::StagingOutcome::Unchanged { findings },
        ) = hook
            .last_run
            .as_ref()
            .and_then(|record| record.staging.as_ref())
        {
            for finding in findings {
                ctx.out.line(&format!("  finding: {finding}"))?;
            }
        }
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
    match &record.staging {
        None => {}
        Some(repository::StagingOutcome::Staged { files, findings }) => {
            parts.push(format!("staged in committed form: {}", files.join(", ")));
            if !findings.is_empty() {
                parts.push(format!(
                    "{} finding(s) left files as they were",
                    findings.len()
                ));
            }
        }
        Some(repository::StagingOutcome::Unchanged { findings }) => {
            parts.push("every file was already in committed form".to_owned());
            if !findings.is_empty() {
                parts.push(format!(
                    "{} finding(s) left files as they were",
                    findings.len()
                ));
            }
        }
        Some(repository::StagingOutcome::Failed { error }) => {
            parts.push(format!("staging failed: {error}"));
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
    /// session the commit is the git user's own. In an adopted project it
    /// takes the author's review-routing choice (`general/policy/commits`):
    /// --review, or --exempt <BASIS>, which records the commit's exemption
    /// once it exists. Prints the commit's id.
    Commit(CommitArgs),
    /// Reclaim the declared regenerable output of the worktree this
    /// session's todo assignment records; it takes no location. Prints each
    /// path removed and each declared path left with why.
    Reclaim,
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
    /// The commit goes to the independent review of a head that contains
    /// it: a requested review, or its todo's build review.
    #[arg(long)]
    review: bool,
    /// The commit owes no independent review, for this basis; recorded as
    /// its exemption once the commit exists.
    #[arg(long, value_name = "BASIS")]
    exempt: Option<String>,
}

pub fn run_repo(args: RepoArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        RepoCommand::Commit(args) => commit(args, ctx),
        RepoCommand::Reclaim => reclaim(ctx),
    }
}

/// What would let a caller reclaim a todo worktree, by what its assignment
/// is; the command's refusals and its admission name it. A session without
/// an assignment is bound to the attempt its dispatcher reserved. A session
/// whose assignment carries no todo, such as a standing one, cannot also hold
/// a todo's, so the request goes through a separate session. A todo worked
/// without a worktree has nothing to reclaim; what qualifies is a writing
/// assignment bound to a repository todo's worktree.
pub(crate) fn reclaim_door(
    assignment: Option<&instrukt_core::capabilities::Assignment>,
) -> &'static str {
    match assignment {
        None => {
            "the todo attempt its dispatcher reserved would, once bound to this session in that attempt's worktree (`instrukt todo bind <slug> --role <role> --attempt <attempt> --session <session> --worktree <worktree> --branch <branch> --base <base>`, with the worktree, branch and base the reservation names)"
        }
        Some(assignment) if assignment.todo.is_none() => {
            "this session's assignment cannot also hold a todo's, so ask the todo's dispatcher to reserve a writing role attempt in the todo's worktree, bind it to a separate session, and reclaim from that session"
        }
        Some(_) => {
            "its todo is worked without a worktree, so there is nothing to reclaim; reclaiming needs a writing assignment bound to a repository todo's worktree"
        }
    }
}

/// `instrukt repo reclaim`: what the runner removes from a todo worktree,
/// removed from the worktree the calling session's assignment records
/// (`project/instrukt/spec/feature/runner/worktree-reclamation`).
fn reclaim(ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    use instrukt_runner::reclamation::{OnRequest, Request, on_request};
    let env = crate::wiring::environment(ctx)?;
    let session = instrukt_core::config::Configuration::load_global(&ctx.home.for_inspection())?
        .caller(&env)?;
    let assignment = crate::authorization::caller(ctx)?.map(|(assignment, _)| assignment);
    let (Some(session), Some(assignment)) = (session, assignment) else {
        anyhow::bail!(
            "no todo assignment binds this caller to a worktree; {}",
            reclaim_door(None)
        );
    };
    let (Some(project), Some(todo), Some(worktree)) = (
        assignment.project.as_ref(),
        assignment.todo.as_deref(),
        assignment.workspace.as_deref(),
    ) else {
        anyhow::bail!(
            "the assignment {} binds this session to no todo worktree; {}",
            assignment.key,
            reclaim_door(Some(&assignment))
        );
    };
    let work = crate::wiring::work_ports(ctx)?
        .ok_or_else(|| anyhow::anyhow!("the todos module is not available in this build"))?;
    let todo = instrukt_core::ids::TodoId::parse(todo).ok_or_else(|| {
        anyhow::anyhow!(
            "the assignment {} records `{todo}`, which is no todo identity",
            assignment.key
        )
    })?;
    let request = Request {
        project,
        todo: &todo,
        worktree,
        session,
    };
    match on_request(&ctx.home, &request, work.reclamation.as_ref())? {
        OnRequest::NotPublished => anyhow::bail!(
            "{project} is not published on this computer; run `instrukt sync` in its checkout"
        ),
        OnRequest::NotRecorded => anyhow::bail!(
            "{project}'s work record on this computer holds no todo {todo}, so nothing was removed"
        ),
        OnRequest::Withheld(findings) => anyhow::bail!(
            "{project}'s declaration of regenerable output is not admitted on this computer, so nothing was removed; correct it in its instrukt.yaml:\n{}",
            findings.join("\n")
        ),
        OnRequest::Held(holder) => anyhow::bail!(
            "{} is held by {holder}, so nothing was removed; reclaim once it is released",
            worktree.display()
        ),
        OnRequest::NothingDeclared => {
            ctx.out.line(&format!(
                "{project} declares no regenerable output; nothing was removed"
            ))?;
        }
        OnRequest::Reclaimed { removed, left } => {
            if removed.is_empty() {
                ctx.out.line("nothing was removed")?;
            }
            for path in removed {
                ctx.out.line(&format!("removed: {path}"))?;
            }
            for (path, why) in left {
                ctx.out.line(&format!("left: {path} ({why})"))?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
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
        routing: ReviewRouting::from_options(args.review, args.exempt)?,
    };
    match repository::commit(
        &ctx.home,
        &ctx.cwd,
        &crate::wiring::environment(ctx)?,
        &core_wiring(ctx)?,
        &request,
    ) {
        Ok(sha) => {
            ctx.out.line(sha.as_str())?;
            Ok(ExitCode::SUCCESS)
        }
        // The commit stands: its id comes first, then the refusal's door.
        Err(RepositoryError::ExemptionNotRecorded {
            commit,
            basis,
            cause,
        }) => {
            ctx.out.line(commit.as_str())?;
            Err(RepositoryError::ExemptionNotRecorded {
                commit,
                basis,
                cause,
            }
            .into())
        }
        Err(error) => Err(error.into()),
    }
}

fn read_text(path: &Path, what: &str) -> anyhow::Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {what} from {}", path.display()))
}
