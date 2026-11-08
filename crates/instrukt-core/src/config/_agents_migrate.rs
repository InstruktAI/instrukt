//! Version-one agents fields are converted at the owned-file boundary only.
use super::ConfigError;
use serde_yaml_ng::Value;

pub(super) fn current(bytes: &[u8]) -> Result<Option<Vec<u8>>, ConfigError> {
    let mut value: Value = serde_yaml_ng::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
    if let Some(found) = value.get("schema_version").and_then(Value::as_u64)
        && found > 2
    {
        return Err(ConfigError::NewerSchema {
            path: "agents.yaml".into(),
            found,
            current: 2,
        });
    }
    if value.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Ok(None);
    }
    if let Some(profiles) = value.get_mut("profiles").and_then(Value::as_mapping_mut) {
        for profile in profiles.values_mut() {
            let kind = profile
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // These spellings describe the retired v1 file, not runtime translation.
            let retired = match kind {
                "codex" | "grok" | "kiro" | "muse" => Some("reasoningEffort"),
                "antigravity" => Some("thinking_level"),
                "pi" => Some("thinking"),
                "opencode" => Some("variant"),
                "cursor" => Some("reasoning"),
                _ => None,
            };
            for tier in ["light", "standard", "strong", "premium"] {
                if let Some(chain) = profile.get_mut(tier).and_then(Value::as_sequence_mut) {
                    for entry in chain {
                        if let Some(fields) = entry.as_mapping_mut()
                            && let Some(retired) = retired
                            && let Some(effort) = fields.remove(retired)
                        {
                            if fields
                                .get("effort")
                                .is_some_and(|existing| existing != &effort)
                            {
                                return Err(invalid(
                                    "conflicting old effort fields; reconcile them before conversion",
                                ));
                            }
                            fields.insert("effort".into(), effort);
                        }
                    }
                }
            }
        }
    }
    value["schema_version"] = 2.into();
    let text = super::render_yaml(&value).map_err(|e| invalid(e.to_string()))?;
    super::Agents::parse(&text)?;
    let mut out = crate::modeline::leading(bytes)
        .map(|line| [line, b"\n"].concat())
        .unwrap_or_default();
    out.extend_from_slice(text.as_bytes());
    Ok(Some(out))
}
fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        key: "agents.yaml migration".into(),
        message: message.into(),
    }
}
