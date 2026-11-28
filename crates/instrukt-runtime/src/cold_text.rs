//! Non-live cold conversation text mirror contract (used only by history ingestion).
use std::path::PathBuf;
use std::time::Duration;

use instrukt_core::ids::SessionId;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Runtime-owned position. Consumers persist it without interpreting its contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourcePosition(Value);

impl SourcePosition {
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    pub fn value(&self) -> &Value {
        &self.0
    }

    /// Body-free correlation for the opaque runtime-owned position.
    pub fn reference(&self) -> String {
        crate::reference(self.0.to_string().as_bytes())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

/// One batch of the cold text read: what a text mirror of conversations receives.
pub struct TextChanges {
    pub conversations: Vec<ConversationText>,
    pub position: SourcePosition,
    pub has_more: bool,
}

/// One changed conversation of a cold text read, named by its native id whether
/// or not the read found the conversation's own record; it always carries that
/// record or at least one finished message.
pub struct ConversationText {
    conversation: SessionId,
    record: Option<ConversationOwnRecord>,
    messages: Vec<MessageText>,
}

impl ConversationText {
    pub fn new(
        conversation: SessionId,
        record: Option<ConversationOwnRecord>,
        messages: Vec<MessageText>,
    ) -> Option<Self> {
        (record.is_some() || !messages.is_empty()).then_some(Self {
            conversation,
            record,
            messages,
        })
    }

    pub fn conversation(&self) -> &SessionId {
        &self.conversation
    }

    pub fn into_parts(self) -> (SessionId, Option<ConversationOwnRecord>, Vec<MessageText>) {
        (self.conversation, self.record, self.messages)
    }
}

/// What a source holds of a conversation itself.
pub struct ConversationOwnRecord {
    pub title: String,
    pub parent: Option<SessionId>,
    pub native: Value,
}

/// One finished message: its text, and the source's record of it without that
/// text, so the text is held once.
pub struct MessageText {
    id: String,
    role: Role,
    text: String,
    created_at: Option<Timestamp>,
    updated_at: Option<Timestamp>,
    record: Value,
}

/// The parts of a [`MessageText`], taken apart by its consumer.
pub struct MessageParts {
    pub id: String,
    pub role: Role,
    pub text: String,
    pub created_at: Option<Timestamp>,
    pub updated_at: Option<Timestamp>,
    pub record: Value,
}

impl MessageText {
    pub fn from_record(
        id: String,
        role: Role,
        created_at: Option<Timestamp>,
        updated_at: Option<Timestamp>,
        mut record: Value,
    ) -> Option<Self> {
        let Some(Value::String(text)) = record.as_object_mut()?.shift_remove("text") else {
            return None;
        };
        Some(Self {
            id,
            role,
            text,
            created_at,
            updated_at,
            record,
        })
    }

    pub fn into_parts(self) -> MessageParts {
        MessageParts {
            id: self.id,
            role: self.role,
            text: self.text,
            created_at: self.created_at,
            updated_at: self.updated_at,
            record: self.record,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("cold text source I/O at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cold text source SQLite: {0}")]
    Sql(#[from] instrukt_core::state_db::rusqlite::Error),
    #[error("cold text source JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cold text source notifications: {0}")]
    Watch(#[from] notify::Error),
    #[error("cold text source incompatible: {0}")]
    Shape(&'static str),
}

/// What one wait on a cold text source's notifications ended with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceWake {
    Timeout,
    Changed,
    ReadAgain,
}

pub trait ColdTextNotifications: Send {
    fn wait(&mut self, timeout: Duration) -> Result<SourceWake, SourceError>;
}

/// Optional non-live cold conversation text mirror used solely by history ingestion.
pub trait ColdTextSource: Send {
    fn identity(&self) -> &str;
    fn text_since(&self, position: Option<&SourcePosition>) -> Result<TextChanges, SourceError>;
    fn subscribe(&self) -> Result<Box<dyn ColdTextNotifications>, SourceError>;
}
