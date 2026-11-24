//! The review evidence store `state/review.db`
//! (`github-apps`, Review evidence storage): its own SQLite file, written
//! only by the commands a caller waits on, under `state/review.lock`.
//! Readers take no lock.

use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, Transaction, params};
use instrukt_core::state_db::{Migration, Read, Store, StoreDb, StoreLock};
use jiff::Timestamp;

use super::{Moment, ReviewError, Standing, Verdict, Version};
use crate::evidence::ContentDigest;

/// How long a recording command waits for another one holding the lock.
const LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Version 1. A settlement is one head verdict of a commit with its
/// versions; at most one of a commit stands here.
const V1: &str = r#"
CREATE TABLE settlements (
 id INTEGER PRIMARY KEY,
 project TEXT NOT NULL,
 head TEXT NOT NULL,
 standing TEXT NOT NULL CHECK (standing IN ('stands','handed_off','set_aside')),
 cause TEXT,
 changed_at TEXT NOT NULL
);
CREATE UNIQUE INDEX settlements_standing ON settlements (project, head) WHERE standing = 'stands';
CREATE INDEX settlements_commit ON settlements (project, head);
CREATE TABLE versions (
 settlement INTEGER NOT NULL REFERENCES settlements (id),
 number INTEGER NOT NULL,
 verdict TEXT NOT NULL CHECK (verdict IN ('approve','needs_work')),
 name TEXT NOT NULL,
 report TEXT NOT NULL,
 reviewer TEXT NOT NULL,
 entered_by TEXT,
 base TEXT NOT NULL,
 base_on_canonical INTEGER NOT NULL,
 request INTEGER,
 unattributed TEXT NOT NULL,
 moment TEXT NOT NULL,
 recorded_at TEXT NOT NULL,
 recorded_on TEXT NOT NULL,
 PRIMARY KEY (settlement, number)
);
CREATE TABLE exemptions (
 project TEXT NOT NULL,
 commit_id TEXT NOT NULL,
 basis TEXT NOT NULL,
 declared_by TEXT,
 names_session INTEGER NOT NULL,
 recorded_at TEXT NOT NULL,
 recorded_on TEXT NOT NULL,
 PRIMARY KEY (project, commit_id)
);
CREATE TABLE imported_approvals (
 project TEXT NOT NULL,
 candidate TEXT NOT NULL,
 canonical TEXT NOT NULL,
 coverage TEXT NOT NULL,
 name TEXT NOT NULL,
 moment TEXT NOT NULL,
 imported_at TEXT NOT NULL,
 PRIMARY KEY (project, candidate)
);
"#;

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1)];

fn store(home: &Home) -> Store {
    Store {
        name: "the review evidence store",
        file: home.review_db_file(),
        lock: StoreLock::Exclusive(home.review_lock_file(), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

/// Takes `state/review.lock` and opens the store for writing; the lock is
/// held for as long as the handle lives.
pub(super) fn open(home: &Home) -> Result<StoreDb, ReviewError> {
    Ok(StoreDb::open(&store(home))?)
}

/// Opens the store for reading without its lock; `None` when it holds
/// nothing yet. A store at an older schema is brought forward first,
/// except from an inspection home, which reads only a current store.
pub(super) fn reader(home: &Home) -> Result<Option<Connection>, ReviewError> {
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
            drop(open(home)?);
            match store.read(false)? {
                Some(Read::Current(conn)) => Ok(Some(conn)),
                _ => Ok(None),
            }
        }
    }
}

/// What this computer holds of a commit's head verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Held {
    /// No verdict of the commit stands or was handed off here.
    Nothing,
    /// A verdict stands here, with its current version.
    Stands {
        settlement: i64,
        current: Box<Version>,
    },
    /// The verdict that stood here was handed off by an export.
    HandedOff { export: String },
}

