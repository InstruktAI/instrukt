//! Short read-only projection batches from T3's durable SQLite state.
use super::conversations::{
    ConversationChanges, ConversationNotifications, ConversationRecord, ConversationSource,
    RecordKind, Role, SourceError, SourcePosition,
};
use crate::runtime::_t3_home::T3Home;
use instrukt_core::{
    config::Env,
    home::Home,
    state_db::rusqlite::{Connection, OpenFlags, params},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

const BATCH_SIZE: usize = 128;
const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;
const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Projection {
    Threads,
    Messages,
    Runs,
    Subagents,
    TurnItems,
}
const PROJECTIONS: [Projection; 5] = [
    Projection::Threads,
    Projection::Messages,
    Projection::Runs,
    Projection::Subagents,
    Projection::TurnItems,
];
impl Projection {
    fn name(self) -> &'static str {
        match self {
            Self::Threads => "threads",
            Self::Messages => "messages",
            Self::Runs => "runs",
            Self::Subagents => "subagents",
            Self::TurnItems => "turn_items",
        }
    }
    fn table(self) -> String {
        format!("orchestration_v2_projection_{}", self.name())
    }
    fn id(self) -> &'static str {
        match self {
            Self::Threads => "thread_id",
            Self::Messages => "message_id",
            Self::Runs => "run_id",
            Self::Subagents => "subagent_id",
            Self::TurnItems => "turn_item_id",
        }
    }
    fn clock(self) -> &'static str {
        if matches!(self, Self::Runs) {
            "coalesce(completed_at,requested_at)"
        } else {
            "updated_at"
        }
    }
    fn columns(self) -> Vec<(&'static str, &'static str)> {
        let mut columns = vec![
            (self.id(), "TEXT"),
            ("thread_id", "TEXT"),
            ("payload_json", "TEXT"),
        ];
        if matches!(self, Self::Runs) {
            columns.extend([
                ("requested_at", "TEXT"),
                ("completed_at", "TEXT"),
                ("status", "TEXT"),
            ]);
        } else {
            columns.push(("updated_at", "TEXT"));
        }
        match self {
            Self::Threads => columns.extend([
                ("title", "TEXT"),
                ("archived_at", "TEXT"),
                ("deleted_at", "TEXT"),
            ]),
            Self::Messages => columns.extend([
                ("role", "TEXT"),
                ("streaming", "INTEGER"),
                ("created_at", "TEXT"),
            ]),
            Self::Subagents => columns.push(("child_thread_id", "TEXT")),
            Self::TurnItems => columns.extend([("type", "TEXT"), ("status", "TEXT")]),
            Self::Runs => {}
        }
        columns
    }
}

