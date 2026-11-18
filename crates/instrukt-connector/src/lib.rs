//! The connector interface of the company context
//! (`docs/project/spec/domain/company.md`): the values a channel provider's
//! connector and the conversation core exchange. It depends on nothing of
//! instrukt, so a connector builds against it alone.
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// One earlier message of a conversation, kept with the addressed message
/// it accompanies until that message is delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Earlier {
    pub speaker: String,
    pub own: bool,
    pub at: Option<Timestamp>,
    pub text: String,
}

/// What accompanies one addressed message: at most `context_messages` of
/// the most recent earlier messages, and whether earlier ones were left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accompanying {
    pub messages: Vec<Earlier>,
    pub left_out: bool,
}

/// What a connector supplies for one message addressed to the agent, as it
/// arrives. The company whose connector received it is the host's to add.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub connector: String,
    pub conversation: String,
    /// The connector's identity of the message, unique in its conversation.
    pub message: String,
    /// The sender's identity on the connector. It decides nothing:
    /// admission reads the provider's record for it.
    pub sender: String,
    pub written_at: Timestamp,
    pub text: String,
}

/// The sender of an admitted message, as the provider gave them when it was
/// admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    pub name: String,
    pub email: Option<String>,
}