fn version(row: &instrukt_core::state_db::rusqlite::Row<'_>) -> Result<Version, ReviewError> {
    let text = |at: usize| row.get::<_, String>(at);
    let record = |what: &str| ReviewError::Record(what.to_owned());
    Ok(Version {
        number: row.get(0)?,
        verdict: Verdict::parse(&text(1)?).ok_or_else(|| record("verdict"))?,
        name: ContentDigest::new(text(2)?),
        report: text(3)?,
        reviewer: SessionRef::parse(&text(4)?).map_err(|_| record("reviewer"))?,
        entered_by: row
            .get::<_, Option<String>>(5)?
            .map(|s| SessionRef::parse(&s).map_err(|_| record("entering session")))
            .transpose()?,
        base: CommitSha::parse(&text(6)?).map_err(|_| record("base"))?,
        base_on_canonical: row.get(7)?,
        request: row.get(8)?,
        unattributed: serde_json::from_str::<Vec<String>>(&text(9)?)
            .map_err(|_| record("unattributed commits"))?
            .iter()
            .map(|sha| CommitSha::parse(sha).map_err(|_| record("unattributed commit")))
            .collect::<Result<_, _>>()?,
        moment: Moment::parse(&text(10)?).ok_or_else(|| record("moment"))?,
        recorded_at: text(11)?.parse().map_err(|_| record("recorded_at"))?,
        recorded_on: text(12)?,
    })
}

const VERSION_COLUMNS: &str = "number, verdict, name, report, reviewer, entered_by, base, base_on_canonical, request, unattributed, moment, recorded_at, recorded_on";

