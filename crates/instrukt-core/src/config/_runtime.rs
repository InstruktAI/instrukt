//! Compiled runtime declarations, fail-closed selection, and caller context.
//! No executable configuration.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::_error::ConfigError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    vars: BTreeMap<String, String>,
    caller: Option<crate::interfaces::CallerIdentity>,
    caller_variables: Vec<String>,
}
impl Env {
    pub fn from_process() -> Self {
        Self {
            vars: std::env::vars_os()
                .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
                .collect(),
            ..Self::default()
        }
    }
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            vars: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
            ..Self::default()
        }
    }
    pub fn with_caller(mut self, lookup: &dyn crate::interfaces::CallerIdentityLookup) -> Self {
        self.caller = lookup.lookup_caller(&self);
        self.caller_variables = lookup
            .caller_variables()
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        self
    }
    pub(crate) fn caller_identity(
        &self,
        runtime: &str,
    ) -> Option<crate::interfaces::CallerIdentity> {
        self.caller
            .as_ref()
            .filter(|caller| caller.session.runtime.as_str() == runtime)
            .cloned()
    }
    pub(crate) fn without_caller(mut self) -> Self {
        for name in &self.caller_variables {
            self.vars.remove(name);
        }
        self.caller = None;
        self
    }
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }
    /// What [`crate::fsx::FENCE_VARIABLE`] says: the locations it keeps this
    /// process from writing and where refusals are recorded; nothing when it
    /// is unset.
    pub fn write_fence(&self) -> crate::fsx::FenceSetting {
        self.get(crate::fsx::FENCE_VARIABLE)
            .map(crate::fsx::FenceSetting::parse)
            .unwrap_or_default()
    }
}

/// Pure static hook a runtime declaration supplies to normalize legacy
/// retained history message records during storage migration: returns `true`
/// to retain the normalized record, or `false` to drop it.
pub type LegacyHistoryNormalizer = fn(&mut serde_json::Value) -> bool;

/// Native agent operations are independent of unattended host-CLI support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCapabilities {
    pub shared_delegation: bool,
    pub worktree_attach: bool,
    pub agent_selection: bool,
}

/// Static capability and defaults metadata for one compiled runtime adapter,
/// installed once by the CLI composition root before configuration is read.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeDeclaration {
    pub name: &'static str,
    pub archives_threads: bool,
    pub worktree_create: bool,
    pub worktree_attach: bool,
    pub cold_text_source: bool,
    pub link: bool,
    pub notify: bool,
    pub unattended_agent_selection: bool,
    pub native_capabilities: NativeCapabilities,
    pub shipped_yaml: &'static str,
    pub legacy_history_normalizer: Option<LegacyHistoryNormalizer>,
}

impl RuntimeDeclaration {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn caller(&self, env: &Env) -> Option<crate::ids::SessionRef> {
        env.caller_identity(self.name).map(|caller| caller.session)
    }

    pub fn archives_threads(&self) -> bool {
        self.archives_threads
    }

    pub fn native_capabilities(&self) -> NativeCapabilities {
        self.native_capabilities
    }
}

impl PartialEq for RuntimeDeclaration {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl Eq for RuntimeDeclaration {}

static DECLARATIONS: OnceLock<&'static [&'static RuntimeDeclaration]> = OnceLock::new();

/// Install the compiled runtime declarations this binary supports. The first
/// installation holds for the life of the process; a later call changes nothing.
pub fn install_runtime_declarations(declarations: &'static [&'static RuntimeDeclaration]) {
    let _ = DECLARATIONS.set(declarations);
}

/// All installed compiled runtime declarations; empty before installation.
pub fn runtime_declarations() -> &'static [&'static RuntimeDeclaration] {
    DECLARATIONS.get().copied().unwrap_or(&[])
}

/// The installed declaration matching `name`, if registered.
pub fn runtime_declaration(name: &str) -> Option<&'static RuntimeDeclaration> {
    runtime_declarations()
        .iter()
        .copied()
        .find(|declaration| declaration.name == name)
}

/// Fail-closed runtime resolution: when `configured` names a runtime, it must
/// match a registered declaration; when unset, it selects the sole registered
/// declaration and fails closed if zero or multiple declarations are installed.
pub fn resolve_runtime(
    configured: Option<&str>,
) -> Result<&'static RuntimeDeclaration, ConfigError> {
    resolve_runtime_from(configured, runtime_declarations())
}

