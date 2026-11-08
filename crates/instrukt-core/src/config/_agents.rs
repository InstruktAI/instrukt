//! Role-keyed model choices, validated independently of provider model catalogs.
use super::{ConfigError, Tier};
use crate::{
    fsx::{LockGuard, atomic_write},
    home::Home,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};

pub const SHIPPED_AGENTS: &str = include_str!("shipped/agents.yaml");
const VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSelection {
    pub profile: String,
    pub kind: HarnessKind,
    pub instance: String,
    pub model: String,
    pub effort: Option<String>,
    pub options: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessKind {
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "claude")]
    Claude,
    #[serde(rename = "antigravity")]
    Antigravity,
    #[serde(rename = "cursor")]
    Cursor,
    #[serde(rename = "grok")]
    Grok,
    #[serde(rename = "fx")]
    Fx,
    #[serde(rename = "opencode")]
    OpenCode,
    #[serde(rename = "kiro")]
    Kiro,
    #[serde(rename = "pi")]
    Pi,
    #[serde(rename = "muse")]
    Muse,
    #[serde(rename = "acpRegistry")]
    AcpRegistry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleSelection {
    pub order: Option<Vec<String>>,
    pub tier: Option<Tier>,
    // These values belong to the roles/App slice; selection neither parses
    // nor interprets their contents, and retains the single shared role map.
    #[serde(flatten)]
    pub other: BTreeMap<String, serde_yaml_ng::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub instance: String,
    pub kind: HarnessKind,
    pub light: Vec<serde_json::Value>,
    pub standard: Vec<serde_json::Value>,
    pub strong: Vec<serde_json::Value>,
    pub premium: Vec<serde_json::Value>,
}
impl Profile {
    fn chain(&self, tier: Tier) -> &[serde_json::Value] {
        match tier {
            Tier::Light => &self.light,
            Tier::Standard => &self.standard,
            Tier::Strong => &self.strong,
            Tier::Premium => &self.premium,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agents {
    pub schema_version: u64,
    pub default: RoleSelection,
    #[serde(default)]
    pub roles: BTreeMap<String, RoleSelection>,
    pub profiles: BTreeMap<String, Profile>,
    #[serde(default, skip_serializing_if = "serde_yaml_ng::Value::is_null")]
    pub bundles: serde_yaml_ng::Value,
}

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        key: "agents.yaml".into(),
        message: message.into(),
    }
}

impl Agents {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let value: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(text).map_err(|e| invalid(e.to_string()))?;
        if let Some(found) = value
            .get("schema_version")
            .and_then(serde_yaml_ng::Value::as_u64)
            && found > VERSION
        {
            return Err(ConfigError::NewerSchema {
                path: "agents.yaml".into(),
                found,
                current: VERSION,
            });
        }
        let result: Self = serde_yaml_ng::from_value(value).map_err(|e| invalid(e.to_string()))?;
        if result.schema_version != VERSION {
            return Err(invalid(
                "unsupported agents schema_version; upgrade instrukt",
            ));
        }
        if result.default.order.is_none() || result.default.tier.is_none() {
            return Err(invalid("default requires order and tier"));
        }
        for entry in std::iter::once(&result.default).chain(result.roles.values()) {
            if entry
                .other
                .keys()
                .any(|key| !matches!(key.as_str(), "class" | "can" | "github_app"))
            {
                return Err(invalid("unknown role selection field"));
            }
            if let Some(order) = &entry.order
                && (order.is_empty() || order.iter().any(|p| !result.profiles.contains_key(p)))
            {
                return Err(invalid(
                    "orders must be nonempty and reference declared profiles",
                ));
            }
        }
        for (name, profile) in &result.profiles {
            if name.trim().is_empty() || profile.instance.trim().is_empty() {
                return Err(invalid("profile and instance names must be nonempty"));
            }
            for tier in [Tier::Light, Tier::Standard, Tier::Strong, Tier::Premium] {
                if profile.chain(tier).is_empty() {
                    return Err(invalid(format!("{name}.{} chain is empty", tier.as_str())));
                }
                for entry in profile.chain(tier) {
                    resolve_entry(name, profile, entry)?;
                }
            }
        }
        Ok(result)
    }

