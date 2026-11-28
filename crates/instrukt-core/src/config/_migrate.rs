//! One-way conversion of owned configuration: each document kind's older
//! schema versions into its current one, and the machine's old agent
//! selection into `agents.yaml`.
use super::{Agents, ConfigError, Layer, LayerKind, SHIPPED_AGENTS};
use crate::{
    fsx::{LockGuard, atomic_write},
    home::Home,
};
use serde_yaml_ng::Value;

pub(super) fn selection(layer: &Layer, value: &mut Value) -> Result<Option<Value>, ConfigError> {
    if !matches!(layer, Layer::Global) {
        return Ok(None);
    }
    let Some(old) = value.get_mut("agents").and_then(Value::as_mapping_mut) else {
        return Ok(None);
    };
    let mut next: Value =
        serde_yaml_ng::from_str(SHIPPED_AGENTS).map_err(|e| invalid(e.to_string()))?;
    let mut changed = false;
    for key in ["order", "tier"] {
        if let Some(v) = old.get(key) {
            next["default"][key] = v.clone();
            changed = true;
        }
    }
    // A migrated machine-wide order or tier leaves the file naming no role,
    // so every shipped role keeps its shipped order and tier and the
    // migrated ones reach only roles without a shipped entry. A profile
    // chain migrated below reaches every role that selects that profile and
    // tier.
    if changed {
        next["roles"] = Value::Mapping(Default::default());
    }
    for (name, fields) in old.iter() {
        let Some(name) = name.as_str() else {
            continue;
        };
        if matches!(name, "order" | "tier") {
            continue;
        }
        for tier in ["light", "standard", "strong", "premium"] {
            let Some(chain) = fields.get(tier) else {
                continue;
            };
            let profile = if name == "gemini" {
                "antigravity"
            } else {
                name
            };
            if !matches!(profile, "codex" | "claude" | "antigravity") {
                return Err(invalid(format!(
                    "no migration for selection profile `{name}`"
                )));
            }
            let entries = chain
                .as_sequence()
                .ok_or_else(|| invalid("model chain is not a sequence"))?;
            let mut converted = Vec::new();
            for entry in entries {
                let mut entry = entry
                    .as_mapping()
                    .cloned()
                    .ok_or_else(|| invalid("model entry is not a mapping"))?;
                if let Some(options) = entry.remove("options") {
                    let options = options
                        .as_mapping()
                        .ok_or_else(|| invalid("model options are not a mapping"))?;
                    entry.extend(options.clone());
                }
                converted.push(Value::Mapping(entry));
            }
            next["profiles"][profile][tier] = Value::Sequence(converted);
            changed = true;
        }
    }
    if changed {
        if let Some(order) = next["default"]["order"].as_sequence_mut() {
            for item in order {
                if item.as_str() == Some("gemini") {
                    *item = "antigravity".into();
                }
            }
        }
        Agents::parse(&super::render_yaml(&next).map_err(|e| invalid(e.to_string()))?)?;
    }
    old.remove("order");
    old.remove("tier");
    for fields in old.values_mut().filter_map(Value::as_mapping_mut) {
        for tier in ["light", "standard", "strong", "premium"] {
            fields.remove(tier);
        }
    }
    old.retain(|_, v| v.as_mapping().is_none_or(|m| !m.is_empty()));
    Ok(changed.then_some(next))
}

/// The selection carried over from an older global layer, if any.
pub(super) fn carried_selection(global: Option<&Value>) -> Result<Option<Value>, ConfigError> {
    let Some(value) = global.filter(|value| {
        value
            .get("schema_version")
            .and_then(Value::as_u64)
            .is_some_and(|v| v < super::GLOBAL_SCHEMA_VERSION)
    }) else {
        return Ok(None);
    };
    let mut value = value.clone();
    selection(&Layer::Global, &mut value)
}

/// Carries `carried` into `existing` agents-file bytes: renders `carried` when
/// no file exists, keeps `existing` when structurally equal, or refuses when
/// the two disagree.
pub(super) fn carry_into(
    existing: Option<Vec<u8>>,
    carried: &Value,
) -> Result<Vec<u8>, ConfigError> {
    if let Some(bytes) = existing {
        let text = std::str::from_utf8(&bytes).map_err(|_| invalid("expected UTF-8"))?;
        // Do not discard either user's choice when both files already own selection.
        let _: Agents = Agents::parse(text)?;
        if serde_yaml_ng::from_slice::<Value>(&bytes).map_err(|e| invalid(e.to_string()))?
            != *carried
        {
            return Err(invalid(
                "agents.yaml already exists with different selections; reconcile it with the old global selections before retrying",
            ));
        }
        return Ok(bytes);
    }
    let text = super::render_yaml(carried).map_err(|e| invalid(e.to_string()))?;
    let _: Agents = Agents::parse(&text)?;
    Ok(text.into_bytes())
}

