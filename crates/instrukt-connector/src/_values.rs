//! The values a connector and the connector host exchange.
use std::sync::Arc;
use std::time::Duration;

use instrukt_outside::Wait;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// One earlier message of a conversation, kept with the addressed message
/// it accompanies until that message is delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Earlier {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub speaker: String,
    pub own: bool,
    pub at: Option<Timestamp>,
    pub text: String,
}

/// Earlier messages available at capture and their delivery limit. The
/// conversation core applies the limit after excluding known deliveries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accompanying {
    pub messages: Vec<Earlier>,
    /// The connector's delivery limit. Older captures were trimmed already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    pub left_out: bool,
    /// Older pages were left unread because the selection was known.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped_early: bool,
    /// A reply range is complete from this timestamp. Older pages already
    /// fetched may be retained even though they do not prove coverage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complete_from: Option<Timestamp>,
    /// Delivery needs a fresh read because eligibility changed after a short read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refresh: bool,
}

/// What a connector supplies for one message addressed to the agent, as it
/// arrives. The company whose connector received it is the host's to add.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Whether the conversation is a direct message rather than a channel or group.
    #[serde(default)]
    pub is_direct: bool,
    /// Optional connector-supplied thread identifier when the conversation is
    /// threaded (`None` for direct messages or unthreaded places). Keeps
    /// `conversation` opaque to generic company logic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

/// A provider page and the continuation for the next request.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ReadBackPage {
    pub items: Vec<(Incoming, String)>,
    pub next: Option<String>,
    #[serde(default)]
    pub coverage: Option<ReadBackCoverage>,
    #[serde(default)]
    pub window_started: Option<Timestamp>,
}

/// An unavailable channel and the original window still owed to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadBackGap {
    pub channel: String,
    pub direct: bool,
    pub from: Timestamp,
    pub window_started: Timestamp,
}

/// A change in coverage for one channel.
#[derive(Debug, Serialize, Deserialize)]
pub enum ReadBackCoverage {
    ChannelUnavailable {
        channel: String,
        direct: bool,
        expected: bool,
    },
    ChannelCovered(String),
}

/// Why a page could not be read; `wait` is the outside gate's wait when the
/// provider was unavailable.
#[derive(Debug, thiserror::Error)]
#[error("{}", rendered(cause, wait.as_ref()))]
pub struct ReadBackError {
    pub cause: String,
    pub wait: Option<Wait>,
}

/// A cause followed by the outside gate's rendering of its wait, when it has
/// one: what a recorded cause shows the operator.
fn rendered(cause: &str, wait: Option<&Wait>) -> String {
    match wait {
        Some(wait) => format!("{cause}; {wait}"),
        None => cause.to_owned(),
    }
}

/// The sender of an admitted message, as the provider gave them when it was
/// admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    pub name: String,
    pub email: Option<String>,
    pub is_admin: bool,
}

/// The clock a kind keeps its provider's waits by.
pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// Why a connector could not do what it was asked; its rendering is kept
/// where the work it concerns is recorded. `wait` is the outside gate's wait
/// when the provider was unavailable, which the host paces by.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", rendered(cause, wait.as_ref()))]
pub struct ConnectorError {
    pub cause: String,
    pub wait: Option<Wait>,
}

impl ConnectorError {
    /// A failure without a wait.
    pub fn new(cause: impl Into<String>) -> Self {
        Self {
            cause: cause.into(),
            wait: None,
        }
    }

    /// The provider is unavailable until `wait`.
    pub fn waiting(cause: impl Into<String>, wait: Wait) -> Self {
        Self {
            cause: cause.into(),
            wait: Some(wait),
        }
    }
}

/// Evidence returned by one successful provider post; neither field is required.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnswerReceipt {
    pub message_id: Option<String>,
    pub posted_at: Option<Timestamp>,
}

/// The result of an unsuccessful attempt to post.
#[derive(Debug, thiserror::Error)]
#[error("{cause}")]
pub struct AnswerError {
    pub cause: String,
}

/// Receipt returned after posting a private direct message to a user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirectMessageReceipt {
    pub channel: Option<String>,
    pub message_id: Option<String>,
}

/// Failure posting a private direct message to a user, classified by whether
/// the provider failure is retryable (rate limit, server error, transport) or final.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DirectMessageError {
    #[error("{0}")]
    Retryable(String),
    #[error("{0}")]
    Final(String),
}

/// What the provider says about the sender of an addressed message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// One of the company's own, with what the provider gave about them.
    Admitted(Sender),
    /// Not one of the company's own: they receive the fixed reply.
    NotOwn,
    /// Nothing is decided yet; the message waits, with this cause and the
    /// outside gate's wait when the provider was unavailable.
    Undecided { cause: String, wait: Option<Wait> },
}

/// Why an addressed message was refused without starting or waking a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// Sender is not a full member of the company's workspace.
    NotOwn,
    /// Connector admits workspace administrators only and sender is not one.
    NotAdmin,
}

impl RefusalReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotOwn => "not_own",
            Self::NotAdmin => "not_admin",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "not_own" => Some(Self::NotOwn),
            "not_admin" => Some(Self::NotAdmin),
            _ => None,
        }
    }
}

/// Optional provider-owned presentation facts; never conversation identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationDisplay {
    Channel(String),
    Direct(String),
}

/// An addressed message as the agent reads it, and what accompanies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    pub display: Option<ConversationDisplay>,
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
