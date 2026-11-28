//! Retained conversations and their texts, each stored once, and the
//! full-text index over them (`project/instrukt/spec/feature/runner/history`,
//! History store): a conversation's row holds its document without the
//! texts its rows in `history_messages` hold, and `history_fts` reads its
//! text from those rows through the `history_search` view.
#[cfg(test)]
use super::RetainedMessage;
use super::{_db, HistoryError, Match};
#[cfg(test)]
use crate::runtime::cold_text::Role;
use instrukt_core::{
    home::Home,
    state_db::rusqlite::{self, Connection, OptionalExtension, Transaction, params},
};
use serde_json::Value;

/// One retained conversation as its writer gives it.
pub(super) struct Conversation {
    pub native_id: String,
    pub computer: String,
    pub title: String,
    pub parent: Option<String>,
    pub aliases: Vec<String>,
    pub limitations: Vec<String>,
    /// Its document without the texts `texts` hold.
    pub document: Value,
    pub texts: Vec<Text>,
}

/// One retained text of a conversation.
pub(super) struct Text {
    /// Unique within its conversation; orders texts of the same creation time.
    pub native_id: String,
    pub role: Option<String>,
    pub text: String,
    pub created_at: Option<StoredTime>,
    pub updated_at: Option<StoredTime>,
    pub place: Place,
}

/// Where a retained text belongs in its conversation's document, a place
/// given as a JSON pointer.
pub(super) enum Place {
    /// A text of no document: searched by, never shown.
    Nowhere,
    /// The string member at this place.
    Member(String),
    /// `record`, the source's record of a runtime message without its text,
    /// with the text as `text`, appended to the list at this place.
    Appended { list: String, record: Value },
}

impl Place {
    /// The bytes of the record the place carries.
    pub(super) fn record_bytes(&self) -> usize {
        match self {
            Self::Appended { record, .. } => record.to_string().len(),
            Self::Nowhere | Self::Member(_) => 0,
        }
    }
}

/// A retained text's time in the one form it is stored in: fixed width, so
/// that ordering the stored text orders the times.
pub(super) struct StoredTime(String);

impl From<jiff::Timestamp> for StoredTime {
    fn from(time: jiff::Timestamp) -> Self {
        Self(time.strftime("%Y-%m-%dT%H:%M:%S.%9fZ").to_string())
    }
}

pub(super) fn search(home: &Home, query: &str, limit: usize) -> Result<Vec<Match>, HistoryError> {
    let Some(db) = _db::read(home)? else {
        return Ok(Vec::new());
    };
    let mut stmt = db.prepare("SELECT c.computer,c.source,c.native_id,c.title,c.parent,c.aliases,snippet(history_fts,1,'','', '…',24),c.limitations FROM history_fts JOIN history_conversations c ON c.id=history_fts.rowid WHERE history_fts MATCH ?1 ORDER BY rank,c.source,c.native_id LIMIT ?2")?;
    let rows = stmt.query_map(params![query, limit as u64], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
        ))
    })?;
    rows.map(|row| {
        let (computer, source, conversation, title, parent, aliases, excerpt, limitations) = row?;
        Ok(Match {
            store_computer: String::new(),
            computer,
            source,
            conversation,
            title,
            parent,
            aliases: serde_json::from_str(&aliases)?,
            excerpt,
            limitations: serde_json::from_str(&limitations)?,
        })
    })
    .collect()
}

