//! The review evidence store `state/review.db`
//! (`github-apps`, Review evidence storage): its own SQLite file, written
//! only by the commands a caller waits on, under `state/review.lock`.
//! Readers take no lock.

use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, Transaction, params};
use instrukt_core::state_db::{Migration, Store, StoreDb, StoreLock};
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

/// Version 2. Index version report digests (`name`, `ContentDigest::of(report)`)
/// so project-scoped prior-report trace lookups avoid scanning every version.
const V2: &str = r#"
CREATE INDEX versions_name ON versions (name);
"#;

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1), Migration::Sql(V2)];

pub(super) fn store(home: &Home) -> Store {
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

/// Opens the store strictly for reading without its lock; `None` when it holds
/// nothing yet. Older stores are refused directing to `instrukt install`.
pub(super) fn reader(home: &Home) -> Result<Option<Connection>, ReviewError> {
    Ok(store(home).read()?)
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
    version_at(row, 0)
}

/// The version whose columns start at `first` in `row`.
fn version_at(
    row: &instrukt_core::state_db::rusqlite::Row<'_>,
    first: usize,
) -> Result<Version, ReviewError> {
    let text = |at: usize| row.get::<_, String>(first + at);
    let record = |what: &str| ReviewError::Record(what.to_owned());
    Ok(Version {
        number: row.get(first)?,
        verdict: Verdict::parse(&text(1)?).ok_or_else(|| record("verdict"))?,
        name: ContentDigest::new(text(2)?),
        report: text(3)?,
        reviewer: SessionRef::parse(&text(4)?).map_err(|_| record("reviewer"))?,
        entered_by: row
            .get::<_, Option<String>>(first + 5)?
            .map(|s| SessionRef::parse(&s).map_err(|_| record("entering session")))
            .transpose()?,
        base: CommitSha::parse(&text(6)?).map_err(|_| record("base"))?,
        base_on_canonical: row.get(first + 7)?,
        request: row.get(first + 8)?,
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

/// The current version of every head verdict that stands here in
/// `project`, with its commit.
pub(super) fn standing(
    conn: &Connection,
    project: &ProjectSlug,
) -> Result<Vec<(CommitSha, Version)>, ReviewError> {
    let mut statement = conn.prepare(&format!(
        "SELECT s.head, {} FROM settlements s JOIN versions v ON v.settlement = s.id
         WHERE s.project=?1 AND s.standing='stands'
           AND v.number = (SELECT MAX(w.number) FROM versions w WHERE w.settlement = s.id)",
        VERSION_COLUMNS
            .split(", ")
            .map(|column| format!("v.{column}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))?;
    let rows = statement
        .query_map(params![project.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, version_at(row, 1)))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(head, version)| {
            Ok((
                CommitSha::parse(&head).map_err(|_| ReviewError::Record("head".into()))?,
                version?,
            ))
        })
        .collect()
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

/// Hands off every verdict that stands here in `project` to the export
/// `export`.
pub(super) fn hand_off(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    export: &str,
    now: Timestamp,
) -> Result<usize, ReviewError> {
    Ok(tx.execute(
        "UPDATE settlements SET standing='handed_off', cause=?2, changed_at=?3 WHERE project=?1 AND standing='stands'",
        params![project.as_str(), format!("export {export}"), now.to_string()],
    )?)
}

/// Sets settlement `settlement` aside, with the import that did it.
pub(super) fn set_aside(
    tx: &Transaction<'_>,
    settlement: i64,
    cause: &str,
    now: Timestamp,
) -> Result<(), ReviewError> {
    tx.execute(
        "UPDATE settlements SET standing='set_aside', cause=?2, changed_at=?3 WHERE id=?1",
        params![settlement, cause, now.to_string()],
    )?;
    Ok(())
}

/// Starts a settlement of `head` that an import set aside on arrival.
pub(super) fn arrive_set_aside(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    head: &CommitSha,
    cause: &str,
    now: Timestamp,
) -> Result<i64, ReviewError> {
    tx.execute(
        "INSERT INTO settlements (project, head, standing, cause, changed_at) VALUES (?1, ?2, 'set_aside', ?3, ?4)",
        params![project.as_str(), head.as_str(), cause, now.to_string()],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Whether an import already set aside a verdict of `head` named `name`.
pub(super) fn set_aside_named(
    conn: &Connection,
    project: &ProjectSlug,
    head: &CommitSha,
    name: &str,
) -> Result<bool, ReviewError> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM settlements s JOIN versions v ON v.settlement = s.id
         WHERE s.project=?1 AND s.head=?2 AND s.standing='set_aside' AND v.name=?3)",
        params![project.as_str(), head.as_str(), name],
        |row| row.get(0),
    )?)
}

/// Gives version `number` of settlement `settlement` the moment `moment`.
pub(super) fn move_moment(
    tx: &Transaction<'_>,
    settlement: i64,
    number: u64,
    moment: Moment,
) -> Result<(), ReviewError> {
    tx.execute(
        "UPDATE versions SET moment=?3 WHERE settlement=?1 AND number=?2",
        params![settlement, number, moment.text()],
    )?;
    Ok(())
}

/// Every exemption of `project`.
pub(super) fn exemptions(
    conn: &Connection,
    project: &ProjectSlug,
) -> Result<Vec<super::Exemption>, ReviewError> {
    let mut statement =
        conn.prepare("SELECT commit_id FROM exemptions WHERE project=?1 ORDER BY commit_id")?;
    let commits = statement
        .query_map(params![project.as_str()], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut found = Vec::new();
    for commit in commits {
        let commit = CommitSha::parse(&commit).map_err(|_| ReviewError::Record("commit".into()))?;
        found.extend(exemption(conn, project, &commit)?);
    }
    Ok(found)
}

/// Keeps a derived approval an import brought, unless one is held for its
/// candidate.
pub(super) fn import_approval(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    approval: &super::Derived,
) -> Result<(), ReviewError> {
    tx.execute(
        "INSERT INTO imported_approvals (project, candidate, canonical, coverage, name, moment, imported_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (project, candidate) DO NOTHING",
        params![
            project.as_str(),
            approval.candidate.as_str(),
            approval.canonical,
            approval.coverage,
            approval.name.as_str(),
            approval.moment.text(),
            approval.recorded_at.to_string()
        ],
    )?;
    Ok(())
}

const IMPORTED_COLUMNS: &str = "candidate, canonical, coverage, name, moment, imported_at";

fn imported(
    row: &instrukt_core::state_db::rusqlite::Row<'_>,
) -> Result<super::Derived, ReviewError> {
    let text = |at: usize| row.get::<_, String>(at);
    let record = |what: &str| ReviewError::Record(format!("imported derived approval's {what}"));
    Ok(super::Derived {
        candidate: CommitSha::parse(&text(0)?).map_err(|_| record("candidate"))?,
        canonical: text(1)?,
        coverage: text(2)?,
        name: ContentDigest::new(text(3)?),
        moment: Moment::parse(&text(4)?).ok_or_else(|| record("moment"))?,
        recorded_at: text(5)?.parse().map_err(|_| record("time"))?,
        withdrawn: None,
        imported: true,
    })
}

/// The derived approval an import brought for `candidate`, when one did.
pub(super) fn imported_approval(
    conn: &Connection,
    project: &ProjectSlug,
    candidate: &CommitSha,
) -> Result<Option<super::Derived>, ReviewError> {
    conn.query_row(
        &format!(
            "SELECT {IMPORTED_COLUMNS} FROM imported_approvals WHERE project=?1 AND candidate=?2"
        ),
        params![project.as_str(), candidate.as_str()],
        |row| Ok(imported(row)),
    )
    .optional()?
    .transpose()
}

/// Every derived approval imports brought for `project`.
pub(super) fn imported_approvals(
    conn: &Connection,
    project: &ProjectSlug,
) -> Result<Vec<super::Derived>, ReviewError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {IMPORTED_COLUMNS} FROM imported_approvals WHERE project=?1 ORDER BY candidate"
    ))?;
    statement
        .query_map(params![project.as_str()], |row| Ok(imported(row)))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .collect()
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

/// What git shows to be history among the evidence aged past the retention
/// period.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Aged {
    /// Commits the canonical branch contains: all their evidence is history.
    pub contained: Vec<(ProjectSlug, CommitSha)>,
    /// Commits no branch of the project reaches any more: their exemptions
    /// and imported derived approvals are history. A head verdict that
    /// stands stays current, since the status GitHub keeps admits the
    /// commit wherever it is proposed again.
    pub unreached: Vec<(ProjectSlug, CommitSha)>,
}

/// Retires the history older than `cutoff` that this store alone shows to
/// be history: replaced versions of a standing verdict, and verdicts an
/// export handed off or an import set aside. What only git can show to be
/// history is retired for the commits `aged` names.
pub(super) fn retire(
    tx: &Transaction<'_>,
    cutoff: Timestamp,
    aged: &Aged,
) -> Result<(), ReviewError> {
    let cutoff = cutoff.to_string();
    tx.execute(
        "DELETE FROM versions WHERE recorded_at < ?1 AND number < (SELECT MAX(v.number) FROM versions v WHERE v.settlement = versions.settlement)",
        params![cutoff],
    )?;
    tx.execute(
        "DELETE FROM versions WHERE settlement IN (SELECT id FROM settlements WHERE standing IN ('set_aside','handed_off') AND changed_at < ?1)",
        params![cutoff],
    )?;
    tx.execute(
        "DELETE FROM settlements WHERE standing IN ('set_aside','handed_off') AND changed_at < ?1",
        params![cutoff],
    )?;
    for (project, commit) in &aged.contained {
        tx.execute(
            "DELETE FROM versions WHERE settlement IN (SELECT id FROM settlements WHERE project=?1 AND head=?2)",
            params![project.as_str(), commit.as_str()],
        )?;
        tx.execute(
            "DELETE FROM settlements WHERE project=?1 AND head=?2",
            params![project.as_str(), commit.as_str()],
        )?;
    }
    for (project, commit) in aged.contained.iter().chain(&aged.unreached) {
        tx.execute(
            "DELETE FROM exemptions WHERE project=?1 AND commit_id=?2",
            params![project.as_str(), commit.as_str()],
        )?;
        tx.execute(
            "DELETE FROM imported_approvals WHERE project=?1 AND candidate=?2",
            params![project.as_str(), commit.as_str()],
        )?;
    }
    Ok(())
}

/// The commit of `project` whose standing verdict has a version whose report
/// digest equals `digest`, breaking ties to the most recently recorded version.
/// Read-only and unlocked once the store is at its current schema.
pub(super) fn commit_with_report(
    home: &Home,
    project: &ProjectSlug,
    digest: &ContentDigest,
) -> Result<Option<CommitSha>, ReviewError> {
    let Some(conn) = reader(home)? else {
        return Ok(None);
    };
    let head: Option<String> = conn
        .query_row(
            "SELECT s.head
             FROM versions v
             JOIN settlements s ON s.id = v.settlement
             WHERE s.project = ?1 AND s.standing = 'stands' AND v.name = ?2
             ORDER BY julianday(v.recorded_at) DESC, v.settlement DESC, v.number DESC
             LIMIT 1",
            params![project.as_str(), digest.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    head.map(|sha| CommitSha::parse(&sha).map_err(|_| ReviewError::Record("head".into())))
        .transpose()
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
        "SELECT head FROM settlements WHERE project=?1 AND changed_at < ?2
         UNION SELECT commit_id FROM exemptions WHERE project=?1 AND recorded_at < ?2
         UNION SELECT candidate FROM imported_approvals WHERE project=?1 AND imported_at < ?2",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(ch: char) -> CommitSha {
        CommitSha::parse(&ch.to_string().repeat(40)).unwrap()
    }

    fn insert_v1_settlement(
        conn: &Connection,
        project: &str,
        head: &CommitSha,
        standing: &str,
        report: &str,
        recorded_at: &str,
    ) {
        let digest = ContentDigest::of(report);
        conn.execute(
            "INSERT INTO settlements (project, head, standing, changed_at) VALUES (?1, ?2, ?3, ?4)",
            params![project, head.as_str(), standing, recorded_at],
        )
        .unwrap();
        let settlement = conn.last_insert_rowid();
        let moment = Moment::at(recorded_at.parse().unwrap()).text();
        conn.execute(
            "INSERT INTO versions (settlement, number, verdict, name, report, reviewer, entered_by, base, base_on_canonical, request, unattributed, moment, recorded_at, recorded_on)
             VALUES (?1, 1, 'needs_work', ?2, ?3, 't3:rev-1', NULL, ?4, 1, NULL, '[]', ?5, ?6, 'host-a')",
            params![
                settlement,
                digest.as_str(),
                report,
                sha('0').as_str(),
                moment,
                recorded_at,
            ],
        )
        .unwrap();
    }

    #[test]
    fn v1_to_v2_migration_creates_versions_name_index_and_scopes_lookup_by_project() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging-v1.db");
        let report = "# Review\n\nVerdict: needs_work\n";
        let digest = ContentDigest::of(report);
        let head_a_older = sha('a');
        let head_a_newer = sha('b');
        let head_a_handed = sha('c');
        let head_b = sha('d');

        {
            let conn = Connection::open(&staging).unwrap();
            conn.execute_batch(V1).unwrap();
            conn.pragma_update(None, "user_version", 1u32).unwrap();
            insert_v1_settlement(
                &conn,
                "alpha",
                &head_a_older,
                "stands",
                report,
                "2026-11-26T10:00:00Z",
            );
            insert_v1_settlement(
                &conn,
                "alpha",
                &head_a_newer,
                "stands",
                report,
                "2026-11-26T11:00:00Z",
            );
            insert_v1_settlement(
                &conn,
                "alpha",
                &head_a_handed,
                "handed_off",
                report,
                "2026-11-26T12:00:00Z",
            );
            insert_v1_settlement(
                &conn,
                "beta",
                &head_b,
                "stands",
                report,
                "2026-11-26T13:00:00Z",
            );
        }

        let home = Home::at_user_home(dir.path().join("home"));
        std::fs::create_dir_all(home.review_db_file().parent().unwrap()).unwrap();
        let bytes = std::fs::read(&staging).unwrap();
        instrukt_core::fsx::write(home.review_db_file(), &bytes).unwrap();

        let alpha = ProjectSlug::parse("alpha").unwrap();
        let beta = ProjectSlug::parse("beta").unwrap();
        let gamma = ProjectSlug::parse("gamma").unwrap();

        // Both inspection and ordinary reads refuse a V1 file without mutating it on disk.
        let inspection = home.for_inspection();
        assert!(matches!(
            commit_with_report(&inspection, &alpha, &digest),
            Err(ReviewError::Store(
                instrukt_core::state_db::StoreError::Older { found: 1, .. }
            ))
        ));
        assert!(matches!(
            commit_with_report(&home, &alpha, &digest),
            Err(ReviewError::Store(
                instrukt_core::state_db::StoreError::Older { found: 1, .. }
            ))
        ));
        {
            let raw = Connection::open(home.review_db_file()).unwrap();
            let version: u32 = raw
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(version, 1);
        }

        // Explicit migration upgrades on disk to V2 and builds `versions_name`.
        drop(StoreDb::open_migrating(&store(&home)).unwrap());
        assert_eq!(
            commit_with_report(&home, &alpha, &digest).unwrap(),
            Some(head_a_newer.clone())
        );
        assert_eq!(
            commit_with_report(&home, &beta, &digest).unwrap(),
            Some(head_b.clone())
        );
        assert_eq!(commit_with_report(&home, &gamma, &digest).unwrap(), None);
        assert_eq!(
            commit_with_report(&home, &alpha, &ContentDigest::of("unrecorded")).unwrap(),
            None
        );

        let conn = reader(&home).unwrap().unwrap();
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        let has_index: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'versions_name' AND tbl_name = 'versions')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(has_index);
        assert!(matches!(
            held(&conn, &alpha, &head_a_older).unwrap(),
            Held::Stands { .. }
        ));
        assert!(matches!(
            held(&conn, &beta, &head_b).unwrap(),
            Held::Stands { .. }
        ));
    }
}
