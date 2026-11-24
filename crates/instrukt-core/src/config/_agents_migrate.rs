//! Older agents files are converted at the owned-file boundary only: a
//! version-one file's retired effort fields, then a version-two file's
//! `conversation` role, which version three names `company-agent`.
use super::ConfigError;
use serde_yaml_ng::Value;

/// The role name a version-two file gives conversation sessions.
const RETIRED_ROLE: &str = "conversation";
/// The name version three gives that role.
const RENAMED_ROLE: &str = "company-agent";

/// A file converted to the current version, with the version it had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Converted {
    pub(super) from: u64,
    pub(super) bytes: Vec<u8>,
}

/// `bytes` converted to the current version, or `None` when they need no
/// conversion. A newer file, or one this conversion cannot carry over
/// without choosing for the user, is refused and nothing is converted.
pub(super) fn current(bytes: &[u8]) -> Result<Option<Converted>, ConfigError> {
    let mut value: Value = serde_yaml_ng::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
    let found = value.get("schema_version").and_then(Value::as_u64);
    if let Some(found) = found
        && found > super::_agents::VERSION
    {
        return Err(ConfigError::NewerSchema {
            path: "agents.yaml".into(),
            found,
            current: super::_agents::VERSION,
        });
    }
    let retired = retire_session_recover(&mut value) | migrate_orchestrator_roles(&mut value);
    let from = match found {
        Some(from @ (1 | 2)) => from,
        Some(3) if retired => 3,
        _ => return Ok(None),
    };
    if from == 1 {
        efforts(&mut value)?;
    }
    if from < 3 {
        rename_role(&mut value)?;
    }
    value["schema_version"] = super::_agents::VERSION.into();
    let text = super::render_yaml(&value).map_err(|e| invalid(e.to_string()))?;
    super::Agents::parse(&text)?;
    let mut out = crate::modeline::leading(bytes)
        .map(|line| [line, b"\n"].concat())
        .unwrap_or_default();
    out.extend_from_slice(text.as_bytes());
    Ok(Some(Converted { from, bytes: out }))
}

/// Replace the retired split orchestrators ([`super::RETIRED_ORCHESTRATOR_ROLES`])
/// with the unified [`super::TODO_ORCHESTRATOR_ROLE`] entry, preserving customized
/// model/tier/app settings when the unified role is not already configured.
fn migrate_orchestrator_roles(value: &mut Value) -> bool {
    use super::{RETIRED_ORCHESTRATOR_ROLES as RETIRED, TODO_ORCHESTRATOR_ROLE as UNIFIED};
    let Some(roles) = value.get_mut("roles").and_then(Value::as_mapping_mut) else {
        return false;
    };
    let present: Vec<&Value> = RETIRED.iter().filter_map(|role| roles.get(role)).collect();
    if present.is_empty() {
        return false;
    }
    let has_unified = roles.contains_key(UNIFIED);
    if !has_unified && present.len() == RETIRED.len() && present.windows(2).any(|w| w[0] != w[1]) {
        tracing::warn!(
            event = "agents_orchestrator_migration_preference",
            "both prepare-orchestrator and work-orchestrator were customised; adopting work-orchestrator settings for todo-orchestrator"
        );
    }
    let preferred = present.first().copied().cloned();
    let mut inserted = has_unified;
    *roles = std::mem::take(roles)
        .into_iter()
        .filter_map(|(key, entry)| match key.as_str() {
            Some(role) if RETIRED.contains(&role) => {
                if !inserted {
                    inserted = true;
                    preferred
                        .clone()
                        .map(|unified| (Value::from(UNIFIED), unified))
                } else {
                    None
                }
            }
            _ => Some((key, entry)),
        })
        .collect();
    true
}

