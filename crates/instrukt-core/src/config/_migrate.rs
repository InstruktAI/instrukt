//! One-way conversion of owned configuration: each document kind's older
//! schema versions into its current one, and the machine's old agent
//! selection into `agents.yaml`.
use super::{Agents, ConfigError, Layer, LayerKind, SHIPPED_AGENTS};
use crate::{fsx::atomic_write, home::Home};
use serde_yaml_ng::Value;

pub(super) fn selection(
    home: &Home,
    layer: &Layer,
    value: &mut Value,
    persist: bool,
) -> Result<Option<Value>, ConfigError> {
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
    if changed && persist {
        let _lock = crate::fsx::LockGuard::acquire(
            &home.locks_dir().join("agents.lock"),
            std::time::Duration::from_secs(10),
        )?;
        let text = super::render_yaml(&next).map_err(|e| invalid(e.to_string()))?;
        Agents::parse(&text)?;
        let dest = home.agents_file();
        if dest.try_exists().map_err(|e| invalid(e.to_string()))? {
            let existing = std::fs::read_to_string(&dest).map_err(|e| invalid(e.to_string()))?;
            // Do not discard either user's choice when both files already own selection.
            let _: Agents = Agents::parse(&existing)?;
            if serde_yaml_ng::from_str::<Value>(&existing).map_err(|e| invalid(e.to_string()))?
                != next
            {
                return Err(invalid(
                    "agents.yaml already exists with different selections; reconcile it with the old global selections before retrying",
                ));
            }
        } else {
            atomic_write(&dest, text.as_bytes())?;
        }
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
    use super::*;
    use crate::config::{GLOBAL_SCHEMA_VERSION, PROJECT_SCHEMA_VERSION};

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
}
