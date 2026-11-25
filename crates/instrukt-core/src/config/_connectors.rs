//! The connector kinds this binary supports, as data its composition root
//! installs once before any configuration is read (`connectors` architecture,
//! decision 4). Core validates a company's `connectors` map against them and
//! names no kind itself.

use std::sync::OnceLock;

use instrukt_connector::Declaration;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use crate::ids::RoleName;

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
pub fn connector_declaration(key: &str) -> Option<&'static Declaration> {
    declarations()
        .iter()
        .copied()
        .find(|declaration| declaration.key == key)
}

pub(super) fn declaration(key: &str) -> Option<&'static Declaration> {
    connector_declaration(key)
}

/// Who is admitted to address a company connector.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorAdmit {
    #[default]
    Members,
    AdminsOnly,
}

impl ConnectorAdmit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Members => "members",
            Self::AdminsOnly => "admins_only",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "members" => Some(Self::Members),
            "admins_only" => Some(Self::AdminsOnly),
            _ => None,
        }
    }
}

/// How sessions are scoped for a company connector.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionScope {
    #[default]
    Conversation,
    Company,
}

impl SessionScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Company => "company",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "conversation" => Some(Self::Conversation),
            "company" => Some(Self::Company),
            _ => None,
        }
    }
}

/// How replies are posted back to a company connector.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyMode {
    #[default]
    TurnOutput,
    Explicit,
}

impl ReplyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TurnOutput => "turn_output",
            Self::Explicit => "explicit",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "turn_output" => Some(Self::TurnOutput),
            "explicit" => Some(Self::Explicit),
            _ => None,
        }
    }
}

/// One named connector a company home declares: its declared name (`key`),
/// registered kind (`kind`), generic conversation settings (`role`, `admit`,
/// `session_scope`, `reply_mode`), and its entry as the document declares it,
/// references unresolved.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredConnector {
    pub key: String,
    pub kind: String,
    pub role: RoleName,
    pub admit: ConnectorAdmit,
    pub session_scope: SessionScope,
    pub reply_mode: ReplyMode,
    pub entry: Value,
}

/// A declaration with the fields of the one shipped for Slack, and a short
/// setup and manifest standing in for its own, installed by core's own units
/// so their documents and expectations stay as they are.
#[cfg(test)]
pub(crate) fn install_fixture() {
    use instrukt_connector::{Field, Rule};
    static FIXTURE: Declaration = Declaration {
        key: "slack",
        message_chars: 12_000,
        setup: "Create the app from the manifest below; its tokens' references go into `app_token` and `bot_token`.",
        manifest: Some("display_information:\n  name: agent\n"),
        fields: &[
            Field {
                name: "app_token",
                rule: Rule::SecretReference {
                    required: true,
                    source: "the app-level token with the scope `connections:write`, generated in the Slack app's settings",
                },
            },
            Field {
                name: "bot_token",
                rule: Rule::SecretReference {
                    required: true,
                    source: "the bot token Slack issues when the app is installed to the workspace",
                },
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
