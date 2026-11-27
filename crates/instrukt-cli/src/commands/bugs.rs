//! The bug front door and the project's known issues
//! (`project/instrukt/spec/feature/work/todos`, "Bug reports";
//! `project/instrukt/spec/feature/work/known-issue-suppression`).
use crate::context::Ctx;
use anyhow::Context as _;
use instrukt_core::config::{self, Configuration, Module};
use instrukt_core::ids::{ProjectSlug, TodoSlug};
use instrukt_core::publication::PublishedRecord;
use instrukt_runner::interfaces::{ProjectRef, TodoKind};
use instrukt_todos::evidence::TodoStatus;
use instrukt_todos::work::suppressions::KnownIssues;
use instrukt_todos::work::{CreateRequest, Selection, Work};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, clap::Args)]
pub struct BugsArgs {
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Report a defect of the system you work through. Without `--approved`
    /// nothing is filed: the questions to face before filing are printed.
    Report(ReportArgs),
    /// The open bugs of the project: neither delivered nor removed.
    List {
        /// The owning project, by its slug on this computer; without it, the
        /// project the caller stands in.
        #[arg(long, value_name = "SLUG")]
        project: Option<ProjectSlug>,
        #[arg(long)]
        json: bool,
    },
    /// Retain an exact signal identity and its known-issue reason for this project.
    Suppress {
        signal: String,
        #[arg(long)]
        reason: String,
    },
    /// Read the complete retained set before classifying signals; read failures stop filing.
    Suppressions {
        /// The owning project, by its slug on this computer; without it, the
        /// project the caller stands in.
        #[arg(long, value_name = "SLUG")]
        project: Option<ProjectSlug>,
        #[arg(long)]
        json: bool,
    },
    /// Remove a disposition so future signals are eligible for ordinary filing checks.
    Unsuppress { id: String },
}

#[derive(Debug, clap::Args)]
struct ReportArgs {
    /// The defect as observed: the reported symptom.
    description: String,
    /// File it: the questions this command prints were faced.
    #[arg(long)]
    approved: bool,
    /// The bug's slug; without one, `fix-` and the description's words.
    #[arg(long)]
    slug: Option<TodoSlug>,
    /// The account beneath the symptom: observations and their addresses
    /// first, any causal account apart from them.
    #[arg(long)]
    body: Option<String>,
    /// The owning project, by its slug on this computer; without it, the
    /// project the caller stands in.
    #[arg(long, value_name = "SLUG")]
    project: Option<ProjectSlug>,
    /// Audit-only: whether a human was involved in filing.
    #[arg(
        long,
        action = clap::ArgAction::Set,
        value_parser = clap::builder::BoolValueParser::new(),
        value_name = "true|false"
    )]
    human_involved: Option<bool>,
}

/// What a report without `--approved` prints: the questions to face before
/// filing, each with the sources it names, and the approved form. The one
/// copy of these words (`project/instrukt/spec/feature/work/todos`, "Bug
/// reports").
pub const QUESTIONS: &str = "\
Nothing was filed. Face each question before filing this defect:

- Fix in context or file: is the repair documentation-only, or a bounded
  code change of about five minutes: the approach known, the edit bounded,
  needing neither a plan, a builder nor a handoff
  (sdlc/policy/todo-creation-discipline, Fix bounded findings in context)?
  Then make it yourself now in a short-lived one-off worktree and file
  nothing; finding it supplies the authority. A change whose correctness is
  contested belongs in the lifecycle. A company's agent never holds repair
  authority: for it the answer is always to report.
- Bug or change: which current contract does the behaviour break? A failure
  shows that something happened, not that it is wrong. Where no current
  contract states otherwise it is not a bug: file a change with
  `instrukt todo create <slug> <content> --kind change --approved` only when
  the behaviour is wanted, and otherwise file nothing. A report whose premise
  is that nothing followed up first establishes whether the sanctioned
  recovery was attempted.
- Validity: have the intended outcome and every contract it rests on
  converged, with no pending authoring, review or commit still expected to
  change them? A todo enters the lifecycle valid
  (sdlc/policy/todo-creation-discipline); work that has not converged is not
  filed through this door yet.
- Evidence: an occurrence or a dangerous path in hand
  (sdlc/concept/defect-evidence). Put the observations and their addresses
  first, and keep any causal account apart from them: each transition with
  its evidence, the alternatives checked and what remains unknown, or that
  the cause is not established.
- Duplication: already reported, already being fixed, or a triaged known
  issue? In the owning project:
    instrukt todo list --project <slug>
    instrukt bugs list --project <slug>
    instrukt bugs suppressions --project <slug>
- Ordering and dependency: downstream of, or dependent on, work already in
  flight? Route it to that work under sdlc/policy/todo-creation-discipline
  instead of filing.
- Owning project: the project that owns the defect, named with --project. A
  symptom that stops once the observing project's own configuration matches
  an instrukt default is that project's to fix, there.

Once every question is faced, file it:
  instrukt bugs report --approved '<description>' [--slug <slug>] [--body <text>] [--project <slug>] [--human-involved true|false]
";

