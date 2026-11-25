//! The review publication store `state/review-publication.db`
//! (`github-apps`, Review evidence storage): per verdict and destination
//! what GitHub shows of it, and the verdicts recognised as already
//! published. Written by the tick alone, under
//! `state/review-publication.lock`; readers take no lock.

use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, Transaction, params};
use instrukt_core::state_db::{Migration, Read, Store, StoreDb, StoreLock};
use jiff::Timestamp;

use super::_publication::Published;
use super::{Moment, ReviewError, Verdict};
use crate::evidence::ContentDigest;

/// How long a tick waits for the publication lock.
const LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const V1: &str = r#"
CREATE TABLE destinations (
 project TEXT NOT NULL,
 commit_id TEXT NOT NULL,
 destination TEXT NOT NULL,
 name TEXT NOT NULL,
 moment TEXT NOT NULL,
 state TEXT NOT NULL CHECK (state IN ('shown','outstanding','refused')),
 link TEXT,
 cause TEXT,
 updated_at TEXT NOT NULL,
 PRIMARY KEY (project, commit_id, destination)
);
CREATE TABLE recognised (
 project TEXT NOT NULL,
 commit_id TEXT NOT NULL,
 verdict TEXT NOT NULL,
 name TEXT NOT NULL,
 moment TEXT NOT NULL,
 body TEXT,
 recognised_at TEXT NOT NULL,
 PRIMARY KEY (project, commit_id)
);
CREATE TABLE derived (
 project TEXT NOT NULL,
 candidate TEXT NOT NULL,
 canonical TEXT NOT NULL,
 coverage TEXT NOT NULL,
 name TEXT NOT NULL,
 moment TEXT NOT NULL,
 recorded_at TEXT NOT NULL,
 withdrawn_at TEXT,
 uncovered TEXT,
 imported INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY (project, candidate)
);
"#;

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1)];

fn store(home: &Home) -> Store {
    Store {
        name: "the review publication store",
        file: home.review_publication_db_file(),
        lock: StoreLock::Exclusive(home.review_publication_lock_file(), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

/// Takes the publication lock for one write.
pub(super) fn open(home: &Home) -> Result<StoreDb, ReviewError> {
    Ok(StoreDb::open(&store(home))?)
}

/// Opens the store for reading without its lock; `None` when it holds
/// nothing yet.
pub(super) fn reader(home: &Home) -> Result<Option<Connection>, ReviewError> {
    let store = store(home);
    let snapshot = home.is_state_read_only();
    match store.read(snapshot)? {
        Some(Read::Current(conn)) => Ok(Some(conn)),
        Some(Read::Behind) if !snapshot => {
            drop(open(home)?);
            match store.read(false)? {
                Some(Read::Current(conn)) => Ok(Some(conn)),
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

/// Where a verdict is delivered.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Destination {
    /// The review status on the commit.
    Status,
    /// A review on this pull request.
    Pull(u64),
}

impl Destination {
    fn key(&self) -> String {
        match self {
            Destination::Status => "status".to_owned(),
            Destination::Pull(number) => format!("pull:{number}"),
        }
    }

    fn parse(text: &str) -> Option<Self> {
        if text == "status" {
            return Some(Destination::Status);
        }
        text.strip_prefix("pull:")?
            .parse()
            .ok()
            .map(Destination::Pull)
    }
}

/// What a destination's publication stands at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    /// GitHub shows the verdict there.
    Shown,
    /// Not written yet; the cause says why.
    Outstanding,
    /// GitHub refused it; not retried while what was observed holds.
    Refused,
}

impl DeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryState::Shown => "shown",
            DeliveryState::Outstanding => "outstanding",
            DeliveryState::Refused => "refused",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "shown" => Some(DeliveryState::Shown),
            "outstanding" => Some(DeliveryState::Outstanding),
            "refused" => Some(DeliveryState::Refused),
            _ => None,
        }
    }
}

/// One destination's publication of a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub destination: Destination,
    pub name: ContentDigest,
    pub moment: Moment,
    pub state: DeliveryState,
    /// The address of the review delivered there, or the one a status links.
    pub link: Option<String>,
    pub cause: Option<String>,
    pub updated_at: Timestamp,
}

/// A verdict recognised as already published, with the review body that
/// carries it when one was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recognised {
    pub published: Published,
    pub body: Option<String>,
    pub recognised_at: Timestamp,
}

fn bad(what: &str) -> ReviewError {
    ReviewError::Record(format!("publication {what}"))
}

