//! Configuration commands: `instrukt config get|patch|init|validate`.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::config::{self, Configuration, Env, InitOutcome, Layer, Revision, parse_patch};
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
    /// Dotted path, e.g. `modules.todos` or `runtime.conversation`.
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
    let value = configuration.get(args.path.as_deref(), &Env::from_process())?;
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
    Ok(if findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
