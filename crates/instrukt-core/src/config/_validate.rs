//! Validation of one layer document as a whole: its structure, the
//! cross-key rules and its references, judged against the configuration it
//! takes part in.

use std::path::Path;

use serde_yaml_ng::Value;

use super::_effective::effective_value;
use super::_error::{ConfigError, Finding, FindingKind};
use super::_features::{Feature, availability};
use super::_interpolate::{Interpolator, Mode, join, key_text};
use super::_layer::{Layer, read_usable};
use super::_module::Module;
use super::_runtime::Runtime;
use super::_schema::{self, DEFAULT_GITHUB_API_URL, KeyFinding, LayerKind};
use crate::home::Home;

/// The findings of `layer` as stored at `path` with content `value`.
pub(crate) fn findings_for(
    home: &Home,
    layer: &Layer,
    path: &Path,
    value: &Value,
) -> Result<Vec<Finding>, ConfigError> {
    findings_in_scope(home, layer, path, value, true, Given::default())
}

/// What a judgement reads besides the layer itself, given when it was read
/// already: admission judges and admits the same values. What is not given
/// is read from its file.
#[derive(Default, Clone, Copy)]
pub(crate) struct Given<'a> {
    /// The global layer as admitted (`Some(None)`: there is none).
    pub global: Option<Option<&'a Value>>,
    pub agents: Option<&'a super::Agents>,
    /// Each published project's checkout and layer.
    pub projects: Option<&'a [(std::path::PathBuf, Option<Value>)]>,
    /// The judged company's secrets file, read once.
    pub secrets: Option<&'a super::_secrets::SecretSnapshot>,
}

/// [`findings_for`], with what was read already.
pub(crate) fn findings_given(
    home: &Home,
    layer: &Layer,
    path: &Path,
    value: &Value,
    given: Given<'_>,
) -> Result<Vec<Finding>, ConfigError> {
    findings_in_scope(home, layer, path, value, true, given)
}

