//! Conversations (`conversations` spec): a person addresses a company's
//! agent through a connector; each conversation is bound to one plain agent
//! session in the company's home, every addressed message is its own turn,
//! and the agent replies explicitly via `instrukt company reply` (and may
//! optionally publish short active-turn progress via `instrukt company status`).
//! No event is emitted and no subscribed procedure is activated on this path.
//!
//! The record moves each addressed message through its stages: received
//! (retained, with its sender's connector identity, before the connector
//! acknowledges it), refused (its provider's record shows its sender is not
//! one of the company's own), pending (its sender admitted and its
//! accompanying messages captured), sent (accepted by the runtime as its own
//! turn), failed (its turn failed without a reply), and answered (its turn
//! finished with an explicit reply or settled terminally with a standard error
//! reply). Each answer piece stays pending until the connector accepted its post.

mod _availability;
mod _delivery;
mod _envelopes;
mod _intake;
mod _ports;
mod _relay;
mod _replies_db;
mod _speaker;
mod _status;
mod _store;
mod _turn_status;
mod _turns;
pub mod read_back;

pub mod envelopes {
    pub use super::_envelopes::{
        ContextBody, ContextLine, MemberEnvelope, Span, context_body, member, segment,
    };
    pub(crate) use super::_envelopes::{control_field, delivery_boundary};
}

pub use _availability::{
    INTAKE_INTERVAL, NOTICE_AFTER, Notice, NoticeStep, Unfinished, effective_since, notice_posted,
    notice_step, notices, unfinished,
};
pub use _delivery::{FAILED_RETRY_DELAY, FAILED_RETRY_LIMIT, FAILED_RETRY_MAX_ATTEMPTS};
pub(crate) use _intake::context_exclusions;
pub use _intake::{
    Intake, Refusal, Unread, admitted, admitted_demand, captured, demand, intake_failed,
    refusal_posted, refusals, refused, retain_with_policy, unread, waiting,
};
pub use _ports::Ports;
pub use _relay::{ANSWER_OPERATOR_BOUND, Accepted, relay_state, split};
pub use _replies_db::{
    QueuedReply, has_uningested_replies, ingest_explicit_replies, queue_explicit_reply,
};
pub use _speaker::{
    ConversationBinding, SpeakerError, conversation_assignment, conversation_binding, speaker,
    speaker_within,
};
pub use _status::{
    ConnectionState, ConnectorStatus, ConversationStatus, Status, connector_state, status,
    status_at,
};
pub use _store::{COMPANY_SCOPE_CONVERSATION, company_key, key, outstanding};
pub use _turn_status::{
    MAX_STATUS_BYTES, StatusText, TURN_STATUS_SCHEMA_VERSION, TurnStatusFile, TurnStatusSummary,
    read_turn_status, set_turn_status,
};
pub use _turns::{STANDARD_ERROR_REPLY, UNOBSERVED_SENT_GRACE};

use instrukt_core::config::ConfigError;
use instrukt_runner::runtime::RuntimeError;
use instrukt_runner::sessions::SessionsError;
use instrukt_runner::state::StateError;

/// The target every log statement here carries: the module path it logged
/// under before the company context became its own crate, so the same
/// settings select it.
const LOG_TARGET: &str = "instrukt_runner::conversations";

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Sessions(#[from] SessionsError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Lock(#[from] instrukt_core::fsx::FsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{}", .0.cause)]
    Connector(#[from] instrukt_connector::ConnectorError),
    #[error(transparent)]
    Index(#[from] instrukt_core::index::IndexError),
    #[error(
        "conversation: no published procedure declares the role `{role}`; a conversation starts once one does"
    )]
    NoProcedure { role: String },
    #[error("conversation: {0}")]
    Refused(String),
}

#[cfg(test)]
mod tests;
