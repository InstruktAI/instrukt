//! The state database `~/.instrukt/state/instrukt.db`
//! (`project/instrukt/spec/feature/work/state-database`): one SQLite file in
//! WAL journal mode holding the local work record and the runner's state.
//!
//! instrukt is its only writer. Every write runs while the process holds
//! the per-computer writer lock `~/.instrukt/state/writer.lock`; SQLite's
//! own locking is not the serialization contract. Opening for writing
//! applies the pending migrations, each in its own transaction that also
//! sets `PRAGMA user_version`; a reader of older state first takes that same
//! migration path. A database
//! newer than this build knows is refused.
//!
//! The work context owns the file, its migrations and the work tables; the
//! runner context owns the runner tables. Both reach the file through this
//! module, which sits in core because both depend on core.

mod _schema;
mod _snapshot;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

pub use rusqlite;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::fsx::{FsError, LockGuard};
use crate::home::Home;

/// How long a writer waits for another instrukt process holding the
/// writer lock.
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a statement waits on SQLite's own lock, which only another
/// tool or a crashed writer's recovery holds.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The schema version this build writes: the number of migrations it
/// ships.
pub fn schema_version() -> u32 {
    _schema::MIGRATIONS.len() as u32
}

/// The state database could not be opened, migrated, read or written.
#[derive(Debug, thiserror::Error)]
pub enum StateDbError {
    #[error("the state database at {} failed: {source}", path.display())]
    Sqlite {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error(
        "the state database at {} is at schema version {found}, newer than the {known} this instrukt knows; install the newer instrukt",
        path.display()
    )]
    Newer {
        path: PathBuf,
        found: u32,
        known: u32,
    },
    #[error(
        "the state database at {} is at schema version {found}, older than {known}; a command that writes the work record migrates it",
        path.display()
    )]
    Older {
        path: PathBuf,
        found: u32,
        known: u32,
    },
    #[error("{} is a symbolic link; instrukt does not keep its state through links", path.display())]
    Symlink { path: PathBuf },
    #[error(transparent)]
    Lock(#[from] FsError),
    #[error("{message}")]
    Invalid { path: PathBuf, message: String },
}

impl StateDbError {
    /// Wraps a SQLite failure on the database at `path`.
    pub fn sqlite(path: &Path) -> impl Fn(rusqlite::Error) -> Self + '_ {
        move |source| Self::Sqlite {
            path: path.to_path_buf(),
            source,
        }
    }

    /// A stored value this build cannot read.
    pub fn invalid(path: &Path, message: impl Into<String>) -> Self {
        Self::Invalid {
            path: path.to_path_buf(),
            message: format!(
                "the state database at {} holds a value this build cannot read: {}",
                path.display(),
                message.into()
            ),
        }
    }
}

/// The per-computer writer lock. Re-entrant within one process: a write
/// nested in a command that already holds it shares the hold, so nothing
/// in-process ever waits for itself.
#[derive(Debug, Clone)]
pub struct WriterLock {
    _held: Arc<LockGuard>,
}

static HELD: Mutex<Option<HashMap<PathBuf, Weak<LockGuard>>>> = Mutex::new(None);

impl WriterLock {
    /// Takes `home`'s writer lock, waiting a bounded time for another
    /// process holding it.
    pub fn acquire(home: &Home) -> Result<Self, StateDbError> {
        let path = home.writer_lock_file();
        refuse_links(home, &path)?;
        let mut held = HELD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let map = held.get_or_insert_with(HashMap::new);
        if let Some(guard) = map.get(&path).and_then(Weak::upgrade) {
            return Ok(Self { _held: guard });
        }
        let guard = Arc::new(LockGuard::acquire(&path, LOCK_TIMEOUT)?);
        map.insert(path, Arc::downgrade(&guard));
        Ok(Self { _held: guard })
    }
}

/// A connection opened for writing, at the current schema version, with the
/// writer lock held for as long as it lives.
#[derive(Debug)]
pub struct StateDb {
    conn: Connection,
    path: PathBuf,
    _lock: WriterLock,
}

