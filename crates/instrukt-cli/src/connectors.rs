//! The connector kinds this binary is built with: each kind's declaration,
//! installed into configuration before anything reads it, and how the kind
//! is built for a process from its environment and clock.
use std::sync::LazyLock;

use instrukt_company::BuildKind;
use instrukt_connector::Declaration;
use instrukt_connector_slack::{SLACK_DECLARATION, SlackKind};

/// Every connector kind of this build.
pub static REGISTRY: [(&Declaration, BuildKind); 1] = [(&SLACK_DECLARATION, SlackKind::build)];

/// The declarations of [`REGISTRY`], as configuration takes them.
pub static DECLARATIONS: LazyLock<Vec<&'static Declaration>> = LazyLock::new(|| {
    REGISTRY
        .iter()
        .map(|(declaration, _)| *declaration)
        .collect()
});

/// How each kind of [`REGISTRY`] is built, as the runner takes them.
pub fn builders() -> Vec<BuildKind> {
    REGISTRY.iter().map(|(_, build)| *build).collect()
}