/// Extracts any legacy selection from `value` and persists it into `agents.yaml`
/// under `agents.lock` (reusing `agents_held` when the caller already holds it).
pub(super) fn carry_over(
    home: &Home,
    layer: &Layer,
    value: &mut Value,
    agents_held: Option<&LockGuard>,
) -> Result<(), ConfigError> {
    let Some(carried) = selection(layer, value)? else {
        return Ok(());
    };
    let _acquired = match agents_held {
        Some(_) => None,
        None => Some(LockGuard::acquire(
            &super::_agents_file::agents_lock(home),
            std::time::Duration::from_secs(10),
        )?),
    };
    let dest = home.agents_file();
    let existing = super::_agents_file::read_existing(&dest)?;
    let absent = existing.is_none();
    let bytes = carry_into(existing, &carried)?;
    if absent {
        atomic_write(&dest, &bytes)?;
    }
    Ok(())
}
/// `value`, a document of `kind` at schema version `from`, converted to its
/// kind's current version: retired settings removed, every other setting
/// kept, and the kind's version stamped.
pub(super) fn upcycle(value: Value, from: u64, kind: LayerKind) -> Value {
    let mut value = match kind {
        LayerKind::Global => global(value, from),
        LayerKind::Project => project(value, from),
        LayerKind::Company => company(value, from),
    };
    if let Value::Mapping(map) = &mut value {
        map.insert(
            Value::String("schema_version".to_owned()),
            Value::Number(kind.current_schema_version().into()),
        );
    }
    value
}

fn global(mut value: Value, from: u64) -> Value {
    if from < 6
        && let Some(Value::Mapping(modules)) = value.get_mut("modules")
    {
        // A declared connector gives a company conversations; the
        // machine's switch is gone, whatever its value.
        modules.remove(super::_schema::CONVERSATIONS);
    }
    if from < 7
        && let Some(Value::Mapping(runtimes)) = value.get_mut("runtimes")
    {
        // An entry or a `runtimes` map this removal empties goes with it;
        // one that is not a mapping is left for validation to report.
        let mut emptied = false;
        runtimes.retain(|_, entry| {
            let Value::Mapping(actions) = entry else {
                return true;
            };
            // `retire_thread` is the runtime action global schema 6 accepted
            // and the current catalog does not define, whatever it holds later.
            let removed = actions.remove("retire_thread").is_some();
            let keep = !(removed && actions.is_empty());
            emptied |= !keep;
            keep
        });
        if emptied
            && runtimes.is_empty()
            && let Value::Mapping(map) = &mut value
        {
            map.remove("runtimes");
        }
    }
    value
}

fn project(mut value: Value, from: u64) -> Value {
    if from < 3
        && let Value::Mapping(map) = &mut value
    {
        map.remove("actions");
    }
    value
}

