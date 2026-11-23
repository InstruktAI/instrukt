//! The operations of a channel provider and of one company's connector, and
//! the host's side they call back into.
use crate::_declaration::{Declaration, Declared};
use crate::_values::{
    Admission, AnswerError, AnswerReceipt, Captured, ConnectorError, ContextSnapshot,
    DirectMessageError, DirectMessageReceipt, Incoming, Limits, Place, ReadBackError, ReadBackGap,
    ReadBackPage, RefusalReason, Status,
};
use jiff::Timestamp;

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
    /// Provider evidence required before the connection is allowed to serve.
    fn check_permissions(&self) -> Result<(), ConnectorError> {
        Ok(())
    }
    /// What this channel allows.
    fn limits(&self) -> Limits;
    /// Hold the connection until it drops or `quit` says so, handing each
    /// addressed message to `inbox`. A kind whose messages are delivered to
    /// instrukt rather than received over a connection it holds returns at
    /// once.
    fn listen(&self, inbox: &dyn Inbox, quit: &dyn Fn() -> bool) -> Result<(), ConnectorError>;
    /// Fetch one page, resuming a continuation retained by the host.
    fn read_back_page(
        &self,
        _from: Timestamp,
        _position: Option<String>,
        _gaps: &[ReadBackGap],
    ) -> Result<ReadBackPage, ReadBackError> {
        Ok(ReadBackPage::default())
    }
    /// Whether `sender`, a provider identity, is one of the company's own.
    fn admit(&self, sender: &str) -> Admission;
    /// The addressed message `message` of `conversation` as the agent reads
    /// it and the earlier messages in its provider's context range.
    /// Publish the complete selection with cached names before optional enrichment.
    fn capture(
        &self,
        conversation: &str,
        message: &str,
        text: &str,
        excluded: &std::collections::HashSet<String>,
        readable: &dyn Fn(ContextSnapshot),
    ) -> Result<Captured, ConnectorError>;
    /// Post one answer piece, returning the provider's optional id and posting time.
    fn answer(&self, conversation: &str, part: &str) -> Result<AnswerReceipt, AnswerError>;
    /// Post the connector's fixed reply to someone who is not answered.
    fn turn_away(&self, conversation: &str, reason: RefusalReason) -> Result<(), ConnectorError>;
    /// Make one bounded post of the connector's fixed unavailable notice,
    /// beginning with `member`'s mention. The caller records the attempt first.
    fn unavailable_notice(&self, conversation: &str, member: &str) -> Result<(), ConnectorError>;
    /// Send a private direct message to `recipient` (a provider user identity) with `text`,
    /// such as an MCP sign-in link or outcome notice. The connector tags it private so it
    /// never accompanies any later message to the agent.
    fn direct_message(
        &self,
        _recipient: &str,
        _text: &str,
    ) -> Result<DirectMessageReceipt, DirectMessageError> {
        Err(DirectMessageError::Final(
            "this connector does not support direct messages".into(),
        ))
    }
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
    /// One company's named connector, from what that company declared.
    fn open(
        &self,
        company: &str,
        connector: &str,
        declared: &Declared,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn Connector>, ConnectorError>;
}
