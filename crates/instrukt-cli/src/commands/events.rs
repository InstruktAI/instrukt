//! Read producer-generated declarations without enabling events or transport.

use std::process::ExitCode;

use instrukt_runner::events::EventCatalog;

use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct EventsArgs {
    #[command(subcommand)]
    command: EventsCommand,
}

#[derive(Debug, clap::Subcommand)]
enum EventsCommand {
    /// Inspect module infrastructure, scoped transport, retained readers, failures and conversation links.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Inspect retained dead-letter identities, original coordinates and failure facts.
    DeadLetters {
        #[arg(long)]
        json: bool,
    },
    /// Reserve or bind a deliberate native standing assignment without runtime I/O.
    Bind(Box<BindArgs>),
    /// List declared event types and how to inspect their schema. No event
    /// infrastructure or enabled module is required.
    List {
        /// Select the first segment of a type name (instrukt, runner, workflow).
        #[arg(long)]
        domain: Option<String>,
    },
    /// Show a producer's complete declaration as JSON, including origins,
    /// data kinds, related references and any lifecycle state.
    Schema {
        /// The exact declared event type, as shown by `instrukt events list`.
        r#type: String,
    },
    /// Raise a condition about active todos of the project this session
    /// works in, for whoever answers the project's fleet attention: work
    /// that converges or conflicts, an existing solution found mid-flight,
    /// a decision a person owns, or a question its orchestrator cannot
    /// answer. Prints the raise's identity.
    Raise {
        /// The todos the condition concerns; the first is its subject.
        #[arg(required = true)]
        todos: Vec<instrukt_core::ids::TodoSlug>,
        /// The obligation at stake, in your own words.
        #[arg(long)]
        obligation: String,
    },
}

/// The door of a raise refused for its project.
const RAISE_PROJECT_DOOR: &str = "a raise comes from a session working in the project it concerns; this session's assignment is not in a project. Return the condition to the session that dispatched you, or raise it from a session working in that project";

/// The project a raise belongs to: a recorded assignment's project, never
/// the directory's, or for a caller with no assignment the project it
/// stands in. `None` when neither places it in a project.
fn raise_project(
    assignment: Option<&instrukt_core::capabilities::Assignment>,
    standing_in: Option<instrukt_core::ids::ProjectSlug>,
) -> Option<instrukt_core::ids::ProjectSlug> {
    match assignment {
        Some(assignment) => assignment.project.clone(),
        None => standing_in,
    }
}