fn findings_in_scope(
    home: &Home,
    layer: &Layer,
    path: &Path,
    value: &Value,
    other_projects: bool,
    given: Given<'_>,
) -> Result<Vec<Finding>, ConfigError> {
    let global = match (layer.kind(), given.global) {
        (LayerKind::Global, _) => None,
        (_, Some(global)) => global.cloned(),
        (LayerKind::Project | LayerKind::Company, None) => read_usable(home, &Layer::Global)?,
    };
    let this = crate::machine::existing_machine_id(home).ok().flatten();
    let mut found = judge(value, layer.kind(), global.as_ref(), this.as_ref());
    if let Layer::Company(name) = layer {
        found.extend(company_identity(name, value));
        if let Some(profiles) = value.get("profiles") {
            // A missing agents.yaml reads as the shipped defaults; one that
            // cannot be read or is invalid leaves the names unchecked, which
            // is this company's finding.
            let declared = match given.agents {
                Some(agents) => Ok(agents.clone()),
                None => super::Agents::load(&home.for_inspection()),
            };
            super::_company::check_profiles(name, profiles, &declared, &mut found);
        }
        if value.get("connectors").is_some() {
            let declared = match given.agents {
                Some(agents) => Ok(agents.clone()),
                None => super::Agents::load(&home.for_inspection()),
            };
            found.extend(connector_roles(home, name, value, declared.as_ref().ok()));
        }
        let read;
        let secrets = match given.secrets {
            Some(secrets) => secrets,
            None => {
                read = super::_secrets::SecretSnapshot::read(
                    home,
                    &super::SecretSource::Company(name.clone()),
                );
                &read
            }
        };
        found.extend(connector_secrets(secrets, name, value));
        let effective = effective_value(global.as_ref(), None)?;
        found.extend(ingress_signing(
            home,
            &effective,
            value,
            &super::SecretSource::Company(name.clone()),
        ));
    }
    if let Layer::Project(root) = layer {
        found.extend(code_on_github(root, value, global.as_ref())?);
        found.extend(super::_review::findings(
            &home.for_inspection(),
            root,
            value,
            global.as_ref(),
        )?);
    }
    if let Layer::Project(_) = layer {
        let effective = effective_value(global.as_ref(), Some(value))?;
        found.extend(ingress_signing(
            home,
            &effective,
            value,
            &super::SecretSource::Machine,
        ));
    }
    if layer.kind() == LayerKind::Global
        && let Some(backup) = value
            .get(super::_backup::BACKUP_KEY)
            .filter(|v| !v.is_null())
    {
        let machine_secrets =
            super::_secrets::SecretSnapshot::read(home, &super::SecretSource::Machine);
        super::_backup::check_secrets(backup, &machine_secrets, &mut found);
    }
    if layer.kind() == LayerKind::Global
        && let Some(ingress) = value.get("ingress")
        && ingress.get("reactions").is_some()
        && ingress.get("computer").is_none()
    {
        found.push(KeyFinding::new(
            "ingress.reactions",
            FindingKind::Missing,
            "names the computer that reacts to external facts, and there are none without ingress.computer; set ingress.computer or remove ingress.reactions",
        ));
    }
    if layer.kind() == LayerKind::Global
        && let Some(value) = value.get("github")
        && let Some(raw) = value.get("apps")
        && let Ok(apps) = crate::github::inventory(raw)
    {
        let project = value.get("project_app").and_then(Value::as_str);
        if project.is_none_or(|p| !apps.contains_key(p)) {
            found.push(KeyFinding::new(
                "github.project_app",
                FindingKind::UnresolvedReference,
                "name a profile in github.apps",
            ));
        }
        // The effective selection, so the shipped roles' App references are
        // checked when there is no agents.yaml; one that cannot be read or
        // is invalid leaves them unchecked.
        let declared = match given.agents {
            Some(agents) => Some(agents.clone()),
            None => super::Agents::load(&home.for_inspection()).ok(),
        };
        if let Some(agents) = declared {
            for (role, selection) in agents.roles {
                if selection
                    .github_app
                    .as_ref()
                    .is_some_and(|p| !apps.contains_key(p))
                {
                    found.push(KeyFinding::new(
                        &format!("roles.{role}.github_app"),
                        FindingKind::UnresolvedReference,
                        "name a profile in github.apps",
                    ));
                }
            }
        }
    }
    let effective = match layer.kind() {
        LayerKind::Global => effective_value(Some(value), None),
        LayerKind::Project => effective_value(global.as_ref(), Some(value)),
        LayerKind::Company => effective_value(global.as_ref(), None),
    }?;
    if layer.kind() != LayerKind::Company {
        workflow_capabilities(home, &effective, &mut found);
    }
    if other_projects
        && layer.kind() == LayerKind::Global
        && effective
            .get("modules")
            .and_then(|m| m.get("todos"))
            .and_then(Value::as_bool)
            == Some(true)
    {
        let projects: Vec<Option<Value>> = match given.projects {
            Some(projects) => projects.iter().map(|(_, value)| value.clone()).collect(),
            None => {
                let records =
                    crate::publication::PublishedRecord::load_all(home).map_err(|error| {
                        ConfigError::InvalidValue {
                            key: "modules.todos".into(),
                            message: error.to_string(),
                        }
                    })?;
                records
                    .into_iter()
                    .filter(|p| p.checkout.is_dir())
                    .map(|p| read_usable(home, &Layer::Project(p.checkout)))
                    .collect::<Result<_, _>>()?
            }
        };
        for project_value in projects {
            let selected = effective_value(Some(value), project_value.as_ref())?;
            // The default flow was checked above; project-specific selections join admission too.
            if selected
                .get("workflow")
                .and_then(|w| w.get("domain"))
                .and_then(Value::as_str)
                .is_some_and(|domain| domain != "sdlc")
            {
                workflow_capabilities(home, &selected, &mut found);
            }
        }
    }
    Ok(found
        .into_iter()
        .map(|finding| Finding {
            document: path.to_path_buf(),
            key: finding.key,
            kind: finding.kind,
            message: finding.message,
        })
        .collect())
}

/// On the ingress computer, every declared source's signing references
/// resolve from the protected secrets file of the declaring home.
fn ingress_signing(
    home: &Home,
    effective: &Value,
    value: &Value,
    secrets: &super::SecretSource,
) -> Vec<KeyFinding> {
    let mut found = Vec::new();
    if let Ok(ingress) = super::_webhooks::ingress(effective.get("ingress"))
        && std::fs::read_to_string(home.machine_id_file())
            .ok()
            .as_deref()
            .map(str::trim)
            == Some(ingress.computer.as_str())
        && let Ok(sources) = super::_webhooks::parse(value.get("webhooks"))
    {
        for source in sources.values() {
            if let Err(error) = source.resolve(home, secrets) {
                found.push(KeyFinding::new(
                    "webhooks",
                    FindingKind::UnresolvedReference,
                    error.to_string(),
                ));
            }
        }
    }
    found
}

