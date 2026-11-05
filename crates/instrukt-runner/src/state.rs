//! The runner's state in the state database's runner tables
//! (`state-database`, "Runner tables"): the event log and its cursors and
//! handled marks, conditional-request validators and poll deferrals,
//! watched-ref observations, executions, standing-agent bindings and
//! starts, held task failures and the last wake-up.
//!
//! Every write runs in one transaction under the per-computer writer lock,
//! taken for that transaction alone, so a runner write never waits on a
//! work command for longer than the command holds it. The runner lock
//! `state/runner.lock` serializes runner invocations and guards no data.

use std::path::{Path, PathBuf};

use instrukt_core::fsx::{self, FsError};
use instrukt_core::home::Home;
use instrukt_core::state_db::rusqlite::{Connection, Transaction};
use instrukt_core::state_db::{StateDb, StateDbError, StateRead};

/// The runner's state could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error(transparent)]
    Db(#[from] StateDbError),
    #[error("{} is a symbolic link; the runner does not write through links", path.display())]
    Symlink { path: PathBuf },
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Fs(#[from] FsError),
}

/// One home's runner state.
#[derive(Debug, Clone)]
pub struct RunnerState {
    home: Home,
}

impl RunnerState {
    pub fn new(home: &Home) -> Self {
        Self { home: home.clone() }
    }

    pub(crate) fn home(&self) -> &Home {
        &self.home
    }

    /// The machine's runner lock: one runner invocation at a time.
    pub fn runner_lock(&self) -> PathBuf {
        self.home.runner_lock_file()
    }

    /// Runs `change` in one write transaction under the writer lock.
    pub(crate) fn write<T, E: From<StateError>>(
        &self,
        change: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut db = StateDb::open(&self.home).map_err(StateError::from)?;
        let path = db.path().to_path_buf();
        let tx = db.transaction().map_err(StateError::from)?;
        let value = change(&tx)?;
        tx.commit()
            .map_err(StateDbError::sqlite(&path))
            .map_err(StateError::from)?;
        Ok(value)
    }

    /// Runs `read` against the database; `None` when none exists yet,
    /// which is empty state.
    pub(crate) fn read<T>(
        &self,
        read: impl FnOnce(&Connection, &Path) -> Result<T, StateError>,
    ) -> Result<Option<T>, StateError> {
        match StateRead::open(&self.home)? {
            Some(db) => read(db.conn(), db.path()).map(Some),
            None => Ok(None),
        }
    }
}

/// Maps a SQLite failure on the database at `path` into a state error.
pub(crate) fn sql(
    path: &Path,
) -> impl Fn(instrukt_core::state_db::rusqlite::Error) -> StateError + '_ {
    move |source| {
        StateError::Db(StateDbError::Sqlite {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// The path of the transaction's database, for error messages.
pub(crate) fn db_path(conn: &Connection) -> PathBuf {
    conn.path().map(PathBuf::from).unwrap_or_default()
}

/// A stored value the runner cannot read.
pub(crate) fn invalid(path: &Path, message: impl std::fmt::Display) -> StateError {
    StateError::Db(StateDbError::invalid(path, message.to_string()))
}

/// The durable identity of `project`'s local todo `slug`, as the work
/// record holds it — its live entry, else its latest retained one; runner
/// rows name todos by identity.
pub(crate) fn todo_id(
    conn: &Connection,
    project: &str,
    slug: &str,
) -> Result<Option<String>, StateError> {
    use instrukt_core::state_db::rusqlite::OptionalExtension;
    let path = db_path(conn);
    conn.query_row(
        "SELECT id FROM todos WHERE project = ?1 AND slug = ?2
         ORDER BY status IN ('active', 'iceboxed', 'structural_holder') DESC, created_at DESC
         LIMIT 1",
        [project, slug],
        |row| row.get(0),
    )
    .optional()
    .map_err(sql(&path))
}

/// Writes `bytes` to `path` atomically, refusing a symbolic link there.
pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), StateError> {
    refuse_symlink(path)?;
    fsx::atomic_write(path, bytes)?;
    Ok(())
}

/// Refuses `path` when it is a symbolic link; a missing path is fine.
pub(crate) fn refuse_symlink(path: &Path) -> Result<(), StateError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(StateError::Symlink {
            path: path.to_path_buf(),
        }),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StateError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}
