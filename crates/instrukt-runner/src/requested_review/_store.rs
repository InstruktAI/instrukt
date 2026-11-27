//! The requested-review store, `state/requested-review.db` under
//! `state/requested-review.lock`
//! (`project/instrukt/spec/feature/runner/requested-review`, Requested review
//! records): every admitted ask, written only by `instrukt review start`,
//! one short transaction at a time and never while the command waits on the
//! runtime; read without a lock and never migrated on read.

use std::path::{Path, PathBuf};
use std::time::Duration;

use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use instrukt_core::state_db::rusqlite::{
    self, Connection, OptionalExtension, Row, Transaction, params,
};
use instrukt_core::state_db::{Migration, Read, Store, StoreDb, StoreError, StoreLock};
use jiff::Timestamp;
use serde::Serialize;

use super::_admission::Coordinate;

const NAME: &str = "the requested-review store";

/// How long `instrukt review start` waits for the store's lock: only other
/// asks take it, each for a short transaction.
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

const V1: &str = r#"
CREATE TABLE asks (
 project TEXT NOT NULL,
 head TEXT NOT NULL,
 number INTEGER NOT NULL,
 kind TEXT NOT NULL,
 coordinates TEXT NOT NULL,
 rendering TEXT NOT NULL,
 digest TEXT NOT NULL,
 start_key TEXT NOT NULL,
 checkout TEXT NOT NULL,
 delivery TEXT NOT NULL CHECK (delivery IN ('start', 'send')),
 command_id TEXT,
 generation INTEGER,
 admitted_at TEXT NOT NULL,
 accepted_at TEXT,
 PRIMARY KEY (project, head, number)
);
CREATE INDEX asks_start_key ON asks (start_key);
"#;

const V2: &str = r#"
ALTER TABLE asks ADD COLUMN asker TEXT;
ALTER TABLE asks ADD COLUMN earlier_head TEXT;
ALTER TABLE asks ADD COLUMN traced_by TEXT CHECK (traced_by IS NULL OR traced_by IN ('digest', 'path')) CHECK ((earlier_head IS NULL) = (traced_by IS NULL));
ALTER TABLE asks ADD COLUMN report TEXT;
ALTER TABLE asks ADD COLUMN closed_at TEXT;
CREATE INDEX asks_report ON asks (project, report);
"#;

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1), Migration::Step(version_2)];

/// Version 2: adds asker, paired earlier-head trace columns, canonical report
/// destination with its `(project, report)` index, and `closed_at` for dead
/// unaccepted wakes, backfilling `report` from V1 `coordinates` pair arrays in
/// the same transaction.
fn version_2(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(V2)?;
    let rows: Vec<(i64, String)> = {
        let mut statement = tx.prepare("SELECT rowid, coordinates FROM asks")?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    let report_key = Coordinate::Report.name();
    for (rowid, coordinates) in rows {
        let pairs: Vec<(String, String)> = serde_json::from_str(&coordinates).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        if let Some((_, raw)) = pairs.into_iter().find(|(name, _)| name == report_key) {
            let raw_path = PathBuf::from(&raw);
            let normalized = canonical_report(&raw_path).unwrap_or(raw_path);
            tx.execute(
                "UPDATE asks SET report = ?2 WHERE rowid = ?1",
                params![rowid, normalized.to_string_lossy()],
            )?;
        }
    }
    Ok(())
}

fn store(home: &Home) -> Store {
    Store {
        name: NAME,
        file: home.requested_review_db_file(),
        lock: StoreLock::Exclusive(home.requested_review_lock_file(), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

fn sql(home: &Home) -> impl Fn(rusqlite::Error) -> StoreError + '_ {
    move |source| StoreError::Sqlite {
        store: NAME,
        path: home.requested_review_db_file(),
        source,
    }
}

/// Canonicalizes a report destination path: when its parent directory exists,
/// canonicalizes the parent directory and joins the file name so destinations
/// that do not exist yet still compare canonically against existing reports.
pub(super) fn canonical_report(path: &Path) -> Result<PathBuf, std::io::Error> {
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        let dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        Ok(std::fs::canonicalize(dir)?.join(name))
    } else {
        std::fs::canonicalize(path)
    }
}

/// How a re-review's prior report identified its earlier head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TracedBy {
    Digest,
    Path,
}

impl TracedBy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Digest => "digest",
            Self::Path => "path",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "digest" => Some(Self::Digest),
            "path" => Some(Self::Path),
            _ => None,
        }
    }
}