/// A removed command grants no continuing action. All other grants are retained.
pub(super) fn retire_session_recover(value: &mut Value) -> bool {
    fn strip(value: &mut Value) -> bool {
        let Some(grants) = value.as_sequence_mut() else {
            return false;
        };
        let before = grants.len();
        grants
            .retain(|grant| grant.get("action").and_then(Value::as_str) != Some("session-recover"));
        grants.len() != before
    }
    let mut changed = false;
    if let Some(can) = value
        .get_mut("default")
        .and_then(|entry| entry.get_mut("can"))
    {
        changed |= strip(can);
    }
    if let Some(roles) = value.get_mut("roles").and_then(Value::as_mapping_mut) {
        for entry in roles.values_mut() {
            if let Some(can) = entry.get_mut("can") {
                changed |= strip(can);
            }
        }
    }
    if let Some(bundles) = value.get_mut("bundles").and_then(Value::as_mapping_mut) {
        for grants in bundles.values_mut() {
            changed |= strip(grants);
        }
    }
    changed
}

/// Version one to two: retired per-kind effort spellings become `effort`.
fn efforts(value: &mut Value) -> Result<(), ConfigError> {
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
    Ok(())
}

/// Version two to three: `roles.conversation` becomes `roles.company-agent`
/// in its place, every field as it was. A file holding both is refused.
fn rename_role(value: &mut Value) -> Result<(), ConfigError> {
    let Some(roles) = value.get_mut("roles").and_then(Value::as_mapping_mut) else {
        return Ok(());
    };
    if !roles.contains_key(RETIRED_ROLE) {
        return Ok(());
    }
    if roles.contains_key(RENAMED_ROLE) {
        return Err(ConfigError::AgentsRoleConflict {
            retired: RETIRED_ROLE,
            current: RENAMED_ROLE,
        });
    }
    *roles = std::mem::take(roles)
        .into_iter()
        .map(|(key, entry)| {
            if key.as_str() == Some(RETIRED_ROLE) {
                (Value::from(RENAMED_ROLE), entry)
            } else {
                (key, entry)
            }
        })
        .collect();
    Ok(())
}
fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        key: "agents.yaml migration".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures")]
    use super::*;

    const PROFILES: &str = "profiles:\n  claude:\n    instance: claudeAgent\n    kind: claude\n    light: [{model: haiku, effort: high}]\n    standard: [{model: sonnet, effort: high}]\n    strong: [{model: opus, effort: high}]\n    premium: [{model: opus, effort: high}]\n  codex:\n    instance: codex\n    kind: codex\n    light: [{model: luna, EFFORT: high}]\n    standard: [{model: sol, EFFORT: high}]\n    strong: [{model: sol, EFFORT: high}]\n    premium: [{model: astra, EFFORT: high}]\n";

    /// An agents file of `version` whose `roles` are `roles`, with codex's
    /// effort spelled `effort` (`effort`, current) or `reasoningEffort`
    /// (retired in version two).
    fn file(version: u64, roles: &str, effort: &str) -> String {
        format!(
            "# yaml-language-server: $schema=agents.schema.json\nschema_version: {version}\ndefault: {{class: agent, can: [all], order: [claude], tier: standard}}\nroles:\n{roles}{}",
            PROFILES.replace("EFFORT", effort)
        )
    }

    const BEFORE_AND_AFTER: &str = "  builder: {class: focused, can: [read], order: [claude]}\n";
    const AFTER: &str = "  reviewer: {class: focused, can: [read], order: [codex]}\n";

    /// The `conversation` role entry as version two spells it.
    const CONVERSATION: &str =
        "  conversation: {class: standing, can: [read, tools], order: [claude], tier: strong}\n";
    /// A competing entry under the version-three name, with settings of its
    /// own: the class it shares, but other grants, order and tier.
    const COMPANY_AGENT: &str =
        "  company-agent: {class: standing, can: [read], order: [codex], tier: light}\n";
    /// What both older files convert to: the role renamed in its place,
    /// every field as it was, efforts in their current spelling.
    const CONVERTED: &str = "# yaml-language-server: $schema=agents.schema.json\nschema_version: 3\ndefault:\n  class: agent\n  can:\n  - all\n  order:\n  - claude\n  tier: standard\nroles:\n  builder:\n    class: focused\n    can:\n    - read\n    order:\n    - claude\n  company-agent:\n    class: standing\n    can:\n    - read\n    - tools\n    order:\n    - claude\n    tier: strong\n  reviewer:\n    class: focused\n    can:\n    - read\n    order:\n    - codex\nprofiles:\n  claude:\n    instance: claudeAgent\n    kind: claude\n    light:\n    - model: haiku\n      effort: high\n    standard:\n    - model: sonnet\n      effort: high\n    strong:\n    - model: opus\n      effort: high\n    premium:\n    - model: opus\n      effort: high\n  codex:\n    instance: codex\n    kind: codex\n    light:\n    - model: luna\n      effort: high\n    standard:\n    - model: sol\n      effort: high\n    strong:\n    - model: sol\n      effort: high\n    premium:\n    - model: astra\n      effort: high\n";

    #[test]
    fn a_version_two_file_names_its_conversation_role_company_agent() {
        let roles = format!("{BEFORE_AND_AFTER}{CONVERSATION}{AFTER}");
        let converted = current(file(2, &roles, "effort").as_bytes()).unwrap();
        assert_eq!(
            converted,
            Some(Converted {
                from: 2,
                bytes: CONVERTED.as_bytes().to_vec()
            })
        );
    }

    #[test]
    fn a_version_two_file_holding_both_role_entries_is_refused_and_not_converted() {
        let roles = format!("{BEFORE_AND_AFTER}{CONVERSATION}{COMPANY_AGENT}{AFTER}");
        let refused = current(file(2, &roles, "effort").as_bytes());
        assert!(
            matches!(
                refused,
                Err(ConfigError::AgentsRoleConflict {
                    retired: "conversation",
                    current: "company-agent"
                })
            ),
            "{refused:?}"
        );
    }

    #[test]
    fn a_version_one_file_reaches_version_three_through_both_steps() {
        let roles = format!("{BEFORE_AND_AFTER}{CONVERSATION}{AFTER}");
        let converted = current(file(1, &roles, "reasoningEffort").as_bytes()).unwrap();
        assert_eq!(
            converted,
            Some(Converted {
                from: 1,
                bytes: CONVERTED.as_bytes().to_vec()
            })
        );
    }

    #[test]
    fn a_current_file_needs_no_conversion() {
        let roles = format!("{BEFORE_AND_AFTER}{COMPANY_AGENT}{AFTER}");
        assert_eq!(current(file(3, &roles, "effort").as_bytes()).unwrap(), None);
    }

    #[test]
    fn split_orchestrator_roles_migrate_to_todo_orchestrator_preserving_work_settings() {
        let roles = format!(
            "{BEFORE_AND_AFTER}  prepare-orchestrator: {{class: agent, can: [read], order: [claude], tier: standard}}\n  work-orchestrator: {{class: agent, can: [read], order: [codex], tier: premium}}\n{AFTER}"
        );
        let converted = current(file(3, &roles, "effort").as_bytes())
            .unwrap()
            .expect("split orchestrators should trigger migration");
        let parsed: Value = serde_yaml_ng::from_slice(&converted.bytes).unwrap();
        let map = parsed["roles"].as_mapping().unwrap();
        assert!(!map.contains_key("prepare-orchestrator"));
        assert!(!map.contains_key("work-orchestrator"));
        let unified = map
            .get("todo-orchestrator")
            .expect("todo-orchestrator created");
        assert_eq!(unified["tier"].as_str(), Some("premium"));
        assert_eq!(unified["order"][0].as_str(), Some("codex"));
    }

    #[test]
    fn prepare_only_orchestrator_migrates_to_todo_orchestrator() {
        let roles = format!(
            "{BEFORE_AND_AFTER}  prepare-orchestrator: {{class: agent, can: [read], order: [claude], tier: strong}}\n{AFTER}"
        );
        let converted = current(file(3, &roles, "effort").as_bytes())
            .unwrap()
            .expect("prepare-orchestrator should migrate");
        let parsed: Value = serde_yaml_ng::from_slice(&converted.bytes).unwrap();
        let map = parsed["roles"].as_mapping().unwrap();
        assert!(!map.contains_key("prepare-orchestrator"));
        assert_eq!(map["todo-orchestrator"]["tier"].as_str(), Some("strong"));
    }
}
