//! Workflow's subcommand in the todo namespace: `instrukt todo work <slug>`
//! (`docs/project/spec/domain/workflow.md`).
//!
//! Deliberate pickup claims the active todo (after verifying its `<slug>/work`
//! workspace placement for repository work) and outputs its current factual
//! ledger — no state-machine transition table, pure facts.

use std::process::ExitCode;

use instrukt_core::ids::TodoSlug;
use instrukt_todos::evidence::TodoStatus;
use instrukt_todos::workflow::{Pickup, WorkEntry, Workflow};

use crate::commands::not_built;
use crate::context::Ctx;
use crate::wiring::{work_evidence, work_ports};

#[derive(Debug, clap::Args)]
pub struct FlowArgs {
    /// The todo's slug.
    slug: TodoSlug,
    /// Native recorded-workspace evidence as JSON, from a file or - for stdin.
    #[arg(long)]
    placement_evidence: Option<String>,
    /// Output the factual ledger as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Subcommand)]
pub enum FlowCommand {
    /// Record an execution claim and show the todo's ledger.
    Work(FlowArgs),
}

/// Picks the todo up for the calling session when active (establishing its
/// `<slug>/work` workspace placement if needed), then prints the factual
/// ledger (text or `--json`).
pub fn run(command: FlowCommand, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let FlowCommand::Work(args) = command;
    let (Some(evidence), Some(ports)) = (work_evidence(ctx)?, work_ports(ctx)?) else {
        return not_built("todo work");
    };
    let env = crate::wiring::environment(ctx)?;
    let workflow = Workflow::new(ctx.home.clone(), evidence, env.clone());
    let config = instrukt_core::config::Configuration::load_at(&ctx.home, &ctx.cwd)?;
    let work = instrukt_todos::work::Work::new(
        ctx.home.clone(),
        env.clone(),
        ctx.clock.shared(),
        crate::runtimes::registry(),
    );
    let project = work.project_at(&ctx.cwd)?;
    let domain = config.workflow_domain()?;
    let flavor = instrukt_todos::workflow::Flavor::load(
        &ctx.home.docs_dir().join(domain.as_str()).join("workflow"),
    )?;
    let role = flavor.orchestrator();
    let placement_evidence = args
        .placement_evidence
        .as_deref()
        .map(crate::commands::todo::native_input)
        .transpose()?
        .map(|text| native_placement(&text, role.as_str()))
        .transpose()?;
    if let Some(placement) = work.workspace_entry(
        &project,
        &instrukt_todos::work::WorkspaceRequest {
            todo: &args.slug,
            role,
            cwd: &ctx.cwd,
            evidence: placement_evidence.as_ref(),
        },
    )? && !placement.admitted
    {
        let instruction = config.get_str("runtime.start_thread", &env)?;
        ctx.out.json(&serde_json::json!({
            "placement": placement,
            "instruction": instruction,
            "continuation": format!("instrukt todo work {} --placement-evidence <file|->", args.slug),
        }))?;
        return Ok(ExitCode::SUCCESS);
    }
    let entry = workflow.pickup(&ctx.cwd, &args.slug, ports.claims.as_ref())?;
    if args.json {
        ctx.out.json(&serde_json::json!({
            "pickup": pickup(&entry.pickup),
            "ledger": entry.ledger,
        }))?;
    } else {
        ctx.out.text(&render(&entry))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn render(entry: &WorkEntry) -> String {
    let mut out = format!("pickup: {}\n", pickup(&entry.pickup));
    out.push_str(&entry.ledger.render_text());
    out
}

fn pickup(pickup: &Pickup) -> String {
    match pickup {
        Pickup::Claimed => "claimed by this session".to_owned(),
        Pickup::AlreadyHeld => "held by this session".to_owned(),
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

/// Normalize the older public proof wire format at CLI entry; stored evidence is current-shaped.
fn native_placement(
    text: &str,
    role: &str,
) -> serde_json::Result<instrukt_todos::work::DriverPlacement> {
    let mut value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(proof) = value.as_object_mut()
        && !proof.contains_key("title")
        && let Some(branch) = proof.get("branch").and_then(serde_json::Value::as_str)
        && let Some(key) = proof.get("key").and_then(serde_json::Value::as_str)
        && let Some(title) = instrukt_runner::runtime::legacy_driver_title(role, branch, key)
    {
        proof.insert("title".into(), title.into());
    }
    serde_json::from_value(value)
}
