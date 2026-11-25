//! The requested-review store, `state/requested-review.db` under
//! `state/requested-review.lock`
//! (`project/instrukt/spec/feature/runner/requested-review`, Requested review
//! records): every admitted ask, written only by `instrukt review start`,
//! one short transaction at a time and never while the command waits on the
//! runtime; read without a lock.

use std::path::PathBuf;
use std::time::Duration;

use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, Row, params};
use instrukt_core::state_db::{Migration, Read, Store, StoreDb, StoreError, StoreLock};
use jiff::Timestamp;

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

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1)];

fn store(home: &Home) -> Store {
    Store {
        name: NAME,
        file: home.requested_review_db_file(),
        lock: StoreLock::Exclusive(home.requested_review_lock_file(), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

fn sql(home: &Home) -> impl Fn(instrukt_core::state_db::rusqlite::Error) -> StoreError + '_ {
    move |source| StoreError::Sqlite {
        store: NAME,
        path: home.requested_review_db_file(),
        source,
    }
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
    /// The project's registered checkout the reviewer is started in.
    pub checkout: &'a std::path::Path,
    pub delivery: Delivery,
    /// The generation a send goes to; none for a start.
    pub send_generation: Option<u64>,
    pub admitted_at: Timestamp,
}

/// A recorded ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Ask {
    pub number: u64,
    pub kind: String,
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
}

const COLUMNS: &str = "number, kind, digest, start_key, checkout, delivery, command_id, generation, admitted_at, accepted_at";

fn ask(row: &Row<'_>) -> instrukt_core::state_db::rusqlite::Result<Ask> {
    use instrukt_core::state_db::rusqlite::Error::FromSqlConversionFailure;
    use instrukt_core::state_db::rusqlite::types::Type;
    let moment = |index: usize, text: String| {
        text.parse::<Timestamp>()
            .map_err(|error| FromSqlConversionFailure(index, Type::Text, Box::new(error)))
    };
    Ok(Ask {
        number: row.get(0)?,
        kind: row.get(1)?,
        digest: row.get(2)?,
        start_key: row.get(3)?,
        checkout: PathBuf::from(row.get::<_, String>(4)?),
        delivery: match row.get::<_, String>(5)?.as_str() {
            "send" => Delivery::Send,
            _ => Delivery::Start,
        },
        command_id: row.get(6)?,
        generation: row.get(7)?,
        admitted_at: moment(8, row.get(8)?)?,
        accepted_at: row
            .get::<_, Option<String>>(9)?
            .map(|text| moment(9, text))
            .transpose()?,
    })
}

/// The command identity of a send: derived from the start key, the
/// generation it goes to and the ask, so a replay of the ask reuses it and a
/// later identical ask is a new command (`requested-review`, Starting the
/// reviewer).
pub(super) fn command_identity(
    start_key: &str,
    generation: u64,
    project: &ProjectSlug,
    head: &CommitSha,
    number: u64,
) -> String {
    use sha2::{Digest, Sha256};
    let scope = serde_json::json!([
        start_key,
        generation,
        project.as_str(),
        head.as_str(),
        number
    ]);
    format!(
        "instrukt:review-send:{:x}",
        Sha256::digest(scope.to_string().as_bytes())
    )
}

/// Records `new` as the head's next ask, with a send's command identity,
/// before anything reaches the runtime.
pub(super) fn record(home: &Home, new: &NewAsk<'_>) -> Result<Ask, StoreError> {
    let mut db = StoreDb::open(&store(home))?;
    db.write(|tx| {
        let number: u64 = tx
            .query_row(
                "SELECT coalesce(max(number), 0) + 1 FROM asks WHERE project=?1 AND head=?2",
                params![new.project.as_str(), new.head.as_str()],
                |row| row.get(0),
            )
            .map_err(sql(home))?;
        let command_id = new.send_generation.map(|generation| {
            command_identity(new.start_key, generation, new.project, new.head, number)
        });
        tx.execute(
            "INSERT INTO asks (project, head, number, kind, coordinates, rendering, digest, start_key, checkout, delivery, command_id, generation, admitted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
            ],
        )
        .map_err(sql(home))?;
        Ok(Ask {
            number,
            kind: new.kind.to_owned(),
            digest: new.digest.to_owned(),
            start_key: new.start_key.to_owned(),
            checkout: new.checkout.to_path_buf(),
            delivery: new.delivery,
            command_id,
            generation: new.send_generation,
            admitted_at: new.admitted_at,
            accepted_at: None,
        })
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

/// A connection to the store without its lock; `None` when it holds
/// nothing yet. A store at an older schema is brought forward first, except
/// from an inspection home, which reads only a current store.
fn reader(home: &Home) -> Result<Option<Connection>, StoreError> {
    let store = store(home);
    if home.is_state_read_only() {
        return Ok(match store.read(true)? {
            Some(Read::Current(conn)) => Some(conn),
            Some(Read::Behind) | None => None,
        });
    }
    match store.read(false)? {
        Some(Read::Current(conn)) => Ok(Some(conn)),
        None => Ok(None),
        Some(Read::Behind) => {
            drop(StoreDb::open(&store)?);
            match store.read(false)? {
                Some(Read::Current(conn)) => Ok(Some(conn)),
                _ => Ok(None),
            }
        }
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

/// The latest ask recorded under `start_key`, accepted or not: the
/// requested reviewer's assignment resolves from it.
#[expect(
    dead_code,
    reason = "the requested reviewer's assignment resolution reads it"
)]
pub(crate) fn latest_for_key(home: &Home, start_key: &str) -> Result<Option<Ask>, StoreError> {
    let Some(conn) = reader(home)? else {
        return Ok(None);
    };
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM asks WHERE start_key=?1 ORDER BY rowid DESC LIMIT 1"),
        params![start_key],
        ask,
    )
    .optional()
    .map_err(sql(home))
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
                source: instrukt_core::state_db::rusqlite::Error::FromSqlConversionFailure(
                    1,
                    instrukt_core::state_db::rusqlite::types::Type::Text,
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
