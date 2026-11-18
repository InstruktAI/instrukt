//! The connector interface of the company context
//! (`docs/project/spec/domain/company.md`): what a channel provider's
//! connector and the company's connector host exchange. It depends on nothing
//! of instrukt, so a connector builds against it alone.
//!
//! - [`Kind`] — one channel provider: its [`Declaration`] and the opening of
//!   one company's [`Connector`].
//! - [`Connector`] — one company's channel: what the connector contract of
//!   `docs/project/spec/feature/company/conversations.md` asks of it.
//! - [`Inbox`] — the host's side of [`Connector::listen`].
//! - [`Declaration`] and [`Declared`] — the fields a kind's `connectors.<kind>`
//!   entry takes, as data configuration validates against, and the values one
//!   company declared.
use std::collections::BTreeMap;
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

/// What a connector declares about its channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most characters one posted part of an answer holds.
    pub message_chars: usize,
    /// How often the working indicator is set again while it stays; `None`
    /// when the provider has no indicator.
    pub working_refresh: Option<Duration>,
}

/// Where the working indicator for an addressed message sits, as its
/// connector names it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Place(pub String, pub String);

/// The host's side of [`Connector::listen`].
pub trait Inbox: Sync {
    /// Retain an addressed message; returns once it is durably retained, so
    /// the connector acknowledges its provider only afterwards.
    fn retain(&self, incoming: Incoming) -> Result<(), ConnectorError>;
    /// The connection is up; an earlier unretained cause no longer holds.
    fn connected(&self);
    /// A message could not be retained and awaits the provider's retry.
    fn unretained(&self, cause: &str);
}

/// One company's channel.
pub trait Connector: Send + Sync {
    /// What this channel allows.
    fn limits(&self) -> Limits;
    /// Hold the connection until it drops or `quit` says so, handing each
    /// addressed message to `inbox`. A kind whose messages are delivered to
    /// instrukt rather than received over a connection it holds returns at
    /// once.
    fn listen(&self, inbox: &dyn Inbox, quit: &dyn Fn() -> bool) -> Result<(), ConnectorError>;
    /// Whether `sender`, a provider identity, is one of the company's own.
    fn admit(&self, sender: &str) -> Admission;
    /// The addressed message `message` of `conversation` as the agent reads
    /// it, with what accompanies it: since `previous`, the conversation's
    /// previous addressed message, or from the conversation's start.
    fn capture(
        &self,
        conversation: &str,
        message: &str,
        previous: Option<&str>,
        text: &str,
    ) -> Result<Captured, ConnectorError>;
    /// Post one part of an answer to `conversation`.
    fn answer(&self, conversation: &str, part: &str) -> Result<(), ConnectorError>;
    /// Post the connector's fixed reply to someone who is not answered.
    fn turn_away(&self, conversation: &str) -> Result<(), ConnectorError>;
    /// Where the working indicator for `message` of `conversation` sits.
    fn place(&self, conversation: &str, message: &str) -> Place;
    /// Set or clear the working indicator at `place`; best effort.
    fn working(&self, place: &Place, on: bool) -> Result<(), ConnectorError>;
}

/// Resolves a `${NAME}` reference against the declaring company's secrets.
pub trait SecretResolver: Send + Sync {
    fn resolve(&self, reference: &str) -> Result<String, ConnectorError>;
}

/// One channel provider.
pub trait Kind: Send + Sync {
    /// The fields its `connectors.<kind>` entry takes.
    fn declaration(&self) -> &'static Declaration;
    /// One company's connector, from what that company declared.
    fn open(
        &self,
        company: &str,
        declared: &Declared,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn Connector>, ConnectorError>;
}

/// The fields a kind's `connectors.<kind>` entry takes, as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declaration {
    /// The kind's key under `connectors`.
    pub key: &'static str,
    pub fields: &'static [Field],
}

/// One field of a declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub rule: Rule,
}

/// What a field's value must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// An exact `${NAME}` reference into the home's secrets file.
    SecretReference { required: bool },
    /// A positive whole number, `default` when omitted; `meaning` says what
    /// it is, as its finding and editor schema state it.
    PositiveCount { default: u64, meaning: &'static str },
}

/// The values one company declared for a kind, by field name; references
/// stay unresolved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    Count(u64),
}

impl Declared {
    /// Declares `name` as the text `value`.
    pub fn insert_text(&mut self, name: &str, value: &str) {
        self.values
            .insert(name.to_owned(), Value::Text(value.to_owned()));
    }

    /// Declares `name` as the count `value`.
    pub fn insert_count(&mut self, name: &str, value: u64) {
        self.values.insert(name.to_owned(), Value::Count(value));
    }

    /// The text declared for `name`, if it was declared as text.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.values.get(name)? {
            Value::Text(value) => Some(value),
            Value::Count(_) => None,
        }
    }

    /// The count declared for `name`, if it was declared as a count.
    pub fn count(&self, name: &str) -> Option<u64> {
        match self.values.get(name)? {
            Value::Count(value) => Some(*value),
            Value::Text(_) => None,
        }
    }
}