/// The secrets a company's declared connectors reference are present and
/// nonblank in the company's own secrets file: a finding at each that is
/// not, naming the door.
fn connector_secrets(
    secrets: &super::_secrets::SecretSnapshot,
    name: &crate::ids::ProjectSlug,
    value: &Value,
) -> Vec<KeyFinding> {
    let Some(connectors) = value.get("connectors").and_then(Value::as_mapping) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (key, entry) in connectors {
        let Some(connector_name) = key.as_str() else {
            continue;
        };
        let kind_name = entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or(connector_name);
        let Some(kind) = super::_connectors::declaration(kind_name) else {
            continue;
        };
        for field in kind.fields {
            if !matches!(field.rule, instrukt_connector::Rule::SecretReference { .. }) {
                continue;
            }
            let Some(reference) = entry.get(field.name).and_then(Value::as_str) else {
                continue;
            };
            if let Err(error) = secrets.resolve(reference) {
                found.push(KeyFinding::new(
                    &join("connectors", &join(connector_name, field.name)),
                    FindingKind::Missing,
                    format!("{error}; set it with `instrukt secrets set <NAME> --company {name}`"),
                ));
            }
        }
    }
    found
}

/// Every explicit `role` on a company's declared connectors must name a role
/// configured in `agents.yaml` or declared by a delivered published procedure.
fn connector_roles(
    home: &Home,
    company: &crate::ids::ProjectSlug,
    value: &Value,
    agents: Option<&super::Agents>,
) -> Vec<KeyFinding> {
    let Some(connectors) = value.get("connectors").and_then(Value::as_mapping) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (key, entry) in connectors {
        let Some(connector_name) = key.as_str() else {
            continue;
        };
        let Some(role_str) = entry.get("role").and_then(Value::as_str) else {
            continue;
        };
        if crate::ids::RoleName::parse(role_str).is_err() {
            continue;
        }
        let configured = agents.map_or_else(
            || role_str == super::COMPANY_AGENT_ROLE,
            |agents| agents.roles.contains_key(role_str),
        );
        if !configured && !role_procedure_available(home, role_str) {
            found.push(KeyFinding::new(
                &join("connectors", &join(connector_name, "role")),
                FindingKind::UnresolvedReference,
                format!(
                    "company `{company}` connector `{connector_name}` names role `{role_str}`, which is neither configured in agents.yaml nor declared by a published procedure"
                ),
            ));
        }
    }
    found
}

/// A company home's document names the company its folder is named for.
pub(crate) fn company_identity(
    name: &crate::ids::ProjectSlug,
    value: &Value,
) -> Option<KeyFinding> {
    match value.get("company") {
        None => Some(KeyFinding::new(
            "company",
            FindingKind::Missing,
            format!("missing; a company home names its company (`company: {name}`)"),
        )),
        Some(Value::String(text)) if text == name.as_str() => None,
        Some(Value::String(text)) => Some(KeyFinding::new(
            "company",
            FindingKind::InvalidValue,
            format!("names company `{text}`, but the home folder is `{name}`"),
        )),
        // A value of the wrong shape is the schema's own finding.
        Some(_) => None,
    }
}

