//! Todo commands: the work record's surfaces (`instrukt todo create|list|
//! show|update|refresh|freeze|unfreeze|remove|reactivate|deliver|stamp|
//! artifacts`), with workflow's `prepare|work` flattened in from
//! [`todo_flow`] (`docs/local/spec/feature/work/`).
//!
//! The work builder owns this file; the workflow builder owns
//! `todo_flow.rs`. The one-line abouts below appear in the `todo --help`
//! snapshot. The `modules.todos` refusal that names its door covers the
//! whole namespace and runs in [`run`], before dispatch.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use instrukt_core::config::{self, Env, Module};
use instrukt_core::git;
use instrukt_core::ids::{CommitSha, SessionRef, TodoSlug};
use instrukt_runner::interfaces::{
    AgentPreference, CatchUp, ProjectRef, TaskSchedule, Thinking, TodoKind,
};
use instrukt_todos::evidence::{
    Actor, ApprovalGate, ApprovedBy, ArtifactName, BuildVerdict, DriftClass, HoldSource,
    ModelProvenance, PreparationReview, ReviewVerdict, StampFact, TaskResult, TodoStatus,
};
use instrukt_todos::work::{
    CreateRequest, DiffLine, ListEntry, Outcome, ReflectionStatus, Selection, StampRequest,
    TodoAddress, TodoView, UpdateRequest, Work, describe,
};
use jiff::{SignedDuration, Timestamp};

use crate::commands::todo_flow;
use crate::context::Ctx;

/// The recovery window `remove` reports.
const RECOVERY_WINDOW: SignedDuration = SignedDuration::from_hours(7 * 24);

#[derive(Debug, clap::Args)]
pub struct TodoArgs {
    #[command(subcommand)]
    command: TodoCommand,
}

#[derive(Debug, clap::Subcommand)]
enum TodoCommand {
    /// Record a new todo with its kind and intent.
    Create(CreateArgs),
    /// The project's work: active by default, or iceboxed, removed or delivered.
    List(ListArgs),
    /// A todo's record, evidence, holds, claim and session bindings.
    Show(AddressArg),
    /// Change a todo's metadata, relationships, hold or claim.
    Update(UpdateArgs),
    /// Refresh a conflicted todo from GitHub.
    Refresh(SlugArg),
    /// Park a todo in the icebox.
    Freeze(SlugArg),
    /// Return a parked todo to active consideration.
    Unfreeze(SlugArg),
    /// Withdraw a todo, retained for recovery.
    Remove(SlugArg),
    /// Restore withdrawn work.
    Reactivate(ReactivateArgs),
    /// Record the confirmed delivered commit.
    Deliver(DeliverArgs),
    /// Record an attributed fact bound to evidence.
    Stamp(StampArgs),
    /// List, read, write and diff a todo's artifacts.
    Artifacts(ArtifactsArgs),
    #[command(flatten)]
    Flow(todo_flow::FlowCommand),
}

#[derive(Debug, clap::Args)]
struct SlugArg {
    /// The todo's slug.
    slug: TodoSlug,
}

#[derive(Debug, clap::Args)]
struct AddressArg {
    /// The live todo's slug, or `<slug>@<identity-prefix>` for a retained
    /// entry.
    #[arg(value_name = "TODO")]
    todo: TodoAddress,
}

#[derive(Debug, clap::Args)]
struct CreateArgs {
    /// The new todo's project-local slug.
    slug: TodoSlug,
    /// The intent: what is requested, as the first `input` version.
    content: String,
    /// feature, change, bug or task.
    #[arg(long)]
    kind: TodoKind,
    /// Create it; without this flag the admission questions are printed and
    /// nothing is created.
    #[arg(long)]
    approved: bool,
    /// Park it in the icebox.
    #[arg(long)]
    icebox: bool,
    /// Prerequisites, comma-separated slugs.
    #[arg(long, value_name = "SLUGS")]
    after: Option<String>,
    /// The group parent.
    #[arg(long, value_name = "PARENT")]
    group: Option<TodoSlug>,
    /// A task's schedule: an ISO-8601 instant or `cron:<five fields>`.
    #[arg(long, value_name = "VALUE")]
    schedule: Option<String>,
    /// A task's missed-run policy.
    #[arg(long, value_name = "once|skip")]
    catch_up: Option<CatchUpArg>,
    /// A task's preferred agent.
    #[arg(long, value_name = "NAME")]
    agent: Option<String>,
    /// The preferred agent's thinking mode.
    #[arg(long, value_name = "slow|med|fast", requires = "agent")]
    thinking: Option<ThinkingArg>,
    /// Audit-only: whether a human was involved in filing.
    #[arg(
        long,
        action = clap::ArgAction::Set,
        value_parser = clap::builder::BoolValueParser::new(),
        value_name = "true|false"
    )]
    human_involved: Option<bool>,
}

