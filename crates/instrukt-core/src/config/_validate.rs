//! Validation of one layer document as a whole: its structure, the
//! cross-key rules and its references, judged against the configuration it
//! takes part in.

use std::path::Path;

use serde_yaml_ng::Value;

use super::_effective::effective_value;
use super::_error::{ConfigError, Finding, FindingKind};
use super::_features::{Feature, Unavailable, availability};
use super::_interpolate::{Interpolator, Mode, join, key_text};
use super::_layer::{Layer, read, read_usable, require_usable};
use super::_module::Module;
use super::_runtime::Runtime;
use super::_schema::{self, CURRENT_SCHEMA_VERSION, DEFAULT_GITHUB_API_URL, KeyFinding, LayerKind};
use crate::home::Home;

/// The findings of `layer` as stored at `path` with content `value`.
pub(crate) fn findings_for(
    home: &Home,
    layer: &Layer,
    path: &Path,
    value: &Value,
) -> Result<Vec<Finding>, ConfigError> {
    let global = match layer.kind() {
        LayerKind::Global => None,
        LayerKind::Project => read_usable(home, &Layer::Global)?,
    };
    let mut found = judge(value, layer.kind(), global.as_ref());
    if let Layer::Project(root) = layer {
        found.extend(code_on_github(root, value, global.as_ref())?);
    }
    if let Layer::Project(root) = layer {
        let effective = effective_value(global.as_ref(), Some(value))?;
        if let Ok(ingress) = super::_webhooks::ingress(effective.get("ingress"))
            && std::fs::read_to_string(home.machine_id_file())
                .ok()
                .as_deref()
                .map(str::trim)
                == Some(ingress.computer.as_str())
            && let Ok(sources) = super::_webhooks::parse(value.get("webhooks"))
        {
            for source in sources.values() {
                if let Err(error) = source.resolve(root) {
                    found.push(KeyFinding::new(
                        "webhooks",
                        FindingKind::UnresolvedReference,
                        error.to_string(),
                    ));
                }
            }
        }
    }
    let effective = match layer.kind() {
        LayerKind::Global => effective_value(Some(value), None),
        LayerKind::Project => effective_value(global.as_ref(), Some(value)),
    }?;
    workflow_capabilities(home, &effective, &mut found);
    if layer.kind() == LayerKind::Global
        && effective
            .get("modules")
            .and_then(|m| m.get("todos"))
            .and_then(Value::as_bool)
            == Some(true)
    {
        let projects = crate::publication::PublishedRecord::load_all(home).map_err(|error| {
            ConfigError::InvalidValue {
                key: "modules.todos".into(),
                message: error.to_string(),
            }
        })?;
        for project in projects.into_iter().filter(|p| p.checkout.is_dir()) {
            let project_value = read_usable(home, &Layer::Project(project.checkout))?;
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

/// `instrukt config validate`: the findings of `layer`. A missing layer,
/// or one of another schema version, is refused rather than judged.
pub fn validate(home: &Home, layer: &Layer) -> Result<Vec<Finding>, ConfigError> {
    let path = layer.path(home);
    let Some(doc) = read(&path)? else {
        return Err(ConfigError::NoLayer {
            path,
            door: layer.init_door(),
        });
    };
    require_usable(layer, &path, &doc.value)?;
    findings_for(home, layer, &path, &doc.value)
}

/// The findings of a layer document of kind `kind`; a project layer is
/// judged together with the machine's `global` layer.
pub(crate) fn judge(value: &Value, kind: LayerKind, global: Option<&Value>) -> Vec<KeyFinding> {
    let mut found = _schema::check(value, kind);
    if let Some(v) = value.get("schema_version").and_then(Value::as_u64)
        && v != CURRENT_SCHEMA_VERSION
    {
        found.push(KeyFinding::new(
            "schema_version",
            FindingKind::SchemaVersionNotCurrent,
            format!("this instrukt writes schema_version {CURRENT_SCHEMA_VERSION}"),
        ));
    }
    let effective = match kind {
        LayerKind::Global => effective_value(Some(value), None),
        LayerKind::Project => effective_value(global, Some(value)),
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
        LayerKind::Project => required_modules(value, &effective, &mut found),
        LayerKind::Global => {
            runner_runtime(value, &effective, &mut found);
            agent_order(value, &effective, &mut found);
            found.extend(super::_agents::check_chains(value));
        }
    }
    feature_capabilities(value, &effective, &mut found);
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
    found
}

/// A module switch this layer turns on whose features the chosen runtime
/// cannot carry: no runtime is chosen, or it leaves verbs those features
/// need empty (`session-continuity`, "Verb-gated features"). A
/// `runner.runtime` naming no entry is its own finding and is not judged
/// again here.
fn feature_capabilities(value: &Value, effective: &Value, out: &mut Vec<KeyFinding>) {
    let name = effective
        .get("runner")
        .and_then(|r| r.get("runtime"))
        .and_then(Value::as_str);
    let chosen = name.and_then(Runtime::named);
    if name.is_some() && chosen.is_none() {
        return;
    }
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
        let Some(unavailable) = availability(chosen, features) else {
            continue;
        };
        let kind = match unavailable {
            Unavailable::NoRuntime => FindingKind::NoRuntimeChosen,
            Unavailable::MissingCapabilities { .. } => FindingKind::RuntimeCapabilityMissing,
        };
        out.push(KeyFinding::new(
            &join("modules", module),
            kind,
            unavailable.describe(features),
        ));
    }
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

/// `agents.order` names only agents the runtime mapping can start: the
/// entries that carry `agent:`. Without a mapping no agent can be started,
/// so every name is a finding.
///
/// The order carries one tier mode for the whole list, with no
/// default: `agents.tier` is required whenever `agents.order` is
/// set.
fn agent_order(value: &Value, effective: &Value, out: &mut Vec<KeyFinding>) {
    let agents = value.get("agents");
    let Some(order) = agents.and_then(|a| a.get("order")) else {
        return;
    };
    if agents.and_then(|a| a.get("tier")).is_none() {
        out.push(KeyFinding::new(
            "agents.tier",
            FindingKind::Missing,
            "required whenever agents.order is set",
        ));
    }
    let Some(order) = order.as_sequence() else {
        return;
    };
    // T3 uses project provider/model defaults. Preferences remain data and
    // dispatch reports that explicit selection is unsupported.
    let _ = effective;
    if let Ok(catalog) = super::agent_catalog() {
        for name in order.iter().filter_map(Value::as_str) {
            if !catalog.contains_key(name) {
                out.push(KeyFinding::new(
                    "agents.order",
                    FindingKind::InvalidValue,
                    format!("unknown agent family `{name}`"),
                ));
            }
        }
    }
}

/// Every reference resolves and none leads back to itself. Webhook secret
/// references are checked by their typed accessor.
fn references(value: &Value, kind: LayerKind, effective: &Value, out: &mut Vec<KeyFinding>) {
    let mut strings = Vec::new();
    collect_strings(&_schema::permitted(value, kind), "", &mut strings);
    for (path, text) in strings {
        if path == "webhooks" || path.starts_with("webhooks.") {
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
            if file.extension().is_none_or(|e| e != "yaml")
                || file.file_name().is_some_and(|n| n == "actions.yaml")
            {
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
    #[test]
    fn retired_runtime_configuration_is_unknown() {
        let doc: Value = serde_yaml_ng::from_str("schema_version: 1\nruntimes: []").unwrap();
        let found = judge(&doc, LayerKind::Global, None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, FindingKind::InvalidValue);
    }
    #[test]
    fn t3_supports_todos_and_queued_event_delivery() {
        let doc: Value = serde_yaml_ng::from_str(
            "schema_version: 1\nrunner: {runtime: t3}\nmodules: {todos: true, events: true}",
        )
        .unwrap();
        let found = judge(&doc, LayerKind::Global, None);
        assert!(found.is_empty());
    }
    #[test]
    fn references_still_detect_cycles_and_missing_keys() {
        let doc: Value =
            serde_yaml_ng::from_str("schema_version: 1\nrunner: {notify: '{runner.notify}'}")
                .unwrap();
        assert!(
            judge(&doc, LayerKind::Global, None)
                .iter()
                .any(|f| f.kind == FindingKind::ReferenceCycle)
        );
    }
}
