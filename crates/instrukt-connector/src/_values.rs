//! The values a connector and the connector host exchange.
use std::sync::Arc;
use std::time::Duration;

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
    /// The context bound expired without a usable read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unread: bool,
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

/// The clock a kind keeps its provider's waits by.
pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// Why a connector could not do what it was asked; the cause is kept where
/// the work it concerns is recorded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{cause}")]
pub struct ConnectorError {
    pub cause: String,
}

/// The result of an unsuccessful attempt to post.
#[derive(Debug, thiserror::Error)]
#[error("{cause}")]
pub struct AnswerError {
    pub cause: String,
    /// The provider confirmed that it rejected the post.
    pub rejected: bool,
}

/// What the provider says about the sender of an addressed message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// One of the company's own, with what the provider gave about them.
    Admitted(Sender),
    /// Not one of the company's own: they receive the fixed reply.
    NotOwn,
    /// Nothing is decided yet; the message waits, with this cause.
    Undecided(String),
}

/// An addressed message as the agent reads it, and what accompanies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// The addressed text, mentions written as names.
    pub text: String,
    pub accompanying: Accompanying,
}

/// Render context already read, using the names available at delivery.
/// This snapshot performs no provider I/O.
pub type ContextSnapshot = Box<dyn FnOnce() -> Captured + Send>;

/// What a connector declares about its channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most characters one posted part of an answer holds.
    pub message_chars: usize,
    /// The provider request bound for posting an answer.
    pub answer_timeout: Duration,
    /// How often the working indicator is set again while it stays; `None`
    /// when the provider has no indicator.
    pub working_refresh: Option<Duration>,
}

/// Where the status for an addressed message sits, as its connector names
/// it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Place(pub String, pub String);

/// What a status shows a person of their unfinished message.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    /// The assistant is working on the request.
    Working,
    /// The assistant cannot work on it now, and the message is saved.
    Unavailable,
    /// The newest message the agent has written in the running turn, as
    /// written: shown as it is.
    Update(String),
}
