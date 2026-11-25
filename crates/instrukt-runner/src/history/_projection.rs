//! Commits a text read's conversations with the runtime-opaque source
//! position.
use super::{
    _earlier::MESSAGES,
    _store::{self, Conversation, Text, stored_time},
    HistoryError,
};
use crate::runtime::conversations::{ConversationText, SourcePosition};
use instrukt_core::{
    home::Home,
    state_db::{
        StoreDb,
        rusqlite::{OptionalExtension, params},
    },
};
use serde_json::json;

pub(super) fn position(db: &StoreDb, source: &str) -> Result<Option<SourcePosition>, HistoryError> {
    let stored: Option<String> = db
        .conn()
        .query_row(
            "SELECT position_json FROM history_source_positions WHERE source=?1",
            [source],
            |r| r.get(0),
        )
        .optional()?;
    stored
        .map(|s| serde_json::from_str(&s).map_err(HistoryError::from))
        .transpose()
}

/// Retains each of `conversations`, a text read's batch: its row from its
/// own record, when the batch carries it, and its messages added or
/// updated, none removed; then commits them with `position`, the batch's.
/// A conversation the store has not retained whose own record the batch
/// lacks fails the batch.
pub(super) fn apply(
    home: &Home,
    db: &mut StoreDb,
    source: &str,
    conversations: Vec<ConversationText>,
    position: &SourcePosition,
) -> Result<usize, HistoryError> {
    let computer = instrukt_core::machine::machine_id(home)?.to_string();
    let loaded = conversations.len();
    db.write(|tx| -> Result<usize, HistoryError> {
        for changed in conversations {
            let texts = changed
                .messages
                .into_iter()
                .map(|m| {
                    Ok(Text {
                        native_id: m.id,
                        role: serde_json::to_value(m.role)?.as_str().map(str::to_owned),
                        text: m.text,
                        created_at: m.created_at.map(stored_time),
                        updated_at: m.updated_at.map(stored_time),
                        place: Some(MESSAGES.into()),
                        record: Some(m.native),
                    })
                })
                .collect::<Result<Vec<_>, HistoryError>>()?;
            match changed.record {
                Some(record) => _store::write_conversation(
                    tx,
                    source,
                    None,
                    &Conversation {
                        aliases: vec![changed.conversation_id.clone()],
                        native_id: changed.conversation_id,
                        computer: computer.clone(),
                        title: record.title,
                        parent: record.parent_id,
                        limitations: vec![
                            "Only content retained by the source runtime is available.".into(),
                        ],
                        document: json!({"messages": [], "thread": record.native}),
                        texts,
                    },
                )?,
                None => _store::write_texts_of(tx, source, &changed.conversation_id, &texts)?,
            }
        }
        tx.execute("INSERT INTO history_source_positions VALUES(?1,?2) ON CONFLICT(source) DO UPDATE SET position_json=excluded.position_json",params![source,serde_json::to_string(position)?])?;
        Ok(loaded)
    })
}
