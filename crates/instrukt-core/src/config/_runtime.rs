//! Compiled runtime selection and caller context. No executable configuration.
use std::collections::BTreeMap;

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

/// A runtime compiled into instrukt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    T3,
}
impl Runtime {
    /// The one runtime compiled into this instrukt: what an unset
    /// `runner.runtime` means.
    pub const COMPILED: Self = Self::T3;
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "t3" => Some(Self::T3),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::T3 => "t3",
        }
    }
    pub fn caller(self, env: &Env) -> Option<crate::ids::SessionRef> {
        env.caller_identity(self.name())
            .map(|caller| caller.session)
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
/// Native agent operations are independent of unattended host-CLI support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCapabilities {
    pub shared_delegation: bool,
    pub worktree_attach: bool,
    pub agent_selection: bool,
}
impl Runtime {
    pub fn native_capabilities(self) -> NativeCapabilities {
        match self {
            Self::T3 => NativeCapabilities {
                shared_delegation: true,
                worktree_attach: true,
                agent_selection: true,
            },
        }
    }
}
impl WorkflowCapability {
    pub fn supported_by(self, runtime: Runtime) -> bool {
        match self {
            Self::SharedDelegation => runtime.native_capabilities().shared_delegation,
            // These requirements describe unattended placement/adapter operations.
            // Native model options remain distinct from the consumed launch operations.
            other => match runtime {
                Runtime::T3 => matches!(other, Self::WorktreeCreate | Self::WorktreeAttach),
            },
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

/// Agent actions as `(name, contract)`: the contract is the one line the
/// configuration schema prints for the action; its execution wording lives
/// only in configuration data.
pub(crate) const ACTIONS: [(&str, &str); 15] = [
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
        "Inspect an identified thread or own child's state or result.",
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
        "discuss",
        "Open, take part in and close a runner-owned conversation link between two peer threads.",
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
        "Dormantly settle an addressed conversation, or this agent's own, so it stays wakeable in place with full context; never another standing agent's, and a standing agent's own only at a rest its procedure names.",
    ),
    (
        "retire_thread",
        "Terminally retire a delivered or withdrawn todo's orchestrator thread, retaining its transcript; never a standing agent.",
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