/// Whether `modules.todos` is on in the configuration project layer
/// `value` takes part in, judged with the machine's `global` layer.
pub(crate) fn todos_on(value: &Value, global: Option<&Value>) -> Result<bool, ConfigError> {
    Ok(effective_value(global, Some(value))?
        .get("modules")
        .and_then(|m| m.get(Module::Todos.key()))
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// Todos on in the effective configuration of the project rooted at
/// `root` need its code on GitHub (`github-record`, "Code on GitHub"): a
/// finding at `modules.todos` when it is not.
fn code_on_github(
    root: &Path,
    value: &Value,
    global: Option<&Value>,
) -> Result<Option<KeyFinding>, ConfigError> {
    if !todos_on(value, global)? {
        return Ok(None);
    }
    let configured = value
        .get("github")
        .and_then(|g| g.get("repository"))
        .and_then(Value::as_str);
    // The API address comes from the machine's own layer, never a
    // repository's.
    let api_url = global
        .and_then(|g| g.get("github"))
        .and_then(|g| g.get("api_url"))
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_GITHUB_API_URL);
    match crate::github::resolve_code_host(configured, api_url, root)? {
        crate::github::CodeHost::GitHub { .. } => Ok(None),
        crate::github::CodeHost::Elsewhere { reason } => Ok(Some(KeyFinding::new(
            "modules.todos",
            FindingKind::TodosNeedGitHub,
            crate::github::todos_need_github(&reason),
        ))),
    }
}

pub(super) fn role_procedure_available(home: &Home, role_name: &str) -> bool {
    use crate::index::{GlobalIndex, RoleProcedure};
    crate::ids::RoleName::parse(role_name)
        .ok()
        .is_some_and(|role| {
            GlobalIndex::load_readable(&home.for_inspection())
                .ok()
                .flatten()
                .is_some_and(|index| {
                    index
                        .procedures_declaring(&role)
                        .iter()
                        .any(|entry| matches!(entry, RoleProcedure::Delivered(_)))
                })
        })
}

pub(super) fn company_procedure_available(home: &Home) -> bool {
    role_procedure_available(home, super::COMPANY_AGENT_ROLE)
}

pub(super) fn missing_role_procedure(
    home: &Home,
    company: &crate::ids::ProjectSlug,
    connector: Option<&str>,
    role: &str,
) -> Finding {
    let key = match connector {
        Some(name) if role != super::COMPANY_AGENT_ROLE => join("connectors", name),
        _ => "connectors".into(),
    };
    Finding {
        document: Layer::Company(company.clone()).located(home),
        key,
        kind: FindingKind::InvalidValue,
        message: format!(
            "company `{company}` needs a published procedure declaring the {role} role"
        ),
    }
}

/// Local publication prerequisite for starting one named connector with `role`.
/// A custom role configured in `agents.yaml` without its own procedure falls
/// back to the `company-agent` procedure.
pub fn connector_role_procedure_finding(
    home: &Home,
    company: &crate::ids::ProjectSlug,
    connector: &str,
    role: &str,
    agents: Option<&super::Agents>,
) -> Option<Finding> {
    if role_procedure_available(home, role) {
        return None;
    }
    if role != super::COMPANY_AGENT_ROLE
        && agents.is_some_and(|agents| agents.roles.contains_key(role))
        && company_procedure_available(home)
    {
        return None;
    }
    let needed_role = if role != super::COMPANY_AGENT_ROLE
        && agents.is_some_and(|agents| agents.roles.contains_key(role))
    {
        super::COMPANY_AGENT_ROLE
    } else {
        role
    };
    Some(missing_role_procedure(
        home,
        company,
        Some(connector),
        needed_role,
    ))
}

/// Local publication prerequisite for starting a company's default connectors.
pub fn connector_procedure_finding(
    home: &Home,
    company: &crate::ids::ProjectSlug,
) -> Option<Finding> {
    (!company_procedure_available(home))
        .then(|| missing_role_procedure(home, company, None, super::COMPANY_AGENT_ROLE))
}

/// `instrukt config validate`: the findings of `layer`. A missing layer,
/// or one of a newer schema version, is refused rather than judged. A
/// project layer is judged as converted in memory; one committed at an
/// older schema version is no finding, its pending conversion reported by
/// the inspection notice, and the next commit brings it current
/// (`docs/project/spec/feature/configuration/owned-files.md`).
pub fn validate(home: &Home, layer: &Layer) -> Result<Vec<Finding>, ConfigError> {
    let inspection = home.for_inspection();
    let view = super::_layer::inspect_layer(&inspection, layer)?;
    let path = view.path;
    let Some(value) = view.value else {
        return Err(ConfigError::NoLayer {
            path,
            door: layer.init_door(),
        });
    };
    let mut findings = findings_for(&inspection, layer, &path, &value)?;
    if let Layer::Company(name) = layer {
        findings.extend(super::_company_document::local_material(
            &inspection,
            name,
            &path,
        )?);
        if let Some(entries) = value.get("connectors").and_then(Value::as_mapping)
            && !entries.is_empty()
        {
            let agents = super::Agents::load(&inspection).ok();
            let mut reported_default = false;
            for (key, entry) in entries {
                let Some(connector_name) = key.as_str() else {
                    continue;
                };
                let role = entry
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or(super::COMPANY_AGENT_ROLE);
                if let Some(finding) = connector_role_procedure_finding(
                    &inspection,
                    name,
                    connector_name,
                    role,
                    agents.as_ref(),
                ) {
                    if finding.key == "connectors" {
                        if reported_default {
                            continue;
                        }
                        reported_default = true;
                    }
                    findings.push(finding);
                }
            }
        }
    }
    if layer.kind() == LayerKind::Global
        && let Err(error) = super::Agents::load(&inspection)
    {
        findings.push(Finding {
            document: home.agents_file(),
            key: "agents".into(),
            kind: FindingKind::InvalidValue,
            message: error.to_string(),
        });
    }
    Ok(findings)
}

/// Validate the addressed layer using machine records, without other checkout reads.
pub fn validate_local(home: &Home, layer: &Layer) -> Result<Vec<Finding>, ConfigError> {
    let view = super::_layer::inspect_layer(home, layer)?;
    let Some(value) = view.value else {
        return Err(ConfigError::NoLayer {
            path: view.path,
            door: layer.init_door(),
        });
    };
    let mut found = findings_in_scope(home, layer, &view.path, &value, false, Given::default())?;
    if layer.kind() == LayerKind::Global
        && let Err(error) = super::Agents::load(&home.for_inspection())
    {
        found.push(Finding {
            document: home.agents_file(),
            key: "agents".into(),
            kind: FindingKind::InvalidValue,
            message: error.to_string(),
        });
    }
    Ok(found)
}

/// The finding for a document of kind `kind` at schema version `found` when
/// that is not the one this instrukt writes for its kind. A judged document
/// is already converted in memory, so this refuses a patch that writes a
/// non-current version.
fn schema_version_finding(found: u64, kind: LayerKind) -> Option<KeyFinding> {
    let current = kind.current_schema_version();
    (found != current).then(|| {
        KeyFinding::new(
            "schema_version",
            FindingKind::SchemaVersionNotCurrent,
            format!("this instrukt writes schema_version {current}"),
        )
    })
}

/// The findings of a layer document of kind `kind`; a project layer is
/// judged together with the machine's `global` layer.
pub(crate) fn judge(
    value: &Value,
    kind: LayerKind,
    global: Option<&Value>,
    this: Option<&crate::machine::MachineId>,
) -> Vec<KeyFinding> {
    let mut found = _schema::check(value, kind);
    found.extend(
        value
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|found| schema_version_finding(found, kind)),
    );
    let effective = match kind {
        LayerKind::Global => effective_value(Some(value), None),
        LayerKind::Project => effective_value(global, Some(value)),
        // A company home combines with no layer; its event switch is the
        // machine's.
        LayerKind::Company => effective_value(global, None),
    };
    let effective = match effective {
        Ok(value) => value,
        Err(error) => {
            found.push(KeyFinding::new(
                "runtimes",
                FindingKind::InvalidValue,
                error.to_string(),
            ));
            return found;
        }
    };
    match kind {
        LayerKind::Project => {
            required_modules(value, &effective, &mut found);
            event_transport(&effective, &mut found);
            found.extend(super::_identity::findings(value));
        }
        LayerKind::Global => {
            runner_runtime(value, &effective, &mut found);
            event_transport(&effective, &mut found);
            found.extend(thread_retention_unavailable(&effective));
        }
        LayerKind::Company => {
            super::_mcp::check_company_cross_refs(value, global, &mut found);
        }
    }
    feature_capabilities(value, &effective, this, &mut found);
    references(value, kind, &effective, &mut found);
    if let Some(sources) = value.get("webhooks").and_then(Value::as_mapping)
        && !sources.is_empty()
        && effective
            .get("modules")
            .and_then(|m| m.get("events"))
            .and_then(Value::as_bool)
            != Some(true)
    {
        found.push(KeyFinding::new(
            "webhooks",
            FindingKind::ModuleSwitchedOff,
            "webhook sources require modules.events enabled",
        ));
    }
    if kind == LayerKind::Company
        && value
            .get("connectors")
            .and_then(Value::as_mapping)
            .is_some_and(|connectors| !connectors.is_empty())
    {
        connector_capabilities(&effective, &mut found);
    }
    found
}