fn source_error(detail: &'static str) -> SourceError {
    SourceError::Shape(detail)
}
fn open(path: &Path) -> Result<Connection, SourceError> {
    let mut uri = url::Url::from_file_path(path)
        .map_err(|_| source_error("database path must be absolute"))?;
    uri.set_query(Some("mode=ro"));
    let db = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    Ok(db)
}
fn columns(db: &Connection, table: &str) -> Result<BTreeMap<String, String>, SourceError> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    Ok(stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?.to_ascii_uppercase(),
            ))
        })?
        .collect::<Result<_, _>>()?)
}
fn validate(path: &Path, location: &T3Home) -> Result<(), SourceError> {
    let db = open(path)?;
    for projection in PROJECTIONS {
        let found = columns(&db, &projection.table())?;
        for (name, kind) in projection.columns() {
            if found.get(name).map(String::as_str) != Some(kind) {
                return Err(source_error(
                    "required projection column or type is incompatible",
                ));
            }
        }
    }
    let migration = match columns(&db, "effect_sql_migrations") {
        Ok(found) if found.get("migration_id").map(String::as_str) == Some("INTEGER") => match db
            .query_row(
                "SELECT max(migration_id) FROM effect_sql_migrations",
                [],
                |r| r.get::<_, Option<i64>>(0),
            ) {
            Ok(level) => level,
            Err(error) => {
                tracing::debug!(event="history_migration_diagnostic_unavailable",error=%error);
                None
            }
        },
        Ok(_) => None,
        Err(error) => {
            tracing::debug!(event="history_migration_diagnostic_unavailable",error=%error);
            None
        }
    };
    let release = match std::fs::read(location.descriptor()) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v["serverVersion"].as_str().map(str::to_owned)),
        Err(error) => {
            tracing::debug!(event="history_release_diagnostic_unavailable",error=%error);
            None
        }
    };
    tracing::debug!(event="history_source_shape_validated",migration_level=?migration,t3_release=?release);
    Ok(())
}
fn batch(
    path: &Path,
    projection: Projection,
    cursor: &Cursor,
    selected: Option<&str>,
) -> Result<Vec<Row>, SourceError> {
    let db = open(path)?;
    let streaming = if matches!(projection, Projection::Messages) {
        "streaming"
    } else {
        "0"
    };
    let extra = match projection {
        Projection::Messages => "role",
        Projection::TurnItems => "type",
        Projection::Runs => "status",
        _ => "''",
    };
    let predicate = if selected.is_some() {
        format!("{}=?1", projection.id())
    } else {
        format!("({}, {})>(?1,?2)", projection.clock(), projection.id())
    };
    let sql = format!(
        "SELECT {},thread_id,{},payload_json,{streaming},{extra} FROM {} WHERE {predicate} ORDER BY {},{} LIMIT {BATCH_SIZE}",
        projection.id(),
        projection.clock(),
        projection.table(),
        projection.clock(),
        projection.id()
    );
    let mut stmt = db.prepare(&sql)?;
    let mut rows = if let Some(id) = selected {
        stmt.query([id])?
    } else {
        stmt.query(params![cursor.updated_at, cursor.native_id])?
    };
    let mut values = Vec::new();
    let mut bytes = 0;
    while let Some(row) = rows.next()? {
        let native_id: String = row.get(0)?;
        let thread_id: String = row.get(1)?;
        let updated_at: String = row.get(2)?;
        let raw: String = row.get(3)?;
        if !values.is_empty() && bytes + raw.len() > MAX_BATCH_BYTES {
            break;
        }
        bytes += raw.len();
        let streaming: i64 = row.get(4)?;
        let extra: String = row.get(5)?;
        if native_id.is_empty()
            || thread_id.is_empty()
            || updated_at.parse::<jiff::Timestamp>().is_err()
            || !matches!(streaming, 0 | 1)
            || raw.len() > MAX_RECORD_BYTES
        {
            return Err(source_error("invalid or oversized projection row"));
        }
        let payload: Value = serde_json::from_str(&raw)?;
        if payload["id"] != native_id
            || (!matches!(projection, Projection::Threads) && payload["threadId"] != thread_id)
        {
            return Err(source_error("projection row identity mismatch"));
        }
        match projection {
            Projection::Threads => {
                if payload["title"].as_str().is_none()
                    || !payload["lineage"].is_object()
                    || !payload["lineage"]
                        .get("parentThreadId")
                        .is_some_and(|v| v.is_null() || v.is_string())
                {
                    return Err(source_error("invalid thread shape"));
                }
            }
            Projection::Messages => {
                if payload["text"].as_str().is_none()
                    || payload["role"] != extra
                    || payload["streaming"].as_bool() != Some(streaming == 1)
                {
                    return Err(source_error("invalid message shape"));
                }
            }
            Projection::TurnItems => {
                if payload["type"] != extra {
                    return Err(source_error("invalid turn-item shape"));
                }
            }
            Projection::Runs => {
                if payload["status"] != extra {
                    return Err(source_error("invalid run shape"));
                }
            }
            Projection::Subagents => {}
        }
        let still_streaming = streaming == 1 || payload["streaming"].as_bool() == Some(true);
        values.push(Row {
            native_id,
            thread_id,
            updated_at,
            payload,
            streaming: still_streaming,
        });
    }
    Ok(values)
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Cursor {
    updated_at: String,
    native_id: String,
}
struct Row {
    native_id: String,
    thread_id: String,
    updated_at: String,
    payload: Value,
    streaming: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Pending {
    projection: Projection,
    id: String,
}

#[derive(Serialize, Deserialize)]
struct Position {
    version: u8,
    cursors: BTreeMap<String, Cursor>,
    pending: BTreeSet<Pending>,
    pending_after: Option<Pending>,
}
impl Default for Position {
    fn default() -> Self {
        Self {
            version: 1,
            cursors: BTreeMap::new(),
            pending: BTreeSet::new(),
            pending_after: None,
        }
    }
}

pub(super) struct T3Conversations {
    location: T3Home,
    path: PathBuf,
    identity: String,
}
impl T3Conversations {
    pub(super) fn new(home: &Home, env: &Env) -> Self {
        let location = T3Home::discover(home, env);
        let path = location.database();
        // Identity belongs to the T3 state location, independently of how its data is read.
        let descriptor = location.descriptor();
        let identity = format!(
            "t3:{}",
            hex::encode(Sha256::digest(descriptor.to_string_lossy().as_bytes()))
        );
        Self {
            location,
            path,
            identity,
        }
    }
}
fn normalized(projection: Projection, row: Row) -> Result<ConversationRecord, SourceError> {
    let value = &row.payload;
    let timestamp = |name: &str| -> Result<Option<jiff::Timestamp>, SourceError> {
        value
            .get(name)
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| source_error("timestamp type incompatible"))
                    .and_then(|s| {
                        s.parse()
                            .map_err(|_| source_error("invalid source timestamp"))
                    })
            })
            .transpose()
    };
    let kind = match projection {
        Projection::Threads => RecordKind::Conversation,
        Projection::Messages => RecordKind::Message,
        Projection::Runs => RecordKind::Run,
        Projection::Subagents => RecordKind::Subagent,
        Projection::TurnItems => RecordKind::TurnItem,
    };
    let role = if matches!(projection, Projection::Messages | Projection::TurnItems) {
        Some(serde_json::from_value::<Role>(value["role"].clone())?)
    } else {
        None
    };
    let mut texts = Vec::new();
    for key in [
        "text", "input", "output", "markdown", "diffStr", "result", "progress",
    ] {
        if let Some(v) = value.get(key).filter(|v| !v.is_null()) {
            texts.push(
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string()),
            );
        }
    }
    let completed = match projection {
        Projection::Messages => true,
        Projection::Threads => false,
        _ => {
            timestamp("completedAt")?.is_some()
                || matches!(
                    value["status"].as_str(),
                    Some("completed" | "failed" | "interrupted" | "cancelled" | "rolled_back")
                )
        }
    };
    Ok(ConversationRecord {
        kind,
        id: row.native_id,
        conversation_id: row.thread_id,
        parent_id: value["lineage"]["parentThreadId"]
            .as_str()
            .map(str::to_owned),
        run_id: value["runId"].as_str().map(str::to_owned),
        role,
        title: value["title"].as_str().map(str::to_owned),
        text: (!texts.is_empty()).then(|| texts.join("\n\n")),
        created_at: timestamp("createdAt")?.or(timestamp("requestedAt")?),
        updated_at: Some(
            row.updated_at
                .parse()
                .map_err(|_| source_error("invalid row timestamp"))?,
        ),
        completed,
        native: row.payload,
    })
}
impl ConversationSource for T3Conversations {
    fn identity(&self) -> &str {
        &self.identity
    }
    fn changed_since(
        &self,
        position: Option<&SourcePosition>,
    ) -> Result<ConversationChanges, SourceError> {
        validate(&self.path, &self.location)?;
        let mut state = match position {
            Some(position) => serde_json::from_value::<Position>(position.0.clone())?,
            None => Position::default(),
        };
        if state.version != 1 {
            return Err(source_error("unknown source-position version"));
        }
        for projection in PROJECTIONS {
            let cursor = state
                .cursors
                .get(projection.name())
                .cloned()
                .unwrap_or_default();
            let rows = batch(&self.path, projection, &cursor, None)?;
            if let Some(last) = rows.last() {
                let next = Cursor {
                    updated_at: last.updated_at.clone(),
                    native_id: last.native_id.clone(),
                };
                let mut records = Vec::new();
                for row in rows {
                    if row.streaming {
                        state.pending.insert(Pending {
                            projection,
                            id: row.native_id,
                        });
                        continue;
                    }
                    if matches!(projection, Projection::Messages | Projection::TurnItems) {
                        state.pending.remove(&Pending {
                            projection,
                            id: row.native_id.clone(),
                        });
                    }
                    records.push(normalized(projection, row)?);
                }
                state.cursors.insert(projection.name().into(), next);
                state.pending_after = None;
                return Ok(ConversationChanges {
                    records,
                    position: SourcePosition(serde_json::to_value(state)?),
                    has_more: true,
                });
            }
        }
        let ids: Vec<Pending> = state
            .pending
            .iter()
            .filter(|id| state.pending_after.as_ref().is_none_or(|after| *id > after))
            .take(BATCH_SIZE)
            .cloned()
            .collect();
        let mut records = Vec::new();
        for pending in &ids {
            let rows = batch(
                &self.path,
                pending.projection,
                &Cursor::default(),
                Some(&pending.id),
            )?;
            if rows.is_empty() {
                state.pending.remove(pending);
            }
            for row in rows {
                if !row.streaming {
                    state.pending.remove(pending);
                    records.push(normalized(pending.projection, row)?);
                }
            }
        }
        let more = ids
            .last()
            .is_some_and(|last| state.pending.iter().any(|id| id > last));
        state.pending_after = if more { ids.last().cloned() } else { None };
        Ok(ConversationChanges {
            records,
            position: SourcePosition(serde_json::to_value(state)?),
            has_more: more,
        })
    }
    fn subscribe(&self) -> Result<Box<dyn ConversationNotifications>, SourceError> {
        super::_t3_notifications::subscribe(&self.path)
    }
}
