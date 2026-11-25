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
        reject_yaml_tags(&patch, "patch", own_company)?;
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
        // reference or secret-bearing field (`login`, `credential`, `sign_in.client`, `sign_in.identity`).
        let inspection = ctx.home.for_inspection();
        let existing_doc = std::fs::read_to_string(layer.path(&inspection))
            .ok()
            .and_then(|raw| serde_yaml_ng::from_str::<Value>(&raw).ok());
        let existing_mcp = existing_doc.as_ref().and_then(|doc| doc.get("mcp"));
        if let Some(mcp_patch) = map.get(Value::String("mcp".into())) {
            enforce_admin_dm_mcp_allowlist(mcp_patch, existing_mcp, own_company)?;
        }
    }
    let expected = Revision::from_text(&args.revision);
    // Readiness needs GitHub, so it is asked before the document's lock is
    // taken; the patch's revision check binds the judged document to the
    // written one.
    if let Some(repository) = config::review_switch(&ctx.home, &layer, &expected, &patch)? {
        review_readiness_finding(ctx, &layer, &repository)?.map_or(Ok(()), |finding| {
            Err(config::ConfigError::Invalid(config::Findings(vec![
                finding,
            ])))
        })?;
    }
    config::patch(&ctx.home, &layer, &expected, &patch)?;
    Ok(ExitCode::SUCCESS)
}

/// Work's readiness for automatic review on `repository`, as a finding at
/// `review.automatic` of `layer` when it is unmet or GitHub cannot be asked.
fn review_readiness_finding(
    ctx: &Ctx,
    layer: &Layer,
    repository: &str,
) -> anyhow::Result<Option<config::Finding>> {
    use instrukt_core::interfaces::{AutomaticReviewReadiness as _, ReviewReadiness};
    let message = match ctx.github.review_readiness(repository, ctx.clock.now()) {
        ReviewReadiness::Met { .. } => return Ok(None),
        ReviewReadiness::Unmet(message) => message,
        ReviewReadiness::CannotAsk(cause) => format!(
            "GitHub cannot be asked whether automatic review's identities are ready on {repository}: {cause}; nothing is decided, try again once GitHub answers"
        ),
    };
    Ok(Some(config::Finding {
        document: layer.path(&ctx.home),
        key: config::REVIEW_AUTOMATIC_KEY.to_owned(),
        kind: config::FindingKind::AutomaticReviewNotReady,
        message,
    }))
}

