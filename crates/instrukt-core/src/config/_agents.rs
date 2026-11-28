//! Role-keyed model choices, validated independently of provider model catalogs.
use super::_agents_file::read_existing;
use super::{ConfigError, Tier};
use crate::{
    home::Home,
    ids::{ProfileName, ProviderInstanceId},
};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;
use std::collections::BTreeMap;

pub const SHIPPED_AGENTS: &str = include_str!("shipped/agents.yaml");
pub(super) const VERSION: u64 = 4;

/// Neutral agent/tier preference serialized inside persisted runtime-start
/// requests and manual standing reservations (`agent` + optional `tier`).
/// Omits `deny_unknown_fields` to remain wire-compatible with persisted rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPreference {
    pub agent: ProfileName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<Tier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSelection {
    pub profile: ProfileName,
    pub kind: HarnessKind,
    pub instance: ProviderInstanceId,
    pub model: String,
    pub effort: Option<Effort>,
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
#[serde(deny_unknown_fields)]
pub struct RoleSelection {
    pub order: Option<Vec<ProfileName>>,
    pub tier: Option<Tier>,
    pub class: crate::capabilities::RoleClass,
    pub can: Vec<crate::capabilities::Allowance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_app: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub instance: ProviderInstanceId,
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

/// The role the runner gives a conversation session.
pub const COMPANY_AGENT_ROLE: &str = "company-agent";

/// The standing role that attends to a project's fleet of active todos.
pub const SDLC_COORDINATOR_ROLE: &str = "sdlc-coordinator";

/// The name a version-three file gives [`SDLC_COORDINATOR_ROLE`].
pub const RETIRED_COORDINATOR_ROLE: &str = "coordinator";

/// The role that drives a todo through preparation and work.
pub const TODO_ORCHESTRATOR_ROLE: &str = "todo-orchestrator";

/// The split orchestrator roles [`TODO_ORCHESTRATOR_ROLE`] replaced, in the
/// order agents migration prefers their settings when neither file entry is
/// unified yet: work's first.
pub const RETIRED_ORCHESTRATOR_ROLES: [&str; 2] = ["work-orchestrator", "prepare-orchestrator"];

/// The role whose `agents.yaml` entry governs work recorded under `role`: a
/// retired split orchestrator's is [`TODO_ORCHESTRATOR_ROLE`]'s, because
/// agents migration removes the split entries; any other role is its own.
pub fn current_role(role: &str) -> &str {
    if RETIRED_ORCHESTRATOR_ROLES.contains(&role) {
        TODO_ORCHESTRATOR_ROLE
    } else {
        role
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agents {
    pub schema_version: u64,
    pub default: RoleSelection,
    #[serde(default)]
    pub roles: BTreeMap<String, RoleSelection>,
    pub profiles: BTreeMap<ProfileName, Profile>,
    #[serde(default)]
    pub bundles: BTreeMap<String, Vec<crate::capabilities::Grant>>,
}

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        key: "agents.yaml".into(),
        message: message.into(),
    }
}

/// The maps the file merges into the shipped defaults key by key.
const KEYED_MAPS: [&str; 3] = ["roles", "profiles", "bundles"];

fn shipped() -> Result<Value, ConfigError> {
    serde_yaml_ng::from_str(SHIPPED_AGENTS).map_err(|e| invalid(e.to_string()))
}

fn without_empty_keyed_maps(file: &mut Value) {
    if let Some(fields) = file.as_mapping_mut() {
        for key in KEYED_MAPS {
            if fields
                .get(key)
                .is_some_and(|map| map.is_null() || map.as_mapping().is_some_and(|m| m.is_empty()))
            {
                fields.remove(key);
            }
        }
    }
}

/// The effective selection document: the shipped defaults with `file` laid
/// over them. Maps merge by key, a field the file sets replaces the shipped
/// one and a field it leaves out is inherited, and any other value (a list
/// included) replaces whole. One of [`KEYED_MAPS`] that the file gives no
/// entries names nothing, so a bare `roles:` keeps every shipped role.
fn effective(mut file: Value) -> Result<Value, ConfigError> {
    let mut merged = shipped()?;
    without_empty_keyed_maps(&mut file);
    // A role with no shipped entry takes its omitted class and grants from
    // the effective `default`; a shipped role takes them from its own entry
    // through the overlay.
    let unshipped: Vec<Value> = file
        .get("roles")
        .and_then(Value::as_mapping)
        .into_iter()
        .flat_map(|roles| roles.keys())
        .filter(|name| merged["roles"].get(*name).is_none())
        .cloned()
        .collect();
    super::_merge::overlay(&mut merged, &file);
    let default = merged.get("default").cloned().unwrap_or(Value::Null);
    if let Some(roles) = merged.get_mut("roles").and_then(Value::as_mapping_mut) {
        for name in &unshipped {
            if let Some(fields) = roles.get_mut(name).and_then(Value::as_mapping_mut) {
                for field in ["class", "can"] {
                    if !fields.contains_key(field)
                        && let Some(inherited) = default.get(field)
                    {
                        fields.insert(field.into(), inherited.clone());
                    }
                }
            }
        }
    }
    Ok(merged)
}

/// `file` reduced to the entries that differ from the shipped defaults,
/// keeping `schema_version` first: the inverse of [`effective`].
pub(super) fn overrides_only(mut file: Value) -> Result<Value, ConfigError> {
    let mut base = shipped()?;
    without_empty_keyed_maps(&mut file);
    let version = file
        .as_mapping_mut()
        .and_then(|fields| fields.remove("schema_version"));
    if let Some(fields) = base.as_mapping_mut() {
        fields.remove("schema_version");
    }
    super::_merge::prune(&mut file, &base);
    if let Value::Mapping(fields) = file {
        let mut ordered = serde_yaml_ng::Mapping::new();
        if let Some(version) = version {
            ordered.insert("schema_version".into(), version);
        }
        for (key, value) in fields {
            ordered.insert(key, value);
        }
        return Ok(Value::Mapping(ordered));
    }
    Ok(file)
}

impl Agents {
    /// The effective selection `text` states: the shipped defaults with the
    /// file's overrides merged over them, validated as one document. The
    /// file's own `schema_version` names the shape of its overrides, so it is
    /// required and never inherited.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        Self::validated(Self::effective_document(text)?)
    }

    /// `text` parsed, checked for schema version and coordinator-role conflict,
    /// with retired grants stripped and merged over the shipped defaults.
    pub(super) fn effective_document(text: &str) -> Result<Value, ConfigError> {
        let mut value: Value = serde_yaml_ng::from_str(text).map_err(|e| invalid(e.to_string()))?;
        match value.get("schema_version") {
            None => {
                return Err(ConfigError::UnknownSchema {
                    path: "agents.yaml".into(),
                });
            }
            Some(found) => {
                if let Some(found) = found.as_u64()
                    && found > VERSION
                {
                    return Err(ConfigError::NewerSchema {
                        path: "agents.yaml".into(),
                        found,
                        current: VERSION,
                    });
                }
            }
        }
        // Both names of one role leave no single entry to take; the file is
        // refused as it is, whatever its version.
        if value
            .get("roles")
            .and_then(Value::as_mapping)
            .is_some_and(|roles| {
                roles.contains_key(RETIRED_COORDINATOR_ROLE)
                    && roles.contains_key(SDLC_COORDINATOR_ROLE)
            })
        {
            return Err(ConfigError::AgentsRoleConflict {
                retired: RETIRED_COORDINATOR_ROLE,
                current: SDLC_COORDINATOR_ROLE,
            });
        }
        super::_agents_migrate::retire_session_recover(&mut value);
        effective(value)
    }

    /// Validates an already-merged effective selection `document`.
    pub(super) fn validated(document: Value) -> Result<Self, ConfigError> {
        let result: Self =
            serde_yaml_ng::from_value(document).map_err(|e| invalid(e.to_string()))?;
        if result.schema_version != VERSION {
            return Err(invalid(
                "unsupported agents schema_version; upgrade instrukt",
            ));
        }
        if result.default.order.is_none() || result.default.tier.is_none() {
            return Err(invalid("default requires order and tier"));
        }
        for entry in std::iter::once(&result.default).chain(result.roles.values()) {
            if let Some(order) = &entry.order
                && (order.is_empty() || order.iter().any(|p| !result.profiles.contains_key(p)))
            {
                return Err(invalid(
                    "orders must be nonempty and reference declared profiles",
                ));
            }
        }
        result.capabilities("default")?;
        for role in result.roles.keys() {
            result.capabilities(role)?;
        }
        for (name, profile) in &result.profiles {
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

    pub fn capabilities(
        &self,
        role: &str,
    ) -> Result<crate::capabilities::Capabilities, ConfigError> {
        use crate::capabilities::{Allowance, Capabilities};
        let entry = self.roles.get(role).unwrap_or(&self.default);
        let mut grants = std::collections::BTreeSet::new();
        for allowance in &entry.can {
            match allowance {
                Allowance::Grant(grant) => {
                    grants.insert(*grant);
                }
                Allowance::Bundle(name) => {
                    let bundle = self
                        .bundles
                        .get(name)
                        .ok_or_else(|| invalid(format!("unknown capability bundle `{name}`")))?;
                    grants.extend(bundle);
                }
            }
        }
        Ok(Capabilities {
            role: role.into(),
            class: entry.class,
            grants,
        })
    }

    /// The effective selection: the admitted one when served, else the file
    /// merged over the shipped defaults, or the shipped defaults when there
    /// is no file. A writing home first brings the file to its current state
    /// ([`Self::converge_file`]); an inspection home writes and removes
    /// nothing.
    pub fn load(home: &Home) -> Result<Self, ConfigError> {
        if let Some(admitted) = super::_admitted::served(home) {
            return Ok(admitted.agents().clone());
        }
        if home.is_inspection() {
            let existing = read_existing(&home.agents_file())?;
            let global = super::_layer::read(&home.global_config())?.map(|doc| doc.value);
            return Self::from_documents(existing, global.as_ref());
        }
        super::_layer::migrate_if_needed(home, &super::Layer::Global)?;
        Self::converge_file(home)?;
        let existing = read_existing(&home.agents_file())?;
        Self::from_documents(existing, None)
    }

    /// Read `agents.yaml` merged over the shipped defaults without consulting
    /// `instrukt.yaml` (used when an active company session continues under the
    /// runner's admitted generation while `instrukt.yaml` is temporarily broken).
    pub fn load_agents_only(home: &Home) -> Result<Self, ConfigError> {
        let existing = read_existing(&home.agents_file())?;
        Self::from_documents(existing, None)
    }

    /// The selection document text from `agents.yaml`'s bytes (`None` when
    /// there is no file) and the global layer as read (`None` when there is
    /// none), without reading or writing anything: an older `agents.yaml`
    /// converted in memory, an older global layer's selection carried over
    /// through `_migrate::carry_into`, and the shipped defaults when neither
    /// states any.
    pub(super) fn selection_text(
        agents: Option<Vec<u8>>,
        global: Option<&Value>,
    ) -> Result<String, ConfigError> {
        let existing = match agents {
            Some(bytes) => Some(
                super::_agents_migrate::current(&bytes)?.map_or(bytes, |converted| converted.bytes),
            ),
            None => None,
        };
        let bytes = match super::_migrate::carried_selection(global)? {
            Some(carried) => Some(super::_migrate::carry_into(existing, &carried)?),
            None => existing,
        };
        match bytes {
            Some(bytes) => String::from_utf8(bytes).map_err(|_| invalid("expected UTF-8")),
            None => Ok(SHIPPED_AGENTS.to_owned()),
        }
    }

    /// The effective selection from `agents.yaml`'s bytes (`None` when there
    /// is no file) and the global layer as read (`None` when there is none),
    /// without reading or writing anything: an older `agents.yaml` converted
    /// in memory, the selection of an older global layer carried over, each
    /// merged over the shipped defaults, and the shipped defaults alone when
    /// neither states any.
    pub(crate) fn from_documents(
        agents: Option<Vec<u8>>,
        global: Option<&Value>,
    ) -> Result<Self, ConfigError> {
        Self::parse(&Self::selection_text(agents, global)?)
    }

    pub fn undeclared_roles<'a>(&'a self, installed: &[&str]) -> Vec<&'a str> {
        self.roles
            .keys()
            .map(String::as_str)
            // `company-agent` is the role the runner gives conversation
            // sessions, whatever procedure is published for it.
            .filter(|r| {
                !matches!(*r, "default" | "event-handler" | COMPANY_AGENT_ROLE)
                    && !installed.contains(r)
            })
            .collect()
    }

    /// The profiles a selection walks, each with its tier: a task override
    /// first, then `home_order` when the session's home names one (a
    /// company home's `profiles`), else the role mapping's order. The tier
    /// comes from the task, then the role mapping, then `default`.
    pub fn order(
        &self,
        role: Option<&str>,
        task_profile: Option<&ProfileName>,
        task_tier: Option<Tier>,
        home_order: Option<&[ProfileName]>,
    ) -> Result<Vec<(ProfileName, Tier)>, ConfigError> {
        let entry = role
            .and_then(|r| self.roles.get(r))
            .unwrap_or(&self.default);
        let order = match home_order {
            Some(order) => order,
            None => entry
                .order
                .as_deref()
                .or(self.default.order.as_deref())
                .ok_or_else(|| invalid("missing default order"))?,
        };
        let tier = task_tier
            .or(entry.tier)
            .or(self.default.tier)
            .ok_or_else(|| invalid("missing default tier"))?;
        let mut names = Vec::new();
        for name in task_profile.into_iter().chain(order.iter()) {
            if !self.profiles.contains_key(name) {
                return Err(invalid(format!("unknown profile `{name}`")));
            }
            if !names.iter().any(|(n, _)| n == name) {
                names.push((name.clone(), tier));
            }
        }
        Ok(names)
    }

    pub fn select(
        &self,
        role: Option<&str>,
        task_profile: Option<&ProfileName>,
        task_tier: Option<Tier>,
        home_order: Option<&[ProfileName]>,
    ) -> Result<Vec<AgentSelection>, ConfigError> {
        self.order(role, task_profile, task_tier, home_order)?
            .into_iter()
            .map(|(name, tier)| self.model_chain(&name, tier))
            .collect::<Result<Vec<_>, _>>()
            .map(|chains| chains.into_iter().flatten().collect())
    }
    pub fn model_chain(
        &self,
        name: &ProfileName,
        tier: Tier,
    ) -> Result<Vec<AgentSelection>, ConfigError> {
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

/// Runtime-neutral effort vocabulary; harness membership is checked before selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
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
impl Effort {
    pub fn allowed_for(self, kind: HarnessKind) -> bool {
        match kind {
            HarnessKind::Claude => matches!(
                self,
                Self::Low | Self::Medium | Self::High | Self::Xhigh | Self::Max | Self::Ultrathink
            ),
            HarnessKind::Antigravity => {
                matches!(self, Self::Low | Self::Medium | Self::High | Self::Xhigh)
            }
            HarnessKind::Kiro | HarnessKind::Grok => matches!(
                self,
                Self::None | Self::Low | Self::Medium | Self::High | Self::Xhigh | Self::Max
            ),
            HarnessKind::Fx => matches!(
                self,
                Self::Auto | Self::Low | Self::Medium | Self::High | Self::Xhigh | Self::Max
            ),
            HarnessKind::Cursor => matches!(
                self,
                Self::None
                    | Self::Minimal
                    | Self::Low
                    | Self::Medium
                    | Self::High
                    | Self::Xhigh
                    | Self::Max
            ),
            HarnessKind::OpenCode => {
                matches!(self, Self::Low | Self::Medium | Self::High | Self::Xhigh)
            }
            HarnessKind::Pi => matches!(
                self,
                Self::Off
                    | Self::Minimal
                    | Self::Low
                    | Self::Medium
                    | Self::High
                    | Self::Xhigh
                    | Self::Max
            ),
            HarnessKind::Codex | HarnessKind::Muse => matches!(
                self,
                Self::None
                    | Self::Minimal
                    | Self::Low
                    | Self::Medium
                    | Self::High
                    | Self::Xhigh
                    | Self::Max
                    | Self::Ultra
            ),
            HarnessKind::AcpRegistry => true,
        }
    }
}
#[derive(Deserialize)]
struct ModelEntry {
    model: String,
    effort: Option<Effort>,
    #[serde(flatten)]
    settings: BTreeMap<String, serde_json::Value>,
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
// Extra settings remain per-kind records; effort keeps its semantic type.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CodexSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    service_tier: Option<ServiceTier>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ClaudeSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fast_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_window: Option<ContextWindow>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CursorSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fast_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_window: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenCodeSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoSettings {}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcpSettings {
    #[serde(default)]
    options: BTreeMap<String, serde_json::Value>,
}
fn settings<T: serde::de::DeserializeOwned + Serialize>(
    settings: &BTreeMap<String, serde_json::Value>,
) -> Result<BTreeMap<String, serde_json::Value>, ConfigError> {
    let parsed: T = serde_json::from_value(serde_json::Value::Object(
        settings.clone().into_iter().collect(),
    ))
    .map_err(|e| invalid(e.to_string()))?;
    let value = serde_json::to_value(parsed).map_err(|e| invalid(e.to_string()))?;
    serde_json::from_value(value).map_err(|e| invalid(e.to_string()))
}
fn resolve_entry(
    name: &ProfileName,
    profile: &Profile,
    raw: &serde_json::Value,
) -> Result<AgentSelection, ConfigError> {
    if raw
        .as_object()
        .is_some_and(|fields| fields.values().any(serde_json::Value::is_null))
    {
        return Err(invalid("model fields cannot be null"));
    }
    let entry: ModelEntry =
        serde_json::from_value(raw.clone()).map_err(|e| invalid(e.to_string()))?;
    if entry.model.trim().is_empty() {
        return Err(invalid("model name must be nonempty"));
    }
    if entry
        .effort
        .is_some_and(|effort| !effort.allowed_for(profile.kind))
    {
        return Err(invalid("invalid harness effort value"));
    }
    let mut options = entry.settings;
    match profile.kind {
        HarnessKind::Codex => {
            options = settings::<CodexSettings>(&options)?;
        }
        HarnessKind::Claude => {
            options = settings::<ClaudeSettings>(&options)?;
        }
        HarnessKind::Cursor => {
            options = settings::<CursorSettings>(&options)?;
        }
        HarnessKind::OpenCode => {
            options = settings::<OpenCodeSettings>(&options)?;
        }
        HarnessKind::AcpRegistry => {
            let parsed: AcpSettings =
                serde_json::from_value(serde_json::Value::Object(options.into_iter().collect()))
                    .map_err(|e| invalid(e.to_string()))?;
            options = parsed.options;
            if options
                .values()
                .any(|value| !value.is_string() && !value.is_boolean() && !value.is_number())
            {
                return Err(invalid("ACP native options must be scalar values"));
            }
        }
        HarnessKind::Antigravity
        | HarnessKind::Grok
        | HarnessKind::Fx
        | HarnessKind::Kiro
        | HarnessKind::Pi
        | HarnessKind::Muse => {
            options = settings::<NoSettings>(&options)?;
        }
    }
    Ok(AgentSelection {
        profile: name.clone(),
        kind: profile.kind,
        instance: profile.instance.clone(),
        model: entry.model,
        effort: entry.effort,
        options,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "pure configuration parser setup")]
mod tests {
    use super::{Agents, ConfigError, SHIPPED_AGENTS};
    #[test]
    fn a_newer_schema_is_refused_before_interpreting_its_profile_shape() {
        let newer = format!(
            "schema_version: {}\nprofiles: {{future: {{kind: future-harness}}}}\n",
            super::VERSION + 1
        );
        assert!(matches!(
            Agents::parse(&newer),
            Err(ConfigError::NewerSchema { found, current, .. })
                if found == super::VERSION + 1 && current == super::VERSION
        ));
    }

    #[test]
    fn a_file_naming_both_coordinator_roles_is_refused() {
        let both = "schema_version: 4\nroles:\n  coordinator: {tier: light}\n  sdlc-coordinator: {tier: premium}\n";
        assert!(matches!(
            Agents::parse(both),
            Err(ConfigError::AgentsRoleConflict {
                retired: super::RETIRED_COORDINATOR_ROLE,
                current: super::SDLC_COORDINATOR_ROLE,
            })
        ));
    }

    #[test]
    fn a_file_naming_neither_coordinator_role_resolves_the_shipped_sdlc_coordinator() {
        let shipped = Agents::parse(SHIPPED_AGENTS).unwrap();
        let kept = Agents::parse("schema_version: 4\nroles: {builder: {tier: light}}\n").unwrap();
        let role = super::SDLC_COORDINATOR_ROLE;
        let (kept_grants, shipped_grants) = (
            kept.capabilities(role).unwrap(),
            shipped.capabilities(role).unwrap(),
        );
        assert_eq!(
            (kept_grants.class, kept_grants.grants),
            (shipped_grants.class, shipped_grants.grants)
        );
        assert_eq!(
            (&kept.roles[role].order, kept.roles[role].tier),
            (&shipped.roles[role].order, shipped.roles[role].tier)
        );
    }

    #[test]
    fn the_conversation_role_is_never_reported_undeclared_while_an_ordinary_role_is() {
        let agents = Agents::parse(SHIPPED_AGENTS).unwrap();
        let installed: Vec<&str> = agents
            .roles
            .keys()
            .map(String::as_str)
            .filter(|role| *role != "builder" && *role != super::COMPANY_AGENT_ROLE)
            .collect();
        assert_eq!(agents.undeclared_roles(&installed), vec!["builder"]);
    }

    #[test]
    fn handler_defaults_and_unknown_roles_preserve_their_recorded_name() {
        let agents = Agents::parse(SHIPPED_AGENTS).unwrap();
        let handler = agents.capabilities("event-handler").unwrap();
        assert_eq!(handler.class, crate::capabilities::RoleClass::Standing);
        let unknown = agents.capabilities("custom-handler").unwrap();
        assert_eq!(unknown.role, "custom-handler");
        assert_eq!(unknown.class, crate::capabilities::RoleClass::Agent);
    }

    #[test]
    fn an_unknown_role_class_is_rejected() {
        assert!(
            Agents::parse(&SHIPPED_AGENTS.replacen("class: agent", "class: admin", 1)).is_err()
        );
    }

    #[test]
    fn a_missing_capability_bundle_is_rejected() {
        assert!(
            Agents::parse(&SHIPPED_AGENTS.replacen("can: [all]", "can: [missing]", 1)).is_err()
        );
    }

    #[test]
    fn an_unknown_configured_role_inherits_the_effective_default_capabilities() {
        use crate::capabilities::{Action, RoleClass, Scope};
        let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
        edited["default"]["class"] = "focused".into();
        edited["default"]["can"] =
            serde_yaml_ng::from_str("[{action: docs-read, scope: own}]").unwrap();
        edited["roles"]["custom"] =
            serde_yaml_ng::from_str("{order: [codex], tier: light}").unwrap();
        let resolved = Agents::parse(&serde_yaml_ng::to_string(&edited).unwrap()).unwrap();
        let role = resolved.capabilities("custom").unwrap();
        assert_eq!(role.class, RoleClass::Focused);
        assert_eq!(role.scope(Action::DocsRead), Some(Scope::Own));
        assert_eq!(role.scope(Action::CodeCommit), None);
    }

    #[test]
    fn an_explicit_role_class_wins_while_its_missing_grants_use_the_shipped_role() {
        use crate::capabilities::{Action, RoleClass, Scope};
        let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
        edited["roles"]["builder"]["class"] = "agent".into();
        edited["roles"]["builder"]
            .as_mapping_mut()
            .unwrap()
            .remove("can");
        let role = Agents::parse(&serde_yaml_ng::to_string(&edited).unwrap())
            .unwrap()
            .capabilities("builder")
            .unwrap();
        assert_eq!(role.class, RoleClass::Agent);
        assert_eq!(role.scope(Action::DocsWrite), Some(Scope::Own));
    }

    #[test]
    fn an_explicit_empty_grant_list_wins_while_the_missing_class_uses_the_shipped_role() {
        use crate::capabilities::RoleClass;
        let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
        edited["roles"]["builder"]
            .as_mapping_mut()
            .unwrap()
            .remove("class");
        edited["roles"]["builder"]["can"] = serde_yaml_ng::from_str("[]").unwrap();
        let role = Agents::parse(&serde_yaml_ng::to_string(&edited).unwrap())
            .unwrap()
            .capabilities("builder")
            .unwrap();
        assert_eq!(role.class, RoleClass::Focused);
        assert!(role.grants.is_empty());
    }

    #[test]
    fn explicit_bundle_edits_win_when_a_role_inherits_its_grant_names() {
        use crate::capabilities::Action;
        let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
        edited["roles"]["builder"]
            .as_mapping_mut()
            .unwrap()
            .remove("can");
        edited["bundles"]["worker"] = serde_yaml_ng::from_str("[]").unwrap();
        let role = Agents::parse(&serde_yaml_ng::to_string(&edited).unwrap())
            .unwrap()
            .capabilities("builder")
            .unwrap();
        assert_eq!(role.scope(Action::DocsWrite), None);
    }

    #[test]
    fn a_file_naming_one_field_of_a_shipped_role_inherits_its_other_fields() {
        let shipped = Agents::parse(SHIPPED_AGENTS).unwrap();
        let edited = Agents::parse("schema_version: 4\nroles: {builder: {tier: light}}\n").unwrap();
        let (before, after) = (&shipped.roles["builder"], &edited.roles["builder"]);
        assert_eq!(after.tier, Some(super::Tier::Light));
        assert_eq!(after.order, before.order);
        assert_eq!(after.class, before.class);
        assert_eq!(after.github_app, before.github_app);
        assert_eq!(
            edited.capabilities("builder").unwrap().grants,
            shipped.capabilities("builder").unwrap().grants
        );
    }

    #[test]
    fn a_list_the_file_sets_replaces_the_shipped_list_whole() {
        let edited =
            Agents::parse("schema_version: 4\nroles: {builder: {order: [antigravity]}}\n").unwrap();
        let order: Vec<&str> = edited.roles["builder"]
            .order
            .iter()
            .flatten()
            .map(|profile| profile.as_str())
            .collect();
        assert_eq!(order, ["antigravity"]);
    }

    #[test]
    fn a_roles_key_with_no_entries_keeps_every_shipped_role() {
        let shipped = Agents::parse(SHIPPED_AGENTS).unwrap();
        let bare = Agents::parse("schema_version: 4\nroles:\n").unwrap();
        assert_eq!(
            bare.roles.keys().collect::<Vec<_>>(),
            shipped.roles.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            bare.capabilities("technical-coordinator").unwrap().grants,
            shipped
                .capabilities("technical-coordinator")
                .unwrap()
                .grants
        );
    }

    #[test]
    fn a_file_stating_no_schema_version_is_refused() {
        assert!(matches!(
            Agents::parse("roles: {builder: {tier: light}}\n"),
            Err(ConfigError::UnknownSchema { .. })
        ));
    }

    #[test]
    fn an_antigravity_effort_outside_its_vocabulary_is_refused() {
        let edited = SHIPPED_AGENTS.replacen(
            "standard: [{model: gemini-3.8-flash, effort: high}]",
            "standard: [{model: gemini-3.8-flash, effort: max}]",
            1,
        );
        assert!(matches!(
            Agents::parse(&edited),
            Err(ConfigError::InvalidValue { .. })
        ));
    }

    #[test]
    fn a_full_copy_of_the_shipped_defaults_with_one_change_reduces_to_that_change() {
        let mut copy: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
        let mut reversed = copy["roles"]["builder"]["order"]
            .as_sequence()
            .unwrap()
            .clone();
        reversed.reverse();
        copy["roles"]["builder"]["order"] = serde_yaml_ng::Value::Sequence(reversed.clone());

        let reduced = super::overrides_only(copy).unwrap();
        let mut expected: serde_yaml_ng::Value = serde_yaml_ng::from_str(&format!(
            "schema_version: {}\nroles: {{builder: {{}}}}\n",
            super::VERSION
        ))
        .unwrap();
        expected["roles"]["builder"]["order"] = serde_yaml_ng::Value::Sequence(reversed);
        assert_eq!(reduced, expected);
    }

    #[test]
    fn reducing_a_file_to_its_overrides_leaves_its_effective_selection_unchanged() {
        let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
        let mut reversed = edited["roles"]["builder"]["order"]
            .as_sequence()
            .unwrap()
            .clone();
        reversed.reverse();
        edited["roles"]["builder"]["order"] = serde_yaml_ng::Value::Sequence(reversed);
        edited["roles"]["custom-role"] =
            serde_yaml_ng::from_str("{order: [codex], tier: light}").unwrap();
        edited["bundles"]["extra"] =
            serde_yaml_ng::from_str("[{action: docs-read, scope: any}]").unwrap();

        let before = super::effective(edited.clone()).unwrap();
        let reduced = super::overrides_only(edited).unwrap();
        let after = super::effective(reduced).unwrap();
        assert_eq!(after, before);
    }
}
