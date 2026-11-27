//! The proposal store, `.instrukt/knowledge-proposals.db` in a company's
//! home under `.instrukt/knowledge-proposals.lock`
//! (`project/instrukt/spec/domain/company`, Knowledge proposals extend this
//! boundary, State): written only by recordings a commit waits on, one short
//! transaction at a time; read from a snapshot, never migrated by a reader.

use std::collections::BTreeMap;
use std::time::Duration;

use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::state_db::rusqlite::params;
use instrukt_core::state_db::{Migration, Read, Store, StoreDb, StoreError, StoreLock};

const NAME: &str = "the knowledge-proposal store";

/// How long a recording waits for the store's lock: only other recordings
/// in the same home take it, each for one short transaction.
pub(super) const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

const V1: &str = r#"
CREATE TABLE proposals (
 company TEXT NOT NULL,
 commit_sha TEXT NOT NULL,
 branch TEXT,
 connector TEXT,
 conversation TEXT,
 session TEXT NOT NULL,
 speaker_identity TEXT,
 speaker_name TEXT,
 is_direct INTEGER,
 recorded_at TEXT NOT NULL,
 admitted INTEGER NOT NULL CHECK (admitted IN (0, 1)),
 cause TEXT,
 CHECK ((admitted = 1 AND cause IS NULL) OR (admitted = 0 AND cause IS NOT NULL)),
 PRIMARY KEY (company, commit_sha)
);
"#;

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1)];

pub(super) fn store(home: &Home, company: &ProjectSlug) -> Store {
    Store {
        name: NAME,
        file: home.knowledge_proposals_db_file(company),
        lock: StoreLock::Exclusive(home.knowledge_proposals_lock_file(company), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

/// Who the recorded entry says asked for its commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Asked {
    /// An administrator of the company: their identity and name as the
    /// connector supplied them, and whether the turn was a direct message.
    Administrator {
        identity: Option<String>,
        name: String,
        is_direct: bool,
    },
    /// The speaker could not be established: why.
    Unestablished { cause: String },
}

/// One entry about to be recorded.
#[derive(Debug, Clone)]
pub(super) struct Entry {
    pub company: ProjectSlug,
    pub commit: CommitSha,
    pub branch: Option<String>,
    pub connector: Option<String>,
    pub conversation: Option<String>,
    pub session: String,
    pub recorded_at: jiff::Timestamp,
    pub asked: Asked,
}

/// What the store records of one commit, as a lookup reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Recorded {
    /// An administrator's proposal.
    Proposal,
    /// An unestablished entry, with its cause.
    Unestablished(String),
}

/// Every entry of `company`, by commit, from a snapshot of the store; empty
/// when it holds nothing yet. An older store is brought forward in memory
/// only.
pub(super) fn read(
    home: &Home,
    company: &ProjectSlug,
) -> Result<BTreeMap<String, Recorded>, StoreError> {
    let store = store(home, company);
    let Some(Read::Current(conn)) = store.read(true)? else {
        return Ok(BTreeMap::new());
    };
    let sql = |source| StoreError::Sqlite {
        store: NAME,
        path: store.file.clone(),
        source,
    };
    let mut statement = conn
        .prepare("SELECT commit_sha, admitted, cause FROM proposals WHERE company=?1")
        .map_err(sql)?;
    let rows = statement
        .query_map([company.as_str()], |row| {
            let commit: String = row.get(0)?;
            let admitted: bool = row.get::<_, i64>(1)? == 1;
            let cause: Option<String> = row.get(2)?;
            Ok((commit, admitted, cause))
        })
        .map_err(sql)?;
    let mut entries = BTreeMap::new();
    for row in rows {
        let (commit, admitted, cause) = row.map_err(sql)?;
        let recorded = if admitted {
            Recorded::Proposal
        } else {
            Recorded::Unestablished(cause.unwrap_or_default())
        };
        entries.insert(commit, recorded);
    }
    Ok(entries)
}

/// Takes the store's lock, opens it (creating or migrating it), and in one
/// transaction deletes the entries `retired` names and records `entry`.
pub(super) fn record(home: &Home, entry: &Entry, retired: &[String]) -> Result<(), StoreError> {
    let store = store(home, &entry.company);
    let mut db = StoreDb::open(&store)?;
    let path = store.file.clone();
    let sql = move |source| StoreError::Sqlite {
        store: NAME,
        path: path.clone(),
        source,
    };
    db.write(|tx| {
        for commit in retired {
            tx.execute(
                "DELETE FROM proposals WHERE company=?1 AND commit_sha=?2",
                params![entry.company.as_str(), commit],
            )
            .map_err(&sql)?;
        }
        let (identity, name, is_direct, admitted, cause) = match &entry.asked {
            Asked::Administrator {
                identity,
                name,
                is_direct,
            } => (
                identity.clone(),
                Some(name.clone()),
                Some(*is_direct),
                true,
                None,
            ),
            Asked::Unestablished { cause } => (None, None, None, false, Some(cause.clone())),
        };
        tx.execute(
            "INSERT OR REPLACE INTO proposals (company, commit_sha, branch, connector, conversation, session, speaker_identity, speaker_name, is_direct, recorded_at, admitted, cause)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                entry.company.as_str(),
                entry.commit.as_str(),
                entry.branch,
                entry.connector,
                entry.conversation,
                entry.session,
                identity,
                name,
                is_direct,
                entry.recorded_at.to_string(),
                admitted,
                cause,
            ],
        )
        .map_err(&sql)?;
        Ok::<_, StoreError>(())
    })
}
