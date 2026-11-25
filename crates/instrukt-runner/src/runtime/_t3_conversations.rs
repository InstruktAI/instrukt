//! Short read-only projection batches from T3's durable SQLite state, through
//! two reads: the turn read covers every record T3 holds of a conversation,
//! the text read only a conversation's own record and its messages; each
//! validates, fingerprints and reads only the tables it covers.
use super::_t3_fingerprint::{self, Fingerprint};
use super::conversations::{
    ConversationChanges, ConversationNotifications, ConversationOwnRecord, ConversationRecord,
    ConversationSource, ConversationText, MessageText, RecordKind, Role, Scope, SourceError,
    SourcePosition, TextChanges,
};
use crate::runtime::_t3_home::T3Home;
use instrukt_core::ids::SessionId;
use instrukt_core::state_db::rusqlite::types::ValueRef;
use instrukt_core::{
    config::Env,
    home::Home,
    state_db::rusqlite::{Connection, OpenFlags, OptionalExtension, params},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::{
    collections::{BTreeMap, BTreeSet},
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
/// What the turn read covers: every record T3 holds of a conversation.
const TURN: &[Projection] = &[
    Projection::Threads,
    Projection::Messages,
    Projection::Runs,
    Projection::Subagents,
    Projection::TurnItems,
];
/// What the text read covers: a conversation's own record and its messages.
const TEXT: &[Projection] = &[Projection::Threads, Projection::Messages];
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

pub(super) struct NativeInput {
    pub text: String,
    pub state: super::InputState,
    pub reference: Option<String>,
}

/// The text of the input that opened `thread`'s run in progress: among the
/// thread's runs whose projected status is `starting`, `running` or
/// `waiting`, the one whose projection row was inserted last, and the user
/// message its `userMessageId` names. `None` when no run has such a status.
///
/// T3's projection writer establishes that these statuses mark an active run
/// and that a run's `userMessageId` is its opening input; `preparing` and
/// `queued` runs have not started. It does not establish that a thread has at
/// most one active run, so with several the run whose row was inserted last
/// is taken; a later update to an older run's row keeps that row's place.
/// That the runs of a thread move from queued to active to ended is shown by
/// the test stand-in alone; the T3 contract suite runs no provider turn.
pub(super) fn active_opening(
    home: &Home,
    env: &Env,
    thread: &str,
) -> Result<Option<String>, SourceError> {
    let db = open(&T3Home::discover(home, env).database())?;
    let mut statement = db.prepare(
        "SELECT run_id,payload_json FROM orchestration_v2_projection_runs \
         WHERE thread_id=?1 AND status IN ('starting','running','waiting') \
         ORDER BY rowid DESC LIMIT 1",
    )?;
    let runs = statement
        .query_map(params![thread], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let Some((run_id, payload)) = runs.first() else {
        return Ok(None);
    };
    let run: Value = serde_json::from_str(payload)?;
    if run["id"].as_str() != Some(run_id.as_str()) || run["threadId"] != thread {
        return Err(source_error("run identity"));
    }
    let opening = run["userMessageId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| source_error("run opening input identity"))?;
    let message: String = db.query_row(
        "SELECT payload_json FROM orchestration_v2_projection_messages \
         WHERE message_id=?1 AND thread_id=?2 AND role='user'",
        params![opening, thread],
        |row| row.get(0),
    )?;
    let message: Value = serde_json::from_str(&message)?;
    if message["id"] != opening || message["threadId"] != thread {
        return Err(source_error("opening input identity"));
    }
    message["text"]
        .as_str()
        .map(|text| Some(text.to_owned()))
        .ok_or_else(|| source_error("opening input text"))
}

/// The newest assistant message the runtime has recorded in `thread`'s
/// running turn, taken whole: the run [`active_opening`] takes as the
/// thread's active one, and among that run's `assistant_message` turn items
/// no longer streaming, the one with the highest ordinal, its text as T3
/// recorded it, whatever phase the provider gave it. `None` when no run is
/// active, or the active one has no finished assistant message yet.
fn latest_update(path: &Path, thread: &str) -> Result<Option<String>, SourceError> {
    let db = open(path)?;
    let run: Option<String> = db
        .query_row(
            "SELECT run_id FROM orchestration_v2_projection_runs \
             WHERE thread_id=?1 AND status IN ('starting','running','waiting') \
             ORDER BY rowid DESC LIMIT 1",
            params![thread],
            |row| row.get(0),
        )
        .optional()?;
    let Some(run) = run else {
        return Ok(None);
    };
    let mut statement = db.prepare(
        "SELECT payload_json FROM orchestration_v2_projection_turn_items \
         WHERE thread_id=?1 AND type='assistant_message' \
         AND json_extract(payload_json,'$.runId')=?2 ORDER BY rowid",
    )?;
    let items = statement
        .query_map(params![thread, run], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut newest: Option<(u64, String)> = None;
    for item in items {
        let item: Value = serde_json::from_str(&item)?;
        if item["threadId"] != thread || item["runId"] != run.as_str() {
            return Err(source_error("assistant message identity"));
        }
        if item["streaming"].as_bool() == Some(true) {
            continue;
        }
        let ordinal = item["ordinal"]
            .as_u64()
            .ok_or_else(|| source_error("assistant message ordinal"))?;
        let text = item["text"]
            .as_str()
            .ok_or_else(|| source_error("assistant message text"))?;
        if newest.as_ref().is_none_or(|(at, _)| ordinal >= *at) {
            newest = Some((ordinal, text.to_owned()));
        }
    }
    Ok(newest.map(|(_, text)| text))
}

/// Read exact acceptance and quiescence from one projection snapshot.
pub(super) fn send_evidence(
    home: &Home,
    env: &Env,
    thread: &str,
    text: &str,
    command_id: Option<&str>,
) -> Result<(bool, bool, bool), SourceError> {
    let db = open(&T3Home::discover(home, env).database())?;
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM orchestration_v2_projection_messages
                       WHERE thread_id=?1 AND role='user'
                         AND json_extract(payload_json,'$.text')=?2),
                EXISTS(SELECT 1 FROM orchestration_v2_projection_threads
                       WHERE thread_id=?1 AND archived_at IS NULL AND deleted_at IS NULL)
                AND NOT EXISTS(SELECT 1 FROM orchestration_v2_projection_runs
                       WHERE thread_id=?1
                         AND (status IS NULL OR status NOT IN ('completed','failed','interrupted','cancelled','rolled_back'))),
                EXISTS(SELECT 1 FROM orchestration_command_receipts
                       WHERE command_id=?3 AND aggregate_kind='thread' AND aggregate_id=?1
                         AND command_type='message.dispatch' AND status='rejected')",
        params![thread, text, command_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?)
}

/// Run allocation precedes entry: queued/preparing inputs already have run IDs.
pub(super) fn inputs(
    home: &Home,
    env: &Env,
    thread: &str,
    text: Option<&str>,
) -> Result<Vec<NativeInput>, SourceError> {
    let db = open(&T3Home::discover(home, env).database())?;
    let mut statement = db.prepare(
        "SELECT m.message_id,m.payload_json,r.payload_json FROM \
         (SELECT message_id,thread_id,payload_json,rowid AS native_position \
          FROM orchestration_v2_projection_messages WHERE thread_id=?1 AND role='user' \
          ORDER BY rowid DESC LIMIT 100) m \
         LEFT JOIN orchestration_v2_projection_runs r ON r.thread_id=m.thread_id \
         AND r.run_id=CASE WHEN json_valid(m.payload_json) THEN json_extract(m.payload_json,'$.runId') END \
         WHERE (?2 IS NULL OR CASE WHEN json_valid(m.payload_json) THEN json_extract(m.payload_json,'$.text')=?2 ELSE 0 END) \
         ORDER BY m.native_position DESC LIMIT CASE WHEN ?2 IS NULL THEN 100 ELSE 1 END",
    )?;
    let mut rows = statement.query(params![thread, text])?;
    let mut inputs = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let message: Value = serde_json::from_str(&row.get::<_, String>(1)?)?;
        if message["id"] != id || message["threadId"] != thread || message["role"] != "user" {
            return Err(source_error("invalid input identity"));
        }
        let text = message["text"]
            .as_str()
            .ok_or_else(|| source_error("input text"))?
            .to_owned();
        let reference = match &message["runId"] {
            Value::Null => None,
            Value::String(id) if !id.is_empty() => Some(id.clone()),
            _ => return Err(source_error("input run reference")),
        };
        let mut state = super::InputState::Accepted;
        if let Some(payload) = row.get::<_, Option<String>>(2)? {
            let run: Value = serde_json::from_str(&payload)?;
            if run["id"].as_str() != reference.as_deref() || run["threadId"] != thread {
                return Err(source_error("input run relationship"));
            }
            let opening = run["userMessageId"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| source_error("run opening input identity"))?;
            let status = run["status"]
                .as_str()
                .ok_or_else(|| source_error("input run status"))?;
            if !matches!(
                status,
                "preparing"
                    | "queued"
                    | "starting"
                    | "running"
                    | "waiting"
                    | "completed"
                    | "interrupted"
                    | "failed"
                    | "cancelled"
                    | "rolled_back"
            ) {
                return Err(source_error("unsupported input run status"));
            }
            // T3 appends steering messages before its steer effect runs. The run's
            // earlier start proves entry only for its opening userMessageId.
            if opening == id
                && !matches!(status, "queued" | "preparing")
                && !run["startedAt"].is_null()
            {
                run["startedAt"]
                    .as_str()
                    .ok_or_else(|| source_error("run entry timestamp"))?
                    .parse::<jiff::Timestamp>()
                    .map_err(|_| source_error("run entry timestamp"))?;
                state = super::InputState::Consumed;
            }
        }
        inputs.push(NativeInput {
            text,
            state,
            reference,
        });
    }
    Ok(inputs)
}
fn open(path: &Path) -> Result<Connection, SourceError> {
    if !path.is_absolute() {
        return Err(source_error("database path must be absolute"));
    }
    let db = instrukt_core::fsx::open_database_uri(
        path,
        "mode=ro",
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
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
/// Checks the shape of the source tables a read consumes, `projections`,
/// before every read. It runs on each read, so its records are trace-level.
fn validate(path: &Path, location: &T3Home, projections: &[Projection]) -> Result<(), SourceError> {
    let db = open(path)?;
    for &projection in projections {
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
                tracing::trace!(event="history_migration_diagnostic_unavailable",error=%error);
                None
            }
        },
        Ok(_) => None,
        Err(error) => {
            tracing::trace!(event="history_migration_diagnostic_unavailable",error=%error);
            None
        }
    };
    let release = match std::fs::read(location.descriptor()) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v["serverVersion"].as_str().map(str::to_owned)),
        Err(error) => {
            tracing::trace!(event="history_release_diagnostic_unavailable",error=%error);
            None
        }
    };
    tracing::trace!(event="history_source_shape_validated",migration_level=?migration,t3_release=?release);
    Ok(())
}
fn batch(
    db: &Connection,
    projection: Projection,
    thread: &str,
    after: &str,
) -> Result<Vec<Row>, SourceError> {
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
    observed: BTreeSet<String>,
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
/// Discover raw content fingerprints of the source tables `projections`
/// within the caller's read snapshot.
fn discover(
    db: &Connection,
    scope: Scope<'_>,
    projections: &[Projection],
    presence: &mut Option<super::Presence>,
) -> Result<BTreeMap<String, Fingerprint>, SourceError> {
    let threads: Vec<Option<&str>> = match scope {
        Scope::All => vec![None],
        Scope::Only(ids) => ids
            .iter()
            .map(|id| Some(id.as_str()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    };
    let mut fingerprints = BTreeMap::new();
    for thread in threads {
        if matches!(scope, Scope::Only([_]))
            && let Some(thread) = thread
        {
            use super::Presence;
            *presence = Some(db.query_row(
                "SELECT archived_at IS NOT NULL, deleted_at IS NOT NULL FROM orchestration_v2_projection_threads WHERE thread_id=?1",
                [thread], |row| Ok(match (row.get::<_, bool>(0)?, row.get::<_, bool>(1)?) {
                    (_, true) => Presence::Absent,
                    (true, false) => Presence::Archived,
                    (false, false) => Presence::Present,
                }),
            ).optional()?.unwrap_or(Presence::Absent));
        }
        for &projection in projections {
            let mut after = 0_i64;
            loop {
                let sql = format!(
                    "SELECT rowid,{},thread_id,{},payload_json,{},{} FROM {} WHERE rowid>?1{} ORDER BY rowid LIMIT {BATCH_SIZE}",
                    projection.id(),
                    projection.clock(),
                    projection.streaming_column(),
                    projection.extra_column(),
                    projection.table(),
                    if thread.is_some() {
                        " AND thread_id=?2"
                    } else {
                        ""
                    }
                );
                let mut statement = db.prepare(&sql)?;
                let mut rows = match thread {
                    Some(thread) => statement.query(params![after, thread])?,
                    None => statement.query([after])?,
                };
                let mut any = false;
                while let Some(row) = rows.next()? {
                    any = true;
                    after = row.get(0)?;
                    if row
                        .get_ref(4)?
                        .as_str()
                        .map_err(|_| source_error("source payload is not text"))?
                        .len()
                        > MAX_RECORD_BYTES
                    {
                        return Err(source_error("source record exceeds size bound"));
                    }
                    let value = Fingerprint::row([
                        ValueRef::Text(projection.name().as_bytes()),
                        row.get_ref(0)?,
                        row.get_ref(1)?,
                        row.get_ref(2)?,
                        row.get_ref(3)?,
                        row.get_ref(4)?,
                        row.get_ref(5)?,
                        row.get_ref(6)?,
                    ]);
                    _t3_fingerprint::merge(&mut fingerprints, row.get(2)?, value);
                }
                if !any {
                    break;
                }
            }
        }
    }
    Ok(fingerprints)
}
fn conversation(thread: &str) -> Result<SessionId, SourceError> {
    SessionId::parse(thread).map_err(|_| source_error("invalid conversation reference"))
}
/// The time a native record holds under `name`, when it holds one.
fn payload_time(value: &Value, name: &str) -> Result<Option<jiff::Timestamp>, SourceError> {
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
}
fn row_time(row: &Row) -> Result<jiff::Timestamp, SourceError> {
    row.updated_at
        .parse()
        .map_err(|_| source_error("invalid row timestamp"))
}
fn normalized(projection: Projection, row: Row) -> Result<ConversationRecord, SourceError> {
    let value = &row.payload;
    let timestamp = |name: &str| payload_time(value, name);
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
    let updated_at = Some(row_time(&row)?);
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
        updated_at,
        completed,
        native: row.payload,
    })
}
/// One conversation's rows of the source tables a read covers, read whole
/// from a committed snapshot: the finished ones, with the fingerprint of
/// every row.
struct Rows {
    fingerprint: Fingerprint,
    finished: Vec<(Projection, Row)>,
    streaming: bool,
    incomplete_runs: BTreeSet<String>,
}

fn rows(db: &Connection, thread: &str, projections: &[Projection]) -> Result<Rows, SourceError> {
    let mut read = Rows {
        fingerprint: Fingerprint::default(),
        finished: Vec::new(),
        streaming: false,
        incomplete_runs: BTreeSet::new(),
    };
    for &projection in projections {
        let mut after = String::new();
        loop {
            let rows = batch(db, projection, thread, &after)?;
            let Some(last) = rows.last() else {
                break;
            };
            after = last.native_id.clone();
            for row in rows {
                read.fingerprint.add(&row.fingerprint);
                if row.streaming {
                    read.streaming = true;
                    read.incomplete_runs
                        .insert(row.payload["runId"].as_str().unwrap_or("*").to_owned());
                } else {
                    read.finished.push((projection, row));
                }
            }
        }
    }
    Ok(read)
}

/// One conversation read whole by the turn read.
struct Read {
    fingerprint: Fingerprint,
    records: Vec<ConversationRecord>,
    streaming: bool,
    incomplete_runs: BTreeSet<String>,
}

impl T3Conversations {
    fn read(&self, db: &Connection, thread: &str) -> Result<Read, SourceError> {
        let Rows {
            fingerprint,
            finished,
            streaming,
            incomplete_runs,
        } = rows(db, thread, TURN)?;
        Ok(Read {
            fingerprint,
            records: finished
                .into_iter()
                .map(|(projection, row)| normalized(projection, row))
                .collect::<Result<_, _>>()?,
            streaming,
            incomplete_runs,
        })
    }
}

/// What the text read returns of `thread`'s finished rows of [`TEXT`]:
/// `None` when it found neither the conversation's own record nor a
/// finished message.
fn text(
    thread: &str,
    finished: Vec<(Projection, Row)>,
) -> Result<Option<ConversationText>, SourceError> {
    let mut found = ConversationText {
        conversation_id: thread.to_owned(),
        record: None,
        messages: Vec::new(),
    };
    for (projection, row) in finished {
        match projection {
            Projection::Threads => {
                let title = row.payload["title"]
                    .as_str()
                    .ok_or_else(|| source_error("invalid thread shape"))?
                    .to_owned();
                found.record = Some(ConversationOwnRecord {
                    title,
                    parent_id: row.payload["lineage"]["parentThreadId"]
                        .as_str()
                        .map(str::to_owned),
                    native: row.payload,
                });
            }
            Projection::Messages => {
                let updated_at = Some(row_time(&row)?);
                let created_at = payload_time(&row.payload, "createdAt")?
                    .or(payload_time(&row.payload, "requestedAt")?);
                let role = serde_json::from_value::<Role>(row.payload["role"].clone())?;
                let mut native = row.payload;
                let text = match native.as_object_mut().and_then(|n| n.shift_remove("text")) {
                    Some(Value::String(text)) => text,
                    _ => return Err(source_error("invalid message shape")),
                };
                found.messages.push(MessageText {
                    id: row.native_id,
                    role,
                    text,
                    created_at,
                    updated_at,
                    native,
                });
            }
            Projection::Runs | Projection::Subagents | Projection::TurnItems => {
                return Err(source_error(
                    "the text read returned a record it does not cover",
                ));
            }
        }
    }
    Ok((found.record.is_some() || !found.messages.is_empty()).then_some(found))
}

/// What one settled conversation contributes to a batch; its fingerprint is
/// loaded unless it is still streaming.
fn contribute(
    changes: &mut ConversationChanges,
    state: &mut Position,
    thread: &str,
    read: Read,
) -> Result<(), SourceError> {
    let Read {
        fingerprint,
        records,
        streaming,
        incomplete_runs,
    } = read;
    if !streaming {
        state.loaded.insert(thread.to_owned(), fingerprint);
    }
    match super::_t3_completed::completed(
        &records,
        &incomplete_runs.into_iter().collect::<Vec<_>>(),
    ) {
        Ok(turns) => changes.completed_turns.extend(turns),
        Err(error) => changes
            .turn_failures
            .push(super::conversations::TurnEvidenceFailure {
                conversation: conversation(thread)?,
                cause: error.to_string(),
            }),
    }
    let mut named = false;
    for record in records
        .iter()
        .filter(|record| matches!(record.kind, RecordKind::Conversation))
    {
        named = true;
        changes
            .conversations
            .push(conversation(&record.conversation_id)?);
    }
    if !named {
        changes.missing.push(conversation(thread)?);
    }
    changes.records.extend(records);
    Ok(())
}

fn empty_changes(position: SourcePosition, has_more: bool) -> ConversationChanges {
    ConversationChanges {
        records: Vec::new(),
        position,
        has_more,
        completed_turns: Vec::new(),
        scoped_presence: None,
        conversations: Vec::new(),
        turn_failures: Vec::new(),
        missing: Vec::new(),
    }
}

/// Where a pass over every conversation stands for one read.
enum Step {
    /// Read this conversation next.
    Read(String),
    /// The pass is over; these conversations it observed earlier are gone.
    End(Vec<SessionId>),
}

impl T3Conversations {
    /// The next conversation of a pass over every conversation of the
    /// source tables `projections`, one per batch. The pass discovers them
    /// once and keeps that inventory while its own positions come back; any
    /// other position starts a new pass from the first conversation, so a
    /// position retained across a restart is caught up whole. The caller
    /// records the position it returns with [`Self::expect`].
    fn step(
        &self,
        position: Option<&SourcePosition>,
        state: &mut Position,
        projections: &[Projection],
    ) -> Result<Step, SourceError> {
        let fingerprints = {
            let mut cache = self.pass.borrow_mut();
            if cache
                .as_ref()
                .is_none_or(|pass| pass.expected.as_ref() != position)
            {
                *cache = Some(Pass {
                    fingerprints: {
                        let mut db = open(&self.path)?;
                        let snapshot = db.transaction()?;
                        discover(&snapshot, Scope::All, projections, &mut None)?
                    },
                    expected: None,
                });
                state.after = None;
            }
            cache
                .as_ref()
                .map(|pass| pass.fingerprints.clone())
                .ok_or_else(|| source_error("missing source fingerprint pass"))?
        };
        let next = fingerprints.iter().find(|(thread, fingerprint)| {
            state.after.as_ref().is_none_or(|after| *thread > after)
                && state.loaded.get(*thread) != Some(*fingerprint)
        });
        let Some((thread, _)) = next else {
            let missing = state
                .observed
                .iter()
                .filter(|thread| !fingerprints.contains_key(*thread))
                .map(|id| {
                    SessionId::parse(id)
                        .map_err(|_| source_error("invalid missing conversation reference"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            state.after = None;
            *self.pass.borrow_mut() = None;
            return Ok(Step::End(missing));
        };
        state.observed.insert(thread.clone());
        state.after = Some(thread.clone());
        Ok(Step::Read(thread.clone()))
    }

    /// The pass continues while `position`, the one a step's batch returned,
    /// comes back.
    fn expect(&self, position: &SourcePosition) {
        if let Some(pass) = self.pass.borrow_mut().as_mut() {
            pass.expected = Some(position.clone());
        }
    }

    /// The turn read over every conversation, one per batch.
    fn all(
        &self,
        position: Option<&SourcePosition>,
        mut state: Position,
    ) -> Result<ConversationChanges, SourceError> {
        let thread = match self.step(position, &mut state, TURN)? {
            Step::Read(thread) => thread,
            Step::End(missing) => {
                let mut changes =
                    empty_changes(SourcePosition(serde_json::to_value(state)?), false);
                changes.missing = missing;
                return Ok(changes);
            }
        };
        let read = {
            let mut db = open(&self.path)?;
            let snapshot = db.transaction()?;
            self.read(&snapshot, &thread)?
        };
        let mut changes = empty_changes(SourcePosition(serde_json::Value::Null), true);
        contribute(&mut changes, &mut state, &thread, read)?;
        changes.position = SourcePosition(serde_json::to_value(state)?);
        self.expect(&changes.position);
        Ok(changes)
    }

    /// The text read, one conversation per batch: a conversation whose
    /// finished text is read is marked loaded with the fingerprint of what
    /// it read, unless one of its messages is still streaming.
    fn texts(
        &self,
        position: Option<&SourcePosition>,
        mut state: Position,
    ) -> Result<TextChanges, SourceError> {
        let thread = match self.step(position, &mut state, TEXT)? {
            Step::Read(thread) => thread,
            Step::End(_) => {
                return Ok(TextChanges {
                    conversations: Vec::new(),
                    position: SourcePosition(serde_json::to_value(state)?),
                    has_more: false,
                });
            }
        };
        let Rows {
            fingerprint,
            finished,
            streaming,
            incomplete_runs: _,
        } = {
            let mut db = open(&self.path)?;
            let snapshot = db.transaction()?;
            rows(&snapshot, &thread, TEXT)?
        };
        if !streaming {
            state.loaded.insert(thread.clone(), fingerprint);
        }
        let changes = TextChanges {
            conversations: text(&thread, finished)?.into_iter().collect(),
            position: SourcePosition(serde_json::to_value(state)?),
            has_more: true,
        };
        self.expect(&changes.position);
        Ok(changes)
    }

    /// Every changed conversation of the scope in one batch: a scope names
    /// one or two conversations, so a scoped read never leaves a pass half
    /// done, and every scoped read compares the whole scope with what its
    /// position loaded. Conversations outside the scope are not read and
    /// leave the position.
    fn only(
        &self,
        ids: &[SessionId],
        mut state: Position,
    ) -> Result<ConversationChanges, SourceError> {
        let only = ids.iter().map(SessionId::as_str).collect::<BTreeSet<_>>();
        state
            .loaded
            .retain(|thread, _| only.contains(thread.as_str()));
        state
            .observed
            .retain(|thread| only.contains(thread.as_str()));
        state.after = None;
        let mut changes = empty_changes(SourcePosition(serde_json::Value::Null), false);
        let mut db = open(&self.path)?;
        let snapshot = db.transaction()?;
        let fingerprints = discover(
            &snapshot,
            Scope::Only(ids),
            TURN,
            &mut changes.scoped_presence,
        )?;
        changes.missing = state
            .observed
            .iter()
            .filter(|thread| !fingerprints.contains_key(*thread))
            .map(|id| conversation(id))
            .collect::<Result<Vec<_>, _>>()?;
        for (thread, expected) in fingerprints {
            if state.loaded.get(&thread) == Some(&expected) {
                continue;
            }
            state.observed.insert(thread.clone());
            let read = self.read(&snapshot, &thread)?;
            contribute(&mut changes, &mut state, &thread, read)?;
        }
        changes.position = SourcePosition(serde_json::to_value(state)?);
        Ok(changes)
    }
}

impl ConversationSource for T3Conversations {
    fn identity(&self) -> &str {
        &self.identity
    }
    fn changed_since(
        &self,
        position: Option<&SourcePosition>,
        scope: Scope<'_>,
    ) -> Result<ConversationChanges, SourceError> {
        validate(&self.path, &self.location, TURN)?;
        let mut state = migrate_position(position)?;
        state.observed.extend(state.loaded.keys().cloned());
        match scope {
            Scope::All => self.all(position, state),
            Scope::Only(ids) => self.only(ids, state),
        }
    }
    fn text_since(&self, position: Option<&SourcePosition>) -> Result<TextChanges, SourceError> {
        validate(&self.path, &self.location, TEXT)?;
        let mut state = migrate_position(position)?;
        state.observed.extend(state.loaded.keys().cloned());
        self.texts(position, state)
    }
    fn subscribe(&self) -> Result<Box<dyn ConversationNotifications>, SourceError> {
        super::_t3_notifications::subscribe(&self.path)
    }
    fn latest_update(&self, thread: &SessionId) -> Result<Option<String>, SourceError> {
        latest_update(&self.path, thread.as_str())
    }
}

/// Creation evidence survives deletion of the native thread projection.
pub(super) fn accepted_start(
    home: &Home,
    env: &Env,
    thread: &str,
    command: &str,
) -> Result<bool, SourceError> {
    let db = open(&T3Home::discover(home, env).database())?;
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM orchestration_command_receipts WHERE command_id=?1 AND aggregate_kind='thread' AND aggregate_id=?2 AND command_type='thread.create' AND status='accepted')",params![command,thread],|row|row.get(0))?)
}