pub(crate) fn resolve_runtime_from(
    configured: Option<&str>,
    declarations: &[&'static RuntimeDeclaration],
) -> Result<&'static RuntimeDeclaration, ConfigError> {
    let available = || {
        declarations
            .iter()
            .map(|d| d.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match configured {
        Some(name) => declarations
            .iter()
            .copied()
            .find(|declaration| declaration.name == name)
            .ok_or_else(|| {
                let message = if declarations.is_empty() {
                    format!(
                        "unknown compiled runtime adapter `{name}`; no compiled runtime adapters are registered"
                    )
                } else {
                    format!(
                        "unknown compiled runtime adapter `{name}`; available: {}",
                        available()
                    )
                };
                ConfigError::InvalidValue {
                    key: "runner.runtime".into(),
                    message,
                }
            }),
        None => match declarations {
            [only] => Ok(*only),
            [] => Err(ConfigError::InvalidValue {
                key: "runner.runtime".into(),
                message: "no compiled runtime adapters are registered".into(),
            }),
            _ => Err(ConfigError::InvalidValue {
                key: "runner.runtime".into(),
                message: format!(
                    "multiple compiled runtime adapters are registered ({}); set runner.runtime explicitly",
                    available()
                ),
            }),
        },
    }
}

/// Optional runtime capabilities declared by a workflow, never inferred from prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCapability {
    WorktreeCreate,
    SharedDelegation,
    WorktreeAttach,
    Link,
    Notify,
    AgentSelection,
}
impl WorkflowCapability {
    pub fn supported_by(self, runtime: &RuntimeDeclaration) -> bool {
        match self {
            Self::SharedDelegation => runtime.native_capabilities.shared_delegation,
            Self::WorktreeCreate => runtime.worktree_create,
            Self::WorktreeAttach => runtime.worktree_attach,
            Self::Link => runtime.link,
            Self::Notify => runtime.notify,
            Self::AgentSelection => runtime.unattended_agent_selection,
        }
    }
}
impl std::fmt::Display for WorkflowCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WorktreeCreate => "worktree_create",
            Self::SharedDelegation => "shared_delegation",
            Self::WorktreeAttach => "worktree_attach",
            Self::Link => "link",
            Self::Notify => "notify",
            Self::AgentSelection => "agent_selection",
        })
    }
}

/// A declaration with the capabilities of the default runtime and short
/// stand-in shipped actions, installed by core's own unit tests.
#[cfg(test)]
pub(crate) fn install_fixture() {
    static FIXTURE: RuntimeDeclaration = RuntimeDeclaration {
        name: "t3",
        archives_threads: true,
        worktree_create: true,
        worktree_attach: true,
        cold_text_source: true,
        link: false,
        notify: false,
        unattended_agent_selection: false,
        native_capabilities: NativeCapabilities {
            shared_delegation: true,
            worktree_attach: true,
            agent_selection: true,
        },
        shipped_yaml: "\
runtimes:
  t3:
    dispatch: fixture dispatch
    message_child: fixture message_child
    message_peer: fixture message_peer
    read_thread: fixture read_thread
    inspect_thread: fixture inspect_thread
    schedule_check: fixture schedule_check
    look_around: fixture look_around
    steer: fixture steer
    start_thread: fixture start_thread
    cancel_child: fixture cancel_child
    finish_thread: fixture finish_thread
    discover_environments: fixture discover_environments
    connect_environment: fixture connect_environment
    dispatch_remote: fixture dispatch_remote
",
        legacy_history_normalizer: None,
    };
    static INSTALLED: [&RuntimeDeclaration; 1] = [&FIXTURE];
    install_runtime_declarations(&INSTALLED);
}

