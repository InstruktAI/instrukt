//! Configuration commands: `instrukt config get|patch|init|validate|schema`,
//! each addressing the project layer, the global layer (`--global`) or a
//! company home's document (`--company <name>`).
//!
//! Inside a company-agent session, caller scope is enforced strictly to
//! authenticated workspace administrators speaking in a direct message, and
//! strictly to that company's own `mcp` declarations (`configuration-editing`,
//! "Company-agent caller scope").

use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::config::{
    self, Configuration, InitOutcome, Layer, LayerKind, Revision, parse_patch,
};
use instrukt_core::ids::ProjectSlug;
use serde_yaml_ng::Value;

use crate::{authorization, context::Ctx, wiring};

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
    /// Show what the addressed document can declare, from the types validation
    /// reads.
    Schema(SchemaArgs),
    /// Apply a merge patch (YAML mapping) to the project layer, or the
    /// global layer with --global, if it is still at --revision.
    Patch(PatchArgs),
    /// Create the layer with the current schema version and defaults, or
    /// migrate an existing older one. With --company, also append each
    /// ignore line the home's .gitignore lacks — /instrukt.yaml,
    /// /secrets.env, /projects/ and /.instrukt/ — keeping every other line,
    /// in a new home or an existing one.
    Init(LayerArgs),
    /// Report the layer's findings; exits non-zero when there are any. With
    /// --company, two kinds: the configuration's, among them each
    /// local-only path the home's repository tracks or does not ignore, and
    /// the findings of rendering the home's AGENTS.md from its working tree.
    Validate(LayerArgs),
}

#[derive(Debug, clap::Args)]
struct LayerArgs {
    /// Address the global layer (~/.instrukt/instrukt.yaml).
    #[arg(long, conflicts_with = "company")]
    global: bool,
    /// Address this company home's document (~/.instrukt/companies/<name>/instrukt.yaml).
    #[arg(long)]
    company: Option<ProjectSlug>,
}

#[derive(Debug, clap::Args)]
struct SchemaArgs {
    /// Dotted key path, e.g. `mcp` or `mcp_clients`.
    path: Option<String>,
    /// Describe the global layer (`~/.instrukt/instrukt.yaml`).
    #[arg(long, conflicts_with = "company")]
    global: bool,
    /// Describe a company home's document (`~/.instrukt/companies/<name>/instrukt.yaml`).
    #[arg(long)]
    company: Option<ProjectSlug>,
    /// Render as JSON Schema.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct GetArgs {
    /// Dotted path, e.g. `modules.todos` or `runtime.session_id`.
    path: Option<String>,
    /// Read the global layer alone.
    #[arg(long, conflicts_with = "company")]
    global: bool,
    /// Read this company home's document alone.
    #[arg(long)]
    company: Option<ProjectSlug>,
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
    #[arg(long, conflicts_with = "company")]
    global: bool,
    /// Patch this company home's document.
    #[arg(long)]
    company: Option<ProjectSlug>,
    /// The patch, inline. Without --yaml or --from-file it is read from
    /// standard input.
    #[arg(long, conflicts_with = "from_file", allow_hyphen_values = true)]
    yaml: Option<String>,
    /// Read the patch from this file.
    #[arg(long)]
    from_file: Option<PathBuf>,
}

pub fn run(args: ConfigArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let company_scope = check_company_agent_scope(ctx, &args.command)?;
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
        ConfigCommand::Schema(args) => schema(args, ctx),
        ConfigCommand::Patch(args) => patch(args, ctx, company_scope.as_ref()),
        ConfigCommand::Init(args) => init(&args, ctx),
        ConfigCommand::Validate(args) => validate(&args, ctx),
    }
}

