//! One-way conversion of owned configuration into the current selection shape.
use super::{Agents, ConfigError, Layer, SHIPPED_AGENTS};
use crate::{fsx::atomic_write, home::Home};
use serde_yaml_ng::Value;

pub(super) fn selection(home: &Home, layer: &Layer, value: &mut Value) -> Result<(), ConfigError> {
    if !matches!(layer, Layer::Global) {
        return Ok(());
    }
    let Some(old) = value.get_mut("agents").and_then(Value::as_mapping_mut) else {
        return Ok(());
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
    // Role entries inherit machine choices migrated from the old global defaults.
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
            let option = match profile {
                "codex" => "reasoningEffort",
                "claude" => "effort",
                "antigravity" => "thinking_level",
                _ => {
                    return Err(invalid(format!(
                        "no migration for selection profile `{name}`"
                    )));
                }
            };
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
                if let Some(effort) = entry.remove("effort") {
                    entry.insert(option.into(), effort);
                }
                converted.push(Value::Mapping(entry));
            }
            next["profiles"][profile][tier] = Value::Sequence(converted);
            changed = true;
        }
    }
    if changed {
        let _lock = crate::fsx::LockGuard::acquire(
            &home.locks_dir().join("agents.lock"),
            std::time::Duration::from_secs(10),
        )?;
        if let Some(order) = next["default"]["order"].as_sequence_mut() {
            for item in order {
                if item.as_str() == Some("gemini") {
                    *item = "antigravity".into();
                }
            }
        }
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
    Ok(())
}
fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        key: "configuration migration".into(),
        message: message.into(),
    }
}
