//! A named session's conversation, as work reads it for a handed-over
//! approval: its messages in order, each with its author's role and text.

use instrukt_core::config::Env;
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;

use crate::runtime::Presence;
use crate::runtime::conversations::{ConversationRecord, RecordKind, Role, Scope};

/// One message of a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

/// Which holder answered a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationHolder {
    /// The runtime's own conversation store.
    Runtime,
    /// The history store's retained messages of a session the runtime no
    /// longer holds.
    History,
}

/// A named session's conversation, or why it cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conversation {
    Read {
        holder: ConversationHolder,
        messages: Vec<Message>,
    },
    CannotRead(String),
}

/// Which holder answers for a session the runtime's source reports with
/// `presence`: the runtime while it holds the session, the history store
/// once it reports it absent. A failed read never picks the holder.
fn holder(presence: Option<Presence>) -> ConversationHolder {
    match presence {
        Some(Presence::Absent) => ConversationHolder::History,
        Some(Presence::Present | Presence::Archived) | None => ConversationHolder::Runtime,
    }
}

/// `records`' messages, ordered as they were written.
fn messages(mut records: Vec<ConversationRecord>) -> Vec<Message> {
    records.retain(|record| matches!(record.kind, RecordKind::Message));
    records.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    records
        .into_iter()
        .filter_map(|record| {
            Some(Message {
                role: record.role?,
                text: record.text?,
            })
        })
        .collect()
}

/// `session`'s conversation: read from the selected runtime's conversation
/// source for that one session, or, for a session that source no longer
/// holds, from the messages the history store retained of it. Both are read
/// without a lock and change nothing.
pub fn conversation(home: &Home, env: &Env, session: &SessionRef) -> Conversation {
    let source = match crate::history::source(home, env) {
        Ok(source) => source,
        Err(error) => return Conversation::CannotRead(error.to_string()),
    };
    let only = [session.id.clone()];
    let mut records = Vec::new();
    let mut position = None;
    let presence = loop {
        let batch = match source.changed_since(position.as_ref(), Scope::Only(&only)) {
            Ok(batch) => batch,
            Err(error) => return Conversation::CannotRead(error.to_string()),
        };
        records.extend(
            batch
                .records
                .into_iter()
                .filter(|record| record.conversation_id == session.id.as_str()),
        );
        position = Some(batch.position);
        if !batch.has_more {
            break batch.scoped_presence;
        }
    };
    match holder(presence) {
        ConversationHolder::Runtime => Conversation::Read {
            holder: ConversationHolder::Runtime,
            messages: messages(records),
        },
        ConversationHolder::History => {
            match crate::history::retained_messages(home, source.identity(), session.id.as_str()) {
                Ok(Some(retained)) => Conversation::Read {
                    holder: ConversationHolder::History,
                    messages: retained
                        .into_iter()
                        .map(|message| Message {
                            role: message.role,
                            text: message.text,
                        })
                        .collect(),
                },
                Ok(None) => Conversation::CannotRead(format!(
                    "neither the runtime nor the history store holds session {session}"
                )),
                Err(error) => Conversation::CannotRead(error.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_the_runtime_holds_is_read_from_the_runtime() {
        assert_eq!(holder(Some(Presence::Present)), ConversationHolder::Runtime);
    }

    #[test]
    fn an_archived_session_is_still_read_from_the_runtime() {
        assert_eq!(
            holder(Some(Presence::Archived)),
            ConversationHolder::Runtime
        );
    }

    #[test]
    fn a_session_the_runtime_reports_absent_is_read_from_history() {
        assert_eq!(holder(Some(Presence::Absent)), ConversationHolder::History);
    }
}
