//! Optional durable conversation changes and native source-change notifications.
use instrukt_core::ids::{NativeTurnRef, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};

/// Runtime-owned position. Consumers persist it without interpreting its contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourcePosition(pub(super) Value);
impl SourcePosition {
    /// Body-free correlation for the opaque runtime-owned position.
    pub fn reference(&self) -> String {
        super::_diagnostics::reference(self.0.to_string().as_bytes())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Conversation,
    Message,
    Run,
    Subagent,
    TurnItem,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecord {
    pub kind: RecordKind,
    pub id: String,
    pub conversation_id: String,
    pub parent_id: Option<String>,
    pub run_id: Option<String>,
    pub role: Option<Role>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub created_at: Option<jiff::Timestamp>,
    pub updated_at: Option<jiff::Timestamp>,
    /// True only when this record supplies native completion evidence.
    pub completed: bool,
    pub native: Value,
}
/// Ordered conversational evidence translated by the selected runtime adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedTurn {
    pub conversation: SessionId,
    pub native_turn: NativeTurnRef,
    pub order: u64,
    /// Whether the native run ended successfully.
    pub succeeded: bool,
    pub contributions: Vec<TurnContribution>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthorKind {
    Human,
    Agent,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorEvidence {
    pub kind: AuthorKind,
    pub identity: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnContribution {
    pub order: u64,
    pub author: AuthorEvidence,
    pub text: String,
    pub occurred_at: Option<jiff::Timestamp>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnEvidenceFailure {
    pub conversation: SessionId,
    pub cause: String,
}
pub struct ConversationChanges {
    pub records: Vec<ConversationRecord>,
    pub position: SourcePosition,
    pub has_more: bool,
    pub completed_turns: Vec<CompletedTurn>,
    /// Presence in the source for a scope of exactly one conversation.
    pub scoped_presence: Option<super::Presence>,
    pub conversations: Vec<SessionId>,
    pub turn_failures: Vec<TurnEvidenceFailure>,
    /// Previously observed native conversations absent from this source pass.
    pub missing: Vec<SessionId>,
}

/// The native conversations one read covers.
#[derive(Debug, Clone, Copy)]
pub enum Scope<'a> {
    /// Every conversation the source holds.
    All,
    /// Only these conversations: no other conversation is read, waited for
    /// or reported missing.
    Only(&'a [SessionId]),
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("conversation source I/O at {}: {source}",path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("conversation source SQLite: {0}")]
    Sql(#[from] instrukt_core::state_db::rusqlite::Error),
    #[error("conversation source JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("conversation source notifications: {0}")]
    Watch(#[from] notify::Error),
    #[error("conversation source incompatible: {0}")]
    Shape(&'static str),
}
pub trait ConversationNotifications: Send {
    /// Waits for a change, a native notification or a missing source file that is now watched, or returns false only for the cancellation wait bound.
    fn wait(&mut self, timeout: Duration) -> Result<bool, SourceError>;
}
pub trait ConversationSource: Send {
    fn identity(&self) -> &str;
    /// Returns the next bounded change batch within `scope`; the position
    /// commits with its records.
    fn changed_since(
        &self,
        position: Option<&SourcePosition>,
        scope: Scope<'_>,
    ) -> Result<ConversationChanges, SourceError>;
    fn subscribe(&self) -> Result<Box<dyn ConversationNotifications>, SourceError>;
    /// The newest assistant message the runtime has recorded in `thread`'s
    /// running turn, taken whole, whatever its phase: what a person is shown
    /// while that turn runs. `None` when no turn of the thread runs, or the
    /// running one has written nothing yet.
    fn latest_update(&self, thread: &SessionId) -> Result<Option<String>, SourceError>;
}

impl RecordKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Message => "message",
            Self::Run => "run",
            Self::Subagent => "subagent",
            Self::TurnItem => "turn_item",
        }
    }
}