/// A declared connector gives its company conversations, whose verbs the
/// machine's chosen runtime must carry (`session-continuity`, "Verb-gated
/// features").
fn connector_capabilities(effective: &Value, out: &mut Vec<KeyFinding>) {
    // An unset runtime is the compiled one; one naming no entry is its own
    // finding and is not judged again here.
    let Some(chosen) = chosen_runtime(effective) else {
        return;
    };
    let features = &[Feature::Conversations];
    let Some(unavailable) = availability(chosen, features) else {
        return;
    };
    out.push(KeyFinding::new(
        "connectors",
        FindingKind::RuntimeCapabilityMissing,
        unavailable.describe(features),
    ));
}

/// A retention window set while the chosen runtime cannot archive threads:
/// a finding at the window naming the runtime and the missing capability
/// (`project/instrukt/spec/feature/configuration/layered-configuration`).
/// Judged on the effective global document, so choosing such a runtime
/// while a window is set finds it too.
pub(super) fn thread_retention_unavailable(effective: &Value) -> Option<KeyFinding> {
    super::_thread_retention::admitted(Some(effective))?;
    let chosen = chosen_runtime(effective)?;
    let features = &[Feature::ThreadRetention];
    let unavailable = availability(chosen, features)?;
    Some(KeyFinding::new(
        super::_thread_retention::ARCHIVE_FINISHED_AFTER_KEY,
        FindingKind::RuntimeCapabilityMissing,
        unavailable.describe(features),
    ))
}

/// The runtime the effective document `effective` chooses: the compiled
/// one when `runner.runtime` is unset; `None` when it names no compiled
/// runtime, which is its own finding.
fn chosen_runtime(effective: &Value) -> Option<Runtime> {
    match effective
        .get("runner")
        .and_then(|r| r.get("runtime"))
        .and_then(Value::as_str)
    {
        Some(name) => Runtime::named(name),
        None => Some(Runtime::COMPILED),
    }
}