#[derive(Debug, clap::Args)]
struct BindArgs {
    procedure: instrukt_core::ids::SnippetId,
    /// Enter a company-home procedure in this company's home.
    #[arg(long)]
    company: Option<instrukt_core::ids::ProjectSlug>,
    #[arg(long)]
    reserve: bool,
    #[arg(long)]
    reservation: Option<String>,
    #[arg(long)]
    session: Option<instrukt_core::ids::SessionRef>,
    #[arg(long)]
    evidence: Option<String>,
    #[arg(long)]
    not_started: bool,
    /// Confirm a previously observed same-key thread has stopped and been deleted.
    #[arg(long, requires = "not_started")]
    stopped: Option<instrukt_core::ids::SessionRef>,
    #[arg(long)]
    reason: Option<String>,
    #[arg(long)]
    args: Option<String>,
    #[arg(long)]
    brief: Option<String>,
    #[arg(long)]
    json: bool,
}
fn input(source: &str) -> anyhow::Result<String> {
    use std::io::Read as _;
    if source == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        Ok(text)
    } else {
        Ok(std::fs::read_to_string(source)?)
    }
}
pub fn run(args: EventsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let catalog = EventCatalog::builtin()?;
    match args.command {
        EventsCommand::Status { json } => {
            let status = super::runner::with_installation(
                ctx,
                true,
                None,
                instrukt_runner::installation::events_status,
            )?;
            if json {
                ctx.out.json(&status)?;
            } else {
                ctx.out.line(&serde_json::to_string_pretty(&status)?)?;
            }
        }

        EventsCommand::DeadLetters { json } => {
            let records = super::runner::with_installation(
                ctx,
                true,
                None,
                instrukt_runner::installation::dead_letters,
            )?;
            if json {
                ctx.out.json(&records)?;
            } else {
                ctx.out.line(&serde_json::to_string_pretty(&records)?)?;
            }
        }
        EventsCommand::Bind(args) => {
            // A company home combines with no project layer: its standing
            // entry reads the machine's configuration.
            let config = if args.company.is_some() {
                instrukt_core::config::Configuration::load_global(&ctx.home)?
            } else {
                instrukt_core::config::Configuration::load_at(&ctx.home, &ctx.cwd)?
            };
            let env = crate::wiring::environment(ctx)?;
            let invocation = instrukt_runner::keyed::RuntimeInvocation::acquire(
                &ctx.home,
                std::time::Duration::from_secs(120),
            )?;
            if args.reserve {
                if args.reservation.is_some()
                    || args.session.is_some()
                    || args.evidence.is_some()
                    || args.not_started
                {
                    anyhow::bail!("--reserve takes no binding or resolution fields");
                }
                let entry = instrukt_runner::launch::procedure(&ctx.home, &args.procedure)?;
                let brief = args.brief.as_deref().map(input).transpose()?;
                let result = instrukt_runner::standing::reserve_manual(
                    &invocation,
                    &instrukt_runner::standing::ManualEntry {
                        now: ctx.clock.now(),
                        config: &config,
                        env: &env,
                        entry: &entry,
                        company: args.company.as_ref(),
                        args: args.args.as_deref(),
                        brief: brief.as_deref(),
                    },
                )?;
                ctx.out.json(&result)?;
            } else {
                let token = args
                    .reservation
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("--reservation is required"))?;
                let evidence = args
                    .evidence
                    .as_deref()
                    .map(input)
                    .transpose()?
                    .map(|text| {
                        serde_json::from_str::<instrukt_runner::standing::ManualEvidence>(&text)
                    })
                    .transpose()?;
                if args.not_started {
                    if args.session.is_some() || evidence.is_some() {
                        anyhow::bail!("--not-started takes --reason and optional --stopped");
                    }
                } else if evidence
                    .as_ref()
                    .is_none_or(|e| Some(&e.session) != args.session.as_ref())
                {
                    anyhow::bail!("--session and matching --evidence are required");
                }
                instrukt_runner::standing::bind_manual(
                    &invocation,
                    &instrukt_runner::standing::ManualBinding {
                        config: &config,
                        env: &env,
                        procedure: args.procedure.as_str(),
                        token,
                        evidence: evidence.as_ref(),
                        reason: args.reason.as_deref(),
                        stopped: args.stopped.as_ref(),
                        now: ctx.clock.now(),
                    },
                )?;
                ctx.out.json(&serde_json::json!({"reservation":token,"session":args.session.map(|s|s.to_string())}))?;
            }
        }

        EventsCommand::List { domain } => {
            for declaration in catalog.list(domain.as_deref()) {
                let subject = declaration
                    .subject_kind
                    .map_or("any reference", |kind| kind.as_str());
                ctx.out.line(&format!(
                    "{} v{} — {} (subject: {}, delivery: {})",
                    declaration.name,
                    declaration.version,
                    declaration.description,
                    subject,
                    match declaration.delivery_kind {
                        instrukt_core::event_type::DeliveryKind::Occurrence => "occurrence",
                        instrukt_core::event_type::DeliveryKind::State => "state",
                    }
                ))?;
                ctx.out
                    .line(&format!("  instrukt events schema {}", declaration.name))?;
            }
        }
        EventsCommand::Raise { todos, obligation } => {
            let (session, recorded) = crate::authorization::caller_identity(ctx)?;
            let standing_in = match recorded {
                Some(_) => None,
                None => {
                    instrukt_core::config::Configuration::load_at(&ctx.home, &ctx.cwd)?.project()?
                }
            };
            let project = raise_project(recorded.as_ref().map(|(a, _)| a), standing_in)
                .ok_or_else(|| anyhow::anyhow!(RAISE_PROJECT_DOOR))?;
            let fleet = crate::wiring::fleet_record(ctx)?;
            let identity = instrukt_runner::fleet_attention::raise(
                &ctx.home,
                fleet.as_ref(),
                &project,
                session.as_ref(),
                &todos,
                &obligation,
                ctx.clock.now(),
            )?;
            ctx.out.line(&format!("raised {identity}"))?;
        }
        EventsCommand::Schema { r#type } => {
            let declaration = catalog.get(&r#type).ok_or_else(|| anyhow::anyhow!("unknown event type `{type}`; use `instrukt events list` to discover declared types"))?;
            ctx.out.line(&serde_json::to_string_pretty(declaration)?)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test: failed setup fails the test")]

    use instrukt_core::capabilities::Assignment;
    use instrukt_core::ids::{ProjectSlug, RoleName};

    use super::raise_project;

    fn slug(name: &str) -> ProjectSlug {
        ProjectSlug::parse(name).unwrap()
    }

    fn assigned(project: Option<&str>) -> Assignment {
        Assignment {
            worker: false,
            role: RoleName::parse("sdlc-coordinator").unwrap(),
            project: project.map(slug),
            procedure: None,
            todo: None,
            workspace: None,
            key: "standing".into(),
            platform: false,
            company: None,
            dispatcher: None,
        }
    }

    #[test]
    fn an_assigned_session_raises_in_its_assignments_project_wherever_it_stands() {
        assert_eq!(
            raise_project(Some(&assigned(Some("demo"))), Some(slug("other"))),
            Some(slug("demo"))
        );
    }

    #[test]
    fn an_assignment_outside_a_project_never_falls_back_to_the_directory() {
        assert_eq!(
            raise_project(Some(&assigned(None)), Some(slug("demo"))),
            None
        );
    }

    #[test]
    fn a_session_without_an_assignment_raises_in_the_project_it_stands_in() {
        assert_eq!(raise_project(None, Some(slug("demo"))), Some(slug("demo")));
    }

    #[test]
    fn a_session_without_an_assignment_outside_any_project_has_none() {
        assert_eq!(raise_project(None, None), None);
    }
}
