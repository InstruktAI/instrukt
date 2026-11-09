//! Configuration commands: `instrukt config get|patch|init|validate`.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::config::{self, Configuration, InitOutcome, Layer, Revision, parse_patch};
use serde_yaml_ng::Value;

use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ConfigCommand {
    /// Print the value at a dotted path, or the whole configuration,
    /// interpolated. Reads the effective configuration (the global layer
    /// with the project layer applied) unless --global.
    Get(GetArgs),
    /// Inspect machine-local configuration.
    Show {
        #[arg(value_parser=["agents"])]
        name: String,
    },
    /// Apply a merge patch (YAML mapping) to the project layer, or the
    /// global layer with --global, if it is still at --revision.
    Patch(PatchArgs),
    /// Create the layer with the current schema version and defaults, or
    /// migrate an existing older one.
    Init(LayerArgs),
    /// Report the layer's findings; exits non-zero when there are any.
    Validate(LayerArgs),
}

#[derive(Debug, clap::Args)]
struct LayerArgs {
    /// Address the global layer (~/.instrukt/instrukt.yaml).
    #[arg(long)]
    global: bool,
}

#[derive(Debug, clap::Args)]
struct GetArgs {
    /// Dotted path, e.g. `modules.todos` or `runtime.session_id`.
    path: Option<String>,
    /// Read the global layer alone.
    #[arg(long)]
    global: bool,
    /// Render as JSON.
    #[arg(long)]
    json: bool,
    /// Print the addressed layer's current revision instead of a value.
    #[arg(long, conflicts_with_all = ["path", "json"])]
    revision: bool,
}

#[derive(Debug, clap::Args)]
struct PatchArgs {
    /// The revision the patch was made from (`config get --revision`).
    #[arg(long)]
    revision: String,
    /// Patch the global layer.
    #[arg(long)]
    global: bool,
    /// The patch, inline. Without --yaml or --from-file it is read from
    /// standard input.
    #[arg(long, conflicts_with = "from_file", allow_hyphen_values = true)]
    yaml: Option<String>,
    /// Read the patch from this file.
    #[arg(long)]
    from_file: Option<PathBuf>,
}

pub fn run(args: ConfigArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        ConfigCommand::Get(args) => get(args, ctx),
        ConfigCommand::Show { name: _ } => {
            let mut agents = config::Agents::load(&ctx.home)?;
            for entry in agents.roles.values_mut() {
                entry.order = entry.order.take().or_else(|| agents.default.order.clone());
                entry.tier = entry.tier.or(agents.default.tier);
            }
            let mut inspected = serde_json::to_value(&agents)?;
            for role in agents.roles.keys() {
                inspected["roles"][role]["effective_grants"] =
                    serde_json::to_value(agents.capabilities(role)?.grants)?;
            }
            inspected["default"]["effective_grants"] =
                serde_json::to_value(agents.capabilities("default")?.grants)?;
            ctx.out.json(&inspected)?;
            Ok(ExitCode::SUCCESS)
        }
        ConfigCommand::Patch(args) => patch(args, ctx),
        ConfigCommand::Init(args) => init(&args, ctx),
        ConfigCommand::Validate(args) => validate(&args, ctx),
    }
}

fn layer(global: bool, ctx: &Ctx) -> anyhow::Result<Layer> {
    if global {
        Ok(Layer::Global)
    } else {
        Ok(Layer::project_containing(&ctx.cwd)?)
    }
}

fn get(args: GetArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    if args.revision {
        let layer = layer(args.global, ctx)?;
        let revision = config::revision(&ctx.home, &layer)?;
        ctx.out.line(revision.as_str())?;
        return Ok(ExitCode::SUCCESS);
    }
    let configuration = if args.global {
        Configuration::load_global(&ctx.home)?
    } else {
        Configuration::load_at(&ctx.home, &ctx.cwd)?
    };
    let value = configuration.get(args.path.as_deref(), &crate::wiring::environment(ctx)?)?;
    if args.json {
        ctx.out.json(&value)?;
        return Ok(ExitCode::SUCCESS);
    }
    match value {
        Value::String(text) => ctx.out.line(&text)?,
        other => {
            let text = config::render_yaml(&other).context("cannot render the value")?;
            ctx.out.text(&text)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn patch(args: PatchArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    if args.global
        && let Some(root) = instrukt_core::git::worktree_root(&ctx.cwd)?
    {
        config::migrate(&ctx.home, &Layer::Project(root))?;
    }
    let layer = layer(args.global, ctx)?;
    let text = match (&args.yaml, &args.from_file) {
        (Some(text), _) => text.clone(),
        (None, Some(file)) => std::fs::read_to_string(file)
            .with_context(|| format!("cannot read the patch from {}", file.display()))?,
        (None, None) => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .context("cannot read the patch from standard input")?;
            text
        }
    };
    let patch = parse_patch(&text)?;
    config::patch(
        &ctx.home,
        &layer,
        &Revision::from_text(&args.revision),
        &patch,
    )?;
    Ok(ExitCode::SUCCESS)
}

fn init(args: &LayerArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let layer = layer(args.global, ctx)?;
    let path = layer.path(&ctx.home);
    let outcome = config::init(&ctx.home, &layer, None)?;
    if args.global
        && let Some(notice) = config::Agents::deploy(&ctx.home, config::SHIPPED_AGENTS)?
    {
        ctx.out.line(&notice)?;
    }
    let what = match outcome {
        InitOutcome::Created => "created".to_owned(),
        InitOutcome::AlreadyCurrent => "already at the current schema; kept".to_owned(),
        InitOutcome::Migrated { from } => format!(
            "migrated from schema_version {from} to {}",
            config::CURRENT_SCHEMA_VERSION
        ),
    };
    ctx.out.line(&format!("{}: {what}", path.display()))?;
    Ok(ExitCode::SUCCESS)
}

fn validate(args: &LayerArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let layer = layer(args.global, ctx)?;
    let findings = config::validate(&ctx.home, &layer)?;
    for finding in &findings {
        ctx.out.line(&finding.to_string())?;
    }
    if args.global
        && let Ok(agents) = config::Agents::load(&ctx.home)
    {
        let index = instrukt_core::index::GlobalIndex::load(&ctx.home)?.unwrap_or_default();
        let installed: Vec<&str> = index
            .snippets
            .iter()
            .filter(|entry| entry.taxonomy == "procedure" && entry.delivered_by.is_empty())
            .filter_map(|entry| entry.role.as_deref())
            .collect();
        let undeclared = agents.undeclared_roles(&installed);
        if !undeclared.is_empty() {
            ctx.out.line(&format!(
                "{}: roles with no installed procedure: {}",
                ctx.home.agents_file().display(),
                undeclared.join(", ")
            ))?;
        }
    }
    Ok(if findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