/// A module switch this layer turns on whose features the chosen runtime
/// cannot carry: no runtime is chosen, or it leaves verbs those features
/// need empty (`session-continuity`, "Verb-gated features"). A
/// `runner.runtime` naming no entry is its own finding and is not judged
/// again here.
fn feature_capabilities(
    value: &Value,
    effective: &Value,
    this: Option<&crate::machine::MachineId>,
    out: &mut Vec<KeyFinding>,
) {
    // An unset runtime is the compiled one; one naming no entry is its own
    // finding and is not judged again here.
    let Some(chosen) = chosen_runtime(effective) else {
        return;
    };
    for module in ["events", "todos"] {
        let on = value
            .get("modules")
            .and_then(|m| m.get(module))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let features = Feature::switched_on_by(module);
        if !on || features.is_empty() {
            continue;
        }
        // Receiving and publishing need no agent: events on the ingress that
        // is not the reacting computer need no runtime.
        if module == "events" && receiving_only(effective, this) {
            continue;
        }
        let Some(unavailable) = availability(chosen, features) else {
            continue;
        };
        out.push(KeyFinding::new(
            &join("modules", module),
            FindingKind::RuntimeCapabilityMissing,
            unavailable.describe(features),
        ));
    }
}

/// Whether this computer is the ingress and another computer reacts to
/// external facts, as the effective global settings name them.
pub(super) fn receiving_only(effective: &Value, this: Option<&crate::machine::MachineId>) -> bool {
    let Some(this) = this else {
        return false;
    };
    let ingress = effective.get("ingress");
    let named = |key: &str| {
        ingress
            .and_then(|ingress| ingress.get(key))
            .and_then(Value::as_str)
            .and_then(crate::machine::MachineId::parse)
    };
    named("computer").as_ref() == Some(this)
        && super::_webhooks::reacting(ingress).is_some_and(|reacting| &reacting != this)
}

/// `requires`: every module it names is switched on in the effective
/// configuration.
fn required_modules(value: &Value, effective: &Value, out: &mut Vec<KeyFinding>) {
    let Some(required) = value.get("requires").and_then(Value::as_sequence) else {
        return;
    };
    for (i, item) in required.iter().enumerate() {
        let Some(module) = item.as_str().and_then(Module::from_key) else {
            continue;
        };
        let on = effective
            .get("modules")
            .and_then(|m| m.get(module.key()))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !on {
            out.push(KeyFinding::new(
                &join("requires", &i.to_string()),
                FindingKind::ModuleSwitchedOff,
                format!("requires module `{}`, which is switched off", module.key()),
            ));
        }
    }
}

/// Whether event responses are on in `effective`.
pub(crate) fn events_on(effective: &Value) -> bool {
    effective
        .get("modules")
        .and_then(|m| m.get(Module::Events.key()))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Event responses need the event transport they are carried on: a finding
/// at `modules.events` while it is on and `redis.cluster.url` is not set.
/// Whether `effective` switches event responses on without naming the
/// event transport they are carried on.
pub(crate) fn needs_transport(effective: &Value) -> bool {
    let url = effective
        .get("redis")
        .and_then(|r| r.get("cluster"))
        .and_then(|c| c.get("url"))
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty());
    events_on(effective) && url.is_none()
}

fn event_transport(effective: &Value, out: &mut Vec<KeyFinding>) {
    if needs_transport(effective) {
        out.push(KeyFinding::new(
            "modules.events",
            FindingKind::EventsNeedTransport,
            "event responses need the event transport, and `redis.cluster.url` is not set; set it in the global layer first (`instrukt config patch --global`)",
        ));
    }
}

fn runner_runtime(value: &Value, _effective: &Value, out: &mut Vec<KeyFinding>) {
    if let Some(name) = value
        .get("runner")
        .and_then(|r| r.get("runtime"))
        .and_then(Value::as_str)
        && Runtime::named(name).is_none()
    {
        out.push(KeyFinding::new(
            "runner.runtime",
            FindingKind::UnknownRuntime,
            format!("unknown compiled runtime adapter `{name}`; available: t3"),
        ));
    }
}

/// Every reference resolves and none leads back to itself. Webhook secret
/// references are checked by their typed accessor.
fn references(value: &Value, kind: LayerKind, effective: &Value, out: &mut Vec<KeyFinding>) {
    let mut strings = Vec::new();
    collect_strings(&_schema::permitted(value, kind), "", &mut strings);
    for (path, text) in strings {
        if super::_interpolate::stored_reference(&path) {
            continue;
        }
        let error = Interpolator::new(effective, Mode::Check)
            .expand_text(&text, Some(&path))
            .err();
        let Some(error) = error else { continue };
        let kind = match error {
            ConfigError::Cycle { .. } => FindingKind::ReferenceCycle,
            _ => FindingKind::UnresolvedReference,
        };
        out.push(KeyFinding::new(&path, kind, error.to_string()));
    }
}

