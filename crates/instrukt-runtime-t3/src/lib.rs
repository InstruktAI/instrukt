//! T3 Code runtime adapter implementation wired solely by `instrukt-cli`.
mod _factory;
mod _projection;
mod _t3;
mod _t3_conversations;
mod _t3_diagnostics;
mod _t3_fingerprint;
mod _t3_home;
mod _t3_notifications;
mod _t3_options;
mod _t3_readiness;

use instrukt_core::config::{NativeCapabilities, RuntimeDeclaration};

pub use _factory::T3Factory;
pub use _t3::T3;
pub use _t3_conversations::{
    normalize_legacy_history_message, reports_background_command, without_captured_context,
};

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
