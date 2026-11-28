//! T3 Code runtime adapter implementation wired solely by `instrukt-cli`.
mod _projection;
mod _t3;
mod _t3_conversations;
mod _t3_diagnostics;
mod _t3_fingerprint;
mod _t3_home;
mod _t3_notifications;
mod _t3_options;
mod _t3_readiness;

use std::sync::Arc;

use instrukt_core::config::{Env, NativeCapabilities, RuntimeDeclaration};
use instrukt_runtime::{RuntimeAdapter, RuntimeFactory};

pub use _t3::T3;
pub use _t3_conversations::{reports_background_command, without_captured_context};

/// Normalizes a legacy T3 history message record in place.
/// Returns `false` when the record is a background command report that should be dropped.
pub fn normalize_legacy_history_message(message: &mut serde_json::Value) -> bool {
    if reports_background_command(message) {
        return false;
    }
    without_captured_context(message);
    true
}

/// Static declaration for the compiled T3 runtime adapter, installed at CLI startup.
pub static T3_DECLARATION: RuntimeDeclaration = RuntimeDeclaration {
    name: "t3",
    archives_threads: true,
    worktree_create: true,
    worktree_attach: true,
    cold_text_source: true,
    link: false,
    notify: false,
    unattended_agent_selection: true,
    native_capabilities: NativeCapabilities {
        shared_delegation: true,
        worktree_attach: true,
        agent_selection: true,
    },
    shipped_yaml: include_str!("../shipped/t3.yaml"),
    legacy_history_normalizer: Some(normalize_legacy_history_message),
};

/// Factory constructing one-shot or long-lived T3 adapters.
#[derive(Debug, Default)]
pub struct T3Factory;

impl RuntimeFactory for T3Factory {
    fn runtime_name(&self) -> &'static str {
        T3_DECLARATION.name
    }

    fn create(&self, env: &Env) -> Arc<dyn RuntimeAdapter> {
        Arc::new(T3::with_env(env.clone()))
    }

    fn create_long_lived(&self, env: &Env) -> Arc<dyn RuntimeAdapter> {
        Arc::new(T3::long_lived(env.clone()))
    }
}