/// Enforce `configuration-editing` "Company-agent caller scope".
/// Returns `Ok(Some(own_company))` when invoked inside an authorized company-agent
/// admin DM session, `Ok(None)` when outside any company-agent session, or `Err`
/// naming the direct-message administrator / local-operator door on any violation.
fn check_company_agent_scope(
    ctx: &Ctx,
    command: &ConfigCommand,
) -> anyhow::Result<Option<ProjectSlug>> {
    const DOOR: &str = "ask in a direct message as a workspace administrator, or have the local operator run the command outside any session";

    let inspection = ctx.home.for_inspection();
    let global = Configuration::load_global(&inspection)?;
    let env = wiring::environment(ctx)?;
    let Some(session) = global.caller(&env)? else {
        return Ok(None);
    };
    let assignment = authorization::caller(ctx)?.map(|(assignment, _)| assignment);
    let own_company = match assignment.and_then(|a| a.company) {
        Some(company) => company,
        None => {
            if let Some(company) = company_cwd(ctx) {
                anyhow::bail!(
                    "config access refused: this session works in company `{company}`'s home with no recorded binding yet; retry once bound, or have the local operator run outside any session"
                );
            }
            return Ok(None);
        }
    };

    // Establish active turn's speaker and require is_admin && is_direct.
    let runtime = global.runner_runtime()?;
    let adapter = instrukt_runner::runtime::adapter(runtime);
    let speaker = instrukt_company::conversations::speaker(&ctx.home, &session, adapter.as_ref())
        .map_err(|error| anyhow::anyhow!("config access refused: {error}; {DOOR}"))?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "config access refused: company-agent configuration requires an active direct-message turn from a workspace administrator; {DOOR}"
            )
        })?;
    if !speaker.is_admin {
        anyhow::bail!(
            "config access refused: speaker `{}` is not a workspace administrator; {DOOR}",
            speaker.name
        );
    }
    if !speaker.is_direct {
        anyhow::bail!(
            "config access refused: company service configuration is allowed only in a direct message, not a shared channel; {DOOR}"
        );
    }

    let require_own_company = |global_flag: bool,
                               target: Option<&ProjectSlug>|
     -> anyhow::Result<()> {
        if global_flag {
            anyhow::bail!(
                "config access refused: a company-agent session cannot access `--global`; {DOOR}"
            );
        }
        match target {
            Some(asked) if asked == &own_company => Ok(()),
            Some(asked) => anyhow::bail!(
                "config access refused: `--company {asked}` names another company than `{own_company}`; {DOOR}"
            ),
            None => anyhow::bail!(
                "config access refused: a company-agent session must address `--company {own_company}` (never the project layer); {DOOR}"
            ),
        }
    };

    let is_mcp_path =
        |path: Option<&str>| path.is_some_and(|p| p == "mcp" || p.starts_with("mcp."));

    match command {
        ConfigCommand::Schema(args) => {
            require_own_company(args.global, args.company.as_ref())?;
            if !is_mcp_path(args.path.as_deref()) {
                anyhow::bail!(
                    "config access refused: a company-agent session may only inspect `mcp` schema (`instrukt config schema --company {own_company} mcp`); {DOOR}"
                );
            }
        }
        ConfigCommand::Get(args) => {
            require_own_company(args.global, args.company.as_ref())?;
            if !args.revision && !is_mcp_path(args.path.as_deref()) {
                anyhow::bail!(
                    "config access refused: a company-agent session may only read `mcp` or `--revision` for `--company {own_company}`; {DOOR}"
                );
            }
        }
        ConfigCommand::Validate(args) => {
            require_own_company(args.global, args.company.as_ref())?;
        }
        ConfigCommand::Patch(args) => {
            require_own_company(args.global, args.company.as_ref())?;
        }
        ConfigCommand::Init(_) | ConfigCommand::Show { .. } => {
            anyhow::bail!(
                "config access refused: this config verb is not permitted inside a company-agent session; {DOOR}"
            );
        }
    }

    Ok(Some(own_company))
}

fn company_cwd(ctx: &Ctx) -> Option<ProjectSlug> {
    let companies = ctx.home.companies_dir().canonicalize().ok()?;
    let here = ctx.cwd.canonicalize().ok()?;
    let name = here.strip_prefix(&companies).ok()?.components().next()?;
    ProjectSlug::parse(name.as_os_str().to_str()?).ok()
}