/// The earlier head a re-review's prior report traced to, paired with how it
/// was identified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EarlierHead {
    pub head: CommitSha,
    pub traced_by: TracedBy,
}

/// How an ask reaches its requested reviewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Delivery {
    /// A keyed start whose opening input is the rendering.
    Start,
    /// A send into the reviewer's idle session under a fixed command
    /// identity.
    Send,
}

impl Delivery {
    fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Send => "send",
        }
    }
}

/// An admitted ask about to be recorded.
pub(super) struct NewAsk<'a> {
    pub project: &'a ProjectSlug,
    pub head: &'a CommitSha,
    pub kind: &'a str,
    /// The admitted coordinates, as JSON.
    pub coordinates: &'a str,
    pub rendering: &'a str,
    pub digest: &'a str,
    pub start_key: &'a str,
    /// The worktree the reviewer is started in.
    pub checkout: &'a Path,
    pub delivery: Delivery,
    /// The generation a send goes to; none for a start.
    pub send_generation: Option<u64>,
    pub admitted_at: Timestamp,
    pub asker: Option<&'a SessionRef>,
    pub trace: Option<&'a EarlierHead>,
    /// Canonical report destination path.
    pub report: &'a Path,
}

/// Outcome of recording an ask under the store's writer lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RecordOutcome {
    Recorded(Box<Ask>),
    /// Another unclosed ask for `(project, head)` was recorded concurrently.
    Conflict,
}

/// A recorded ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Ask {
    pub project: String,
    pub number: u64,
    pub kind: String,
    pub rendering: String,
    pub digest: String,
    pub start_key: String,
    pub checkout: PathBuf,
    pub delivery: Delivery,
    /// A send's command identity, recorded before the send.
    pub command_id: Option<String>,
    /// The generation the ask reached (a send's target before acceptance).
    pub generation: Option<u64>,
    pub admitted_at: Timestamp,
    pub accepted_at: Option<Timestamp>,
    pub asker: Option<SessionRef>,
    pub trace: Option<EarlierHead>,
    pub report: PathBuf,
    pub closed_at: Option<Timestamp>,
}

