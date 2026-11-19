//! The connector kinds this binary is built with: each kind's declaration,
//! installed into configuration before anything reads it, and how the kind
//! is built for a process from its environment and clock.
use std::sync::LazyLock;

use instrukt_connector::{Clock, ConnectorError, Declaration, Kind};
use instrukt_core::config::Env;
use instrukt_runner::slack::{SLACK_DECLARATION, SlackKind};

/// How a kind is built once for a process.
pub type Build = fn(&Env, Clock) -> Result<Box<dyn Kind>, ConnectorError>;

/// Every connector kind of this build.
pub static REGISTRY: [(&Declaration, Build); 1] = [(&SLACK_DECLARATION, SlackKind::build)];

/// The declarations of [`REGISTRY`], as configuration takes them.
pub static DECLARATIONS: LazyLock<Vec<&'static Declaration>> = LazyLock::new(|| {
    REGISTRY
        .iter()
        .map(|(declaration, _)| *declaration)
        .collect()
});

#[cfg(test)]
mod tests {
    use instrukt_connector::Rule;
    use instrukt_core::config::{FindingKind, connector_findings, install_connector_declarations};

    use super::*;

    fn judged(text: &str) -> Vec<(String, FindingKind)> {
        install_connector_declarations(&DECLARATIONS);
        connector_findings(&serde_yaml_ng::from_str(text).unwrap(), "connectors")
    }

    #[test]
    fn a_slack_declaration_needs_both_references_and_a_positive_context() {
        use FindingKind::{InvalidValue, Missing, UnknownKey};
        assert_eq!(
            judged("slack: {app_token: literal, context_messages: 0, extra: 1}\nteams: {}\n"),
            [
                ("connectors.slack.bot_token".to_owned(), Missing),
                ("connectors.slack.app_token".to_owned(), InvalidValue),
                ("connectors.slack.context_messages".to_owned(), InvalidValue),
                ("connectors.slack.extra".to_owned(), UnknownKey),
                ("connectors.teams".to_owned(), UnknownKey),
            ]
        );
        // Both references suffice; the context defaults to 30.
        assert_eq!(
            judged("slack: {app_token: '${A}', bot_token: '${B}'}\n"),
            []
        );
        let context = SLACK_DECLARATION
            .fields
            .iter()
            .find(|field| field.name == "context_messages")
            .map(|field| field.rule);
        assert!(matches!(
            context,
            Some(Rule::PositiveCount { default: 30, .. })
        ));
    }
}
