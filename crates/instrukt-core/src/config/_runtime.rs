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
}

/// A runtime compiled into instrukt; selection is always explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    T3,
}
impl Runtime {
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
    /// Families in the compiled T3 catalogue; provider ids remain runtime facts.
    pub fn startable_agents(self) -> Option<&'static [&'static str]> {
        match self {
            Self::T3 => Some(&["claude", "codex", "gemini"]),
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
            // Native MCP attachment or model options do not implement those CLI routes.
            other => match runtime {
                Runtime::T3 => other == Self::WorktreeCreate,
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

/// Agent action names; their execution wording lives only in configuration data.
pub(crate) const ACTIONS: [&str; 14] = [
    "dispatch",
    "message_child",
    "message_peer",
    "read_thread",
    "inspect_thread",
    "look_around",
    "steer",
    "discuss",
    "start_thread",
    "cancel_child",
    "finish_thread",
    "discover_environments",
    "connect_environment",
    "dispatch_remote",
];