/// The project a work read or report addresses: `named` when given, a
/// project publication state records on this computer, read in its
/// checkout; otherwise the adopted project the caller stands in. Refused,
/// naming `--project` and the projects this computer records, when neither
/// resolves.
pub fn owning_project(
    ctx: &Ctx,
    named: Option<&ProjectSlug>,
) -> anyhow::Result<(ProjectSlug, PathBuf)> {
    let refused = |what: String| -> anyhow::Error {
        let known = PublishedRecord::load_all(&ctx.home)
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| record.project.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let choices = if known.is_empty() {
            "this computer records no project".to_owned()
        } else {
            format!("this computer records {}", known.join(", "))
        };
        anyhow::anyhow!("{what}; name the owning project with `--project <slug>`: {choices}")
    };
    if let Some(named) = named {
        let record = PublishedRecord::load(&ctx.home, named)
            .with_context(|| format!("cannot read the publication state of `{named}`"))?
            .ok_or_else(|| refused(format!("this computer records no project `{named}`")))?;
        return Ok((named.clone(), record.checkout));
    }
    let root = instrukt_core::git::worktree_root(&ctx.cwd)?
        .ok_or_else(|| refused("this is not an adopted project".to_owned()))?;
    let project = Configuration::load_at(&ctx.home, &root)?
        .project()?
        .ok_or_else(|| refused("this is not an adopted project".to_owned()))?;
    Ok((project, root))
}

/// The work record of the project at `root`, with the todos module on there.
fn work_at(ctx: &Ctx, root: &std::path::Path) -> anyhow::Result<(Work, ProjectRef)> {
    if !config::module_enabled(&ctx.home, Some(root), Module::Todos)
        .context("cannot read the todos module switch")?
    {
        anyhow::bail!(
            "the todos module is off in {}; switch it on with `modules.todos: true` in its instrukt.yaml",
            root.display()
        );
    }
    let work = Work::new(
        ctx.home.clone(),
        crate::wiring::environment(ctx)?,
        ctx.clock.shared(),
    );
    let project = work.project_at(root)?;
    Ok((work, project))
}

/// `fix-` followed by the description's words, at most 40 characters.
fn default_slug(description: &str) -> anyhow::Result<TodoSlug> {
    let mut slug = "fix".to_owned();
    for word in description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        let word = word.to_ascii_lowercase();
        if slug.len() + 1 + word.len() > 40 {
            break;
        }
        slug.push('-');
        slug.push_str(&word);
    }
    if slug == "fix" {
        anyhow::bail!("the description holds no word to name the bug by; name it with `--slug`");
    }
    Ok(slug.parse()?)
}

/// The phase a bug's recorded assignments place it in.
fn phase(record: &instrukt_todos::work::TodoRecord) -> &'static str {
    match record
        .bindings
        .iter()
        .rev()
        .find_map(|binding| binding.attempt.branch.as_deref())
    {
        Some(branch) if branch.ends_with("/work") => "work",
        Some(branch) if branch.ends_with("/prepare") => "preparation",
        _ => "not started",
    }
}

pub fn run(args: BugsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        Command::Report(args) => report(args, ctx),
        Command::List { project, json } => {
            let (_, root) = owning_project(ctx, project.as_ref())?;
            let (work, project) = work_at(ctx, &root)?;
            let mut bugs = Vec::new();
            for selection in [Selection::Active, Selection::Iceboxed] {
                bugs.extend(
                    work.list(&project, selection)?
                        .entries
                        .into_iter()
                        .map(|entry| entry.record)
                        .filter(|record| record.kind == TodoKind::Bug),
                );
            }
            if json {
                ctx.out.json(
                    &bugs
                        .iter()
                        .map(|record| {
                            serde_json::json!({
                                "slug": record.slug,
                                "status": status_word(record.status),
                                "phase": phase(record),
                            })
                        })
                        .collect::<Vec<_>>(),
                )?;
            } else if bugs.is_empty() {
                ctx.out.line("no open bugs")?;
            } else {
                for record in &bugs {
                    ctx.out.line(&format!(
                        "{} | {} | {}",
                        record.slug,
                        status_word(record.status),
                        phase(record)
                    ))?;
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Suppressions { project, json } => {
            let (project, _) = owning_project(ctx, project.as_ref())?;
            let records = KnownIssues::new(&ctx.home, &project).list()?;
            if json {
                ctx.out.json(&records)?;
            } else {
                for record in records {
                    ctx.out.line(&format!(
                        "{} | {} | {}",
                        record.id, record.signal, record.reason
                    ))?;
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Suppress { signal, reason } => {
            let (project, _) = owning_project(ctx, None)?;
            ctx.out
                .json(&KnownIssues::new(&ctx.home, &project).retain(&signal, &reason)?)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Unsuppress { id } => {
            let (project, _) = owning_project(ctx, None)?;
            KnownIssues::new(&ctx.home, &project).remove(&id)?;
            ctx.out.line(&format!("removed suppression {id}"))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn status_word(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Iceboxed => "iceboxed",
        _ => "active",
    }
}

/// The front door: the questions without `--approved`, a filed bug-kind
/// todo with it.
fn report(args: ReportArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    // The owning project is resolved first: a report naming no project this
    // computer holds is refused before anything else, with or without
    // `--approved`.
    let (_, root) = owning_project(ctx, args.project.as_ref())?;
    if !args.approved {
        eprint!("{QUESTIONS}");
        return Ok(ExitCode::SUCCESS);
    }
    let (work, project) = work_at(ctx, &root)?;
    let slug = match args.slug {
        Some(slug) => slug,
        None => default_slug(&args.description)?,
    };
    let content = match args.body {
        Some(body) if !body.trim().is_empty() => format!("{}\n\n{body}", args.description),
        _ => args.description,
    };
    let outcome = work.create(
        &project,
        CreateRequest {
            slug,
            content,
            kind: TodoKind::Bug,
            icebox: false,
            after: Vec::new(),
            group: None,
            schedule: None,
            catch_up: None,
            agent: None,
            human_involved: args.human_involved,
            hold: None,
        },
    )?;
    let record = &outcome.value;
    ctx.out.line(&format!(
        "filed {} (bug) in project {} with identity {}",
        record.slug, project.slug, record.id
    ))?;
    crate::commands::todo::reflection(ctx, &outcome)
}