/// Reject any YAML tag (`Value::Tagged`) anywhere in a company-agent patch so
/// tagged scalars or subtrees can never bypass allow-list or secret checks.
fn reject_yaml_tags(value: &Value, path: &str, company: &ProjectSlug) -> anyhow::Result<()> {
    match value {
        Value::Tagged(_) => {
            anyhow::bail!(
                "config patch refused: company-agent direct-message patches for `{company}` may not use YAML tags (at `{path}`); send plain YAML values or have the local operator apply the patch outside any session"
            );
        }
        Value::Mapping(map) => {
            for (k, v) in map {
                reject_yaml_tags(k, path, company)?;
                let child = format!("{path}.{}", k.as_str().unwrap_or("<key>"));
                reject_yaml_tags(v, &child, company)?;
            }
            Ok(())
        }
        Value::Sequence(items) => {
            for (idx, v) in items.iter().enumerate() {
                reject_yaml_tags(v, &format!("{path}[{idx}]"), company)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Unwrap any YAML tag on an existing value read from disk before inspecting it.
fn untag(mut value: &Value) -> &Value {
    while let Value::Tagged(tagged) = value {
        value = &tagged.value;
    }
    value
}

/// Returns true if an existing `mcp.<server>` entry carries any secret-bearing
/// configuration (`login`, `credential`, `sign_in.client`, `sign_in.identity`,
/// or any `${...}` secret reference).
fn server_carries_secret_binding(server: &Value) -> bool {
    let server = untag(server);
    if contains_secret_reference(server) {
        return true;
    }
    let Some(map) = server.as_mapping() else {
        return false;
    };
    if map.contains_key(Value::String("login".into()))
        || map.contains_key(Value::String("credential".into()))
    {
        return true;
    }
    if let Some(sign_in) = map
        .get(Value::String("sign_in".into()))
        .map(untag)
        .and_then(Value::as_mapping)
        && (sign_in.contains_key(Value::String("client".into()))
            || sign_in.contains_key(Value::String("identity".into())))
    {
        return true;
    }
    false
}

/// Returns true if `value` contains any `${...}` secret reference anywhere in its subtree.
fn contains_secret_reference(value: &Value) -> bool {
    match untag(value) {
        Value::String(text) => text.contains("${"),
        Value::Mapping(map) => map.values().any(contains_secret_reference),
        Value::Sequence(items) => items.iter().any(contains_secret_reference),
        _ => false,
    }
}

/// Strict allow-list for company-agent admin-DM `mcp` patches:
/// - Only non-secret fields are patchable: `url`, `description`, `speaker_login: sign-in`,
///   and `sign_in.{mode, scopes, client_ref, accept_return_without_issuer}` (or `null` to delete).
/// - Refuses `login`, `credential`, `speaker_login: stored`, `sign_in.client`, `sign_in.identity`,
///   any `${NAME}` secret reference, and changing `url` on an existing secret-bearing server.
fn enforce_admin_dm_mcp_allowlist(
    incoming_mcp: &Value,
    existing_mcp: Option<&Value>,
    company: &ProjectSlug,
) -> anyhow::Result<()> {
    const SECRET_DOOR: &str = "use a host `sign_in.client_ref`, dynamic registration, or `sign_in.mode: issuer`, or have the local operator update secret-bearing entries outside any session";

    if incoming_mcp.is_null() {
        return Ok(());
    }
    let Some(servers) = incoming_mcp.as_mapping() else {
        anyhow::bail!("config patch refused: `mcp` must be a YAML mapping");
    };
    let existing_servers = existing_mcp.map(untag).and_then(Value::as_mapping);

    for (server_key, incoming_server) in servers {
        let server_name = server_key.as_str().unwrap_or("<server>");
        if contains_secret_reference(incoming_server) {
            anyhow::bail!(
                "config patch refused: company-agent direct-message patches for `{company}` may not introduce or change secret references (under `mcp.{server_name}`); {SECRET_DOOR}"
            );
        }
        if incoming_server.is_null() {
            continue;
        }
        let Some(server_map) = incoming_server.as_mapping() else {
            anyhow::bail!("config patch refused: `mcp.{server_name}` must be a mapping or null");
        };
        let existing_server = existing_servers.and_then(|m| m.get(server_key));

        // Refuse changing `url` on an existing server that still binds company secrets,
        // member credentials (`credential`), declared `sign_in.client`, or `sign_in.identity`.
        if let Some(existing_srv) = existing_server
            && server_carries_secret_binding(existing_srv)
            && let Some(new_url) = server_map
                .get(Value::String("url".into()))
                .and_then(Value::as_str)
        {
            let old_url = untag(existing_srv)
                .as_mapping()
                .and_then(|m| m.get(Value::String("url".into())))
                .map(untag)
                .and_then(Value::as_str);
            if old_url != Some(new_url) {
                anyhow::bail!(
                    "config patch refused: company-agent direct-message patches for `{company}` may not change `mcp.{server_name}.url` while it binds a secret, member credential, declared client, or identity endpoint; {SECRET_DOOR}"
                );
            }
        }

        for (field_key, field_val) in server_map {
            let field = field_key.as_str().unwrap_or("<key>");
            match field {
                "url" | "description" => {
                    if !(field_val.is_string() || field_val.is_null()) {
                        anyhow::bail!(
                            "config patch refused: `mcp.{server_name}.{field}` must be a plain string"
                        );
                    }
                }
                "speaker_login" => {
                    let allowed =
                        field_val.is_null() || matches!(field_val.as_str(), Some("sign-in"));
                    if !allowed {
                        anyhow::bail!(
                            "config patch refused: company-agent direct-message patches for `{company}` may only set `mcp.{server_name}.speaker_login` to `sign-in` (not `stored`); {SECRET_DOOR}"
                        );
                    }
                }
                "sign_in" => {
                    if field_val.is_null() {
                        continue;
                    }
                    let Some(sign_in_map) = field_val.as_mapping() else {
                        anyhow::bail!(
                            "config patch refused: `mcp.{server_name}.sign_in` must be a mapping or null"
                        );
                    };
                    for (si_key, si_val) in sign_in_map {
                        let si_field = si_key.as_str().unwrap_or("<key>");
                        match si_field {
                            "mode" | "client_ref" => {
                                if !(si_val.is_string() || si_val.is_null()) {
                                    anyhow::bail!(
                                        "config patch refused: `mcp.{server_name}.sign_in.{si_field}` must be a plain string"
                                    );
                                }
                            }
                            "scopes" => {
                                let valid = si_val.is_null()
                                    || si_val
                                        .as_sequence()
                                        .is_some_and(|seq| seq.iter().all(Value::is_string));
                                if !valid {
                                    anyhow::bail!(
                                        "config patch refused: `mcp.{server_name}.sign_in.scopes` must be a list of strings"
                                    );
                                }
                            }
                            "accept_return_without_issuer" => {
                                if !(si_val.is_bool() || si_val.is_null()) {
                                    anyhow::bail!(
                                        "config patch refused: `mcp.{server_name}.sign_in.accept_return_without_issuer` must be a boolean"
                                    );
                                }
                            }
                            "client" | "identity" => {
                                anyhow::bail!(
                                    "config patch refused: company-agent direct-message patches for `{company}` may not set `mcp.{server_name}.sign_in.{si_field}`; {SECRET_DOOR}"
                                );
                            }
                            other => {
                                anyhow::bail!(
                                    "config patch refused: `mcp.{server_name}.sign_in.{other}` is not allowed in a company-agent direct-message patch; {SECRET_DOOR}"
                                );
                            }
                        }
                    }
                }
                "login" | "credential" => {
                    anyhow::bail!(
                        "config patch refused: company-agent direct-message patches for `{company}` may not set `mcp.{server_name}.{field}`; {SECRET_DOOR}"
                    );
                }
                other => {
                    anyhow::bail!(
                        "config patch refused: `mcp.{server_name}.{other}` is not allowed in a company-agent direct-message patch; {SECRET_DOOR}"
                    );
                }
            }
        }
    }
    Ok(())
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
    let mut findings = config::validate(&ctx.home, &layer)?;
    // Readiness is asked only once nothing local stands at the setting, so
    // an unreachable GitHub never hides a local finding.
    if !findings
        .iter()
        .any(|f| f.key == config::REVIEW_AUTOMATIC_KEY)
        && let Some(repository) = config::review_target(&ctx.home, &layer)?
    {
        findings.extend(review_readiness_finding(ctx, &layer, &repository)?);
    }
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
