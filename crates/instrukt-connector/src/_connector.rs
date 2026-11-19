//! The operations of a channel provider and of one company's connector, and
//! the host's side they call back into.
use crate::_declaration::{Declaration, Declared};
use crate::_values::{Admission, Captured, ConnectorError, Incoming, Limits, Place, Status};

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
    /// The text of each post of this connector's own in `conversation` after
    /// its provider message `since`, oldest first: how a part whose post lost
    /// its response, or whose acceptance went unrecorded, is recognised.
    fn own_posts(&self, conversation: &str, since: &str) -> Result<Vec<String>, ConnectorError>;
    /// Post the connector's fixed reply to someone who is not answered.
    fn turn_away(&self, conversation: &str) -> Result<(), ConnectorError>;
    /// The one notice that the assistant is temporarily unavailable, worded
    /// by the connector and beginning with `member`'s mention; it is posted
    /// through [`Connector::answer`] and recognised among
    /// [`Connector::own_posts`] by its beginning.
    fn unavailable_notice(&self, member: &str) -> String;
    /// Where the status for `message` of `conversation` sits.
    fn place(&self, conversation: &str, message: &str) -> Place;
    /// Show `state` at `place`, or clear what is shown there with `None`;
    /// best effort.
    fn status(&self, place: &Place, state: Option<&Status>) -> Result<(), ConnectorError>;
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