fn layer(global: bool, company: Option<&ProjectSlug>, ctx: &Ctx) -> anyhow::Result<Layer> {
    if let Some(name) = company {
        Ok(Layer::Company(name.clone()))
    } else if global {
        Ok(Layer::Global)
    } else {
        Ok(Layer::project_containing(&ctx.cwd)?)
    }
}

fn schema(args: SchemaArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let kind = if args.company.is_some() {
        LayerKind::Company
    } else if args.global {
        LayerKind::Global
    } else {
        LayerKind::Project
    };
    let rendered = config::describe_schema(kind, args.path.as_deref(), args.json)?;
    ctx.out.line(&rendered)?;
    Ok(ExitCode::SUCCESS)
}

fn get(args: GetArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    if args.revision {
        let layer = layer(args.global, args.company.as_ref(), ctx)?;
        let revision = config::revision(&ctx.home, &layer)?;
        ctx.out.line(revision.as_str())?;
        return Ok(ExitCode::SUCCESS);
    }
    let configuration = if let Some(name) = &args.company {
        Configuration::load_company(&ctx.home, name)?
    } else if args.global {
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

fn patch(
    args: PatchArgs,
    ctx: &mut Ctx,
    company_scope: Option<&ProjectSlug>,
) -> anyhow::Result<ExitCode> {
    if args.global
        && let Some(root) = instrukt_core::git::worktree_root(&ctx.cwd)?
    {
        config::migrate(&ctx.home, &Layer::Project(root))?;
    }
    let layer = layer(args.global, args.company.as_ref(), ctx)?;
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
    if let Some(own_company) = company_scope {
        const DOOR: &str = "ask in a direct message as a workspace administrator for non-secret `mcp` settings (`sign_in.client_ref`, dynamic registration, or `sign_in.mode: issuer`), or have the local operator run the command outside any session";
        let Some(map) = patch.as_mapping() else {
            anyhow::bail!("patch must be a YAML mapping");
        };
        let forbidden: Vec<String> = map
            .keys()
            .filter_map(|k| {
                let name = k.as_str().unwrap_or("<non-string>");
                (name != "mcp").then(|| name.to_owned())
            })
            .collect();
        if map.is_empty() || !forbidden.is_empty() {
            anyhow::bail!(
                "config patch refused: a company-agent session may only patch keys under `mcp` (forbidden keys: {}); {DOOR}",
                if forbidden.is_empty() {
                    "empty patch".to_owned()
                } else {
                    forbidden.join(", ")
                }
            );
        }
        // Prevent a company-agent patch from introducing or rebinding any `${NAME}` secret
        // reference (e.g. pointing a new server URL at `${SLACK_BOT_TOKEN}` or `${INSTRUKT_ISSUER_SIGNING_KEY}`).
        let inspection = ctx.home.for_inspection();
        let existing_doc = std::fs::read_to_string(layer.path(&inspection))
            .ok()
            .and_then(|raw| serde_yaml_ng::from_str::<Value>(&raw).ok());
        let existing_mcp = existing_doc.as_ref().and_then(|doc| doc.get("mcp"));
        if let Some(mcp_patch) = map.get(Value::String("mcp".into())) {
            reject_new_secret_references(mcp_patch, existing_mcp, "mcp", own_company)?;
            reject_url_rebind_on_secret_server(mcp_patch, existing_mcp, own_company)?;
        }
    }
    config::patch(
        &ctx.home,
        &layer,
        &Revision::from_text(&args.revision),
        &patch,
    )?;
    Ok(ExitCode::SUCCESS)
}

/// Returns true if `value` contains any `${...}` secret reference anywhere in its subtree.
fn contains_secret_reference(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains("${"),
        Value::Mapping(map) => map.values().any(contains_secret_reference),
        Value::Sequence(items) => items.iter().any(contains_secret_reference),
        _ => false,
    }
}

/// Refuse changing `url` on an existing `mcp.<server>` entry that still carries any
/// `${NAME}` secret reference (such as `login` or `sign_in.client`), so a company-agent
/// DM patch cannot redirect an existing company secret to a new endpoint.
fn reject_url_rebind_on_secret_server(
    incoming_mcp: &Value,
    existing_mcp: Option<&Value>,
    company: &ProjectSlug,
) -> anyhow::Result<()> {
    let (Some(in_map), Some(ex_map)) = (
        incoming_mcp.as_mapping(),
        existing_mcp.and_then(Value::as_mapping),
    ) else {
        return Ok(());
    };
    for (server_key, incoming_server) in in_map {
        let Some(existing_server) = ex_map.get(server_key) else {
            continue;
        };
        if !contains_secret_reference(existing_server) {
            continue;
        }
        let Some(new_url) = incoming_server
            .as_mapping()
            .and_then(|m| m.get(Value::String("url".into())))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let old_url = existing_server
            .as_mapping()
            .and_then(|m| m.get(Value::String("url".into())))
            .and_then(Value::as_str);
        if old_url != Some(new_url) {
            let server_name = server_key.as_str().unwrap_or("<server>");
            anyhow::bail!(
                "config patch refused: company-agent direct-message patches for `{company}` may not change `mcp.{server_name}.url` while it still references a company secret; clear the secret reference or have the local operator update it outside any session"
            );
        }
    }
    Ok(())
}

/// Refuse any `${...}` secret reference inside a company-agent `mcp` patch unless
/// the exact same string already existed at the exact same path in the company's
/// current `mcp` configuration.
fn reject_new_secret_references(
    incoming: &Value,
    existing: Option<&Value>,
    path: &str,
    company: &ProjectSlug,
) -> anyhow::Result<()> {
    match incoming {
        Value::String(text) if text.contains("${") => {
            let unchanged = existing.and_then(Value::as_str) == Some(text.as_str());
            if !unchanged {
                anyhow::bail!(
                    "config patch refused: company-agent direct-message patches for `{company}` may not introduce or change secret references (at `{path}`); use a host `sign_in.client_ref`, dynamic registration, or `sign_in.mode: issuer`, or have the local operator set secret references outside any session"
                );
            }
            Ok(())
        }
        Value::Mapping(map) => {
            let existing_map = existing.and_then(Value::as_mapping);
            for (k, v) in map {
                let key = k.as_str().unwrap_or("<key>");
                let child_path = format!("{path}.{key}");
                let existing_child = existing_map.and_then(|m| m.get(k));
                reject_new_secret_references(v, existing_child, &child_path, company)?;
            }
            Ok(())
        }
        Value::Sequence(items) => {
            let existing_seq = existing.and_then(Value::as_sequence);
            for (idx, v) in items.iter().enumerate() {
                let child_path = format!("{path}[{idx}]");
                let existing_child = existing_seq.and_then(|s| s.get(idx));
                reject_new_secret_references(v, existing_child, &child_path, company)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn init(args: &LayerArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let layer = layer(args.global, args.company.as_ref(), ctx)?;
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
    let layer = layer(args.global, args.company.as_ref(), ctx)?;
    let findings = config::validate(&ctx.home, &layer)?;
    for finding in &findings {
        ctx.out.line(&finding.to_string())?;
    }
    // A company home's rendering is publication's to judge; this command
    // reports its findings beside configuration's.
    let mut rendering = Vec::new();
    if let Some(name) = &args.company {
        rendering = instrukt_core::publication::company_findings(&ctx.home.company_dir(name))?;
        for finding in &rendering {
            ctx.out.line(&finding.to_string())?;
        }
    }
    if args.global
        && let Ok(agents) = config::Agents::load(&ctx.home)
    {
        let index =
            instrukt_core::index::GlobalIndex::load_readable(&ctx.home)?.unwrap_or_default();
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
    Ok(if findings.is_empty() && rendering.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