    /// Deploy or upgrade defaults, retaining edits against the last deployed
    /// digest. An announcement is returned when new defaults cannot replace edits.
    pub fn deploy(home: &Home, defaults: &str) -> Result<Option<String>, ConfigError> {
        Self::parse(defaults)?;
        let _lock = LockGuard::acquire(
            &home.locks_dir().join("agents.lock"),
            Duration::from_secs(10),
        )?;
        let path = home.agents_file();
        let state = home.agents_deployment_file();
        let digest = |bytes: &[u8]| hex::encode(Sha256::digest(bytes));
        let new_digest = digest(defaults.as_bytes());
        let current = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => return Err(ConfigError::Read { path, source }),
        };
        let prior: Option<Deployment> = match std::fs::read(&state) {
            Ok(bytes) => Some(
                serde_json::from_slice(&bytes)
                    .map_err(|_| invalid("invalid defaults deployment record"))?,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(ConfigError::Read {
                    path: state,
                    source,
                });
            }
        };
        if prior.as_ref().is_some_and(|p| p.schema_version != VERSION) {
            return Err(invalid("unsupported defaults deployment schema"));
        }
        let unedited = current.as_ref().is_none_or(|b| {
            prior.as_ref().is_some_and(|p| {
                p.digest == digest(b) || p.pending.as_deref() == Some(digest(b).as_str())
            })
        });
        if unedited {
            if current.as_deref() != Some(defaults.as_bytes()) {
                let prepared = Deployment {
                    schema_version: VERSION,
                    digest: prior.as_ref().map(|p| p.digest.clone()).unwrap_or_default(),
                    pending: Some(new_digest.clone()),
                };
                atomic_write(
                    &state,
                    &serde_json::to_vec(&prepared).map_err(|e| invalid(e.to_string()))?,
                )?;
                atomic_write(&path, defaults.as_bytes())?;
            }
            let record = serde_json::to_vec(&Deployment {
                schema_version: VERSION,
                digest: new_digest,
                pending: None,
            })
            .map_err(|e| invalid(e.to_string()))?;
            atomic_write(&state, &record)?;
            Ok(None)
        } else if prior.as_ref().is_none_or(|p| p.digest != new_digest) {
            Ok(Some(format!(
                "{}: user edits retained; new agents defaults are available",
                path.display()
            )))
        } else {
            Ok(None)
        }
    }

    pub fn load(home: &Home) -> Result<Self, ConfigError> {
        super::_layer::migrate_if_needed(home, &super::Layer::Global)?;
        if let Some(notice) = Self::deploy(home, SHIPPED_AGENTS)? {
            tracing::warn!(event="agents_defaults_available", message=%notice);
        }
        let path = home.agents_file();
        let text =
            std::fs::read_to_string(&path).map_err(|source| ConfigError::Read { path, source })?;
        Self::parse(&text)
    }

    pub fn undeclared_roles<'a>(&'a self, installed: &[&str]) -> Vec<&'a str> {
        self.roles
            .keys()
            .map(String::as_str)
            .filter(|r| !matches!(*r, "default" | "event-handler") && !installed.contains(r))
            .collect()
    }

    pub fn order(
        &self,
        role: Option<&str>,
        task_profile: Option<&str>,
        task_tier: Option<Tier>,
    ) -> Result<Vec<(String, Tier)>, ConfigError> {
        let entry = role
            .and_then(|r| self.roles.get(r))
            .unwrap_or(&self.default);
        let order = entry
            .order
            .as_ref()
            .or(self.default.order.as_ref())
            .ok_or_else(|| invalid("missing default order"))?;
        let tier = task_tier
            .or(entry.tier)
            .or(self.default.tier)
            .ok_or_else(|| invalid("missing default tier"))?;
        let mut names = Vec::new();
        for name in task_profile
            .into_iter()
            .chain(order.iter().map(String::as_str))
        {
            if !self.profiles.contains_key(name) {
                return Err(invalid(format!("unknown profile `{name}`")));
            }
            if !names.iter().any(|(n, _)| n == name) {
                names.push((name.to_owned(), tier));
            }
        }
        Ok(names)
    }

    pub fn select(
        &self,
        role: Option<&str>,
        task_profile: Option<&str>,
        task_tier: Option<Tier>,
    ) -> Result<Vec<AgentSelection>, ConfigError> {
        self.order(role, task_profile, task_tier)?
            .into_iter()
            .map(|(name, tier)| self.model_chain(&name, tier))
            .collect::<Result<Vec<_>, _>>()
            .map(|chains| chains.into_iter().flatten().collect())
    }
    pub fn model_chain(&self, name: &str, tier: Tier) -> Result<Vec<AgentSelection>, ConfigError> {
        let profile = self
            .profiles
            .get(name)
            .ok_or_else(|| invalid(format!("unknown profile `{name}`")))?;
        profile
            .chain(tier)
            .iter()
            .map(|entry| resolve_entry(name, profile, entry))
            .collect()
    }
}
#[derive(Serialize, Deserialize)]
struct Deployment {
    schema_version: u64,
    digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Effort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
    Ultrathink,
    Off,
    Auto,
}
#[derive(Serialize, Deserialize)]
enum ContextWindow {
    #[serde(rename = "200k")]
    Standard,
    #[serde(rename = "1m")]
    Extended,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ServiceTier {
    Default,
    Priority,
    Flex,
}
macro_rules! model_shape {
    ($name:ident {$($field:ident : $ty:ty),* $(,)?}) => {
        #[derive(Serialize,Deserialize)]
        #[serde(deny_unknown_fields,rename_all="camelCase")]
        struct $name { model:String, $(#[serde(default,skip_serializing_if="Option::is_none")] $field:Option<$ty>),* }
    };
}
model_shape!(CodexModel {
    reasoning_effort: Effort,
    service_tier: ServiceTier
});
model_shape!(ClaudeModel {
    effort: Effort,
    fast_mode: bool,
    thinking: bool,
    context_window: ContextWindow
});
model_shape!(ReasoningModel {
    reasoning_effort: Effort
});
model_shape!(PiModel { thinking: Effort });
model_shape!(OpenCodeModel {
    variant: Effort,
    agent: String
});
model_shape!(FxModel { effort: Effort });
model_shape!(CursorModel {
    effort: Effort,
    reasoning: Effort,
    context_window: String,
    fast_mode: bool,
    thinking: bool
});
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AntigravityModel {
    model: String,
    thinking_level: Option<Effort>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcpModel {
    model: String,
    #[serde(default)]
    options: BTreeMap<String, serde_json::Value>,
}

fn typed<T: serde::de::DeserializeOwned + Serialize>(
    raw: &serde_json::Value,
) -> Result<serde_json::Value, ConfigError> {
    let parsed: T = serde_json::from_value(raw.clone()).map_err(|e| invalid(e.to_string()))?;
    serde_json::to_value(parsed).map_err(|e| invalid(e.to_string()))
}
fn resolve_entry(
    name: &str,
    profile: &Profile,
    raw: &serde_json::Value,
) -> Result<AgentSelection, ConfigError> {
    use HarnessKind::*;
    if raw
        .as_object()
        .is_some_and(|fields| fields.values().any(serde_json::Value::is_null))
    {
        return Err(invalid("model fields cannot be null"));
    }
    let (value, effort_key) = match profile.kind {
        Codex => (typed::<CodexModel>(raw)?, Some("reasoningEffort")),
        Claude => (typed::<ClaudeModel>(raw)?, Some("effort")),
        Antigravity => (typed::<AntigravityModel>(raw)?, Some("thinking_level")),
        Grok | Kiro | Muse => (typed::<ReasoningModel>(raw)?, Some("reasoningEffort")),
        Pi => (typed::<PiModel>(raw)?, Some("thinking")),
        OpenCode => (typed::<OpenCodeModel>(raw)?, Some("variant")),
        Cursor => (typed::<CursorModel>(raw)?, Some("effort")),
        Fx => (typed::<FxModel>(raw)?, Some("effort")),
        AcpRegistry => (typed::<AcpModel>(raw)?, None),
    };
    let mut options = value
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("model entry must be a mapping"))?;
    let model = options
        .remove("model")
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| invalid("model name must be nonempty"))?;
    let effort = effort_key
        .and_then(|key| options.get(key))
        .or_else(|| {
            if profile.kind == Cursor {
                options.get("reasoning")
            } else {
                None
            }
        })
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let mut efforts = effort.iter().map(String::as_str).collect::<Vec<_>>();
    if profile.kind == Cursor
        && let Some(reasoning) = options.get("reasoning").and_then(|v| v.as_str())
    {
        efforts.push(reasoning);
    }
    for effort in efforts {
        let allowed = match profile.kind {
            Claude => matches!(
                effort,
                "low" | "medium" | "high" | "xhigh" | "max" | "ultrathink"
            ),
            Antigravity => matches!(effort, "low" | "medium" | "high"),
            Kiro | Grok => matches!(effort, "none" | "low" | "medium" | "high" | "xhigh" | "max"),
            Fx => matches!(effort, "auto" | "low" | "medium" | "high" | "xhigh" | "max"),
            Cursor => matches!(
                effort,
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
            ),
            OpenCode => matches!(effort, "low" | "medium" | "high" | "xhigh"),
            Pi => matches!(
                effort,
                "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
            ),
            _ => !matches!(effort, "off" | "ultrathink" | "auto"),
        };
        if !allowed {
            return Err(invalid("invalid harness effort value"));
        }
    }
    if profile.kind == AcpRegistry {
        options = options
            .remove("options")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        if options
            .values()
            .any(|v| !v.is_string() && !v.is_boolean() && !v.is_number())
        {
            return Err(invalid("ACP native options must be scalar values"));
        }
    }
    Ok(AgentSelection {
        profile: name.into(),
        kind: profile.kind,
        instance: profile.instance.clone(),
        model,
        effort,
        options: options.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_schema_is_refused_before_interpreting_its_profile_shape() {
        let newer = "schema_version: 2\nprofiles: {future: {kind: future-harness}}\n";
        assert!(matches!(
            Agents::parse(newer),
            Err(ConfigError::NewerSchema {
                found: 2,
                current: 1,
                ..
            })
        ));
    }
}
