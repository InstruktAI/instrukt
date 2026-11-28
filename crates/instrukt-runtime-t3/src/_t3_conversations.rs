//! Non-live read-only projection batches from T3's SQLite state:
//! strictly isolated behind optional `ColdTextSource` (`history`) and `Archive` (`thread-retention`).
//! Zero live operational paths call into this module.
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use instrukt_core::config::Env;
use instrukt_core::home::Home;
use instrukt_core::ids::SessionId;
use instrukt_core::state_db::rusqlite::{
    Connection, OpenFlags, OptionalExtension, params, types::ValueRef,
};
use instrukt_runtime::cold_text::{
    ColdTextNotifications, ColdTextSource, ConversationOwnRecord, ConversationText, MessageText,
    Role, SourceError, SourcePosition, TextChanges,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::_t3_fingerprint::{self, Fingerprint};
use crate::_t3_home::T3Home;

const BATCH_SIZE: usize = 128;
const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;
const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Projection {
    Threads,
    Messages,
}

const TEXT: &[Projection] = &[Projection::Threads, Projection::Messages];

impl Projection {
    fn name(self) -> &'static str {
        match self {
            Self::Threads => "threads",
            Self::Messages => "messages",
        }
    }

    fn table(self) -> String {
        format!("orchestration_v2_projection_{}", self.name())
    }

    fn id(self) -> &'static str {
        match self {
            Self::Threads => "thread_id",
            Self::Messages => "message_id",
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
            Self::Threads => "''",
        }
    }

    fn columns(self) -> Vec<(&'static str, &'static str)> {
        let mut columns = vec![
            (self.id(), "TEXT"),
            ("thread_id", "TEXT"),
            ("payload_json", "TEXT"),
            ("updated_at", "TEXT"),
        ];
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
        }
        columns
    }
}

fn source_error(detail: &'static str) -> SourceError {
    SourceError::Shape(detail)
}

const NO_UNENDED_RUN: &str = "NOT EXISTS(SELECT 1 FROM orchestration_v2_projection_runs \
     WHERE thread_id=?1 AND (status IS NULL OR status NOT IN \
     ('completed','failed','interrupted','cancelled','rolled_back')))";

/// What thread retention reads of a listed thread from its projection row.
pub(crate) struct RetentionRow {
    pub pinned: bool,
    pub opened_at: Result<Option<jiff::Timestamp>, ()>,
}

/// The projection rows of `threads`, by thread id: their pin and latest opening
/// (`opens.openedAt`, epoch milliseconds). A thread without a row is absent.
pub(crate) fn retention_rows(
    home: &Home,
    env: &Env,
    threads: &[&str],
) -> Result<BTreeMap<String, RetentionRow>, SourceError> {
    let location = T3Home::discover(home, env);
    let path = location.database();
    validate(&path, &location, &[Projection::Threads])?;
    let db = open(&path)?;
    let ids = serde_json::to_string(threads)?;
    let mut statement = db.prepare(
        "SELECT thread_id,payload_json FROM orchestration_v2_projection_threads \
         WHERE thread_id IN (SELECT value FROM json_each(?1))",
    )?;
    let rows = statement
        .query_map(params![ids], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, payload)| {
            let payload = payload
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .filter(|payload| payload["id"] == id.as_str());
            let row = match payload {
                Some(payload) => RetentionRow {
                    pinned: !payload["pinnedAt"].is_null(),
                    opened_at: opened_at(&payload),
                },
                None => RetentionRow {
                    pinned: true,
                    opened_at: Err(()),
                },
            };
            (id, row)
        })
        .collect())
}

fn opened_at(payload: &Value) -> Result<Option<jiff::Timestamp>, ()> {
    let opens = &payload["opens"];
    if opens.is_null() {
        return Ok(None);
    }
    let millis = opens["openedAt"]
        .as_f64()
        .filter(|ms| ms.is_finite())
        .ok_or(())?;
    let millis = millis.round() as i64;
    jiff::Timestamp::from_millisecond(millis)
        .map(Some)
        .map_err(|_| ())
}

const ARCHIVE_READ: &[(&str, &[(&str, &str)])] = &[
    (
        "orchestration_v2_projection_threads",
        &[
            ("thread_id", "TEXT"),
            ("payload_json", "TEXT"),
            ("updated_at", "TEXT"),
            ("archived_at", "TEXT"),
            ("deleted_at", "TEXT"),
        ],
    ),
    (
        "orchestration_v2_projection_runs",
        &[("thread_id", "TEXT"), ("status", "TEXT")],
    ),
    (
        "orchestration_v2_projection_runtime_requests",
        &[("thread_id", "TEXT"), ("status", "TEXT")],
    ),
    (
        "orchestration_v2_projection_subagents",
        &[("thread_id", "TEXT"), ("status", "TEXT")],
    ),
];

