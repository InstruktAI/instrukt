//! Retained conversations of the earlier shape brought to the current one,
//! by the rules of `project/instrukt/spec/feature/runner/history` (History
//! store): the store's own migration and the move out of the state database
//! both convert through here, and the TeleClaude import retains each mirror
//! by the mirror rule. The rule is selected by the kind of a conversation's
//! source, never guessed from what it holds.
use super::{
    _store::{Text, stored_time},
    HistoryError,
};
use instrukt_core::state_db::rusqlite::{self, Connection, params};
use serde_json::{Value, json};

/// A conversation of the earlier shape could not be converted.
#[derive(Debug, thiserror::Error)]
pub(super) enum ConversionError {
    #[error("retained history SQLite operation failed: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("a retained document or record is not the JSON its writer produced: {0}")]
    Json(#[from] serde_json::Error),
    #[error("a retained record holds a time that does not parse: {0}")]
    Time(#[from] jiff::Error),
    #[error("{0}")]
    Shape(&'static str),
}

impl From<ConversionError> for HistoryError {
    fn from(error: ConversionError) -> Self {
        match error {
            ConversionError::Sql(error) => Self::Sql(error),
            ConversionError::Json(error) => Self::Json(error),
            other => Self::Source(other.to_string()),
        }
    }
}

impl From<ConversionError> for rusqlite::Error {
    /// A migration step's failure: a stored value that did not convert,
    /// with the conversion failure as its cause.
    fn from(error: ConversionError) -> Self {
        match error {
            ConversionError::Sql(error) => error,
            other => {
                Self::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(other))
            }
        }
    }
}

/// The columns of an earlier-shape conversation row, read from
/// `history_conversations` of the history store or the state database. A
/// runtime conversation's document and searchable text are never loaded:
/// its records hold everything the current shape keeps of it.
pub(super) const EARLIER: &str = "SELECT id,source,native_id,computer,title,parent,aliases,limitations,CASE WHEN substr(source,1,3)='t3:' THEN NULL ELSE document END,CASE WHEN substr(source,1,3)='t3:' THEN NULL ELSE body END FROM history_conversations";

/// One conversation row of the earlier shape.
pub(super) struct Earlier {
    pub id: i64,
    pub source: String,
    pub native_id: String,
    pub computer: String,
    pub title: String,
    pub parent: Option<String>,
    pub aliases: String,
    pub limitations: String,
    /// The earlier document and searchable text, absent for a runtime
    /// conversation.
    held: Option<(String, String)>,
}

impl Earlier {
    /// Maps a row of [`EARLIER`].
    pub(super) fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let document: Option<String> = r.get(8)?;
        let body: Option<String> = r.get(9)?;
        Ok(Self {
            id: r.get(0)?,
            source: r.get(1)?,
            native_id: r.get(2)?,
            computer: r.get(3)?,
            title: r.get(4)?,
            parent: r.get(5)?,
            aliases: r.get(6)?,
            limitations: r.get(7)?,
            held: document.zip(body),
        })
    }

    /// The retained bytes the row carries.
    pub(super) fn bytes(&self) -> usize {
        self.held.as_ref().map_or(0, |(d, b)| d.len() + b.len())
    }
}

/// A conversation's document without its texts, and the texts.
pub(super) struct Current {
    pub document: Value,
    pub texts: Vec<Text>,
}

impl Current {
    /// The retained bytes the current shape keeps.
    pub(super) fn bytes(&self) -> usize {
        self.document.to_string().len()
            + self
                .texts
                .iter()
                .map(|t| t.text.len() + t.record.as_ref().map_or(0, |r| r.to_string().len()))
                .sum::<usize>()
    }
}

/// Converts `earlier`, reading a runtime conversation's records from
/// `records`, the database that holds the earlier rows.
pub(super) fn convert(records: &Connection, earlier: &Earlier) -> Result<Current, ConversionError> {
    if earlier.source.starts_with("t3:") {
        return runtime(records, &earlier.source, &earlier.native_id);
    }
    let Some((document, body)) = &earlier.held else {
        return Err(ConversionError::Shape(
            "an imported conversation's document was not read",
        ));
    };
    let document: Value = serde_json::from_str(document)?;
    if is_mirror(&document) {
        return Ok(mirror(document));
    }
    Ok(Current {
        document,
        texts: (!body.is_empty())
            .then(|| Text {
                native_id: "text".into(),
                role: None,
                text: body.clone(),
                created_at: None,
                updated_at: None,
                place: None,
                record: None,
            })
            .into_iter()
            .collect(),
    })
}

/// A runtime conversation: its conversation record's native record as
/// `thread`, and one text for each of its message records, placed in
/// `messages`, each record without the context its runtime captured rather
/// than a person wrote. Its run, subagent and turn-item records are not read.
fn runtime(records: &Connection, source: &str, thread: &str) -> Result<Current, ConversionError> {
    let mut statement = records.prepare_cached(
        "SELECT projection,native_id,payload_json FROM history_projection_records WHERE source=?1 AND thread_id=?2 AND projection IN ('conversation','message')",
    )?;
    let rows = statement.query_map(params![source, thread], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut conversation = None;
    let mut texts = Vec::new();
    for row in rows {
        let (projection, native_id, payload) = row?;
        let mut record: Value = serde_json::from_str(&payload)?;
        let mut native =
            record
                .get_mut("native")
                .map(Value::take)
                .ok_or(ConversionError::Shape(
                    "a retained record has no native record",
                ))?;
        if projection == "conversation" {
            conversation = Some(native);
            continue;
        }
        crate::runtime::without_captured_context(&mut native);
        // The native record's text, where it has one; a record an earlier
        // state migration repaired holds that text twice in its own.
        let text = match native.get("text") {
            Some(Value::String(_)) => {
                match native.as_object_mut().and_then(|n| n.shift_remove("text")) {
                    Some(Value::String(text)) => text,
                    _ => return Err(ConversionError::Shape("a retained message's text moved")),
                }
            }
            _ => record["text"].as_str().unwrap_or_default().to_owned(),
        };
        let time = |name: &str| -> Result<Option<String>, ConversionError> {
            record[name]
                .as_str()
                .map(|time| Ok(stored_time(time.parse()?)))
                .transpose()
        };
        texts.push(Text {
            native_id,
            role: record["role"].as_str().map(str::to_owned),
            text,
            created_at: time("created_at")?,
            updated_at: time("updated_at")?,
            place: Some(MESSAGES.into()),
            record: Some(native),
        });
    }
    let thread = conversation.ok_or(ConversionError::Shape(
        "a runtime conversation has no retained conversation record",
    ))?;
    Ok(Current {
        document: json!({"messages": [], "thread": thread}),
        texts,
    })
}

/// Where a runtime conversation's messages belong in its document.
pub(super) const MESSAGES: &str = "/messages";

/// Whether `document` is a mirror document: an object holding a mirror and
/// its sources, as the TeleClaude import writes it.
fn is_mirror(document: &Value) -> bool {
    document.get("mirror").is_some_and(Value::is_object)
        && document.get("sources").is_some_and(Value::is_array)
}

/// A mirror document whole except for the texts it is searched by: its
/// transcript text where it has one, otherwise each source's archival text
/// and each of that source's mirror message texts, in the document's order,
/// a message's text under its role. Each of these that is a non-empty
/// string leaves its member, which stays in place holding nothing, so the
/// document keeps its shape; every other member, and every mirror message
/// entry at its position, stays as it is. A text's native id is its place
/// with fixed-width positions, so the texts sort in document order.
pub(super) fn mirror(mut document: Value) -> Current {
    let mut texts = Vec::new();
    let mut take = |member: Option<&mut Value>, native_id: String, place: String, role| {
        if let Some(slot) = member.filter(|s| s.as_str().is_some_and(|s| !s.is_empty()))
            && let Value::String(text) = slot.take()
        {
            texts.push(Text {
                native_id,
                role,
                text,
                created_at: None,
                updated_at: None,
                place: Some(place),
                record: None,
            });
        }
    };
    let transcript = document
        .pointer("/mirror/conversation_text")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty());
    if transcript {
        take(
            document.pointer_mut("/mirror/conversation_text"),
            "mirror/conversation_text".into(),
            "/mirror/conversation_text".into(),
            None,
        );
    } else if let Some(sources) = document.get_mut("sources").and_then(Value::as_array_mut) {
        for (i, source) in sources.iter_mut().enumerate() {
            take(
                source.get_mut("archival_text"),
                format!("sources/{i:09}/archival_text"),
                format!("/sources/{i}/archival_text"),
                None,
            );
            let Some(messages) = source.get_mut("messages").and_then(Value::as_array_mut) else {
                continue;
            };
            for (j, message) in messages.iter_mut().enumerate() {
                let role = message["role"].as_str().map(str::to_owned);
                take(
                    message.get_mut("text"),
                    format!("sources/{i:09}/messages/{j:09}/text"),
                    format!("/sources/{i}/messages/{j}/text"),
                    role,
                );
            }
        }
    }
    Current { document, texts }
}
