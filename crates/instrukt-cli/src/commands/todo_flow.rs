//! Workflow's subcommands in the todo namespace: `instrukt todo prepare
//! <slug>` and `instrukt todo work <slug>` (`docs/project/spec/domain/
//! workflow.md`, "Flavor-facing contract"; `preparation`,
//! `delivery-cycle`).
//!
//! Owned by the workflow builder in wave 3; flattened into `todo` by
//! `todo.rs`, which also runs the `modules.todos` refusal before
//! dispatching here. Keep the one-line abouts: they appear in the
//! `todo --help` snapshot.

use std::process::ExitCode;

use instrukt_core::ids::TodoSlug;
use instrukt_todos::evidence::TodoStatus;
use instrukt_todos::workflow::{Flow, Next, Pickup, Rendering, Responsible, Workflow};

use crate::commands::not_built;
use crate::context::Ctx;
use crate::wiring::{work_evidence, work_ports};

#[derive(Debug, clap::Args)]
pub struct FlowArgs {
    /// The todo to pick up and derive the next obligation of.
    slug: TodoSlug,
    /// Native recorded-workspace evidence as JSON, from a file or - for stdin.
    #[arg(long)]
    placement_evidence: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
pub enum FlowCommand {
    /// A todo's next preparation obligation (deliberate pickup).
    Prepare(FlowArgs),
    /// A todo's next delivery obligation (deliberate pickup).
    Work(FlowArgs),
}

/// Picks the todo up for the calling session when it is active, then
/// prints the derived state, the selected transition, the responsible
/// party, the rendered command, the effects owed and the instructions. An
/// obligation that cannot be rendered is printed with what is missing and
/// exits non-zero: it is not executable.
pub fn run(command: FlowCommand, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let (flow, args, name) = match command {
        FlowCommand::Prepare(args) => (Flow::Prepare, args, "todo prepare"),
        FlowCommand::Work(args) => (Flow::Work, args, "todo work"),
    };
    let (Some(evidence), Some(ports)) = (work_evidence(ctx)?, work_ports(ctx)?) else {
        return not_built(name);
    };
    let workflow = Workflow::new(ctx.home.clone(), evidence, crate::wiring::environment(ctx)?);
    let config = instrukt_core::config::Configuration::load_at(&ctx.home, &ctx.cwd)?;
    let env = crate::wiring::environment(ctx)?;
    let work = instrukt_todos::work::Work::new(ctx.home.clone(), env.clone(), ctx.clock.shared());
    let project = work.project_at(&ctx.cwd)?;
    let domain = config.workflow_domain()?;
    let flavor = instrukt_todos::workflow::Flavor::load(
        &ctx.home.docs_dir().join(domain.as_str()).join("workflow"),
    )?;
    let role = flavor
        .orchestrator(flow.as_str())
        .ok_or_else(|| anyhow::anyhow!("flow has no orchestrator"))?;
    let placement_evidence = args
        .placement_evidence
        .as_deref()
        .map(crate::commands::todo::native_input)
        .transpose()?
        .map(|text| serde_json::from_str::<instrukt_todos::work::DriverPlacement>(&text))
        .transpose()?;
    if let Some(placement) = work.workspace_entry(
        &project,
        &instrukt_todos::work::WorkspaceRequest {
            todo: &args.slug,
            role,
            cwd: &ctx.cwd,
            preparation: flow == Flow::Prepare,
            evidence: placement_evidence.as_ref(),
        },
    )? && !placement.admitted
    {
        let instruction = config.get_str("runtime.start_thread", &env)?;
        ctx.out.json(&serde_json::json!({"placement":placement,"instruction":instruction,"continuation":format!("instrukt todo {} {} --placement-evidence <file|->",flow.as_str(),args.slug)}))?;
        return Ok(ExitCode::SUCCESS);
    }
    let next = workflow.next(&ctx.cwd, flow, &args.slug, ports.claims.as_ref())?;
    let (text, executable) = render(&next);
    ctx.out.text(&text)?;
    Ok(if executable {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// The text an orchestrator reads, and whether everything owed rendered.
fn render(next: &Next) -> (String, bool) {
    let derivation = &next.derivation;
    let mut out = String::new();
    let mut executable = true;
    line(
        &mut out,
        "todo",
        &format!("{} ({})", next.todo, next.project),
    );
    line(&mut out, "flow", derivation.flow.as_str());
    line(&mut out, "claim", &pickup(&next.pickup));
    line(&mut out, "state", derivation.state.as_str());
    let Some(obligation) = &derivation.obligation else {
        line(&mut out, "action", "none: nothing is owed at this state");
        return (out, executable);
    };
    line(
        &mut out,
        "transition",
        &format!(
            "{} -> {} (#{})",
            obligation.transition.from, obligation.transition.to, obligation.transition.index
        ),
    );
    line(&mut out, "action", obligation.action.as_str());
    let party = match &obligation.responsible {
        Responsible::Role(role) => format!("{role} (native shared assignment)"),
        Responsible::Orchestrator(role) => format!("{role} (orchestrator step)"),
        Responsible::Procedure(procedure) => match &next.responsible_role {
            Some(role) => format!("{role} ({procedure})"),
            None => format!("{procedure} (role not in the docs index)"),
        },
    };
    line(&mut out, "responsible", &party);
    executable &= rendered(&mut out, "command", &obligation.command);
    if let Some(instruction) = &obligation.dispatch_instruction {
        executable &= rendered(&mut out, "native dispatch (orchestrator)", instruction);
    }
    if obligation.effects.is_empty() {
        line(&mut out, "effects", "none");
    } else {
        out.push_str("effects:\n");
        for effect in &obligation.effects {
            executable &= rendered(&mut out, &format!("  {}", effect.name), &effect.command);
        }
    }
    match &obligation.brief {
        Ok(brief) => {
            out.push_str("instructions:\n");
            out.push_str(brief);
            if !brief.ends_with('\n') {
                out.push('\n');
            }
        }
        Err(failure) => {
            executable = false;
            line(
                &mut out,
                "instructions",
                &format!("not rendered: {failure}"),
            );
        }
    }
    (out, executable)
}

fn line(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

/// Writes a rendered value, or what kept it from rendering; `false` for
/// the latter.
fn rendered(out: &mut String, key: &str, value: &Rendering) -> bool {
    match value {
        Ok(text) => {
            line(out, key, text);
            true
        }
        Err(failure) => {
            line(out, key, &format!("not rendered: {failure}"));
            false
        }
    }
}

fn pickup(pickup: &Pickup) -> String {
    match pickup {
        Pickup::Claimed => "claimed by this session".to_owned(),
        Pickup::AlreadyHeld => "held by this session".to_owned(),
        Pickup::NotReflected => {
            "claimed by this session; reflection to GitHub is pending".to_owned()
        }
        Pickup::ReadOnly(status) => {
            format!("none; the todo is {} (read-only)", status_name(*status))
        }
    }
}

fn status_name(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Active => "active",
        TodoStatus::Iceboxed => "iceboxed",
        TodoStatus::StructuralHolder => "a structural holder",
        TodoStatus::Delivered => "delivered",
        TodoStatus::Removed => "removed",
    }
}