/// Whether `thread` still stands as it was observed for archiving, at `now`.
pub(crate) fn archive_holds(
    home: &Home,
    env: &Env,
    thread: &str,
    latest_activity: jiff::Timestamp,
    now: jiff::Timestamp,
) -> Result<bool, SourceError> {
    let db = open(&T3Home::discover(home, env).database())?;
    for (table, required) in ARCHIVE_READ {
        let found = columns(&db, table)?;
        if required
            .iter()
            .any(|(name, kind)| found.get(*name).map(String::as_str) != Some(*kind))
        {
            return Err(source_error(
                "required projection column or type is incompatible",
            ));
        }
    }
    let row: Option<(String, String, bool)> = db
        .query_row(
            &format!(
                "SELECT payload_json,updated_at,{NO_UNENDED_RUN} \
                AND NOT EXISTS(SELECT 1 FROM orchestration_v2_projection_runtime_requests \
                       WHERE thread_id=?1 AND status='pending') \
                AND NOT EXISTS(SELECT 1 FROM orchestration_v2_projection_subagents \
                       WHERE thread_id=?1 AND (status IS NULL OR status NOT IN \
                       ('completed','failed','interrupted','cancelled','idle'))) \
             FROM orchestration_v2_projection_threads \
             WHERE thread_id=?1 AND archived_at IS NULL AND deleted_at IS NULL"
            ),
            params![thread],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((payload, updated_at, quiet)) = row else {
        return Ok(false);
    };
    let payload: Value = serde_json::from_str(&payload)?;
    let instant = |value: &Value| {
        value
            .as_str()
            .and_then(|text| text.parse::<jiff::Timestamp>().ok())
    };
    let not_after = |at: Option<jiff::Timestamp>| at.is_some_and(|at| at <= latest_activity);
    Ok(quiet
        && payload["id"] == thread
        && payload["settledOverride"] == "settled"
        && instant(&payload["settledAt"]).is_some()
        && payload["pinnedAt"].is_null()
        && (payload["snoozedUntil"].is_null()
            || instant(&payload["snoozedUntil"]).is_some_and(|until| until <= now))
        && not_after(updated_at.parse().ok())
        && opened_at(&payload).is_ok_and(|opened| opened.is_none_or(|at| at <= latest_activity)))
}

/// Whether `thread`'s projection row shows it archived.
pub(crate) fn archived(home: &Home, env: &Env, thread: &str) -> Result<bool, SourceError> {
    let db = open(&T3Home::discover(home, env).database())?;
    Ok(db
        .query_row(
            "SELECT archived_at IS NOT NULL FROM orchestration_v2_projection_threads \
             WHERE thread_id=?1",
            params![thread],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
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
        "SELECT {},thread_id,updated_at,payload_json,{streaming},{extra},rowid FROM {} WHERE thread_id=?1 AND {}>?2 ORDER BY {} LIMIT {BATCH_SIZE}",
        projection.id(),
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
        }
        let still_streaming = streaming == 1 || payload["streaming"].as_bool() == Some(true);
        values.push(Row {
            native_id,
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
            observed: BTreeSet::new(),
            after: None,
        }
    }
}

fn migrate_position(position: Option<&SourcePosition>) -> Result<Position, SourceError> {
    match position {
        None => Ok(Position::default()),
        Some(p) if p.value()["version"] == 1 => Ok(Position::default()),
        Some(p) if p.value()["version"] == 2 => Ok(serde_json::from_value(p.value().clone())?),
        _ => Err(source_error("unknown source-position version")),
    }
}

struct Pass {
    fingerprints: BTreeMap<String, Fingerprint>,
    expected: Option<SourcePosition>,
}

pub(crate) struct T3Conversations {
    location: T3Home,
    path: PathBuf,
    identity: String,
    pass: RefCell<Option<Pass>>,
}

impl T3Conversations {
    pub(crate) fn new(home: &Home, env: &Env) -> Self {
        let location = T3Home::discover(home, env);
        let path = location.database();
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

fn discover(
    db: &Connection,
    projections: &[Projection],
) -> Result<BTreeMap<String, Fingerprint>, SourceError> {
    let mut fingerprints = BTreeMap::new();
    for &projection in projections {
        let mut after = 0_i64;
        loop {
            let sql = format!(
                "SELECT rowid,{},thread_id,updated_at,payload_json,{},{} FROM {} WHERE rowid>?1 ORDER BY rowid LIMIT {BATCH_SIZE}",
                projection.id(),
                projection.streaming_column(),
                projection.extra_column(),
                projection.table(),
            );
            let mut statement = db.prepare(&sql)?;
            let mut rows = statement.query([after])?;
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
    Ok(fingerprints)
}

fn conversation(thread: &str) -> Result<SessionId, SourceError> {
    SessionId::parse(thread).map_err(|_| source_error("invalid conversation reference"))
}

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

fn created_at(value: &Value) -> Result<Option<jiff::Timestamp>, SourceError> {
    Ok(payload_time(value, "createdAt")?.or(payload_time(value, "requestedAt")?))
}

fn message_role(value: &Value) -> Result<Role, SourceError> {
    Ok(serde_json::from_value::<Role>(value["role"].clone())?)
}

fn row_time(row: &Row) -> Result<jiff::Timestamp, SourceError> {
    row.updated_at
        .parse()
        .map_err(|_| source_error("invalid row timestamp"))
}

struct Rows {
    fingerprint: Fingerprint,
    finished: Vec<(Projection, Row)>,
    streaming: bool,
}

fn rows(db: &Connection, thread: &str, projections: &[Projection]) -> Result<Rows, SourceError> {
    let mut read = Rows {
        fingerprint: Fingerprint::default(),
        finished: Vec::new(),
        streaming: false,
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
                } else {
                    read.finished.push((projection, row));
                }
            }
        }
    }
    Ok(read)
}

/// Removes captured-context payloads from a T3 message's native record.
pub fn without_captured_context(message: &mut Value) {
    let Some(records) = message
        .pointer_mut("/context/records")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let remove = |holder: &mut Value, member: &str| {
        if let Some(holder) = holder.as_object_mut() {
            holder.shift_remove(member);
        }
    };
    for record in records {
        match record["kind"].as_str() {
            Some("terminal") => remove(record, "text"),
            Some("review-comment") => remove(record, "diff"),
            Some("element") => remove(record, "htmlPreview"),
            Some("preview-annotation") => {
                if let Some(elements) = record.get_mut("elements").and_then(Value::as_array_mut) {
                    for element in elements {
                        remove(element, "htmlPreview");
                    }
                }
            }
            _ => {}
        }
    }
}

/// Whether `message`, a T3 message's native record, is T3's report that a background command finished.
pub fn reports_background_command(message: &Value) -> bool {
    matches!(
        message
            .pointer("/notification/source/kind")
            .and_then(Value::as_str),
        Some("command" | "background_command")
    )
}

fn text(
    thread: &str,
    finished: Vec<(Projection, Row)>,
) -> Result<Option<ConversationText>, SourceError> {
    let mut record = None;
    let mut messages = Vec::new();
    for (projection, row) in finished {
        match projection {
            Projection::Threads => {
                let title = row.payload["title"]
                    .as_str()
                    .ok_or_else(|| source_error("invalid thread shape"))?
                    .to_owned();
                let parent = row.payload["lineage"]["parentThreadId"]
                    .as_str()
                    .map(conversation)
                    .transpose()?;
                record = Some(ConversationOwnRecord {
                    title,
                    parent,
                    native: row.payload,
                });
            }
            Projection::Messages => {
                if reports_background_command(&row.payload) {
                    continue;
                }
                let updated_at = Some(row_time(&row)?);
                let created_at = created_at(&row.payload)?;
                let role = message_role(&row.payload)?;
                let mut native = row.payload;
                without_captured_context(&mut native);
                messages.push(
                    MessageText::from_record(row.native_id, role, created_at, updated_at, native)
                        .ok_or_else(|| source_error("invalid message shape"))?,
                );
            }
        }
    }
    Ok(ConversationText::new(
        conversation(thread)?,
        record,
        messages,
    ))
}

enum Step {
    Read(String),
    End,
}

impl T3Conversations {
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
                        discover(&snapshot, projections)?
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
            state.after = None;
            *self.pass.borrow_mut() = None;
            return Ok(Step::End);
        };
        state.observed.insert(thread.clone());
        state.after = Some(thread.clone());
        Ok(Step::Read(thread.clone()))
    }

    fn expect(&self, position: &SourcePosition) {
        if let Some(pass) = self.pass.borrow_mut().as_mut() {
            pass.expected = Some(position.clone());
        }
    }

    fn texts(
        &self,
        position: Option<&SourcePosition>,
        mut state: Position,
    ) -> Result<TextChanges, SourceError> {
        let thread = match self.step(position, &mut state, TEXT)? {
            Step::Read(thread) => thread,
            Step::End => {
                return Ok(TextChanges {
                    conversations: Vec::new(),
                    position: SourcePosition::new(serde_json::to_value(state)?),
                    has_more: false,
                });
            }
        };
        let Rows {
            fingerprint,
            finished,
            streaming,
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
            position: SourcePosition::new(serde_json::to_value(state)?),
            has_more: true,
        };
        self.expect(&changes.position);
        Ok(changes)
    }
}

impl ColdTextSource for T3Conversations {
    fn identity(&self) -> &str {
        &self.identity
    }

    fn text_since(&self, position: Option<&SourcePosition>) -> Result<TextChanges, SourceError> {
        validate(&self.path, &self.location, TEXT)?;
        let mut state = migrate_position(position)?;
        state.observed.extend(state.loaded.keys().cloned());
        self.texts(position, state)
    }

    fn subscribe(&self) -> Result<Box<dyn ColdTextNotifications>, SourceError> {
        crate::_t3_notifications::subscribe(&self.path)
    }
}