#[derive(Debug, clap::Args)]
#[group(multiple = false)]
struct ListArgs {
    /// The icebox instead of active work.
    #[arg(long)]
    iceboxed: bool,
    /// Withdrawn work within its recovery window.
    #[arg(long)]
    removed: bool,
    /// Delivered work.
    #[arg(long)]
    delivered: bool,
}

#[derive(Debug, clap::Args)]
struct UpdateArgs {
    /// The todo's slug.
    slug: TodoSlug,
    /// Rename it (refused once implementation starts).
    #[arg(long = "slug", value_name = "NEW_SLUG")]
    new_slug: Option<TodoSlug>,
    /// Change its kind (refused once implementation starts).
    #[arg(long)]
    kind: Option<TodoKind>,
    /// A task's schedule; an empty value clears it.
    #[arg(long, value_name = "VALUE")]
    schedule: Option<String>,
    /// Group it under a parent; an empty value leaves its group.
    #[arg(long, value_name = "PARENT")]
    group: Option<String>,
    /// Replace its prerequisites (comma-separated slugs); empty clears them.
    #[arg(long, value_name = "SLUGS")]
    after: Option<String>,
    /// Place a dispatch hold; needs `--reason`.
    #[arg(long, requires = "reason")]
    hold: bool,
    /// Why the work must wait.
    #[arg(long, requires = "hold")]
    reason: Option<String>,
    /// Release the dispatch hold.
    #[arg(long)]
    release: bool,
    /// Release the execution claim.
    #[arg(long)]
    unclaim: bool,
    /// A task's preferred agent.
    #[arg(long, value_name = "NAME")]
    agent: Option<String>,
    /// The preferred agent's thinking mode.
    #[arg(long, value_name = "slow|med|fast", requires = "agent")]
    thinking: Option<ThinkingArg>,
    /// Permit orphaning artifacts in a kind change.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, clap::Args)]
struct ReactivateArgs {
    /// The withdrawn todo's identity, or its slug when unambiguous.
    todo: String,
}

#[derive(Debug, clap::Args)]
struct DeliverArgs {
    /// The todo's slug.
    slug: TodoSlug,
    /// The delivered commit (full SHA).
    #[arg(long)]
    commit: CommitSha,
}

#[derive(Debug, clap::Args)]
struct StampArgs {
    /// The todo's slug.
    slug: TodoSlug,
    /// A review verdict, with `--phase`.
    #[arg(
        long,
        value_name = "approve|needs_work|changes_requested",
        requires = "phase"
    )]
    verdict: Option<VerdictArg>,
    /// The reviewed phase.
    #[arg(long, requires = "verdict")]
    phase: Option<PhaseArg>,
    /// Approval of the current prd, with `--by` (or `--carry-from`).
    #[arg(long)]
    prd_approved: bool,
    /// Approval of the current task plan, with `--by` (or `--carry-from`).
    #[arg(long)]
    task_approved: bool,
    /// A peer approval's reasoning (required with `--by peer`).
    #[arg(long, value_name = "TEXT")]
    reasoning: Option<String>,
    /// Carry the approval stamp with this number (`todo show` numbers
    /// stamps) to the current evidence.
    #[arg(long, value_name = "STAMP", conflicts_with = "by")]
    carry_from: Option<u64>,
    /// Who approved.
    #[arg(long, value_name = "human|peer")]
    by: Option<ByArg>,
    /// The session the approval was stated in (`<runtime>:<id>`).
    #[arg(long, value_name = "RUNTIME:ID")]
    session: Option<SessionRef>,
    /// The current `input` version is checked against its conversation.
    #[arg(long)]
    intent_checked: bool,
    /// Return the work to preparation, with `--reason`.
    #[arg(long, requires = "reason")]
    kickback: bool,
    /// The reason a kickback, unresolved diagnosis or failed outcome records.
    #[arg(long)]
    reason: Option<String>,
    /// Architecture is skipped for this reason.
    #[arg(long, value_name = "REASON")]
    architecture_skipped: Option<String>,
    /// The current diagnosis is settled.
    #[arg(long)]
    diagnosis_settled: bool,
    /// The diagnosis cannot be settled, with `--reason`; holds dispatch.
    #[arg(long, requires = "reason")]
    diagnosis_unresolved: bool,
    /// A drift judgment: artifact, cosmetic|substantive, reason.
    #[arg(long, num_args = 3, value_names = ["ARTIFACT", "CLASS", "REASON"])]
    drift_reviewed: Option<Vec<String>>,
    /// The build starts from the current plan.
    #[arg(long)]
    build_started: bool,
    /// The build is complete at the work branch's head.
    #[arg(long)]
    build_complete: bool,
    /// The current deferrals are processed.
    #[arg(long)]
    deferrals_processed: bool,
    /// The work branch's head is handed to integration.
    #[arg(long)]
    finalize_ready: bool,
    /// A task firing starts: the run of `--occurrence`, or now.
    #[arg(long)]
    task_firing_started: bool,
    /// A task execution's result.
    #[arg(long, value_name = "success|error")]
    task_outcome: Option<OutcomeArg>,
    /// The scheduled occurrence a firing start or outcome concerns
    /// (RFC 3339), as the run's opening message names it.
    #[arg(long, value_name = "RFC 3339")]
    occurrence: Option<jiff::Timestamp>,
}

