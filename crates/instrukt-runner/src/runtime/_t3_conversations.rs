//! Short read-only projection batches from T3's durable SQLite state.
use super::_t3_fingerprint::{self, Fingerprint};
use super::conversations::{
    ConversationChanges, ConversationNotifications, ConversationRecord, ConversationSource,
    RecordKind, Role, SourceError, SourcePosition,
};
use crate::runtime::_t3_home::T3Home;
use instrukt_core::state_db::rusqlite::types::ValueRef;
use instrukt_core::{
    config::Env,
    home::Home,
    state_db::rusqlite::{Connection, OpenFlags, params},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

const BATCH_SIZE: usize = 128;
pub(super) const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;
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
    fn streaming_column(self) -> &'static str {
        if matches!(self, Self::Messages) {
            "streaming"
        } else {
            "0"
        }
    }
    fn extra_column(self) -> &'static str {
        match self {
            Self::Messages => "role",
            Self::TurnItems => "type",
            Self::Runs => "status",
            _ => "''",
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
    thread: &str,
    after: &str,
) -> Result<Vec<Row>, SourceError> {
    let db = open(path)?;
    let streaming = projection.streaming_column();
    let extra = projection.extra_column();
    let sql = format!(
        "SELECT {},thread_id,{},payload_json,{streaming},{extra},rowid FROM {} WHERE thread_id=?1 AND {}>?2 ORDER BY {} LIMIT {BATCH_SIZE}",
        projection.id(),
        projection.clock(),
        projection.table(),
        projection.id(),
        projection.id()
    );
    let mut stmt = db.prepare(&sql)?;
    let mut rows = stmt.query(params![thread, after])?;
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
        let rowid: i64 = row.get(6)?;
        let fingerprint = Fingerprint::row([
            ValueRef::Text(projection.name().as_bytes()),
            ValueRef::Integer(rowid),
            ValueRef::Text(native_id.as_bytes()),
            ValueRef::Text(thread_id.as_bytes()),
            ValueRef::Text(updated_at.as_bytes()),
            ValueRef::Text(raw.as_bytes()),
            ValueRef::Integer(streaming),
            ValueRef::Text(extra.as_bytes()),
        ]);
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
            fingerprint,
        });
    }
    Ok(values)
}

