//! Validation of one layer document as a whole: its structure, the
//! cross-key rules and its references, judged against the configuration it
//! takes part in.

use std::path::Path;

use serde_yaml_ng::Value;

use super::_effective::effective_value;
use super::_error::{ConfigError, Finding, FindingKind};
use super::_interpolate::{Interpolator, Mode, join, key_text};
use super::_layer::{Layer, read, read_usable, require_usable};
use super::_module::Module;
use super::_runtime;
use super::_schema::{self, CURRENT_SCHEMA_VERSION, KeyFinding, LayerKind};
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
    Ok(judge(value, layer.kind(), global.as_ref())
        .into_iter()
        .map(|finding| Finding {
            document: path.to_path_buf(),
            key: finding.key,
            kind: finding.kind,
            message: finding.message,
        })
        .collect())
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
    match kind {
        LayerKind::Project => required_modules(value, &effective, &mut found),
        LayerKind::Global => {
            runner_runtime(value, &effective, &mut found);
            agent_order(value, &effective, &mut found);
        }
    }
    references(value, kind, &effective, &mut found);
    found
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

/// `runner.runtime` names an entry of the runtime mapping.
fn runner_runtime(value: &Value, effective: &Value, out: &mut Vec<KeyFinding>) {
    let Some(name) = value
        .get("runner")
        .and_then(|r| r.get("runtime"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let entries = _runtime::entries(effective.get("runtimes"));
    if !entries.iter().any(|entry| entry.name == name) {
        out.push(KeyFinding::new(
            "runner.runtime",
            FindingKind::UnknownRuntime,
            format!("names no runtime of the runtime mapping: `{name}`"),
        ));
    }
}

/// `agents.order` names only agents the runtime mapping can start: the
/// entries that carry `agent:`. Without a mapping no agent can be started,
/// so every name is a finding.
///
/// The order carries one thinking mode for the whole list, with no
/// default: `agents.thinking_mode` is required whenever `agents.order` is
/// set.
fn agent_order(value: &Value, effective: &Value, out: &mut Vec<KeyFinding>) {
    let agents = value.get("agents");
    let Some(order) = agents.and_then(|a| a.get("order")) else {
        return;
    };
    if agents.and_then(|a| a.get("thinking_mode")).is_none() {
        out.push(KeyFinding::new(
            "agents.thinking_mode",
            FindingKind::Missing,
            "required whenever agents.order is set",
        ));
    }
    let Some(order) = order.as_sequence() else {
        return;
    };
    let entries = _runtime::entries(effective.get("runtimes"));
    let startable: Vec<&str> = entries.iter().filter_map(|e| e.agent.as_deref()).collect();
    for (i, item) in order.iter().enumerate() {
        let Some(name) = item.as_str() else { continue };
        if !startable.contains(&name) {
            out.push(KeyFinding::new(
                &join("agents.order", &i.to_string()),
                FindingKind::AgentNotStartable,
                format!("no runtime-mapping entry starts agent `{name}`"),
            ));
        }
    }
}

/// Every reference resolves and none leads back to itself, whichever
/// runtime of the mapping is active.
fn references(value: &Value, kind: LayerKind, effective: &Value, out: &mut Vec<KeyFinding>) {
    let entries = _runtime::entries(effective.get("runtimes"));
    let candidates: Vec<Option<usize>> =
        match effective.get("runtimes").and_then(Value::as_sequence) {
            Some(seq) if !seq.is_empty() => (0..seq.len()).map(Some).collect(),
            _ => vec![None],
        };
    let mut strings = Vec::new();
    collect_strings(&_schema::permitted(value, kind), "", &mut strings);
    for (path, text) in strings {
        let error = candidates.iter().find_map(|active| {
            Interpolator::new(effective, &entries, Mode::Check(*active))
                .expand_text(&text, Some(&path))
                .err()
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    fn judged(findings: &[KeyFinding]) -> Vec<(&str, FindingKind)> {
        findings.iter().map(|f| (f.key.as_str(), f.kind)).collect()
    }

    const RUNTIMES: &str = "runtimes:\n\
        \x20 - {name: host, session_id_env: H, start: s, send: s, alive: s, conversation: c}\n\
        \x20 - {name: cli, agent: claude, session_id_env: C, start: s, send: s, alive: s, conversation: c}\n";

    #[test]
    fn the_order_names_only_agents_the_mapping_can_start() {
        let global = yaml(&format!(
            "schema_version: 1\n{RUNTIMES}agents: {{order: [claude, codex], thinking_mode: med}}\n"
        ));
        assert_eq!(
            judged(&judge(&global, LayerKind::Global, None)),
            [("agents.order.1", FindingKind::AgentNotStartable)]
        );
        let unmapped = yaml("schema_version: 1\nagents: {order: [claude], thinking_mode: fast}\n");
        assert_eq!(
            judged(&judge(&unmapped, LayerKind::Global, None)),
            [("agents.order.0", FindingKind::AgentNotStartable)]
        );
    }

    #[test]
    fn an_order_without_a_thinking_mode_is_a_finding() {
        let global = yaml(&format!(
            "schema_version: 1\n{RUNTIMES}agents: {{order: [claude]}}\n"
        ));
        assert_eq!(
            judged(&judge(&global, LayerKind::Global, None)),
            [("agents.thinking_mode", FindingKind::Missing)]
        );
        let no_order = yaml("schema_version: 1\nagents: {claude: {git_author: 'C <c@x>'}}\n");
        assert_eq!(judge(&no_order, LayerKind::Global, None), []);
    }

    #[test]
    fn a_required_module_must_be_on_under_the_effective_switches() {
        let global = yaml("schema_version: 1\nmodules: {todos: true, events: true}\n");
        let project = yaml(
            "schema_version: 1\nproject: p\nrequires: [todos, events, blueprint]\nmodules: {events: false}\n",
        );
        assert_eq!(
            judged(&judge(&project, LayerKind::Project, Some(&global))),
            [
                ("requires.1", FindingKind::ModuleSwitchedOff),
                ("requires.2", FindingKind::ModuleSwitchedOff),
            ]
        );
    }

    #[test]
    fn the_runner_runtime_must_name_a_mapped_runtime() {
        let global = yaml(&format!(
            "schema_version: 1\n{RUNTIMES}runner: {{runtime: gone}}\n"
        ));
        assert_eq!(
            judged(&judge(&global, LayerKind::Global, None)),
            [("runner.runtime", FindingKind::UnknownRuntime)]
        );
    }

    #[test]
    fn unresolved_and_cyclic_references_are_findings() {
        let global = yaml(&format!("schema_version: 1\n{RUNTIMES}"));
        let project = yaml(
            "schema_version: 1\nproject: p\nactions:\n  ok: 'make {project} ${HOME} {runtime.session_id}'\n  \
             lost: '{nowhere.key}'\n  a: '{actions.b}'\n  b: 'x {actions.a}'\n",
        );
        assert_eq!(
            judged(&judge(&project, LayerKind::Project, Some(&global))),
            [
                ("actions.lost", FindingKind::UnresolvedReference),
                ("actions.a", FindingKind::ReferenceCycle),
                ("actions.b", FindingKind::ReferenceCycle),
            ]
        );
    }

    #[test]
    fn a_runtime_template_referring_to_itself_through_the_runtime_is_a_cycle() {
        let global = yaml(
            "schema_version: 1\nruntimes:\n\
             \x20 - {name: a, session_id_env: A, start: s, send: s, alive: s, conversation: '{runtime.conversation}'}\n",
        );
        assert_eq!(
            judged(&judge(&global, LayerKind::Global, None)),
            [("runtimes.0.conversation", FindingKind::ReferenceCycle)]
        );
    }

    #[test]
    fn a_version_other_than_the_current_one_is_a_finding() {
        let doc = yaml("schema_version: 7\n");
        assert_eq!(
            judged(&judge(&doc, LayerKind::Global, None)),
            [("schema_version", FindingKind::SchemaVersionNotCurrent)]
        );
    }
}