#[derive(Debug, clap::Args)]
struct ArtifactsArgs {
    #[command(subcommand)]
    command: ArtifactsCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ArtifactsCommand {
    /// A todo's artifacts and their retained versions.
    List { slug: TodoAddress },
    /// An artifact's current content.
    Read { slug: TodoAddress, name: String },
    /// Append a version to an artifact.
    Write {
        slug: TodoSlug,
        name: String,
        content: String,
    },
    /// The line diff between two retained versions, by digest.
    Diff {
        slug: TodoAddress,
        name: String,
        old_digest: String,
        new_digest: String,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
enum CatchUpArg {
    Once,
    Skip,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
enum ThinkingArg {
    Slow,
    Med,
    Fast,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
enum VerdictArg {
    Approve,
    NeedsWork,
    ChangesRequested,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
#[allow(
    clippy::enum_variant_names,
    reason = "the values are the phase names `--phase` takes"
)]
enum PhaseArg {
    PrdReview,
    ArchitectureReview,
    PlanReview,
    BuildReview,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
enum ByArg {
    Human,
    Peer,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
enum OutcomeArg {
    Success,
    Error,
}

pub fn run(args: TodoArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = git::worktree_root(&ctx.cwd)?;
    let enabled = config::module_enabled(&ctx.home, root.as_deref(), Module::Todos)
        .context("cannot read the todos module switch")?;
    if !enabled {
        bail!("the todos module is off; switch it on with `modules.todos: true` in instrukt.yaml");
    }
    let command = match args.command {
        TodoCommand::Flow(command) => return todo_flow::run(command, ctx),
        command => command,
    };
    let Some(root) = root else {
        bail!("the work record belongs to a project; run this inside its git repository");
    };
    let work = Work::new(ctx.home.clone(), Env::from_process(), ctx.clock.shared());
    let project = work.project_at(&root)?;
    let session = Session {
        work: &work,
        project: &project,
        cwd: ctx.cwd.clone(),
    };
    session.dispatch(command, ctx)
}

struct Session<'a> {
    work: &'a Work,
    project: &'a ProjectRef,
    cwd: PathBuf,
}

impl Session<'_> {
    fn dispatch(&self, command: TodoCommand, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
        let (work, project) = (self.work, self.project);
        match command {
            TodoCommand::Create(args) => self.create(args, ctx),
            TodoCommand::List(args) => {
                let selection = if args.iceboxed {
                    Selection::Iceboxed
                } else if args.removed {
                    Selection::Removed
                } else if args.delivered {
                    Selection::Delivered
                } else {
                    Selection::Active
                };
                let listing = work.list(project, selection)?;
                if listing.entries.is_empty() {
                    ctx.out.line("no todos")?;
                }
                for entry in &listing.entries {
                    ctx.out.line(&list_line(entry))?;
                }
                if let instrukt_todos::work::GitHubRead::NotRead(reason) = &listing.github {
                    ctx.out.line(&format!(
                        "GitHub not read ({}); showing local items only",
                        describe(reason)
                    ))?;
                }
                Ok(ExitCode::SUCCESS)
            }
            TodoCommand::Show(args) => {
                let view = work.show(project, &args.todo)?;
                ctx.out.text(&render_view(&view))?;
                Ok(ExitCode::SUCCESS)
            }
            TodoCommand::Update(args) => self.update(args, ctx),
            TodoCommand::Refresh(args) => {
                let outcome = work.refresh(project, &args.slug)?;
                ctx.out.line(&format!(
                    "refreshed {} to revision {}",
                    outcome.value.slug, outcome.value.revision
                ))?;
                reflection(ctx, &outcome)
            }
            TodoCommand::Freeze(args) => {
                let outcome = work.freeze(project, &args.slug)?;
                ctx.out
                    .line(&format!("parked {} in the icebox", outcome.value.slug))?;
                reflection(ctx, &outcome)
            }
            TodoCommand::Unfreeze(args) => {
                let outcome = work.unfreeze(project, &args.slug)?;
                ctx.out
                    .line(&format!("returned {} to active work", outcome.value.slug))?;
                reflection(ctx, &outcome)
            }
            TodoCommand::Remove(args) => {
                let outcome = work.remove(project, &args.slug)?;
                let record = &outcome.value;
                let until = record
                    .removal
                    .as_ref()
                    .and_then(|r| r.at.checked_add(RECOVERY_WINDOW).ok());
                ctx.out.line(&format!(
                    "withdrew {} ({}); recoverable until {} with `instrukt todo reactivate {}`",
                    record.slug,
                    record.id,
                    until.map(time).unwrap_or_default(),
                    record.id
                ))?;
                reflection(ctx, &outcome)
            }
            TodoCommand::Reactivate(args) => {
                let outcome = work.reactivate(project, &args.todo)?;
                ctx.out.line(&format!(
                    "restored {} ({}) as {}",
                    outcome.value.slug,
                    outcome.value.id,
                    status_word(outcome.value.status)
                ))?;
                reflection(ctx, &outcome)
            }
            TodoCommand::Deliver(args) => {
                let outcome = work.deliver(project, &args.slug, &args.commit)?;
                ctx.out.line(&format!(
                    "delivered {} in {}",
                    outcome.value.slug, args.commit
                ))?;
                reflection(ctx, &outcome)
            }
            TodoCommand::Stamp(args) => self.stamp(args, ctx),
            TodoCommand::Artifacts(args) => self.artifacts(args.command, ctx),
            TodoCommand::Flow(command) => todo_flow::run(command, ctx),
        }
    }

    fn create(&self, args: CreateArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
        let request = CreateRequest {
            slug: args.slug.clone(),
            content: args.content,
            kind: args.kind,
            icebox: args.icebox,
            after: slugs(args.after.as_deref())?,
            group: args.group,
            schedule: args
                .schedule
                .as_deref()
                .map(TaskSchedule::parse)
                .transpose()?,
            catch_up: args.catch_up.map(|c| match c {
                CatchUpArg::Once => CatchUp::Once,
                CatchUpArg::Skip => CatchUp::Skip,
            }),
            agent: args.agent.map(|agent| AgentPreference {
                agent,
                thinking: args.thinking.map(thinking),
            }),
            human_involved: args.human_involved,
        };
        if !args.approved {
            ctx.out.text(&admission(&args.slug))?;
            return Ok(ExitCode::SUCCESS);
        }
        let outcome = self.work.create(self.project, request)?;
        let record = &outcome.value;
        ctx.out.line(&format!(
            "created {} ({}{}) with identity {}",
            record.slug,
            record.kind,
            if record.status == TodoStatus::Iceboxed {
                ", in the icebox"
            } else {
                ""
            },
            record.id
        ))?;
        reflection(ctx, &outcome)
    }

    fn update(&self, args: UpdateArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
        let request = UpdateRequest {
            slug: args.new_slug,
            kind: args.kind,
            schedule: args
                .schedule
                .map(|value| {
                    if value.trim().is_empty() {
                        Ok(None)
                    } else {
                        TaskSchedule::parse(&value).map(Some)
                    }
                })
                .transpose()?,
            group: args
                .group
                .map(|value| {
                    if value.trim().is_empty() {
                        Ok(None)
                    } else {
                        value.parse::<TodoSlug>().map(Some)
                    }
                })
                .transpose()?,
            after: args.after.as_deref().map(|v| slugs(Some(v))).transpose()?,
            hold: args.hold.then(|| args.reason.unwrap_or_default()),
            release: args.release,
            unclaim: args.unclaim,
            agent: args.agent.map(|agent| AgentPreference {
                agent,
                thinking: args.thinking.map(thinking),
            }),
            force: args.force,
        };
        let outcome = self.work.update(self.project, &args.slug, request)?;
        ctx.out
            .line(&format!("updated {}", outcome.value.record.slug))?;
        for warning in &outcome.value.warnings {
            ctx.out.line(&format!("warning: {warning}"))?;
        }
        reflection(ctx, &outcome)
    }

    fn stamp(&self, args: StampArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
        let request = stamp_request(&args)?;
        let events = crate::wiring::event_log(ctx)?;
        let outcome = self.work.stamp(
            self.project,
            &args.slug,
            &request,
            &self.cwd,
            events.as_deref(),
        )?;
        ctx.out.line(&format!(
            "stamped {}: {}",
            args.slug,
            fact(&outcome.value.fact)
        ))?;
        reflection(ctx, &outcome)
    }

    fn artifacts(&self, command: ArtifactsCommand, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
        let (work, project) = (self.work, self.project);
        match command {
            ArtifactsCommand::List { slug } => {
                let view = work.show(project, &slug)?;
                if view.record.artifacts.is_empty() {
                    ctx.out.line("no artifacts")?;
                }
                for artifact in &view.record.artifacts {
                    ctx.out.text(&render_artifact(artifact))?;
                }
            }
            ArtifactsCommand::Read { slug, name } => {
                let version = work.read_artifact(project, &slug, &name)?;
                let Some(content) = version.content else {
                    bail!(
                        "this computer does not hold the content of `{name}` version {}, and GitHub no longer serves it",
                        version.digest
                    );
                };
                ctx.out.text(&content)?;
                if !content.ends_with('\n') {
                    ctx.out.line("")?;
                }
            }
            ArtifactsCommand::Write {
                slug,
                name,
                content,
            } => {
                let outcome = work.write_artifact(project, &slug, &name, &content)?;
                ctx.out
                    .line(&format!("wrote {name} of {slug}: {}", outcome.value.digest))?;
                return reflection(ctx, &outcome);
            }
            ArtifactsCommand::Diff {
                slug,
                name,
                old_digest,
                new_digest,
            } => {
                for line in work.diff_artifact(project, &slug, &name, &old_digest, &new_digest)? {
                    ctx.out.line(&match line {
                        DiffLine::Same(text) => format!(" {text}"),
                        DiffLine::Removed(text) => format!("-{text}"),
                        DiffLine::Added(text) => format!("+{text}"),
                    })?;
                }
            }
        }
        Ok(ExitCode::SUCCESS)
    }
}

fn stamp_request(args: &StampArgs) -> anyhow::Result<StampRequest> {
    let mut forms: Vec<StampRequest> = Vec::new();
    let reason = || args.reason.clone().unwrap_or_default();
    if let (Some(verdict), Some(phase)) = (args.verdict, args.phase) {
        forms.push(verdict_request(verdict, phase)?);
    }
    for (set, gate) in [
        (args.prd_approved, ApprovalGate::Prd),
        (args.task_approved, ApprovalGate::Task),
    ] {
        if !set {
            continue;
        }
        if let Some(from) = args.carry_from {
            forms.push(StampRequest::CarriedApproval { gate, from });
            continue;
        }
        let by = match args.by {
            Some(ByArg::Human) => ApprovedBy::Human,
            Some(ByArg::Peer) => ApprovedBy::Peer,
            None => bail!(
                "an approval names who approved with `--by human|peer`, or carries one with `--carry-from <stamp>`"
            ),
        };
        forms.push(StampRequest::Approval {
            gate,
            by,
            session: args.session.clone(),
            reasoning: args.reasoning.clone(),
        });
    }
    if (args.reasoning.is_some() || args.carry_from.is_some() || args.by.is_some())
        && !(args.prd_approved || args.task_approved)
    {
        bail!(
            "`--by`, `--reasoning` and `--carry-from` go with `--prd-approved` or `--task-approved`"
        );
    }
    if args.intent_checked {
        forms.push(StampRequest::IntentChecked);
    }
    if args.kickback {
        forms.push(StampRequest::Kickback { reason: reason() });
    }
    if let Some(skip) = &args.architecture_skipped {
        forms.push(StampRequest::ArchitectureSkipped {
            reason: skip.clone(),
        });
    }
    if args.diagnosis_settled {
        forms.push(StampRequest::DiagnosisSettled);
    }
    if args.diagnosis_unresolved {
        forms.push(StampRequest::DiagnosisUnresolved { reason: reason() });
    }
    if let Some(parts) = &args.drift_reviewed {
        let [artifact, class, why] = parts.as_slice() else {
            bail!("`--drift-reviewed` takes an artifact, cosmetic|substantive and a reason");
        };
        let class = match class.as_str() {
            "cosmetic" => DriftClass::Cosmetic,
            "substantive" => DriftClass::Substantive,
            other => bail!("`{other}` is not a drift class: cosmetic or substantive"),
        };
        forms.push(StampRequest::DriftReviewed {
            artifact: ArtifactName::new(artifact.clone()),
            class,
            reason: why.clone(),
        });
    }
    for (set, form) in [
        (args.build_started, StampRequest::BuildStarted),
        (args.build_complete, StampRequest::BuildComplete),
        (args.deferrals_processed, StampRequest::DeferralsProcessed),
        (args.finalize_ready, StampRequest::FinalizeReady),
        (
            args.task_firing_started,
            StampRequest::TaskFiringStarted {
                occurrence: args.occurrence,
            },
        ),
    ] {
        if set {
            forms.push(form);
        }
    }
    if let Some(result) = args.task_outcome {
        forms.push(StampRequest::TaskOutcome {
            result: match result {
                OutcomeArg::Success => TaskResult::Success,
                OutcomeArg::Error => TaskResult::Error,
            },
            reason: args.reason.clone(),
            occurrence: args.occurrence,
        });
    }
    if args.occurrence.is_some() && !(args.task_firing_started || args.task_outcome.is_some()) {
        bail!("`--occurrence` names the occurrence of `--task-firing-started` or `--task-outcome`");
    }
    if args.session.is_some() && !(args.prd_approved || args.task_approved) {
        bail!(
            "`--session` names the session an approval was stated in; it goes with `--prd-approved` or `--task-approved`"
        );
    }
    match forms.len() {
        1 => Ok(forms.remove(0)),
        0 => bail!("name the fact to stamp; `instrukt todo stamp --help` lists the forms"),
        _ => bail!("one stamp records one fact; name a single form"),
    }
}

fn verdict_request(verdict: VerdictArg, phase: PhaseArg) -> anyhow::Result<StampRequest> {
    let preparation = |phase| match verdict {
        VerdictArg::Approve => Ok(StampRequest::PreparationVerdict {
            phase,
            verdict: ReviewVerdict::Approve,
        }),
        VerdictArg::NeedsWork => Ok(StampRequest::PreparationVerdict {
            phase,
            verdict: ReviewVerdict::NeedsWork,
        }),
        VerdictArg::ChangesRequested => {
            bail!("a preparation review's verdict is approve or needs_work")
        }
    };
    match phase {
        PhaseArg::PrdReview => preparation(PreparationReview::PrdReview),
        PhaseArg::ArchitectureReview => preparation(PreparationReview::ArchitectureReview),
        PhaseArg::PlanReview => preparation(PreparationReview::PlanReview),
        PhaseArg::BuildReview => match verdict {
            VerdictArg::Approve => Ok(StampRequest::BuildVerdict {
                verdict: BuildVerdict::Approve,
            }),
            VerdictArg::ChangesRequested => Ok(StampRequest::BuildVerdict {
                verdict: BuildVerdict::ChangesRequested,
            }),
            VerdictArg::NeedsWork => {
                bail!("a build review's verdict is approve or changes_requested")
            }
        },
    }
}

fn slugs(list: Option<&str>) -> anyhow::Result<Vec<TodoSlug>> {
    list.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<TodoSlug>().map_err(anyhow::Error::from))
        .collect()
}

fn thinking(arg: ThinkingArg) -> Thinking {
    match arg {
        ThinkingArg::Slow => Thinking::Slow,
        ThinkingArg::Med => Thinking::Med,
        ThinkingArg::Fast => Thinking::Fast,
    }
}

/// The admission questions `create` prints without `--approved`
/// (`sdlc/policy/todo-creation-discipline`, "Decide before creating").
fn admission(slug: &TodoSlug) -> String {
    format!(
        "Before filing `{slug}`, face these questions; then run the same command with --approved:
- Is the work already covered? Read `instrukt todo list` and the `input` of each plausible match.
- Does a small addition belong in an existing todo that has not started build?
- Has the intended outcome converged, with every contract preparation consumes authored and reviewed? If not, file it with --icebox.
- Is it documentation-only? That executes in context and needs no todo.
- Does a defect claim name the contract it breaks and cite the occurrence?
"
    )
}

/// The reflection line every change prints.
fn reflection<T>(ctx: &mut Ctx, outcome: &Outcome<T>) -> anyhow::Result<ExitCode> {
    ctx.out.line(&format!(
        "reflection: {}",
        reflection_word(&outcome.reflection)
    ))?;
    Ok(ExitCode::SUCCESS)
}

fn reflection_word(status: &ReflectionStatus) -> String {
    match status {
        ReflectionStatus::Reflected => "reflected to GitHub".to_owned(),
        ReflectionStatus::Pending(reason) => format!("pending: {}", describe(reason)),
        ReflectionStatus::Conflicted => {
            "conflicted with a newer GitHub revision; refresh with `instrukt todo refresh <slug>`"
                .to_owned()
        }
    }
}

fn list_line(entry: &ListEntry) -> String {
    let record = &entry.record;
    let mut parts = vec![
        record.slug.to_string(),
        record.kind.to_string(),
        status_word(record.status).to_owned(),
    ];
    if !record.after.is_empty() && !entry.ready {
        let after: Vec<&str> = record.after.iter().map(|r| r.slug.as_str()).collect();
        parts.push(format!("waits for {}", after.join(", ")));
    }
    if let Some(hold) = &record.hold {
        parts.push(format!("on hold: {}", hold.reason));
    }
    if record.claim.is_some() {
        parts.push("claimed".to_owned());
    }
    if !entry.local {
        parts.push("not on this computer".to_owned());
    }
    if let Some(ReflectionStatus::Pending(_)) = &entry.reflection {
        parts.push("reflection pending".to_owned());
    }
    if let Some(ReflectionStatus::Conflicted) = &entry.reflection {
        parts.push("conflicted".to_owned());
    }
    parts.join(" · ")
}

fn render_view(view: &TodoView) -> String {
    let record = &view.record;
    let mut out = Vec::new();
    out.push(format!(
        "{} · {} · {}",
        record.slug,
        record.kind,
        status_word(record.status)
    ));
    out.push(format!("identity: {}", record.id));
    out.push(format!(
        "created: {} by {}{}",
        time(record.created),
        actor(&record.created_by),
        match record.human_involved {
            Some(true) => " (human involved)",
            Some(false) => " (no human involved)",
            None => "",
        }
    ));
    out.push(format!("revision: {}", record.revision));
    let intent = record
        .artifact(instrukt_todos::evidence::artifact_names::INPUT)
        .and_then(|a| a.versions.last());
    if let Some(intent) = intent {
        out.push("intent:".to_owned());
        match &intent.content {
            Some(content) => out.extend(content.lines().map(|l| format!("  {l}"))),
            None => out.push("  (content not on this computer)".to_owned()),
        }
    }
    out.push(match &view.reflection {
        Some(status) => format!("reflection: {}", reflection_word(status)),
        None => "reflection: read from GitHub; not on this computer".to_owned(),
    });
    if let Some(issue) = view.issue {
        out.push(format!("issue: #{issue}"));
    }
    if let Some(pull) = view.pull {
        out.push(format!("pull request: #{pull}"));
    }
    out.push(match &record.claim {
        Some(claim) => format!(
            "claim: held since {}{}",
            time(claim.at),
            claim
                .session
                .as_ref()
                .map(|s| format!(" by session {s}"))
                .unwrap_or_default()
        ),
        None => "claim: not held".to_owned(),
    });
    out.push(match &record.hold {
        Some(hold) => format!(
            "hold: {} ({}, by {}, {})",
            hold.reason,
            match hold.source {
                HoldSource::Manual => "manual",
                HoldSource::Automatic => "automatic",
            },
            actor(&hold.by),
            time(hold.at)
        ),
        None => "hold: none".to_owned(),
    });
    if let Some(group) = &record.group {
        out.push(format!("group: {}", group.slug));
    }
    if !view.children.is_empty() {
        let children: Vec<&str> = view.children.iter().map(|c| c.slug.as_str()).collect();
        out.push(format!("children: {}", children.join(", ")));
    }
    if !view.prerequisites.is_empty() {
        let after: Vec<String> = view
            .prerequisites
            .iter()
            .map(|p| match p.status {
                Some(status) => format!("{} ({})", p.slug, status_word(status)),
                None => format!("{} (status unknown here)", p.slug),
            })
            .collect();
        out.push(format!("prerequisites: {}", after.join(", ")));
    }
    if let Some(schedule) = &record.schedule {
        out.push(format!(
            "schedule: {} (catch up: {})",
            match schedule {
                TaskSchedule::At(at) => time(*at),
                TaskSchedule::Cron(cron) => format!("cron:{cron}"),
            },
            match record.catch_up {
                CatchUp::Once => "once",
                CatchUp::Skip => "skip",
            }
        ));
    }
    if let Some(agent) = &record.agent {
        out.push(format!(
            "agent: {}{}",
            agent.agent,
            agent
                .thinking
                .map(|t| format!(
                    " ({})",
                    match t {
                        Thinking::Slow => "slow",
                        Thinking::Med => "med",
                        Thinking::Fast => "fast",
                    }
                ))
                .unwrap_or_default()
        ));
    }
    if let Some(delivery) = &record.delivery {
        out.push(format!(
            "delivered: {} at {} by {}",
            delivery.commit,
            time(delivery.at),
            actor(&delivery.by)
        ));
    }
    if let Some(removal) = &record.removal {
        out.push(format!("withdrawn: {}", time(removal.at)));
    }
    if !record.bindings.is_empty() {
        out.push("sessions:".to_owned());
        for binding in &record.bindings {
            out.push(format!(
                "  {}: {}{}",
                binding.role,
                binding.session,
                binding
                    .worktree
                    .as_deref()
                    .map(|p| format!(" (worktree {})", p.display()))
                    .unwrap_or_default()
            ));
        }
    }
    if !record.hold_history.is_empty() {
        out.push("released holds:".to_owned());
        for released in &record.hold_history {
            out.push(format!(
                "  {} (released {} by {})",
                released.hold.reason,
                time(released.released),
                actor(&released.by)
            ));
        }
    }
    out.push("artifacts:".to_owned());
    if record.artifacts.is_empty() {
        out.push("  none".to_owned());
    }
    let mut text = out.join("\n") + "\n";
    for artifact in &record.artifacts {
        text.push_str(&indent(&render_artifact(artifact)));
    }
    let mut stamps = vec!["stamps:".to_owned()];
    if record.stamps.is_empty() {
        stamps.push("  none".to_owned());
    }
    for stamp in &record.stamps {
        stamps.push(format!(
            "  #{} {} {} — by {}{}{}{}",
            stamp.sequence,
            time(stamp.at),
            fact(&stamp.fact),
            actor(&stamp.actor),
            stamp
                .role
                .as_ref()
                .map(|r| format!(" as {r}"))
                .unwrap_or_default(),
            match (&stamp.model_provenance, &stamp.model) {
                (ModelProvenance::Reported, Some(model)) =>
                    format!(", model {} {}", model.model, model.version),
                (ModelProvenance::Unknown, _) => ", model unknown".to_owned(),
                _ => String::new(),
            },
            stamp
                .eval_fingerprint
                .as_ref()
                .map(|f| format!(", eval {f}"))
                .unwrap_or_default()
        ));
    }
    text.push_str(&(stamps.join("\n") + "\n"));
    text
}

fn render_artifact(artifact: &instrukt_todos::work::Artifact) -> String {
    let mut lines = vec![format!(
        "{} ({} version{})",
        artifact.name,
        artifact.versions.len(),
        if artifact.versions.len() == 1 {
            ""
        } else {
            "s"
        }
    )];
    for version in &artifact.versions {
        lines.push(format!(
            "  {} by {} at {}{}",
            version.digest,
            actor(&version.writer),
            time(version.created),
            if version.content.is_some() {
                ""
            } else {
                " (content not on this computer)"
            }
        ));
    }
    lines.join("\n") + "\n"
}

fn indent(text: &str) -> String {
    text.lines().map(|l| format!("  {l}\n")).collect()
}

/// A stamp's fact and the evidence it binds, in words.
fn fact(fact: &StampFact) -> String {
    match fact {
        StampFact::PreparationVerdict {
            phase,
            verdict,
            target,
            report,
        } => format!(
            "{} verdict {} — target {target}, report {report}",
            match phase {
                PreparationReview::PrdReview => "prd_review",
                PreparationReview::ArchitectureReview => "architecture_review",
                PreparationReview::PlanReview => "plan_review",
            },
            match verdict {
                ReviewVerdict::Approve => "approve",
                ReviewVerdict::NeedsWork => "needs_work",
            }
        ),
        StampFact::BuildVerdict {
            verdict,
            target,
            report,
        } => format!(
            "build_review verdict {} — commit {target}, report {report}",
            match verdict {
                BuildVerdict::Approve => "approve",
                BuildVerdict::ChangesRequested => "changes_requested",
            }
        ),
        StampFact::Approval {
            gate,
            by,
            target,
            carried_from,
            reasoning,
        } => format!(
            "{} approved by {} — {target}{}{}",
            match gate {
                ApprovalGate::Prd => "prd",
                ApprovalGate::Task => "task plan",
            },
            match by {
                ApprovedBy::Human => "a human",
                ApprovedBy::Peer => "a peer",
            },
            carried_from
                .as_ref()
                .map(|c| format!(", carried from {c}"))
                .unwrap_or_default(),
            reasoning
                .as_ref()
                .map(|r| format!(" — reasoning: {r}"))
                .unwrap_or_default()
        ),
        StampFact::IntentChecked {
            input,
            conversation,
        } => format!("intent checked — input {input} against conversation {conversation}"),
        StampFact::Kickback {
            reason,
            input,
            invalidated_plan,
            ..
        } => format!(
            "kickback: {reason} — input {input}{}",
            invalidated_plan
                .as_ref()
                .map(|p| format!(", invalidated plan {p}"))
                .unwrap_or_default()
        ),
        StampFact::ArchitectureSkipped { reason } => format!("architecture skipped: {reason}"),
        StampFact::DiagnosisSettled { diagnosis } => {
            format!("diagnosis settled — diagnosis {diagnosis}")
        }
        StampFact::DiagnosisUnresolved { diagnosis, reason } => {
            format!("diagnosis unresolved: {reason} — diagnosis {diagnosis}")
        }
        StampFact::DriftReviewed {
            artifact,
            class,
            reason,
            examined,
        } => format!(
            "drift in {artifact} reviewed as {}: {reason} — examined {examined}",
            match class {
                DriftClass::Cosmetic => "cosmetic",
                DriftClass::Substantive => "substantive",
            }
        ),
        StampFact::BuildStarted { base, plan } => {
            format!("build started — base {base}, plan {plan}")
        }
        StampFact::BuildComplete { head } => format!("build complete — head {head}"),
        StampFact::DeferralsProcessed { deferrals } => {
            format!("deferrals processed — deferrals {deferrals}")
        }
        StampFact::FinalizeReady { commit } => format!("finalize ready — commit {commit}"),
        StampFact::TaskFiringStarted { occurrence } => {
            format!("task firing started — occurrence {}", time(*occurrence))
        }
        StampFact::TaskOutcome {
            occurrence,
            result,
            reason,
        } => format!(
            "task outcome {}{} — occurrence {}",
            match result {
                TaskResult::Success => "success",
                TaskResult::Error => "error",
            },
            reason
                .as_ref()
                .map(|r| format!(": {r}"))
                .unwrap_or_default(),
            time(*occurrence)
        ),
        StampFact::ChangesRequested {
            gate,
            target,
            reason,
        } => format!(
            "changes requested on {}: {reason}{}",
            match gate {
                ApprovalGate::Prd => "prd",
                ApprovalGate::Task => "task plan",
            },
            target
                .as_ref()
                .map(|t| format!(" — {t}"))
                .unwrap_or_default()
        ),
    }
}

fn actor(actor: &Actor) -> String {
    match actor {
        Actor::Session(session) => format!("session {session}"),
        Actor::GitHub { login } => format!("@{login}"),
        Actor::Person { name } => name.clone(),
    }
}

fn time(at: Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn status_word(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Active => "active",
        TodoStatus::Iceboxed => "iceboxed",
        TodoStatus::StructuralHolder => "structural holder",
        TodoStatus::Delivered => "delivered",
        TodoStatus::Removed => "withdrawn",
    }
}
