//! Role-keyed model choices, validated independently of provider model catalogs.
use super::{ConfigError, Tier};
use crate::{
    fsx::{LockGuard, atomic_write},
    home::Home,
    ids::{ProfileName, ProviderInstanceId},
};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};

pub const SHIPPED_AGENTS: &str = include_str!("shipped/agents.yaml");
pub(super) const VERSION: u64 = 3;
const DEPLOYMENT_VERSION: u64 = 1;

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

/// The effective selection document: the shipped defaults with `file` laid
/// over them. Maps merge by key, a field the file sets replaces the shipped
/// one and a field it leaves out is inherited, and any other value (a list
/// included) replaces whole. One of [`KEYED_MAPS`] that the file gives no
/// entries names nothing, so a bare `roles:` keeps every shipped role.
fn effective(mut file: Value) -> Result<Value, ConfigError> {
    let mut merged: Value =
        serde_yaml_ng::from_str(SHIPPED_AGENTS).map_err(|e| invalid(e.to_string()))?;
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

impl Agents {
    /// The effective selection `text` states: the shipped defaults with the
    /// file's overrides merged over them, validated as one document. The
    /// file's own `schema_version` names the shape of its overrides, so it is
    /// required and never inherited.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
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
        super::_agents_migrate::retire_session_recover(&mut value);
        let result: Self =
            serde_yaml_ng::from_value(effective(value)?).map_err(|e| invalid(e.to_string()))?;
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

    /// What a writing command does to `agents.yaml` before reading it. A full
    /// copy of shipped defaults the deployment record recognises as unedited
    /// is removed, and then the record; any other file is kept and has a due
    /// shape conversion persisted. Removal comes first, so a recognised copy
    /// is never converted and kept.
    pub fn converge_file(home: &Home) -> Result<(), ConfigError> {
        let path = home.agents_file();
        let state = home.agents_deployment_file();
        let Some(before) = read_existing(&path)? else {
            return Ok(());
        };
        if next_write(&before, Deployment::load(&state).as_ref())? == NextWrite::Keep {
            return Ok(());
        }
        let _lock = LockGuard::acquire(
            &home.locks_dir().join("agents.lock"),
            Duration::from_secs(10),
        )?;
        let Some(current) = read_existing(&path)? else {
            return Ok(());
        };
        match next_write(&current, Deployment::load(&state).as_ref())? {
            NextWrite::Keep => {}
            NextWrite::Remove => remove_copy(&path, &state)?,
            NextWrite::Convert(converted) => {
                atomic_write(&path, &converted.bytes)?;
                tracing::info!(
                    event = "agents_shape_migrated",
                    from_version = converted.from,
                    to_version = VERSION
                );
            }
        }
        Ok(())
    }

    /// What a reading command reports about `agents.yaml`: the change the
    /// next writing command makes to it, if any, without making it.
    pub fn inspection_notice(home: &Home) -> Result<Option<String>, ConfigError> {
        let path = home.agents_file();
        let Some(bytes) = read_existing(&path)? else {
            return Ok(None);
        };
        let state = home.agents_deployment_file();
        Ok(
            match next_write(&bytes, Deployment::load(&state).as_ref())? {
                NextWrite::Keep => None,
                NextWrite::Remove => Some(format!(
                    "{}: an unedited copy of earlier shipped defaults; the next writing command removes it and the shipped defaults apply",
                    path.display()
                )),
                NextWrite::Convert(_) => Some(format!(
                    "{}: selection configuration inspected in memory; file migrates on the next writing command",
                    path.display()
                )),
            },
        )
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
            return Self::from_documents(home, existing, global.as_ref());
        }
        super::_layer::migrate_if_needed(home, &super::Layer::Global)?;
        Self::converge_file(home)?;
        let existing = read_existing(&home.agents_file())?;
        Self::from_documents(home, existing, None)
    }

    /// The effective selection from `agents.yaml`'s bytes (`None` when there
    /// is no file) and the global layer as read (`None` when there is none),
    /// without reading or writing anything: an older `agents.yaml` converted
    /// in memory, the selection of an older global layer carried over, each
    /// merged over the shipped defaults, and the shipped defaults alone when
    /// neither states any.
    pub(crate) fn from_documents(
        home: &Home,
        agents: Option<Vec<u8>>,
        global: Option<&Value>,
    ) -> Result<Self, ConfigError> {
        let path = home.agents_file();
        let existing = match agents {
            Some(bytes) => Some(
                super::_agents_migrate::current(&bytes)?.map_or(bytes, |converted| converted.bytes),
            ),
            None => None,
        };
        let legacy = match global {
            Some(value)
                if value
                    .get("schema_version")
                    .and_then(Value::as_u64)
                    .is_some_and(|v| v < super::GLOBAL_SCHEMA_VERSION) =>
            {
                let mut value = value.clone();
                super::_migrate::selection(home, &super::Layer::Global, &mut value, false)?
            }
            _ => None,
        };
        if let Some(value) = legacy {
            let text = super::render_yaml(&value).map_err(|source| ConfigError::Render {
                path: path.clone(),
                source,
            })?;
            if let Some(existing) = existing
                && serde_yaml_ng::from_slice::<Value>(&existing).map_err(|source| {
                    ConfigError::Parse {
                        path: path.clone(),
                        source,
                    }
                })? != value
            {
                return Err(ConfigError::InvalidValue {
                    key: "configuration migration".into(),
                    message:
                        "agents.yaml differs from old global selection; reconcile before writing"
                            .into(),
                });
            }
            return Self::parse(&text);
        }
        match existing {
            Some(bytes) => {
                Self::parse(
                    std::str::from_utf8(&bytes).map_err(|_| ConfigError::InvalidValue {
                        key: "agents.yaml".into(),
                        message: "expected UTF-8".into(),
                    })?,
                )
            }
            None => Self::parse(SHIPPED_AGENTS),
        }
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
/// What the next writing command does to `agents.yaml`.
#[derive(Debug, PartialEq, Eq)]
enum NextWrite {
    /// Nothing for this operation: the file is not a recognised copy and no
    /// shape conversion applies. Whether it is valid is for [`Agents::parse`]
    /// to decide.
    Keep,
    /// Remove the file and then the deployment record: the record recognises
    /// the file as a copy an earlier release wrote and nobody edited.
    Remove,
    /// Persist the file's own bytes at the current shape.
    Convert(super::_agents_migrate::Converted),
}

/// The decision for `file` beside `record`, the readable deployment record
/// (`None` when there is none, or it is unreadable). Removal comes first, so
/// a recognised copy is never converted and kept.
fn next_write(file: &[u8], record: Option<&Deployment>) -> Result<NextWrite, ConfigError> {
    if record.is_some_and(|record| record.recognises(file)) {
        return Ok(NextWrite::Remove);
    }
    Ok(match super::_agents_migrate::current(file)? {
        Some(converted) => NextWrite::Convert(converted),
        None => NextWrite::Keep,
    })
}

/// Removes a recognised copy at `path`, then its record at `state`.
fn remove_copy(path: &std::path::Path, state: &std::path::Path) -> Result<(), ConfigError> {
    for removed in [path, state] {
        match crate::fsx::remove_file(removed) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(crate::fsx::FsError::Write {
                    path: removed.to_path_buf(),
                    source,
                }
                .into());
            }
        }
    }
    tracing::info!(event = "agents_defaults_copy_removed");
    Ok(())
}