/// The retained document of `conversation`, by native identity or archive
/// alias, with each text that has a place in it back in that place.
pub(super) fn show(home: &Home, source: &str, conversation: &str) -> Result<Value, HistoryError> {
    let Some(db) = _db::read(home)? else {
        return Err(HistoryError::Source("conversation not retained".into()));
    };
    let mut statement=db.prepare("SELECT id,native_id,document FROM history_conversations WHERE source=?1 AND (native_id=?2 OR EXISTS(SELECT 1 FROM json_each(aliases) WHERE value=?2)) ORDER BY native_id=?2 DESC LIMIT 2")?;
    let found = statement
        .query_map(params![source, conversation], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let Some((key, id, document)) = found.first() else {
        return Err(HistoryError::Source("conversation not retained".into()));
    };
    if found.len() > 1 && id != conversation {
        return Err(HistoryError::Source(
            "archive alias is ambiguous; use a retained conversation id".into(),
        ));
    }
    document_of(&db, *key, serde_json::from_str(document)?)
}

/// `document`, the stored document of conversation `key`, with each of its
/// texts that has a place put back there in order: a text with a record is
/// that record with the text as `text`, appended to the list at its place;
/// one without a record is the string at its place.
pub(super) fn document_of(
    db: &Connection,
    key: i64,
    mut document: Value,
) -> Result<Value, HistoryError> {
    let mut statement = db.prepare("SELECT place,record,text FROM history_messages WHERE conversation=?1 AND place IS NOT NULL ORDER BY created_at,native_id")?;
    let texts = statement.query_map([key], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let misplaced = || HistoryError::Source("a retained text has no place in its document".into());
    for text in texts {
        let (place, record, text) = text?;
        match record {
            Some(record) => {
                let mut record: Value = serde_json::from_str(&record)?;
                record
                    .as_object_mut()
                    .ok_or_else(misplaced)?
                    .insert("text".into(), Value::String(text));
                document
                    .pointer_mut(&place)
                    .and_then(Value::as_array_mut)
                    .ok_or_else(misplaced)?
                    .push(record);
            }
            None => {
                let (parent, member) = place.rsplit_once('/').ok_or_else(misplaced)?;
                document
                    .pointer_mut(parent)
                    .and_then(Value::as_object_mut)
                    .ok_or_else(misplaced)?
                    .insert(member.to_owned(), Value::String(text));
            }
        }
    }
    Ok(document)
}

/// The messages that carry a role of conversation `thread` from `source`,
/// in order; `None` when the conversation is not retained.
#[cfg(test)]
pub(super) fn messages(
    db: &Connection,
    source: &str,
    thread: &str,
) -> Result<Option<Vec<RetainedMessage>>, HistoryError> {
    let Some(key) = key(db, source, thread)? else {
        return Ok(None);
    };
    let mut statement = db.prepare("SELECT role,text FROM history_messages WHERE conversation=?1 AND role IS NOT NULL ORDER BY created_at,native_id")?;
    let rows = statement.query_map([key], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    rows.map(|row| {
        let (role, text) = row?;
        Ok(RetainedMessage {
            role: serde_json::from_value::<Role>(Value::String(role))?,
            text,
        })
    })
    .collect::<Result<Vec<_>, HistoryError>>()
    .map(Some)
}

fn key(db: &Connection, source: &str, native_id: &str) -> rusqlite::Result<Option<i64>> {
    db.query_row(
        "SELECT id FROM history_conversations WHERE source=?1 AND native_id=?2",
        params![source, native_id],
        |r| r.get(0),
    )
    .optional()
}

/// Writes `c`, a conversation of `source`, and its texts, and replaces its
/// index entry. An existing row is updated in place and keeps its key; a
/// new row takes `key` when one is given. A retained text `c` does not
/// carry stays.
pub(super) fn write_conversation(
    tx: &Transaction<'_>,
    source: &str,
    key_for_new: Option<i64>,
    c: &Conversation,
) -> Result<(), HistoryError> {
    let aliases = serde_json::to_string(&c.aliases)?;
    let limitations = serde_json::to_string(&c.limitations)?;
    let key = match key(tx, source, &c.native_id)? {
        Some(key) => {
            unindex(tx, key)?;
            tx.execute("UPDATE history_conversations SET computer=?2,title=?3,parent=?4,aliases=?5,document=?6,limitations=?7 WHERE id=?1",params![key,c.computer,c.title,c.parent,aliases,c.document.to_string(),limitations])?;
            key
        }
        None => {
            tx.execute("INSERT INTO history_conversations(id,source,native_id,computer,title,parent,aliases,document,limitations) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![key_for_new,source,c.native_id,c.computer,c.title,c.parent,aliases,c.document.to_string(),limitations])?;
            tx.last_insert_rowid()
        }
    };
    write_texts(tx, key, &c.texts)?;
    index(tx, key)?;
    Ok(())
}

/// Writes `texts` of `conversation`, a retained conversation of `source`,
/// and replaces its index entry; its row is unchanged. A conversation the
/// store has not retained fails the write.
pub(super) fn write_texts_of(
    tx: &Transaction<'_>,
    source: &str,
    conversation: &str,
    texts: &[Text],
) -> Result<(), HistoryError> {
    let Some(key) = key(tx, source, conversation)? else {
        return Err(HistoryError::Source(
            "source conversation metadata is not available yet".into(),
        ));
    };
    unindex(tx, key)?;
    write_texts(tx, key, texts)?;
    index(tx, key)?;
    Ok(())
}

/// Inserts or updates `texts` of conversation `key`; its index entry is the
/// caller's to maintain.
pub(super) fn write_texts(tx: &Transaction<'_>, key: i64, texts: &[Text]) -> rusqlite::Result<()> {
    let mut statement = tx.prepare_cached("INSERT INTO history_messages(conversation,native_id,role,text,created_at,updated_at,place,record) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(conversation,native_id) DO UPDATE SET role=excluded.role,text=excluded.text,created_at=excluded.created_at,updated_at=excluded.updated_at,place=excluded.place,record=excluded.record")?;
    for t in texts {
        let (place, record) = match &t.place {
            Place::Nowhere => (None, None),
            Place::Member(member) => (Some(member.as_str()), None),
            Place::Appended { list, record } => (Some(list.as_str()), Some(record.to_string())),
        };
        statement.execute(params![
            key,
            t.native_id,
            t.role,
            t.text,
            t.created_at.as_ref().map(|time| time.0.as_str()),
            t.updated_at.as_ref().map(|time| time.0.as_str()),
            place,
            record
        ])?;
    }
    Ok(())
}

/// Removes conversation `key`'s index entry, with the title and text it was
/// indexed with: those its rows hold before any of them changes.
fn unindex(tx: &Transaction<'_>, key: i64) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO history_fts(history_fts,rowid,title,body) SELECT 'delete',id,title,body FROM history_search WHERE id=?1",
        [key],
    )?;
    Ok(())
}

/// Indexes conversation `key` with the title and text its rows now hold.
fn index(tx: &Transaction<'_>, key: i64) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO history_fts(rowid,title,body) SELECT id,title,body FROM history_search WHERE id=?1",
        [key],
    )?;
    Ok(())
}