fn collect_strings(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::String(text) => out.push((path.to_owned(), text.clone())),
        Value::Mapping(map) => {
            for (key, child) in map {
                collect_strings(child, &join(path, &key_text(key)), out);
            }
        }
        Value::Sequence(seq) => {
            for (i, child) in seq.iter().enumerate() {
                collect_strings(child, &join(path, &i.to_string()), out);
            }
        }
        _ => {}
    }
}

/// Admission reads typed capability declarations, never action/template prose.
fn workflow_capabilities(home: &Home, effective: &Value, out: &mut Vec<KeyFinding>) {
    if effective
        .get("modules")
        .and_then(|m| m.get("todos"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        return;
    }
    let Some(runtime) = effective
        .get("runner")
        .and_then(|r| r.get("runtime"))
        .and_then(Value::as_str)
        .and_then(Runtime::named)
    else {
        return;
    };
    let domain = effective
        .get("workflow")
        .and_then(|w| w.get("domain"))
        .and_then(Value::as_str)
        .unwrap_or("sdlc");
    let dir = home.docs_dir().join(domain).join("workflow");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            out.push(KeyFinding::new(
                "workflow.domain",
                FindingKind::UnresolvedReference,
                format!("cannot read {}: {error}", dir.display()),
            ));
            return;
        }
    };
    for entry in entries {
        let result = (|| -> Result<(), String> {
            let file = entry.map_err(|e| e.to_string())?.path();
            if file.extension().is_none_or(|e| e != "yaml") {
                return Ok(());
            }
            let text =
                std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let value: Value =
                serde_yaml_ng::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
            let requires = value
                .get("requires")
                .ok_or_else(|| format!("{}: missing requires declaration", file.display()))?;
            let capabilities: Vec<super::WorkflowCapability> =
                serde_yaml_ng::from_value(requires.clone())
                    .map_err(|e| format!("{} requires: {e}", file.display()))?;
            for capability in capabilities {
                if !capability.supported_by(runtime) {
                    out.push(KeyFinding::new(
                        "modules.todos",
                        FindingKind::RuntimeCapabilityMissing,
                        format!(
                            "{} requires {capability}; runtime `{}` lacks this capability",
                            file.display(),
                            runtime.name()
                        ),
                    ));
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            out.push(KeyFinding::new(
                "workflow.domain",
                FindingKind::UnresolvedReference,
                error,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{COMPANY_SCHEMA_VERSION, GLOBAL_SCHEMA_VERSION};

    fn current_document(kind: LayerKind, settings: &str) -> Value {
        serde_yaml_ng::from_str(&format!(
            "schema_version: {}\n{settings}",
            kind.current_schema_version()
        ))
        .unwrap()
    }

    #[test]
    fn retired_runtime_configuration_is_unknown() {
        let doc = current_document(LayerKind::Global, "runtimes: []");
        let found = judge(&doc, LayerKind::Global, None, None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, FindingKind::InvalidValue);
    }
    #[test]
    fn t3_supports_todos_and_queued_event_delivery() {
        let doc = current_document(
            LayerKind::Global,
            "runner: {runtime: t3}\nmodules: {todos: true, events: true}\nredis: {cluster: {url: 'redis://127.0.0.1:6379'}}",
        );
        let found = judge(&doc, LayerKind::Global, None, None);
        assert!(found.is_empty());
    }
    #[test]
    fn a_declared_connector_needs_a_chosen_runtime_that_carries_conversations() {
        crate::config::_connectors::install_fixture();
        let company = current_document(
            LayerKind::Company,
            "company: acme\nconnectors: {slack: {app_token: '${A}', bot_token: '${B}'}}",
        );
        let found = |global: &Value| {
            judge(&company, LayerKind::Company, Some(global), None)
                .iter()
                .map(|f| (f.key.clone(), f.kind))
                .collect::<Vec<_>>()
        };
        // An unset runtime is the compiled one, which carries conversations.
        assert!(found(&current_document(LayerKind::Global, "ticks: 'on'")).is_empty());
        assert!(
            found(&current_document(
                LayerKind::Global,
                "runner: {runtime: t3}"
            ))
            .is_empty()
        );
    }
    #[test]
    fn references_still_detect_cycles_and_missing_keys() {
        let doc = current_document(LayerKind::Global, "runner: {notify: '{runner.notify}'}");
        assert!(
            judge(&doc, LayerKind::Global, None, None)
                .iter()
                .any(|f| f.kind == FindingKind::ReferenceCycle)
        );
    }

    #[test]
    fn named_connectors_default_kind_and_validate_settings() {
        crate::config::_connectors::install_fixture();
        let global = current_document(LayerKind::Global, "runner: {runtime: t3}");

        // Multiple named Slack connectors with explicit kind and policy settings.
        let valid = current_document(
            LayerKind::Company,
            r#"company: acme
connectors:
  slack:
    app_token: '${APP_A}'
    bot_token: '${BOT_A}'
  second:
    kind: slack
    role: company-agent
    admit: admins_only
    session_scope: company
    reply_mode: explicit
    app_token: '${APP_B}'
    bot_token: '${BOT_B}'
"#,
        );
        let findings = judge(&valid, LayerKind::Company, Some(&global), None);
        assert!(findings.is_empty(), "{findings:?}");

        let parsed = super::super::_company::connectors(valid.get("connectors")).unwrap();
        assert_eq!(parsed.len(), 2);
        let first = parsed.iter().find(|c| c.key == "slack").unwrap();
        assert_eq!(first.kind, "slack");
        assert_eq!(first.role.as_str(), "company-agent");
        assert_eq!(first.admit, super::super::ConnectorAdmit::Members);
        assert_eq!(
            first.session_scope,
            super::super::SessionScope::Conversation
        );
        assert_eq!(first.reply_mode, super::super::ReplyMode::TurnOutput);

        let second = parsed.iter().find(|c| c.key == "second").unwrap();
        assert_eq!(second.kind, "slack");
        assert_eq!(second.role.as_str(), "company-agent");
        assert_eq!(second.admit, super::super::ConnectorAdmit::AdminsOnly);
        assert_eq!(second.session_scope, super::super::SessionScope::Company);
        assert_eq!(second.reply_mode, super::super::ReplyMode::Explicit);

        // Unknown kind, invalid admit/session_scope/reply_mode are all reported at their paths.
        let invalid = current_document(
            LayerKind::Company,
            r#"company: acme
connectors:
  unknown-one:
    app_token: '${APP}'
  bad-opts:
    kind: slack
    admit: everyone
    session_scope: global
    reply_mode: silent
    app_token: '${APP}'
    bot_token: '${BOT}'
"#,
        );
        let keys: Vec<(String, FindingKind)> =
            judge(&invalid, LayerKind::Company, Some(&global), None)
                .into_iter()
                .map(|f| (f.key, f.kind))
                .collect();
        assert!(
            keys.contains(&("connectors.unknown-one".into(), FindingKind::UnknownKey)),
            "{keys:?}"
        );
        assert!(
            keys.contains(&(
                "connectors.bad-opts.admit".into(),
                FindingKind::InvalidValue
            )),
            "{keys:?}"
        );
        assert!(
            keys.contains(&(
                "connectors.bad-opts.session_scope".into(),
                FindingKind::InvalidValue
            )),
            "{keys:?}"
        );
        assert!(
            keys.contains(&(
                "connectors.bad-opts.reply_mode".into(),
                FindingKind::InvalidValue
            )),
            "{keys:?}"
        );
    }

    #[test]
    fn named_connector_secrets_and_roles_validate_against_home() {
        crate::config::_connectors::install_fixture();
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf()).for_writing();
        std::fs::create_dir_all(home.root()).unwrap();
        std::fs::write(
            home.global_config(),
            format!("schema_version: {GLOBAL_SCHEMA_VERSION}\nrunner: {{runtime: t3}}\n"),
        )
        .unwrap();
        let company = crate::ids::ProjectSlug::parse("acme").unwrap();
        let company_dir = home.company_dir(&company);
        std::fs::create_dir_all(&company_dir).unwrap();
        std::fs::write(
            company_dir.join(super::super::CONFIG_FILE_NAME),
            format!(
                r#"schema_version: {COMPANY_SCHEMA_VERSION}
company: acme
connectors:
  second:
    kind: slack
    role: nonexistent-role
    app_token: '${{MISSING_APP}}'
    bot_token: '${{MISSING_BOT}}'
"#
            ),
        )
        .unwrap();
        let findings = validate(&home, &Layer::Company(company)).unwrap();
        let keys: Vec<&str> = findings.iter().map(|f| f.key.as_str()).collect();
        assert!(
            keys.contains(&"connectors.second.app_token"),
            "expected missing secret at connectors.second.app_token, got {keys:?}"
        );
        assert!(
            keys.contains(&"connectors.second.bot_token"),
            "expected missing secret at connectors.second.bot_token, got {keys:?}"
        );
        assert!(
            keys.contains(&"connectors.second.role"),
            "expected unknown role finding at connectors.second.role, got {keys:?}"
        );
    }
}