struct Row {
    native_id: String,
    thread_id: String,
    updated_at: String,
    payload: Value,
    streaming: bool,
    fingerprint: Fingerprint,
}
#[derive(Serialize, Deserialize)]
struct Position {
    version: u8,
    loaded: BTreeMap<String, Fingerprint>,
    #[serde(default)]
    observed: std::collections::BTreeSet<String>,
    after: Option<String>,
}
impl Default for Position {
    fn default() -> Self {
        Self {
            version: 2,
            loaded: BTreeMap::new(),
            observed: Default::default(),
            after: None,
        }
    }
}
/// Known timestamp checkpoints become an empty fingerprint inventory before source use.
fn migrate_position(position: Option<&SourcePosition>) -> Result<Position, SourceError> {
    match position {
        None => Ok(Position::default()),
        Some(p) if p.0["version"] == 1 => Ok(Position::default()),
        Some(p) if p.0["version"] == 2 => Ok(serde_json::from_value(p.0.clone())?),
        _ => Err(source_error("unknown source-position version")),
    }
}
struct Pass {
    fingerprints: BTreeMap<String, Fingerprint>,
    expected: Option<SourcePosition>,
}
pub(super) struct T3Conversations {
    location: T3Home,
    path: PathBuf,
    identity: String,
    pass: RefCell<Option<Pass>>,
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
            pass: RefCell::new(None),
        }
    }
}
/// Each aggregate visits only one bounded rowid scope; no read transaction spans scopes.
fn discover(path: &Path) -> Result<BTreeMap<String, Fingerprint>, SourceError> {
    let mut fingerprints = BTreeMap::new();
    for projection in PROJECTIONS {
        let mut after = 0_i64;
        loop {
            let db = open(path)?;
            _t3_fingerprint::register(&db)?;
            let sql = format!(
                "SELECT thread_id,max(source_rowid),{}('{}',source_rowid,native_id,thread_id,native_clock,payload_json,streaming,extra) FROM (SELECT rowid source_rowid,{} native_id,thread_id,{} native_clock,payload_json,{} streaming,{} extra FROM {} WHERE rowid>?1 ORDER BY rowid LIMIT {BATCH_SIZE}) GROUP BY thread_id",
                _t3_fingerprint::FUNCTION,
                projection.name(),
                projection.id(),
                projection.clock(),
                projection.streaming_column(),
                projection.extra_column(),
                projection.table()
            );
            let mut statement = db.prepare(&sql)?;
            let rows = statement
                .query_map([after], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            if rows.is_empty() {
                break;
            }
            for (thread, rowid, value) in rows {
                after = after.max(rowid);
                _t3_fingerprint::merge(&mut fingerprints, thread, serde_json::from_str(&value)?);
            }
        }
    }
    Ok(fingerprints)
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
    let role = if matches!(projection, Projection::Messages) {
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
        let mut state = migrate_position(position)?;
        state.observed.extend(state.loaded.keys().cloned());
        let mut cache = self.pass.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|pass| pass.expected.as_ref() != position)
        {
            *cache = Some(Pass {
                fingerprints: discover(&self.path)?,
                expected: None,
            });
            state.after = None;
        }
        let pass = cache
            .as_mut()
            .ok_or_else(|| source_error("missing source fingerprint pass"))?;
        let next = pass
            .fingerprints
            .iter()
            .find(|(thread, fingerprint)| {
                state.after.as_ref().is_none_or(|after| *thread > after)
                    && state.loaded.get(*thread) != Some(*fingerprint)
            })
            .map(|(id, fp)| (id.clone(), fp.clone()));
        if let Some((thread, expected)) = next {
            state.observed.insert(thread.clone());
            let mut fingerprint = Fingerprint::default();
            let mut records = Vec::new();
            let mut streaming = false;
            let mut incomplete_runs = std::collections::BTreeSet::new();
            for projection in PROJECTIONS {
                let mut after = String::new();
                loop {
                    let rows = batch(&self.path, projection, &thread, &after)?;
                    let Some(last) = rows.last() else {
                        break;
                    };
                    after = last.native_id.clone();
                    for row in rows {
                        fingerprint.add(&row.fingerprint);
                        if row.streaming {
                            streaming = true;
                            incomplete_runs
                                .insert(row.payload["runId"].as_str().unwrap_or("*").to_owned());
                        } else {
                            records.push(normalized(projection, row)?);
                        }
                    }
                }
            }
            if fingerprint != expected {
                *cache = None;
                return Err(source_error(
                    "conversation changed during its read; retry catch-up",
                ));
            }
            if !streaming {
                state.loaded.insert(thread.clone(), fingerprint);
            }
            state.after = Some(thread);
            let position = SourcePosition(serde_json::to_value(state)?);
            pass.expected = Some(position.clone());
            return Ok(ConversationChanges {
                records,
                position,
                has_more: true,
                incomplete_runs: incomplete_runs.into_iter().collect(),
                missing: Vec::new(),
            });
        }
        let missing = state
            .observed
            .iter()
            .filter(|thread| !pass.fingerprints.contains_key(*thread))
            .cloned()
            .collect();
        state.after = None;
        *cache = None;
        Ok(ConversationChanges {
            records: Vec::new(),
            position: SourcePosition(serde_json::to_value(state)?),
            has_more: false,
            incomplete_runs: Vec::new(),
            missing,
        })
    }
    fn subscribe(&self) -> Result<Box<dyn ConversationNotifications>, SourceError> {
        super::_t3_notifications::subscribe(&self.path)
    }
}