fn company(mut value: Value, from: u64) -> Value {
    if from < 6
        && let Some(Value::Mapping(members)) = value.get_mut("members")
    {
        // A member entry no longer names a connector identity; an entry
        // left without an email address stays a finding.
        for (_, entry) in members.iter_mut() {
            if let Value::Mapping(fields) = entry {
                // The connectors that existed when schema 6 was written,
                // whatever this binary supports later.
                const V6_CONNECTOR_KEYS: [&str; 1] = ["slack"];
                for connector in V6_CONNECTOR_KEYS {
                    fields.remove(connector);
                }
            }
        }
    }
    value
}

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        key: "configuration migration".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::upcycle;
    use crate::config::{GLOBAL_SCHEMA_VERSION, LayerKind, PROJECT_SCHEMA_VERSION};
    use serde_yaml_ng::Value;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    /// A global schema-6 document converted to the current global schema.
    fn global_6(text: &str) -> Value {
        upcycle(
            yaml(&format!("schema_version: 6\n{text}")),
            6,
            LayerKind::Global,
        )
    }

    fn current_global(text: &str) -> Value {
        yaml(&format!("schema_version: {GLOBAL_SCHEMA_VERSION}\n{text}"))
    }

    #[test]
    fn a_global_schema_6_runtime_entry_loses_the_retired_action_and_keeps_the_rest() {
        assert_eq!(
            global_6(
                "ticks: 'off'\nrunner: {runtime: t3}\nruntimes:\n  t3:\n    finish_thread: Settle it.\n    retire_thread: Archive it.\n"
            ),
            current_global(
                "ticks: 'off'\nrunner: {runtime: t3}\nruntimes:\n  t3:\n    finish_thread: Settle it.\n"
            ),
        );
    }

    #[test]
    fn a_global_schema_6_runtimes_map_the_removal_empties_goes() {
        assert_eq!(
            global_6("ticks: 'off'\nruntimes:\n  t3:\n    retire_thread: Archive it.\n"),
            current_global("ticks: 'off'\n"),
        );
    }

    #[test]
    fn only_the_runtime_entry_the_removal_empties_goes() {
        assert_eq!(
            global_6(
                "runtimes:\n  t3:\n    retire_thread: Archive it.\n  other:\n    finish_thread: Settle it.\n    retire_thread: Archive it.\n"
            ),
            current_global("runtimes:\n  other:\n    finish_thread: Settle it.\n"),
        );
    }

    #[test]
    fn a_project_schema_6_document_is_already_current() {
        let text = format!(
            "schema_version: {PROJECT_SCHEMA_VERSION}\nproject: p\nruntimes:\n  t3:\n    retire_thread: Archive it.\n"
        );
        assert_eq!(
            upcycle(yaml(&text), PROJECT_SCHEMA_VERSION, LayerKind::Project),
            yaml(&text),
        );
    }
    fn carried_example() -> Value {
        let mut legacy = yaml("schema_version: 1\nagents: {order: [codex, claude]}\n");
        super::selection(&super::Layer::Global, &mut legacy)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn a_carried_selection_is_rendered_into_an_absent_file() {
        let carried = carried_example();
        let bytes = super::carry_into(None, &carried).unwrap();
        let parsed: Value = serde_yaml_ng::from_slice(&bytes).unwrap();
        assert_eq!(parsed, carried);
    }

    #[test]
    fn an_agents_file_equal_to_the_carried_selection_is_kept_as_written() {
        let carried = carried_example();
        let bytes = format!(
            "# operator note\n{}",
            super::super::render_yaml(&carried).unwrap()
        )
        .into_bytes();
        let kept = super::carry_into(Some(bytes.clone()), &carried).unwrap();
        assert_eq!(kept, bytes);
    }

    #[test]
    fn an_agents_file_differing_from_the_carried_selection_is_refused() {
        let carried = carried_example();
        let differing = format!(
            "schema_version: {}\nroles: {{builder: {{tier: light}}}}\n",
            super::super::_agents::VERSION
        )
        .into_bytes();
        let err = super::carry_into(Some(differing), &carried).unwrap_err();
        assert!(
            matches!(err, super::ConfigError::InvalidValue { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn golden_v7_global_document_migrates_losslessly_to_v8_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let home = crate::home::Home::at_user_home(dir.path().to_path_buf()).for_inspection();
        let golden_v7 = "\
schema_version: 7
ticks: 'off'
modules:
  todos: true
  events: false
  history: true
runner:
  runtime: t3
  notify: echo notify
  archive_finished_after: 14d
github:
  api_url: https://api.github.com
";
        crate::fsx::atomic_write(&home.global_config(), golden_v7.as_bytes()).unwrap();
        let view = crate::config::inspect_layer(&home, &crate::config::Layer::Global).unwrap();
        assert_eq!(view.schema_version, Some(7));
        assert!(view.migration.is_some());
        let cfg = crate::config::Configuration::load_global(&home).unwrap();
        assert!(!cfg.ticks_enabled().unwrap());
        assert!(cfg.module_enabled(crate::config::Module::Todos).unwrap());
        assert_eq!(cfg.runner_runtime().unwrap(), crate::config::Runtime::T3);
        assert!(cfg.backup().unwrap().is_none());
        let upcycled = upcycle(yaml(golden_v7), 7, LayerKind::Global);
        assert_eq!(
            upcycled["schema_version"].as_u64(),
            Some(GLOBAL_SCHEMA_VERSION)
        );
        assert_eq!(
            upcycled["runner"]["archive_finished_after"].as_str(),
            Some("14d")
        );
    }

    #[test]
    fn golden_v8_global_document_migrates_losslessly_to_v9_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let home = crate::home::Home::at_user_home(dir.path().to_path_buf()).for_inspection();
        let recipient = age::x25519::Identity::generate().to_public().to_string();
        let golden_v8 = format!(
            "\
schema_version: 8
ticks: 'off'
modules:
  todos: true
  events: false
  history: true
runner:
  runtime: t3
  notify: echo notify
  archive_finished_after: 14d
github:
  api_url: https://api.github.com
backup:
  endpoint: https://s3.example.test
  region: auto
  bucket: machine-backups
  access_key_id: ${{BACKUP_KEY}}
  secret_access_key: ${{BACKUP_SECRET}}
  age_recipient: {recipient}
"
        );
        crate::fsx::atomic_write(&home.global_config(), golden_v8.as_bytes()).unwrap();
        let view = crate::config::inspect_layer(&home, &crate::config::Layer::Global).unwrap();
        assert_eq!(view.schema_version, Some(8));
        assert!(view.migration.is_some());
        let cfg = crate::config::Configuration::load_global(&home).unwrap();
        assert!(!cfg.ticks_enabled().unwrap());
        assert!(cfg.module_enabled(crate::config::Module::Todos).unwrap());
        assert_eq!(cfg.runner_runtime().unwrap(), crate::config::Runtime::T3);
        assert!(cfg.llm_screening().unwrap().is_none());
        assert!(matches!(
            crate::config::inspect_global_screening(&home).unwrap(),
            crate::config::LlmAdmission::Unconfigured
        ));
        let upcycled = upcycle(yaml(&golden_v8), 8, LayerKind::Global);
        assert_eq!(
            upcycled["schema_version"].as_u64(),
            Some(GLOBAL_SCHEMA_VERSION)
        );
        assert_eq!(
            upcycled["backup"]["endpoint"].as_str(),
            Some("https://s3.example.test")
        );
        assert!(upcycled.get("llm").is_none());
    }
}