/// What `conn` holds of `head`'s verdict in `project`.
pub(super) fn held(
    conn: &Connection,
    project: &ProjectSlug,
    head: &CommitSha,
) -> Result<Held, ReviewError> {
    let standing: Option<i64> = conn
        .query_row(
            "SELECT id FROM settlements WHERE project=?1 AND head=?2 AND standing='stands'",
            params![project.as_str(), head.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(settlement) = standing {
        let current = conn.query_row(
            &format!("SELECT {VERSION_COLUMNS} FROM versions WHERE settlement=?1 ORDER BY number DESC LIMIT 1"),
            params![settlement],
            |row| Ok(version(row)),
        )??;
        return Ok(Held::Stands {
            settlement,
            current: Box::new(current),
        });
    }
    let handed: Option<Option<String>> = conn
        .query_row(
            "SELECT cause FROM settlements WHERE project=?1 AND head=?2 AND standing='handed_off' ORDER BY id DESC LIMIT 1",
            params![project.as_str(), head.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match handed {
        Some(export) => Held::HandedOff {
            export: export.unwrap_or_default(),
        },
        None => Held::Nothing,
    })
}

/// Every settlement of `head` in `project` with its versions, oldest first.
pub(super) fn settlements(
    conn: &Connection,
    project: &ProjectSlug,
    head: &CommitSha,
) -> Result<Vec<super::Settlement>, ReviewError> {
    let mut statement = conn.prepare(
        "SELECT id, standing, cause, changed_at FROM settlements WHERE project=?1 AND head=?2 ORDER BY id",
    )?;
    let rows = statement
        .query_map(params![project.as_str(), head.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut found = Vec::new();
    for (id, standing, cause, changed_at) in rows {
        let mut versions = conn.prepare(&format!(
            "SELECT {VERSION_COLUMNS} FROM versions WHERE settlement=?1 ORDER BY number"
        ))?;
        let versions = versions
            .query_map(params![id], |row| Ok(version(row)))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        found.push(super::Settlement {
            standing: Standing::parse(&standing)
                .ok_or_else(|| ReviewError::Record("standing".into()))?,
            cause,
            changed_at: changed_at
                .parse()
                .map_err(|_| ReviewError::Record("changed_at".into()))?,
            versions,
        });
    }
    Ok(found)
}

/// Starts a standing settlement of `head`; returns its id.
pub(super) fn settle(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    head: &CommitSha,
    now: Timestamp,
) -> Result<i64, ReviewError> {
    tx.execute(
        "INSERT INTO settlements (project, head, standing, changed_at) VALUES (?1, ?2, 'stands', ?3)",
        params![project.as_str(), head.as_str(), now.to_string()],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Adds `version` to settlement `settlement`.
pub(super) fn add_version(
    tx: &Transaction<'_>,
    settlement: i64,
    version: &Version,
) -> Result<(), ReviewError> {
    tx.execute(
        &format!("INSERT INTO versions (settlement, {VERSION_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"),
        params![
            settlement,
            version.number,
            version.verdict.as_str(),
            version.name.as_str(),
            version.report,
            version.reviewer.to_string(),
            version.entered_by.as_ref().map(ToString::to_string),
            version.base.as_str(),
            version.base_on_canonical,
            version.request,
            serde_json::to_string(
                &version
                    .unattributed
                    .iter()
                    .map(CommitSha::as_str)
                    .collect::<Vec<_>>()
            )
            .map_err(|e| ReviewError::Record(e.to_string()))?,
            version.moment.text(),
            version.recorded_at.to_string(),
            version.recorded_on,
        ],
    )?;
    Ok(())
}

/// `commit`'s exemption in `project`, when one is held.
pub(super) fn exemption(
    conn: &Connection,
    project: &ProjectSlug,
    commit: &CommitSha,
) -> Result<Option<super::Exemption>, ReviewError> {
    let row = conn
        .query_row(
            "SELECT basis, declared_by, names_session, recorded_at, recorded_on FROM exemptions WHERE project=?1 AND commit_id=?2",
            params![project.as_str(), commit.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(basis, declared_by, names_session, recorded_at, recorded_on)| {
            Ok(super::Exemption {
                commit: commit.clone(),
                basis,
                declared_by: declared_by
                    .map(|s| {
                        SessionRef::parse(&s).map_err(|_| ReviewError::Record("declared_by".into()))
                    })
                    .transpose()?,
                names_session,
                recorded_at: recorded_at
                    .parse()
                    .map_err(|_| ReviewError::Record("recorded_at".into()))?,
                recorded_on,
            })
        },
    )
    .transpose()
}

/// Records `exemption` for `project`.
pub(super) fn exempt(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    exemption: &super::Exemption,
) -> Result<(), ReviewError> {
    tx.execute(
        "INSERT INTO exemptions (project, commit_id, basis, declared_by, names_session, recorded_at, recorded_on) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            project.as_str(),
            exemption.commit.as_str(),
            exemption.basis,
            exemption.declared_by.as_ref().map(ToString::to_string),
            exemption.names_session,
            exemption.recorded_at.to_string(),
            exemption.recorded_on,
        ],
    )?;
    Ok(())
}

/// Retires the history older than `cutoff` that this store alone shows to
/// be history: replaced versions of a standing verdict and verdicts an
/// import set aside. What only git can show to be history — evidence of a
/// commit the canonical branch contains — is retired for the commits
/// `contained` names.
pub(super) fn retire(
    tx: &Transaction<'_>,
    cutoff: Timestamp,
    contained: &[(ProjectSlug, CommitSha)],
) -> Result<(), ReviewError> {
    let cutoff = cutoff.to_string();
    tx.execute(
        "DELETE FROM versions WHERE recorded_at < ?1 AND number < (SELECT MAX(v.number) FROM versions v WHERE v.settlement = versions.settlement)",
        params![cutoff],
    )?;
    tx.execute(
        "DELETE FROM versions WHERE settlement IN (SELECT id FROM settlements WHERE standing='set_aside' AND changed_at < ?1)",
        params![cutoff],
    )?;
    tx.execute(
        "DELETE FROM settlements WHERE standing='set_aside' AND changed_at < ?1",
        params![cutoff],
    )?;
    for (project, commit) in contained {
        tx.execute(
            "DELETE FROM versions WHERE settlement IN (SELECT id FROM settlements WHERE project=?1 AND head=?2)",
            params![project.as_str(), commit.as_str()],
        )?;
        tx.execute(
            "DELETE FROM settlements WHERE project=?1 AND head=?2",
            params![project.as_str(), commit.as_str()],
        )?;
        tx.execute(
            "DELETE FROM exemptions WHERE project=?1 AND commit_id=?2",
            params![project.as_str(), commit.as_str()],
        )?;
    }
    Ok(())
}

/// The commits of `project` whose evidence was last changed before
/// `cutoff`: those a recording command asks git whether the canonical
/// branch contains.
pub(super) fn aged_commits(
    conn: &Connection,
    project: &ProjectSlug,
    cutoff: Timestamp,
) -> Result<Vec<CommitSha>, ReviewError> {
    let cutoff = cutoff.to_string();
    let mut statement = conn.prepare(
        "SELECT head FROM settlements WHERE project=?1 AND changed_at < ?2 UNION SELECT commit_id FROM exemptions WHERE project=?1 AND recorded_at < ?2",
    )?;
    let rows = statement
        .query_map(params![project.as_str(), cutoff], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.iter()
        .map(|sha| CommitSha::parse(sha).map_err(|_| ReviewError::Record("commit".into())))
        .collect()
}
