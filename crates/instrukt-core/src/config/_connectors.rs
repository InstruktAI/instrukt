//! The connector kinds this binary supports, as data its composition root
//! installs once before any configuration is read (`connectors` architecture,
//! decision 4). Core validates a company's `connectors` map against them and
//! names no kind itself.

use std::sync::OnceLock;

use instrukt_connector::Declaration;
use serde_yaml_ng::Value;

/// The installed declarations: immutable data describing the build, set
/// once and never replaced.
static DECLARATIONS: OnceLock<&'static [&'static Declaration]> = OnceLock::new();

/// Install the connector kinds this binary supports. The first installation
/// holds for the life of the process; a later one changes nothing.
pub fn install_connector_declarations(declarations: &'static [&'static Declaration]) {
    let _ = DECLARATIONS.set(declarations);
}

/// The installed declarations; none before installation, so a declared
/// connector is then unsupported.
pub(super) fn declarations() -> &'static [&'static Declaration] {
    DECLARATIONS.get().copied().unwrap_or(&[])
}

/// The installed declaration of the kind `key`.
pub(super) fn declaration(key: &str) -> Option<&'static Declaration> {
    declarations()
        .iter()
        .copied()
        .find(|declaration| declaration.key == key)
}

/// One connector a company home declares: its kind's key and its entry as
/// the document declares it, references unresolved.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredConnector {
    pub key: String,
    pub entry: Value,
}

/// A declaration equal to the one shipped for Slack, installed by core's own
/// units so their documents and expectations stay as they are.
#[cfg(test)]
pub(crate) fn install_fixture() {
    use instrukt_connector::{Field, Rule};
    static FIXTURE: Declaration = Declaration {
        key: "slack",
        fields: &[
            Field {
                name: "app_token",
                rule: Rule::SecretReference { required: true },
            },
            Field {
                name: "bot_token",
                rule: Rule::SecretReference { required: true },
            },
            Field {
                name: "context_messages",
                rule: Rule::PositiveCount {
                    default: 30,
                    meaning: "the largest number of earlier messages that accompany an addressed message: a positive whole number; 30 when omitted",
                },
            },
        ],
    };
    static INSTALLED: [&Declaration; 1] = [&FIXTURE];
    install_connector_declarations(&INSTALLED);
}