impl StateDb {
    /// Takes the writer lock and opens `home`'s database, creating it and
    /// applying every pending migration.
    pub fn open(home: &Home) -> Result<Self, StateDbError> {
        let lock = WriterLock::acquire(home)?;
        let path = home.state_db_file();
        refuse_links(home, &path)?;
        if let Some(parent) = path.parent() {
            crate::fsx::create_dir_all(parent).map_err(|source| FsError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut conn = crate::fsx::open_database(&path, OpenFlags::default())
            .map_err(StateDbError::sqlite(&path))?;
        // SQLite gives the files it keeps beside the database the database's
        // own permissions; protecting the database first protects them too.
        protect(&path)?;
        configure(&conn, &path)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(StateDbError::sqlite(&path))?;
        protect(&path)?;
        migrate(&mut conn, &path)?;
        Ok(Self {
            conn,
            path,
            _lock: lock,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Begins one write transaction: everything written through it
    /// commits together or not at all.
    pub fn transaction(&mut self) -> Result<Transaction<'_>, StateDbError> {
        self.conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StateDbError::sqlite(&self.path))
    }

    /// Runs `change` in one transaction and commits it; an error rolls
    /// everything back.
    pub fn write<T, E: From<StateDbError>>(
        &mut self,
        change: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let path = self.path.clone();
        let tx = self.transaction()?;
        let value = change(&tx)?;
        tx.commit().map_err(StateDbError::sqlite(&path))?;
        Ok(value)
    }
}

/// A current-schema reader; older owned state migrates under its writer lock.
#[derive(Debug)]
pub struct StateRead {
    conn: Connection,
    path: PathBuf,
}

impl StateRead {
    /// Opens `home`'s database for reading; `None` when no database (or no
    /// schema) exists yet, which is empty state.
    pub fn open(home: &Home) -> Result<Option<Self>, StateDbError> {
        let path = home.state_db_file();
        refuse_links(home, &path)?;
        if !path.exists() {
            return Ok(None);
        }
        let conn = if home.is_state_read_only() {
            _snapshot::read(home, &path)?
        } else {
            crate::fsx::open_database(
                &path,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(StateDbError::sqlite(&path))?
        };
        configure(&conn, &path)?;
        let found = user_version(&conn, &path)?;
        let known = schema_version();
        if found == 0 {
            return Ok(None);
        }
        if found > known {
            return Err(StateDbError::Newer { path, found, known });
        }
        if found < known {
            if home.is_state_read_only() {
                return Err(StateDbError::Older { path, found, known });
            }
            drop(conn);
            drop(StateDb::open(home)?);
            return Self::open(home);
        }
        Ok(Some(Self { conn, path }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Makes the database and the files SQLite keeps beside it readable and
/// writable by their owner only.
fn protect(path: &Path) -> Result<(), StateDbError> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut files = vec![path.to_path_buf()];
    for side in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(side);
        files.push(PathBuf::from(name));
    }
    for file in files {
        match crate::fsx::set_permissions(&file, std::fs::Permissions::from_mode(0o600)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(FsError::Write { path: file, source }.into()),
        }
    }
    Ok(())
}

fn configure(conn: &Connection, path: &Path) -> Result<(), StateDbError> {
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(StateDbError::sqlite(path))?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(StateDbError::sqlite(path))?;
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(StateDbError::sqlite(path))?;
    Ok(())
}

fn user_version(conn: &Connection, path: &Path) -> Result<u32, StateDbError> {
    conn.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
        .map_err(StateDbError::sqlite(path))
}

/// Applies each migration past the stored version, each in its own
/// transaction that sets `user_version` too.
fn migrate(conn: &mut Connection, path: &Path) -> Result<(), StateDbError> {
    let found = user_version(conn, path)?;
    let known = schema_version();
    if found > known {
        return Err(StateDbError::Newer {
            path: path.to_path_buf(),
            found,
            known,
        });
    }
    for (index, migration) in _schema::MIGRATIONS.iter().enumerate().skip(found as usize) {
        let version = index as u32 + 1;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StateDbError::sqlite(path))?;
        match migration {
            _schema::Migration::Sql(script) => tx.execute_batch(script),
            _schema::Migration::Step(step) => step(&tx),
        }
        .map_err(StateDbError::sqlite(path))?;
        tx.pragma_update(None, "user_version", version)
            .map_err(StateDbError::sqlite(path))?;
        tx.commit().map_err(StateDbError::sqlite(path))?;
        tracing::info!(
            event = "state_db_migrated",
            path = %path.display(),
            version,
            "the state database moved to a new schema version"
        );
    }
    Ok(())
}

/// Refuses a symbolic link at `path` or any of its ancestors inside the
/// home (the home itself included).
fn refuse_links(home: &Home, path: &Path) -> Result<(), StateDbError> {
    let mut current = Some(path);
    while let Some(at) = current {
        match std::fs::symlink_metadata(at) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(StateDbError::Symlink {
                    path: at.to_path_buf(),
                });
            }
            _ => {}
        }
        if at == home.root() {
            break;
        }
        current = at.parent().filter(|parent| parent.starts_with(home.root()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_migration_applies_in_order_to_an_empty_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(&mut conn, Path::new(":memory:")).unwrap();
        assert_eq!(
            user_version(&conn, Path::new(":memory:")).unwrap(),
            schema_version()
        );
    }

    #[test]
    fn every_earlier_schema_upgrades_to_the_current_schema_without_losing_work() {
        for earlier in 1..schema_version() {
            let mut conn = Connection::open_in_memory().unwrap();
            conn.pragma_update(None, "foreign_keys", true).unwrap();
            for migration in _schema::MIGRATIONS.iter().take(earlier as usize) {
                let tx = conn.transaction().unwrap();
                match migration {
                    _schema::Migration::Sql(script) => tx.execute_batch(script),
                    _schema::Migration::Step(step) => step(&tx),
                }
                .unwrap();
                tx.commit().unwrap();
            }
            conn.pragma_update(None, "user_version", earlier).unwrap();
            conn.execute("INSERT INTO todos(id,project,slug,kind,status,created_at,created_by,revision,catch_up,sequence) VALUES('retained','project','retained','bug','active','2026-11-03T10:00:00Z','{}',1,'once',1)", []).unwrap();
            migrate(&mut conn, Path::new(":memory:")).unwrap();
            assert_eq!(
                user_version(&conn, Path::new(":memory:")).unwrap(),
                schema_version()
            );
            let revision: i64 = conn
                .query_row(
                    "SELECT revision FROM todos WHERE id='retained'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(revision, 1);
            let rows: i64 = conn
                .query_row("SELECT count(cause) FROM poll_deferrals", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0);
        }
    }

    #[test]
    fn version_19_adds_the_worker_state_and_the_skipped_wake_and_keeps_the_relays() {
        let mut conn = Connection::open_in_memory().unwrap();
        for migration in _schema::MIGRATIONS.iter().take(18) {
            let tx = conn.transaction().unwrap();
            match migration {
                _schema::Migration::Sql(script) => tx.execute_batch(script),
                _schema::Migration::Step(step) => step(&tx),
            }
            .unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", 18).unwrap();
        conn.execute("INSERT INTO conversation_relay(id,failure,updated_at) VALUES(1,'T3 is unreachable','2026-11-18T10:00:00Z')", []).unwrap();
        migrate(&mut conn, Path::new(":memory:")).unwrap();
        let relay: String = conn
            .query_row(
                "SELECT failure FROM conversation_relay WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(relay, "T3 is unreachable");
        conn.execute("INSERT INTO runner_workers(worker,failure,since) VALUES('history-extraction','unwatchable','2026-11-18T10:00:00Z')", []).unwrap();
        let workers: i64 = conn
            .query_row("SELECT count(*) FROM runner_workers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(workers, 1);
        conn.execute("INSERT INTO runner_skipped_wake(id,at,cause) VALUES(1,'2026-11-18T10:00:00Z','the dispatch lock is held')", []).unwrap();
        assert!(
            conn.execute("INSERT INTO runner_skipped_wake(id,at,cause) VALUES(2,'2026-11-18T10:01:00Z','again')", [])
                .is_err(),
            "one skipped wake at most"
        );
    }

    #[test]
    fn version_18_keeps_an_earlier_message_with_its_member_key_as_its_senders_name() {
        let mut conn = Connection::open_in_memory().unwrap();
        for migration in _schema::MIGRATIONS.iter().take(17) {
            let tx = conn.transaction().unwrap();
            match migration {
                _schema::Migration::Sql(script) => tx.execute_batch(script),
                _schema::Migration::Step(step) => step(&tx),
            }
            .unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", 17).unwrap();
        conn.execute("INSERT INTO conversation_messages(company,connector,conversation,message,member,written_us,written_at,arrived_at,state,text) VALUES('acme','slack','C1/1.1','1.1','steven',1,'2026-11-01T10:00:00Z','2026-11-01T10:00:00Z','pending','a question')", []).unwrap();
        migrate(&mut conn, Path::new(":memory:")).unwrap();
        let kept = |conn: &Connection| -> (
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            String,
        ) {
            conn.query_row(
                "SELECT sender,sender_name,sender_email,state,text FROM conversation_messages WHERE message='1.1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap()
        };
        let before = kept(&conn);
        assert_eq!(before.0, None);
        assert_eq!(before.1.as_deref(), Some("steven"));
        assert_eq!(before.2, None);
        // A message can be refused now.
        conn.execute("INSERT INTO conversation_messages(company,connector,conversation,message,sender,written_us,written_at,arrived_at,state,text) VALUES('acme','slack','C1/1.1','1.2','U1',2,'2026-11-01T10:01:00Z','2026-11-01T10:01:00Z','refused','hello')", []).unwrap();
        // Applied again, it changes nothing.
        let tx = conn.transaction().unwrap();
        if let Some(_schema::Migration::Step(step)) = _schema::MIGRATIONS.get(17) {
            step(&tx).unwrap();
        }
        tx.commit().unwrap();
        assert_eq!(kept(&conn), before);
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM conversation_messages", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn a_database_newer_than_this_build_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", schema_version() + 1)
            .unwrap();
        let refused = migrate(&mut conn, Path::new(":memory:"));
        assert!(matches!(refused, Err(StateDbError::Newer { .. })));
    }
}