const COLUMNS: &str = "project, number, kind, rendering, digest, start_key, checkout, delivery, command_id, generation, admitted_at, accepted_at, asker, earlier_head, traced_by, report, closed_at";

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct DecodeError(&'static str);

fn ask(row: &Row<'_>) -> rusqlite::Result<Ask> {
    use rusqlite::Error::FromSqlConversionFailure;
    use rusqlite::types::Type;
    let moment = |index: usize, text: String| {
        text.parse::<Timestamp>()
            .map_err(|error| FromSqlConversionFailure(index, Type::Text, Box::new(error)))
    };
    let asker = row
        .get::<_, Option<String>>(12)?
        .map(|text| {
            SessionRef::parse(&text)
                .map_err(|error| FromSqlConversionFailure(12, Type::Text, Box::new(error)))
        })
        .transpose()?;
    let earlier_head = row.get::<_, Option<String>>(13)?;
    let traced_by = row.get::<_, Option<String>>(14)?;
    let trace = match (earlier_head, traced_by) {
        (None, None) => None,
        (Some(head_text), Some(by_text)) => {
            let head = CommitSha::parse(&head_text)
                .map_err(|error| FromSqlConversionFailure(13, Type::Text, Box::new(error)))?;
            let traced_by = TracedBy::parse(&by_text).ok_or_else(|| {
                FromSqlConversionFailure(14, Type::Text, Box::new(DecodeError("unknown traced_by")))
            })?;
            Some(EarlierHead { head, traced_by })
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(FromSqlConversionFailure(
                14,
                Type::Text,
                Box::new(DecodeError(
                    "earlier_head and traced_by must both be set or both be null",
                )),
            ));
        }
    };
    let report = PathBuf::from(row.get::<_, Option<String>>(15)?.unwrap_or_default());
    let closed_at = row
        .get::<_, Option<String>>(16)?
        .map(|text| moment(16, text))
        .transpose()?;
    Ok(Ask {
        project: row.get(0)?,
        number: row.get(1)?,
        kind: row.get(2)?,
        rendering: row.get(3)?,
        digest: row.get(4)?,
        start_key: row.get(5)?,
        checkout: PathBuf::from(row.get::<_, String>(6)?),
        delivery: match row.get::<_, String>(7)?.as_str() {
            "send" => Delivery::Send,
            _ => Delivery::Start,
        },
        command_id: row.get(8)?,
        generation: row.get(9)?,
        admitted_at: moment(10, row.get(10)?)?,
        accepted_at: row
            .get::<_, Option<String>>(11)?
            .map(|text| moment(11, text))
            .transpose()?,
        asker,
        trace,
        report,
        closed_at,
    })
}

/// The command identity of a wake: derived from the start key, the generation
/// it goes to, the ask's project/head/number, and `admitted_at`, so a replay
/// of the ask reuses it and a later ask — even after ask numbers restart —
/// receives a distinct command identity (`requested-review`, Reaching the
/// reviewer).
pub(super) fn command_identity(
    start_key: &str,
    generation: u64,
    project: &ProjectSlug,
    head: &CommitSha,
    number: u64,
    admitted_at: Timestamp,
) -> String {
    use sha2::{Digest, Sha256};
    let scope = serde_json::json!([
        start_key,
        generation,
        project.as_str(),
        head.as_str(),
        number,
        admitted_at.to_string(),
    ]);
    format!(
        "instrukt:review-send:{:x}",
        Sha256::digest(scope.to_string().as_bytes())
    )
}

/// Records `new` as the head's next ask, with a wake's command identity,
/// before anything reaches the runtime. Re-validates under the store lock that
/// no competing unclosed ask appeared concurrently for `(project, head)`.
pub(super) fn record(
    home: &Home,
    new: &NewAsk<'_>,
    expected_last_number: Option<u64>,
) -> Result<RecordOutcome, StoreError> {
    let mut db = StoreDb::open(&store(home))?;
    db.write(|tx| {
        let current_last: Option<u64> = tx
            .query_row(
                "SELECT number FROM asks WHERE project=?1 AND head=?2 AND closed_at IS NULL ORDER BY number DESC LIMIT 1",
                params![new.project.as_str(), new.head.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql(home))?;
        if current_last != expected_last_number {
            return Ok(RecordOutcome::Conflict);
        }
        let number: u64 = tx
            .query_row(
                "SELECT coalesce(max(number), 0) + 1 FROM asks WHERE project=?1 AND head=?2",
                params![new.project.as_str(), new.head.as_str()],
                |row| row.get(0),
            )
            .map_err(sql(home))?;
        let command_id = new.send_generation.map(|generation| {
            command_identity(
                new.start_key,
                generation,
                new.project,
                new.head,
                number,
                new.admitted_at,
            )
        });
        let asker_text = new.asker.map(ToString::to_string);
        let earlier_head = new.trace.map(|t| t.head.as_str());
        let traced_by = new.trace.map(|t| t.traced_by.as_str());
        tx.execute(
            "INSERT INTO asks (project, head, number, kind, coordinates, rendering, digest, start_key, checkout, delivery, command_id, generation, admitted_at, asker, earlier_head, traced_by, report, closed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, NULL)",
            params![
                new.project.as_str(),
                new.head.as_str(),
                number,
                new.kind,
                new.coordinates,
                new.rendering,
                new.digest,
                new.start_key,
                new.checkout.to_string_lossy(),
                new.delivery.as_str(),
                command_id,
                new.send_generation,
                new.admitted_at.to_string(),
                asker_text,
                earlier_head,
                traced_by,
                new.report.to_string_lossy(),
            ],
        )
        .map_err(sql(home))?;
        Ok(RecordOutcome::Recorded(Box::new(Ask {
            project: new.project.to_string(),
            number,
            kind: new.kind.to_owned(),
            rendering: new.rendering.to_owned(),
            digest: new.digest.to_owned(),
            start_key: new.start_key.to_owned(),
            checkout: new.checkout.to_path_buf(),
            delivery: new.delivery,
            command_id,
            generation: new.send_generation,
            admitted_at: new.admitted_at,
            accepted_at: None,
            asker: new.asker.cloned(),
            trace: new.trace.cloned(),
            report: new.report.to_path_buf(),
            closed_at: None,
        })))
    })
}

/// Records that the runtime accepted ask `number` of `head`, at the
/// generation it reached.
pub(super) fn accept(
    home: &Home,
    project: &ProjectSlug,
    head: &CommitSha,
    number: u64,
    generation: u64,
    at: Timestamp,
) -> Result<(), StoreError> {
    let mut db = StoreDb::open(&store(home))?;
    db.write(|tx| {
        tx.execute(
            "UPDATE asks SET accepted_at=?4, generation=?5 WHERE project=?1 AND head=?2 AND number=?3",
            params![project.as_str(), head.as_str(), number, at.to_string(), generation],
        )
        .map_err(sql(home))?;
        Ok(())
    })
}

/// Closes an unaccepted ask whose target session was confirmed archived,
/// deleted, or superseded before replay so it is excluded from `Outstanding`
/// and assignment resolution.
#[cfg(test)]
pub(super) fn close_unaccepted(
    home: &Home,
    project: &ProjectSlug,
    head: &CommitSha,
    number: u64,
    at: Timestamp,
) -> Result<(), StoreError> {
    let mut db = StoreDb::open(&store(home))?;
    db.write(|tx| {
        tx.execute(
            "UPDATE asks SET closed_at=?4 WHERE project=?1 AND head=?2 AND number=?3 AND accepted_at IS NULL AND closed_at IS NULL",
            params![project.as_str(), head.as_str(), number, at.to_string()],
        )
        .map_err(sql(home))?;
        Ok(())
    })
}

/// A connection to the store without its lock; `None` when it holds nothing
/// yet. Uses non-migrating `Store::read(true)` for every home so readers
/// migrate older stores in memory without writing to disk; only writing
/// commands bring the on-disk file to its current version.
fn reader(home: &Home) -> Result<Option<Connection>, StoreError> {
    let store = store(home);
    match store.read(true)? {
        Some(Read::Current(conn)) => Ok(Some(conn)),
        None => Ok(None),
        Some(Read::Behind) => Err(StoreError::Invalid {
            store: NAME,
            path: home.requested_review_db_file(),
            message: "store schema is behind and could not be migrated in memory for reading"
                .into(),
        }),
    }
}

/// Every recorded ask of `head`, oldest first.
pub(super) fn asks(
    home: &Home,
    project: &ProjectSlug,
    head: &CommitSha,
) -> Result<Vec<Ask>, StoreError> {
    let Some(conn) = reader(home)? else {
        return Ok(Vec::new());
    };
    let mut statement = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM asks WHERE project=?1 AND head=?2 ORDER BY number"
        ))
        .map_err(sql(home))?;
    statement
        .query_map(params![project.as_str(), head.as_str()], ask)
        .map_err(sql(home))?
        .collect::<Result<_, _>>()
        .map_err(sql(home))
}

