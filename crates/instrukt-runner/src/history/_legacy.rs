use super::{
    _store::{self, Conversation},
    Extraction, HistoryError,
};
use instrukt_core::{
    home::Home,
    state_db::rusqlite::{Connection, OpenFlags, params, types::ValueRef},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

pub(super) fn import(home: &Home, path: Option<&Path>) -> Result<Extraction, HistoryError> {
    let default = home.user_home().join(".teleclaude/mirrors.db");
    let requested = path.unwrap_or(&default);
    let absolute = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| HistoryError::Io {
                path: requested.to_path_buf(),
                source,
            })?
            .join(requested)
    };
    // Identity uses the selected absolute source path, so checking a completed
    // import never needs the source file or its directory to remain available.
    let path = absolute.as_path();
    let source = format!(
        "teleclaude:{}",
        hex::encode(Sha256::digest(path.to_string_lossy().as_bytes()))
    );
    let _lock =
        instrukt_core::fsx::LockGuard::acquire(&home.history_lock_file(), Duration::from_secs(30))?;
    if _store::position(home, &source)?.1 {
        return Ok(Extraction::default());
    }
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(10))?;
    let tx = db.transaction()?;
    let version: u64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version != 11 {
        return Err(HistoryError::Source(format!(
            "TeleClaude mirror revision {version} is unsupported; importer supports revision 11 and never migrates the source"
        )));
    }
    let mirrors = rows(&tx, "SELECT * FROM mirrors ORDER BY id", None)?;
    let mut conversations = Vec::new();
    for mirror in mirrors {
        let id = mirror["id"]
            .as_i64()
            .ok_or_else(|| HistoryError::Source("mirror lacks integer id".into()))?;
        let session = if let Some(session) = mirror["session"].as_i64() {
            rows(
                &tx,
                "SELECT * FROM mirror_sessions WHERE id=?1",
                Some(session),
            )?
            .into_iter()
            .next()
            .ok_or_else(|| HistoryError::Source("mirror session attribution is missing".into()))?
        } else {
            Value::Null
        };
        let mut sources = rows(
            &tx,
            "SELECT * FROM mirror_sources WHERE mirror=?1 ORDER BY position,id",
            Some(id),
        )?;
        let mut aliases = Vec::new();
        let mut body = mirror["conversation_text"]
            .as_str()
            .ok_or_else(|| HistoryError::Source("mirror lacks conversation text".into()))?
            .to_owned();
        for record in std::iter::once(&mirror)
            .chain(std::iter::once(&session))
            .chain(sources.iter())
        {
            for key in [
                "session_id",
                "source_identity",
                "native_session_id",
                "transcript_path",
            ] {
                if let Some(s) = record[key].as_str().filter(|s| !s.is_empty()) {
                    aliases.push(s.to_owned());
                }
            }
        }
        let needs_source_text = body.is_empty();
        for source in &mut sources {
            let id = source["id"]
                .as_i64()
                .ok_or_else(|| HistoryError::Source("mirror source lacks id".into()))?;
            let messages = rows(
                &tx,
                "SELECT * FROM mirror_messages WHERE source=?1 ORDER BY ordinal",
                Some(id),
            )?;
            if needs_source_text && let Some(s) = source["archival_text"].as_str() {
                body.push('\n');
                body.push_str(s);
            }
            for message in &messages {
                if needs_source_text && let Some(s) = message["text"].as_str() {
                    body.push('\n');
                    body.push_str(s);
                }
            }
            source["messages"] = json!(messages);
        }
        aliases.sort();
        aliases.dedup();
        conversations.push(Conversation {
            native_id:session["session_id"].as_str().map(str::to_owned).unwrap_or_else(||format!("mirror:{id}")),
            computer:mirror["computer"].as_str().ok_or_else(||HistoryError::Source("mirror lacks computer provenance".into()))?.into(),
            title:mirror["title"].as_str().ok_or_else(||HistoryError::Source("mirror lacks title".into()))?.into(),
            parent:session["initiator_session_id"].as_str().map(str::to_owned),aliases,
            document:json!({"mirror":mirror,"session":session,"sources":sources,"source_revision":version}),body,
            limitations:vec!["Legacy archive: only material retained by the TeleClaude mirror is available.".into()],
        });
    }
    tx.commit()?;
    _store::load(
        home,
        &source,
        conversations.len() as u64,
        true,
        &conversations,
    )
}

fn rows(db: &Connection, sql: &str, id: Option<i64>) -> Result<Vec<Value>, HistoryError> {
    let mut stmt = db.prepare(sql)?;
    let names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let mut rows = if let Some(id) = id {
        stmt.query(params![id])?
    } else {
        stmt.query([])?
    };
    let mut values = Vec::new();
    while let Some(row) = rows.next()? {
        let mut value = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let item = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => json!(n),
                ValueRef::Real(n) => json!(n),
                ValueRef::Text(s) => json!(
                    std::str::from_utf8(s)
                        .map_err(|_| HistoryError::Source("mirror has invalid UTF-8".into()))?
                ),
                ValueRef::Blob(_) => {
                    return Err(HistoryError::Source(
                        "mirror has unsupported binary metadata".into(),
                    ));
                }
            };
            value.insert(name.clone(), item);
        }
        values.push(Value::Object(value));
    }
    Ok(values)
}