/// Agent actions as `(name, contract)`: the contract is the one line the
/// configuration schema prints for the action; its execution wording lives
/// only in configuration data.
pub(crate) const ACTIONS: [(&str, &str); 14] = [
    (
        "dispatch",
        "Delegate this agent's worker with shared isolation in the parent's recorded workspace.",
    ),
    (
        "message_child",
        "Send a bounded follow-up, findings or recovery within an outstanding attempt, or a newly bound attempt's brief, to this parent's identified child.",
    ),
    (
        "message_peer",
        "Send text to an addressed peer or coordinator conversation with source attribution.",
    ),
    (
        "read_thread",
        "Read an identified conversation's relevant messages and attribution.",
    ),
    (
        "inspect_thread",
        "Inspect an identified thread or own child's state or result; a scheduled reply check permits inspecting a running child.",
    ),
    (
        "schedule_check",
        "Set, update or clear this agent's one scheduled reply check through the runtime's scheduler, at the interval the agent supplies.",
    ),
    (
        "look_around",
        "Discover current conversations in the supplied project or scope.",
    ),
    (
        "steer",
        "Deliver current direction at the target's supported safe input boundary.",
    ),
    (
        "start_thread",
        "Start a separately addressed conversation with its supplied brief and placement.",
    ),
    (
        "cancel_child",
        "Stop this parent's identified child, retaining its work and evidence.",
    ),
    (
        "finish_thread",
        "Dormantly settle an addressed conversation, or this agent's own, so it stays wakeable in place with full context; external agents and runner automation never settle a standing agent, which may self-settle when parked dormant.",
    ),
    (
        "discover_environments",
        "Discover connected computers or environments, their projects and capabilities.",
    ),
    (
        "connect_environment",
        "Request the runtime's native connection flow for a named missing computer.",
    ),
    (
        "dispatch_remote",
        "Assign a brief to an explicitly addressed connected environment.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentPreference, Tier};
    use crate::ids::ProfileName;

    static ALPHA: RuntimeDeclaration = RuntimeDeclaration {
        name: "alpha",
        archives_threads: true,
        worktree_create: true,
        worktree_attach: true,
        cold_text_source: true,
        link: false,
        notify: false,
        unattended_agent_selection: false,
        native_capabilities: NativeCapabilities {
            shared_delegation: true,
            worktree_attach: true,
            agent_selection: true,
        },
        shipped_yaml: "",
        legacy_history_normalizer: None,
    };

    static BETA: RuntimeDeclaration = RuntimeDeclaration {
        name: "beta",
        archives_threads: false,
        worktree_create: false,
        worktree_attach: false,
        cold_text_source: false,
        link: false,
        notify: false,
        unattended_agent_selection: false,
        native_capabilities: NativeCapabilities {
            shared_delegation: false,
            worktree_attach: false,
            agent_selection: false,
        },
        shipped_yaml: "",
        legacy_history_normalizer: None,
    };

    #[test]
    fn resolve_runtime_fails_closed_when_zero_or_ambiguous_or_unregistered() {
        assert!(matches!(
            resolve_runtime_from(None, &[]),
            Err(ConfigError::InvalidValue { ref key, .. }) if key == "runner.runtime"
        ));
        assert!(matches!(
            resolve_runtime_from(Some("alpha"), &[]),
            Err(ConfigError::InvalidValue { ref key, .. }) if key == "runner.runtime"
        ));
        assert_eq!(
            resolve_runtime_from(None, &[&ALPHA]).unwrap().name(),
            "alpha"
        );
        assert_eq!(
            resolve_runtime_from(Some("alpha"), &[&ALPHA])
                .unwrap()
                .name(),
            "alpha"
        );
        assert!(matches!(
            resolve_runtime_from(Some("unknown"), &[&ALPHA]),
            Err(ConfigError::InvalidValue { ref key, .. }) if key == "runner.runtime"
        ));
        assert!(matches!(
            resolve_runtime_from(None, &[&ALPHA, &BETA]),
            Err(ConfigError::InvalidValue { ref key, .. }) if key == "runner.runtime"
        ));
        assert_eq!(
            resolve_runtime_from(Some("beta"), &[&ALPHA, &BETA])
                .unwrap()
                .name(),
            "beta"
        );
    }

    #[test]
    fn agent_preference_preserves_wire_shape_and_ignores_unknown_legacy_fields() {
        let with_tier: AgentPreference =
            serde_json::from_str(r#"{"agent":"codex","tier":"strong","legacy_extra":"ignored"}"#)
                .unwrap();
        assert_eq!(
            with_tier,
            AgentPreference {
                agent: ProfileName::parse("codex").unwrap(),
                tier: Some(Tier::Strong),
            }
        );
        assert_eq!(
            serde_json::to_string(&with_tier).unwrap(),
            r#"{"agent":"codex","tier":"strong"}"#
        );

        let without_tier: AgentPreference = serde_json::from_str(r#"{"agent":"claude"}"#).unwrap();
        assert_eq!(
            without_tier,
            AgentPreference {
                agent: ProfileName::parse("claude").unwrap(),
                tier: None,
            }
        );
        assert_eq!(
            serde_json::to_string(&without_tier).unwrap(),
            r#"{"agent":"claude"}"#
        );
    }
}