/// `agents.yaml`'s bytes, or `None` when there is no file.
fn read_existing(path: &std::path::Path) -> Result<Option<Vec<u8>>, ConfigError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// What an ignored deployment record leaves, and how an operator returns to
/// the shipped defaults: told whether the record could not be read or could
/// not be understood.
const IGNORED_RECORD_RECOVERY: &str = "the record is ignored: agents.yaml stays as the user's overrides, still converted to the current shape; to return to the shipped defaults, remove agents.yaml together with this record once you have confirmed it holds no edits you want";

#[derive(Deserialize)]
struct Deployment {
    schema_version: u64,
    digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<String>,
}

impl Deployment {
    /// The record an earlier release kept of the defaults it deployed; `None`
    /// when there is none. A record that cannot be read or is in an unknown
    /// shape recognises nothing: the file beside it is kept and the command
    /// proceeds, because the record is only bookkeeping about instrukt's own
    /// earlier write and the person's file is never removed on a guess.
    fn load(state: &std::path::Path) -> Option<Self> {
        let bytes = match std::fs::read(state) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(
                    event = "agents_deployment_record_unreadable",
                    record = %state.display(),
                    %error,
                    "{IGNORED_RECORD_RECOVERY}"
                );
                return None;
            }
        };
        match Self::parse(&bytes) {
            Ok(record) => Some(record),
            Err(cause) => {
                tracing::warn!(
                    event = "agents_deployment_record_invalid",
                    record = %state.display(),
                    %cause,
                    "{IGNORED_RECORD_RECOVERY}"
                );
                None
            }
        }
    }

    /// A deployment record from its bytes, or why it recognises nothing.
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let record: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if record.schema_version != DEPLOYMENT_VERSION {
            return Err(format!(
                "schema_version {} is not the supported {DEPLOYMENT_VERSION}",
                record.schema_version
            ));
        }
        Ok(record)
    }

    /// Whether `bytes` are what an earlier release itself wrote: the deployed
    /// defaults, or a write it recorded before making it.
    fn recognises(&self, bytes: &[u8]) -> bool {
        let found = hex::encode(Sha256::digest(bytes));
        self.digest == found || self.pending.as_deref() == Some(found.as_str())
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
        let newer = "schema_version: 4\nprofiles: {future: {kind: future-harness}}\n";
        assert!(matches!(
            Agents::parse(newer),
            Err(ConfigError::NewerSchema {
                found: 4,
                current: 3,
                ..
            })
        ));
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
        let edited = Agents::parse("schema_version: 3\nroles: {builder: {tier: light}}\n").unwrap();
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
            Agents::parse("schema_version: 3\nroles: {builder: {order: [antigravity]}}\n").unwrap();
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
        let bare = Agents::parse("schema_version: 3\nroles:\n").unwrap();
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

    /// The deployment record an earlier release kept of `bytes`.
    fn record_of(bytes: &str) -> super::Deployment {
        use sha2::Digest as _;
        let record = serde_json::json!({
            "schema_version": 1,
            "digest": hex::encode(sha2::Sha256::digest(bytes.as_bytes())),
        });
        super::Deployment::parse(record.to_string().as_bytes()).unwrap()
    }

    /// The shipped defaults in the shape an earlier release wrote: the split
    /// orchestrator role the current shape retires.
    fn older_shape(text: &str) -> String {
        text.replacen("  todo-orchestrator:", "  work-orchestrator:", 1)
    }

    #[test]
    fn a_recognised_older_copy_is_removed_rather_than_converted() {
        let older = older_shape(SHIPPED_AGENTS);
        assert_eq!(
            super::next_write(older.as_bytes(), Some(&record_of(&older))).unwrap(),
            super::NextWrite::Remove
        );
    }

    #[test]
    fn an_edited_older_file_is_converted_and_kept() {
        let edited = format!("{}# my own note\n", older_shape(SHIPPED_AGENTS));
        let decided = super::next_write(
            edited.as_bytes(),
            Some(&record_of(&older_shape(SHIPPED_AGENTS))),
        )
        .unwrap();
        assert!(matches!(decided, super::NextWrite::Convert(_)));
    }

    #[test]
    fn an_edited_current_file_is_kept_as_it_is() {
        let edited = format!("{SHIPPED_AGENTS}# my own note\n");
        assert_eq!(
            super::next_write(edited.as_bytes(), Some(&record_of(SHIPPED_AGENTS))).unwrap(),
            super::NextWrite::Keep
        );
    }

    #[test]
    fn a_record_in_an_unknown_shape_recognises_nothing() {
        use sha2::Digest as _;
        let record = serde_json::json!({
            "schema_version": 2,
            "digest": hex::encode(sha2::Sha256::digest(SHIPPED_AGENTS.as_bytes())),
        });
        assert!(super::Deployment::parse(record.to_string().as_bytes()).is_err());
        assert!(super::Deployment::parse(b"{not a record").is_err());
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
}