/// Every delivery of `commit` in `project`.
pub(super) fn deliveries(
    conn: &Connection,
    project: &ProjectSlug,
    commit: &CommitSha,
) -> Result<Vec<Delivery>, ReviewError> {
    let mut statement = conn.prepare(
        "SELECT destination, name, moment, state, link, cause, updated_at FROM destinations
         WHERE project=?1 AND commit_id=?2 ORDER BY destination",
    )?;
    let rows = statement
        .query_map(params![project.as_str(), commit.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(destination, name, moment, state, link, cause, at)| {
            Ok(Delivery {
                destination: Destination::parse(&destination).ok_or_else(|| bad("destination"))?,
                name: ContentDigest::new(name),
                moment: Moment::parse(&moment).ok_or_else(|| bad("moment"))?,
                state: DeliveryState::parse(&state).ok_or_else(|| bad("state"))?,
                link,
                cause,
                updated_at: at.parse().map_err(|_| bad("time"))?,
            })
        })
        .collect()
}

/// Records `delivery` of `commit`.
pub(super) fn deliver(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    commit: &CommitSha,
    delivery: &Delivery,
) -> Result<(), ReviewError> {
    tx.execute(
        "INSERT INTO destinations (project, commit_id, destination, name, moment, state, link, cause, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (project, commit_id, destination) DO UPDATE SET name=excluded.name, moment=excluded.moment,
           state=excluded.state, link=excluded.link, cause=excluded.cause, updated_at=excluded.updated_at",
        params![
            project.as_str(),
            commit.as_str(),
            delivery.destination.key(),
            delivery.name.as_str(),
            delivery.moment.text(),
            delivery.state.as_str(),
            delivery.link,
            delivery.cause,
            delivery.updated_at.to_string(),
        ],
    )?;
    Ok(())
}

/// Forgets what was recorded of `destination` of `commit`: a refusal that
/// ended.
pub(super) fn forget(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    commit: &CommitSha,
    destination: &Destination,
) -> Result<(), ReviewError> {
    tx.execute(
        "DELETE FROM destinations WHERE project=?1 AND commit_id=?2 AND destination=?3",
        params![project.as_str(), commit.as_str(), destination.key()],
    )?;
    Ok(())
}

/// The verdict recognised for `commit`, when one is.
pub(super) fn recognised(
    conn: &Connection,
    project: &ProjectSlug,
    commit: &CommitSha,
) -> Result<Option<Recognised>, ReviewError> {
    let row = conn
        .query_row(
            "SELECT verdict, name, moment, body, recognised_at FROM recognised WHERE project=?1 AND commit_id=?2",
            params![project.as_str(), commit.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    row.map(|(verdict, name, moment, body, at)| {
        Ok(Recognised {
            published: Published {
                verdict: Verdict::parse(&verdict).ok_or_else(|| bad("verdict"))?,
                name: ContentDigest::new(name),
                moment: Moment::parse(&moment).ok_or_else(|| bad("moment"))?,
            },
            body,
            recognised_at: at.parse().map_err(|_| bad("time"))?,
        })
    })
    .transpose()
}

/// Records `recognised` as `commit`'s recognised verdict.
pub(super) fn recognise(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    commit: &CommitSha,
    recognised: &Recognised,
) -> Result<(), ReviewError> {
    tx.execute(
        "INSERT INTO recognised (project, commit_id, verdict, name, moment, body, recognised_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (project, commit_id) DO UPDATE SET verdict=excluded.verdict, name=excluded.name,
           moment=excluded.moment, body=excluded.body, recognised_at=excluded.recognised_at",
        params![
            project.as_str(),
            commit.as_str(),
            recognised.published.verdict.as_str(),
            recognised.published.name.as_str(),
            recognised.published.moment.text(),
            recognised.body,
            recognised.recognised_at.to_string(),
        ],
    )?;
    Ok(())
}

/// A derived approval: one the tick recorded, or one an import brought.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub candidate: CommitSha,
    pub canonical: String,
    pub coverage: String,
    pub name: ContentDigest,
    pub moment: Moment,
    /// When the tick recorded it, or when an import brought it.
    pub recorded_at: Timestamp,
    /// When the tick withdrew it, with the commit found uncovered.
    pub withdrawn: Option<(Timestamp, String)>,
    /// An import brought it rather than this computer's tick.
    pub imported: bool,
}

/// The derived approval of `candidate`, withdrawn or not.
pub(super) fn derived_of(
    conn: &Connection,
    project: &ProjectSlug,
    candidate: &CommitSha,
) -> Result<Option<Derived>, ReviewError> {
    let row = conn
        .query_row(
            "SELECT canonical, coverage, name, moment, recorded_at, withdrawn_at, uncovered, imported FROM derived WHERE project=?1 AND candidate=?2",
            params![project.as_str(), candidate.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, bool>(7)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(canonical, coverage, name, moment, at, withdrawn, uncovered, imported)| {
            Ok(Derived {
                candidate: candidate.clone(),
                canonical,
                coverage,
                name: ContentDigest::new(name),
                moment: Moment::parse(&moment).ok_or_else(|| bad("moment"))?,
                recorded_at: at.parse().map_err(|_| bad("time"))?,
                withdrawn: withdrawn
                    .map(|at| {
                        Ok::<_, ReviewError>((
                            at.parse().map_err(|_| bad("time"))?,
                            uncovered.unwrap_or_default(),
                        ))
                    })
                    .transpose()?,
                imported,
            })
        },
    )
    .transpose()
}

/// Records `derived` as the candidate's derived approval, replacing one
/// recorded earlier.
pub(super) fn record_derived(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    derived: &Derived,
) -> Result<(), ReviewError> {
    tx.execute(
        "INSERT INTO derived (project, candidate, canonical, coverage, name, moment, recorded_at, withdrawn_at, uncovered)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL)
         ON CONFLICT (project, candidate) DO UPDATE SET canonical=excluded.canonical, coverage=excluded.coverage,
           name=excluded.name, moment=excluded.moment, recorded_at=excluded.recorded_at, withdrawn_at=NULL, uncovered=NULL,
           imported=0",
        params![
            project.as_str(),
            derived.candidate.as_str(),
            derived.canonical,
            derived.coverage,
            derived.name.as_str(),
            derived.moment.text(),
            derived.recorded_at.to_string(),
        ],
    )?;
    Ok(())
}

/// Withdraws `approval`, with the commit found uncovered. An approval an
/// import brought has no row here yet: its withdrawal is recorded here all
/// the same, as the tick's record of what it did.
pub(super) fn withdraw(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    approval: &Derived,
    uncovered: &CommitSha,
    now: Timestamp,
) -> Result<(), ReviewError> {
    tx.execute(
        "INSERT INTO derived (project, candidate, canonical, coverage, name, moment, recorded_at, withdrawn_at, uncovered, imported)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (project, candidate) DO UPDATE SET withdrawn_at=excluded.withdrawn_at, uncovered=excluded.uncovered
         WHERE withdrawn_at IS NULL",
        params![
            project.as_str(),
            approval.candidate.as_str(),
            approval.canonical,
            approval.coverage,
            approval.name.as_str(),
            approval.moment.text(),
            approval.recorded_at.to_string(),
            now.to_string(),
            uncovered.as_str(),
            approval.imported,
        ],
    )?;
    Ok(())
}

/// Every candidate of `project` this store holds a derived approval of,
/// withdrawn or not.
pub(super) fn candidates(
    conn: &Connection,
    project: &ProjectSlug,
) -> Result<std::collections::BTreeSet<CommitSha>, ReviewError> {
    let mut statement = conn.prepare("SELECT candidate FROM derived WHERE project=?1")?;
    let rows = statement
        .query_map(params![project.as_str()], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    rows.iter()
        .map(|candidate| CommitSha::parse(candidate).map_err(|_| bad("candidate")))
        .collect()
}

/// Every derived approval of `project` the tick did not withdraw.
pub(super) fn standing_derived(
    conn: &Connection,
    project: &ProjectSlug,
) -> Result<Vec<Derived>, ReviewError> {
    let mut statement = conn.prepare(
        "SELECT candidate FROM derived WHERE project=?1 AND withdrawn_at IS NULL ORDER BY candidate",
    )?;
    let candidates = statement
        .query_map(params![project.as_str()], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut found = Vec::new();
    for candidate in candidates {
        let candidate = CommitSha::parse(&candidate).map_err(|_| bad("candidate"))?;
        found.extend(derived_of(conn, project, &candidate)?);
    }
    Ok(found)
}

/// Retires completed history older than `cutoff`: publications no longer
/// outstanding and recognitions of commits `current` does not name;
/// withdrawn derived approvals; and derived approvals of a candidate
/// `current` does not name, which is no longer the integration head
/// (`github-apps`, Review evidence storage).
pub(super) fn retire(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    cutoff: Timestamp,
    current: &[CommitSha],
) -> Result<(), ReviewError> {
    let keep = serde_json::to_string(&current.iter().map(CommitSha::as_str).collect::<Vec<_>>())
        .map_err(|e| ReviewError::Record(e.to_string()))?;
    let cutoff = cutoff.to_string();
    tx.execute(
        "DELETE FROM destinations WHERE project=?1 AND state!='outstanding' AND updated_at < ?2
         AND commit_id NOT IN (SELECT value FROM json_each(?3))",
        params![project.as_str(), cutoff, keep],
    )?;
    tx.execute(
        "DELETE FROM recognised WHERE project=?1 AND recognised_at < ?2
         AND commit_id NOT IN (SELECT value FROM json_each(?3))",
        params![project.as_str(), cutoff, keep],
    )?;
    tx.execute(
        "DELETE FROM derived WHERE project=?1 AND (withdrawn_at < ?2
         OR (withdrawn_at IS NULL AND recorded_at < ?2 AND candidate NOT IN (SELECT value FROM json_each(?3))))",
        params![project.as_str(), cutoff, keep],
    )?;
    Ok(())
}