/// The latest unclosed ask recorded under `start_key`, accepted or not: the
/// requested reviewer's assignment resolves from it.
pub(super) fn latest_for_key(home: &Home, start_key: &str) -> Result<Option<Ask>, StoreError> {
    let Some(conn) = reader(home)? else {
        return Ok(None);
    };
    conn.query_row(
        &format!(
            "SELECT {COLUMNS} FROM asks WHERE start_key=?1 AND closed_at IS NULL ORDER BY rowid DESC LIMIT 1"
        ),
        params![start_key],
        ask,
    )
    .optional()
    .map_err(sql(home))
}

/// The head of the latest unclosed ask in `project` whose canonical report
/// destination equals `canonical_report`, excluding `exclude_head`.
#[cfg(test)]
pub(super) fn latest_by_report(
    home: &Home,
    project: &ProjectSlug,
    canonical_report: &Path,
    exclude_head: &CommitSha,
) -> Result<Option<CommitSha>, StoreError> {
    let Some(conn) = reader(home)? else {
        return Ok(None);
    };
    let head: Option<String> = conn
        .query_row(
            "SELECT head FROM asks WHERE project=?1 AND report=?2 AND head!=?3 AND closed_at IS NULL ORDER BY rowid DESC LIMIT 1",
            params![
                project.as_str(),
                canonical_report.to_string_lossy(),
                exclude_head.as_str(),
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql(home))?;
    head.map(|sha| {
        CommitSha::parse(&sha).map_err(|error| StoreError::Invalid {
            store: NAME,
            path: home.requested_review_db_file(),
            message: format!("invalid head sha in asks: {error}"),
        })
    })
    .transpose()
}

/// Removes the asks of every head of `project` whose last ask was admitted
/// before `cutoff` and which `contained` answers the canonical branch
/// contains: history past the retention period. Returns how many heads
/// retired.
pub(super) fn retire(
    home: &Home,
    project: &ProjectSlug,
    cutoff: Timestamp,
    contained: &dyn Fn(&str) -> bool,
) -> Result<usize, StoreError> {
    let Some(conn) = reader(home)? else {
        return Ok(0);
    };
    let mut statement = conn
        .prepare("SELECT head, admitted_at FROM asks WHERE project=?1")
        .map_err(sql(home))?;
    let rows: Vec<(String, String)> = statement
        .query_map(params![project.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(sql(home))?
        .collect::<Result<_, _>>()
        .map_err(sql(home))?;
    let mut last = std::collections::BTreeMap::<String, Timestamp>::new();
    for (head, admitted) in rows {
        let admitted = admitted
            .parse::<Timestamp>()
            .map_err(|error| StoreError::Sqlite {
                store: NAME,
                path: home.requested_review_db_file(),
                source: rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                ),
            })?;
        let entry = last.entry(head).or_insert(admitted);
        *entry = (*entry).max(admitted);
    }
    let retiring: Vec<String> = last
        .into_iter()
        .filter(|(head, admitted)| *admitted < cutoff && contained(head))
        .map(|(head, _)| head)
        .collect();
    if retiring.is_empty() {
        return Ok(0);
    }
    let mut db = StoreDb::open(&store(home))?;
    db.write(|tx| {
        for head in &retiring {
            tx.execute(
                "DELETE FROM asks WHERE project=?1 AND head=?2",
                params![project.as_str(), head],
            )
            .map_err(sql(home))?;
        }
        Ok(retiring.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(ch: char) -> CommitSha {
        CommitSha::parse(&ch.to_string().repeat(40)).unwrap()
    }

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn v1_to_v2_golden_fixture_backfills_reports_without_mutating_on_read_and_migrates_on_write() {
        let dir = tempfile::tempdir().unwrap();
        let existing_reports = dir.path().join("reports");
        std::fs::create_dir_all(&existing_reports).unwrap();
        let missing_file_report = existing_reports.join("not-created-yet.md");
        let removed_parent = dir.path().join("gone-dir");
        let missing_parent_report = removed_parent.join("old.md");

        let staging = dir.path().join("staging-v1.db");
        let head1 = sha('1');
        let head2 = sha('2');
        let head3 = sha('3');
        let project = ProjectSlug::parse("demo").unwrap();

        {
            let conn = Connection::open(&staging).unwrap();
            conn.execute_batch(V1).unwrap();
            conn.pragma_update(None, "user_version", 1u32).unwrap();
            let coords1 = serde_json::to_string(&vec![
                ("worktree", "/tmp/wt"),
                ("branch", "feat"),
                ("head", head1.as_str()),
                ("base", sha('0').as_str()),
                ("report", missing_file_report.to_str().unwrap()),
            ])
            .unwrap();
            let coords2 = serde_json::to_string(&vec![
                ("worktree", "/tmp/wt"),
                ("branch", "feat"),
                ("head", head2.as_str()),
                ("base", sha('0').as_str()),
                ("report", missing_parent_report.to_str().unwrap()),
            ])
            .unwrap();
            conn.execute(
                "INSERT INTO asks (project, head, number, kind, coordinates, rendering, digest, start_key, checkout, delivery, command_id, generation, admitted_at, accepted_at)
                 VALUES (?1, ?2, 1, 'review', ?3, 'rendered-1', 'sha256:1', 'instrukt:review:demo:h1', '/tmp/wt', 'start', NULL, 1, '2026-11-26T10:00:00Z', '2026-11-26T10:00:01Z')",
                params![project.as_str(), head1.as_str(), coords1],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO asks (project, head, number, kind, coordinates, rendering, digest, start_key, checkout, delivery, command_id, generation, admitted_at, accepted_at)
                 VALUES (?1, ?2, 1, 'review', ?3, 'rendered-2', 'sha256:2', 'instrukt:review:demo:h2', '/tmp/wt', 'start', NULL, 1, '2026-11-26T11:00:00Z', '2026-11-26T11:00:01Z')",
                params![project.as_str(), head2.as_str(), coords2],
            )
            .unwrap();
        }

        let home = Home::at_user_home(dir.path().join("home"));
        std::fs::create_dir_all(home.requested_review_db_file().parent().unwrap()).unwrap();
        let bytes = std::fs::read(&staging).unwrap();
        instrukt_core::fsx::write(home.requested_review_db_file(), &bytes).unwrap();

        // Reading migrates in memory: backfills both rows without ENOENT and keeps on-disk V1.
        let asks1 = asks(&home, &project, &head1).unwrap();
        assert_eq!(asks1.len(), 1);
        assert_eq!(asks1[0].rendering, "rendered-1");
        assert_eq!(asks1[0].asker, None);
        assert_eq!(asks1[0].trace, None);
        assert_eq!(asks1[0].closed_at, None);
        let canonical_missing_file = canonical_report(&missing_file_report).unwrap();
        assert_eq!(asks1[0].report, canonical_missing_file);

        let asks2 = asks(&home, &project, &head2).unwrap();
        assert_eq!(asks2.len(), 1);
        assert_eq!(asks2[0].rendering, "rendered-2");
        assert_eq!(asks2[0].report, missing_parent_report);

        assert_eq!(
            latest_by_report(&home, &project, &canonical_missing_file, &head3).unwrap(),
            Some(head1.clone())
        );
        assert_eq!(
            latest_by_report(&home, &project, &canonical_missing_file, &head1).unwrap(),
            None
        );

        {
            let raw = Connection::open(home.requested_review_db_file()).unwrap();
            let version: u32 = raw
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(version, 1);
        }

        // Writing migrates on disk to user_version == 2.
        let report3 = canonical_report(&existing_reports.join("three.md")).unwrap();
        let asker = SessionRef::parse("t3:dev-1").unwrap();
        let trace = EarlierHead {
            head: head1.clone(),
            traced_by: TracedBy::Digest,
        };
        let outcome = record(
            &home,
            &NewAsk {
                project: &project,
                head: &head3,
                kind: "re-review",
                coordinates: "[]",
                rendering: "rendered-3",
                digest: "sha256:3",
                start_key: "instrukt:review:demo:h1",
                checkout: Path::new("/tmp/wt"),
                delivery: Delivery::Send,
                send_generation: Some(1),
                admitted_at: ts("2026-11-26T12:00:00Z"),
                asker: Some(&asker),
                trace: Some(&trace),
                report: &report3,
            },
            None,
        )
        .unwrap();
        assert!(matches!(outcome, RecordOutcome::Recorded(_)));

        let raw = Connection::open(home.requested_review_db_file()).unwrap();
        let version: u32 = raw
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);

        let asks3 = asks(&home, &project, &head3).unwrap();
        assert_eq!(asks3[0].asker.as_ref(), Some(&asker));
        assert_eq!(asks3[0].trace.as_ref(), Some(&trace));
        assert_eq!(asks3[0].report, report3);
    }

    #[test]
    fn check_constraints_and_strict_read_decoding_reject_invalid_trace_columns() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().join("home"));
        let project = ProjectSlug::parse("demo").unwrap();
        let head = sha('a');
        let earlier = sha('b');
        let report = dir.path().join("r.md");

        // Initialize V2 schema cleanly.
        let _ = record(
            &home,
            &NewAsk {
                project: &project,
                head: &head,
                kind: "review",
                coordinates: "[]",
                rendering: "r",
                digest: "sha256:a",
                start_key: "instrukt:review:demo:a",
                checkout: dir.path(),
                delivery: Delivery::Start,
                send_generation: None,
                admitted_at: ts("2026-11-26T10:00:00Z"),
                asker: None,
                trace: None,
                report: &report,
            },
            None,
        )
        .unwrap();

        let conn = Connection::open(home.requested_review_db_file()).unwrap();
        // Normal SQLite CHECK rejects one-sided NULLs and invalid traced_by.
        assert!(
            conn.execute(
                "INSERT INTO asks (project, head, number, kind, coordinates, rendering, digest, start_key, checkout, delivery, admitted_at, earlier_head, traced_by, report)
                 VALUES ('demo', ?1, 2, 're-review', '[]', 'r', 'd', 'k', '/tmp', 'send', '2026-11-26T10:00:00Z', ?2, NULL, '/tmp/r.md')",
                params![head.as_str(), earlier.as_str()],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO asks (project, head, number, kind, coordinates, rendering, digest, start_key, checkout, delivery, admitted_at, earlier_head, traced_by, report)
                 VALUES ('demo', ?1, 3, 're-review', '[]', 'r', 'd', 'k', '/tmp', 'send', '2026-11-26T10:00:00Z', ?2, 'magic', '/tmp/r.md')",
                params![head.as_str(), earlier.as_str()],
            )
            .is_err()
        );

        // Bypass CHECK constraints to verify read-side strict decoding fails cleanly.
        conn.pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        let bad_head = sha('c');
        conn.execute(
            "INSERT INTO asks (project, head, number, kind, coordinates, rendering, digest, start_key, checkout, delivery, admitted_at, earlier_head, traced_by, report)
             VALUES ('demo', ?1, 1, 're-review', '[]', 'r', 'd', 'k', '/tmp', 'send', '2026-11-26T10:00:00Z', ?2, 'bogus', '/tmp/r.md')",
            params![bad_head.as_str(), earlier.as_str()],
        )
        .unwrap();
        let one_sided_head = sha('d');
        conn.execute(
            "INSERT INTO asks (project, head, number, kind, coordinates, rendering, digest, start_key, checkout, delivery, admitted_at, earlier_head, traced_by, report)
             VALUES ('demo', ?1, 1, 're-review', '[]', 'r', 'd', 'k', '/tmp', 'send', '2026-11-26T10:00:00Z', ?2, NULL, '/tmp/r.md')",
            params![one_sided_head.as_str(), earlier.as_str()],
        )
        .unwrap();
        drop(conn);

        assert!(matches!(
            asks(&home, &project, &bad_head),
            Err(StoreError::Sqlite { .. })
        ));
        assert!(matches!(
            asks(&home, &project, &one_sided_head),
            Err(StoreError::Sqlite { .. })
        ));
    }

    #[test]
    fn close_unaccepted_excludes_dead_wake_and_record_detects_concurrent_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().join("home"));
        let project = ProjectSlug::parse("demo").unwrap();
        let head1 = sha('1');
        let head2 = sha('2');
        let key = "instrukt:review:demo:key";
        let report = dir.path().join("r.md");

        let RecordOutcome::Recorded(ask1) = record(
            &home,
            &NewAsk {
                project: &project,
                head: &head1,
                kind: "review",
                coordinates: "[]",
                rendering: "r1",
                digest: "sha256:1",
                start_key: key,
                checkout: dir.path(),
                delivery: Delivery::Start,
                send_generation: None,
                admitted_at: ts("2026-11-26T10:00:00Z"),
                asker: None,
                trace: None,
                report: &report,
            },
            None,
        )
        .unwrap() else {
            panic!("expected first ask to record");
        };
        accept(
            &home,
            &project,
            &head1,
            ask1.number,
            1,
            ts("2026-11-26T10:00:01Z"),
        )
        .unwrap();

        // Recording with stale expected_last_number detects concurrent conflict.
        let conflict = record(
            &home,
            &NewAsk {
                project: &project,
                head: &head1,
                kind: "dispute",
                coordinates: "[]",
                rendering: "r1b",
                digest: "sha256:1b",
                start_key: key,
                checkout: dir.path(),
                delivery: Delivery::Send,
                send_generation: Some(1),
                admitted_at: ts("2026-11-26T10:00:02Z"),
                asker: None,
                trace: None,
                report: &report,
            },
            None,
        )
        .unwrap();
        assert_eq!(conflict, RecordOutcome::Conflict);

        let RecordOutcome::Recorded(ask2) = record(
            &home,
            &NewAsk {
                project: &project,
                head: &head2,
                kind: "re-review",
                coordinates: "[]",
                rendering: "r2",
                digest: "sha256:2",
                start_key: key,
                checkout: dir.path(),
                delivery: Delivery::Send,
                send_generation: Some(1),
                admitted_at: ts("2026-11-26T11:00:00Z"),
                asker: None,
                trace: None,
                report: &report,
            },
            None,
        )
        .unwrap() else {
            panic!("expected second ask to record");
        };
        assert_eq!(
            latest_for_key(&home, key).unwrap().unwrap().digest,
            "sha256:2"
        );

        close_unaccepted(
            &home,
            &project,
            &head2,
            ask2.number,
            ts("2026-11-26T11:05:00Z"),
        )
        .unwrap();
        assert_eq!(
            latest_for_key(&home, key).unwrap().unwrap().digest,
            "sha256:1"
        );
        assert!(
            asks(&home, &project, &head2).unwrap()[0]
                .closed_at
                .is_some()
        );
    }
}
