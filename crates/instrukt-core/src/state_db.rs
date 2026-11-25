//! The state database `~/.instrukt/state/instrukt.db`
//! (`project/instrukt/spec/feature/work/state-database`): one SQLite file in
//! WAL journal mode holding the local work record and the runner's state.
//! Retained history is not here: it lives in the runner's history store, and
//! moving it out of an earlier database is the only history use of this one.
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
//!
//! The machinery is described by [`Store`] — file, lock, migrations and the
//! name its errors use — so another context's store reuses the same opening,
//! protection, WAL, migration, refusal and snapshot reading; [`StateDb`] and
//! [`StateRead`] are the state database's users of it.

mod _schema;
mod _snapshot;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

pub use _schema::Migration;
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

/// The name the state database's errors use.
const STATE_DATABASE: &str = "the state database";

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
        "the state database at {} is at schema version {found}, newer than the {known} this instrukt knows; upgrade with `{}`",
        path.display(), crate::INSTALLER_COMMAND
    )]
    Newer {
        path: PathBuf,
        found: u32,
        known: u32,
    },
    #[error("{} is a symbolic link; link the instrukt home or state/ directory instead", path.display())]
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

/// A store described by [`Store`] could not be opened, migrated, read or
/// written. Its text names the store it was raised for.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{store} at {} failed: {source}", path.display())]
    Sqlite {
        store: &'static str,
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error(
        "{store} at {} is at schema version {found}, newer than the {known} this instrukt knows; upgrade with `{}`",
        path.display(), crate::INSTALLER_COMMAND
    )]
    Newer {
        store: &'static str,
        path: PathBuf,
        found: u32,
        known: u32,
    },
    #[error("{} is a symbolic link; link the instrukt home or state/ directory instead", path.display())]
    Symlink { path: PathBuf },
    #[error(transparent)]
    Lock(#[from] FsError),
    #[error("{store} at {} holds a value this build cannot read: {message}", path.display())]
    Invalid {
        store: &'static str,
        path: PathBuf,
        message: String,
    },
}

impl StoreError {
    fn sqlite<'a>(store: &'static str, path: &'a Path) -> impl Fn(rusqlite::Error) -> Self + 'a {
        move |source| Self::Sqlite {
            store,
            path: path.to_path_buf(),
            source,
        }
    }

    fn invalid(store: &'static str, path: &Path, message: impl Into<String>) -> Self {
        Self::Invalid {
            store,
            path: path.to_path_buf(),
            message: message.into(),
        }
    }
}

impl From<StoreError> for StateDbError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Sqlite { path, source, .. } => Self::Sqlite { path, source },
            StoreError::Newer {
                path, found, known, ..
            } => Self::Newer { path, found, known },
            StoreError::Symlink { path } => Self::Symlink { path },
            StoreError::Lock(error) => Self::Lock(error),
            ref invalid @ StoreError::Invalid { ref path, .. } => Self::Invalid {
                message: invalid.to_string(),
                path: path.clone(),
            },
        }
    }
}

/// One SQLite store reached through this module: its file, how its writers
/// take turns, the migrations it ships (its schema version is their count)
/// and the name its errors use.
#[derive(Clone)]
pub struct Store {
    pub name: &'static str,
    pub file: PathBuf,
    pub lock: StoreLock,
    pub migrations: &'static [Migration],
}

/// How a store's writers take turns.
#[derive(Debug, Clone)]
pub enum StoreLock {
    /// The process-re-entrant lock at this file ([`WriterLock`]): a write
    /// nested in one that already holds it shares the hold. Another process
    /// is waited for at most `LOCK_TIMEOUT`.
    Reentrant(PathBuf),
    /// A plain exclusive lock at this file, owned by the write handle alone
    /// and waited for at most the given time. A second handle waits for the
    /// first one, in this process as in another.
    Exclusive(PathBuf, Duration),
}

/// What reading a store found when it holds a schema.
pub enum Read {
    /// A connection at the store's current schema version.
    Current(Connection),
    /// The store is at an older schema version; it is brought forward
    /// through [`StoreDb::open`] before it is read.
    Behind,
}

impl Store {
    fn state(home: &Home) -> Self {
        Self {
            name: STATE_DATABASE,
            file: home.state_db_file(),
            lock: StoreLock::Reentrant(home.writer_lock_file()),
            migrations: _schema::MIGRATIONS,
        }
    }

    /// The schema version this build writes for the store.
    pub fn version(&self) -> u32 {
        self.migrations.len() as u32
    }

    /// Opens the store for reading without its lock; `None` when no file
    /// (or no schema) exists yet, which is empty state. With `snapshot`, an
    /// existing file is read without opening it on disk or creating
    /// sidecars, and older state is brought forward in memory only;
    /// otherwise older state is [`Read::Behind`]. A newer store is refused.
    pub fn read(&self, snapshot: bool) -> Result<Option<Read>, StoreError> {
        let path = &self.file;
        refuse_database_links(self.name, path)?;
        if !path.exists() {
            return Ok(None);
        }
        let mut conn = if snapshot {
            _snapshot::read(self.name, path)?
        } else {
            crate::fsx::open_database(
                path,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(StoreError::sqlite(self.name, path))?
        };
        configure(&conn, self.name, path)?;
        let found = stored_version(&conn, self.name, path)?;
        let known = self.version();
        if found == 0 {
            return Ok(None);
        }
        if found > known {
            return Err(StoreError::Newer {
                store: self.name,
                path: path.clone(),
                found,
                known,
            });
        }
        if snapshot {
            migrate_store(&mut conn, self, Applied::InMemory)?;
            conn.pragma_update(None, "query_only", true)
                .map_err(StoreError::sqlite(self.name, path))?;
        } else if found < known {
            return Ok(Some(Read::Behind));
        }
        Ok(Some(Read::Current(conn)))
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
        Ok(Self::acquire_file(
            STATE_DATABASE,
            &home.writer_lock_file(),
        )?)
    }

    fn acquire_file(store: &'static str, path: &Path) -> Result<Self, StoreError> {
        refuse_link(store, path)?;
        let mut held = HELD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let map = held.get_or_insert_with(HashMap::new);
        if let Some(guard) = map.get(path).and_then(Weak::upgrade) {
            return Ok(Self { _held: guard });
        }
        let guard = Arc::new(LockGuard::acquire(path, LOCK_TIMEOUT)?);
        map.insert(path.to_path_buf(), Arc::downgrade(&guard));
        Ok(Self { _held: guard })
    }
}

/// The lock a write handle holds for as long as it lives.
#[derive(Debug)]
enum Held {
    Reentrant(#[allow(dead_code, reason = "held only for its lock")] WriterLock),
    Exclusive(#[allow(dead_code, reason = "held only for its lock")] LockGuard),
}

/// A store's connection opened for writing, at its current schema version,
/// with the store's lock held for as long as it lives.
#[derive(Debug)]
pub struct StoreDb {
    conn: Connection,
    path: PathBuf,
    store: &'static str,
    _lock: Held,
}

impl StoreDb {
    /// Takes the store's lock and opens its file, creating it and applying
    /// every pending migration.
    pub fn open(store: &Store) -> Result<Self, StoreError> {
        let lock = match &store.lock {
            StoreLock::Reentrant(file) => {
                Held::Reentrant(WriterLock::acquire_file(store.name, file)?)
            }
            StoreLock::Exclusive(file, timeout) => {
                refuse_link(store.name, file)?;
                Held::Exclusive(LockGuard::acquire(file, *timeout)?)
            }
        };
        let path = store.file.clone();
        refuse_database_links(store.name, &path)?;
        if let Some(parent) = path.parent() {
            crate::fsx::create_dir_all(parent).map_err(|source| FsError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut conn = crate::fsx::open_database(&path, OpenFlags::default())
            .map_err(StoreError::sqlite(store.name, &path))?;
        let found = stored_version(&conn, store.name, &path)?;
        let known = store.version();
        if found > known {
            return Err(StoreError::Newer {
                store: store.name,
                path,
                found,
                known,
            });
        }
        // SQLite gives the files it keeps beside the database the database's
        // own permissions; protecting the database first protects them too.
        protect(&path)?;
        configure(&conn, store.name, &path)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(StoreError::sqlite(store.name, &path))?;
        protect(&path)?;
        migrate_store(&mut conn, store, Applied::OnDisk)?;
        Ok(Self {
            conn,
            path,
            store: store.name,
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
    pub fn transaction(&mut self) -> Result<Transaction<'_>, StoreError> {
        self.conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::sqlite(self.store, &self.path))
    }

    /// Runs `change` in one transaction and commits it; an error rolls
    /// everything back.
    pub fn write<T, E: From<StoreError>>(
        &mut self,
        change: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let store = self.store;
        let path = self.path.clone();
        let tx = self.transaction()?;
        let value = change(&tx)?;
        tx.commit().map_err(StoreError::sqlite(store, &path))?;
        Ok(value)
    }

    /// Reclaims the store's free pages under the lock this handle holds: a
    /// rewrite without them (`VACUUM`), then one passive checkpoint. Once
    /// the rewrite has committed no free page remains. The checkpoint
    /// shortens the file unless a reader holds a snapshot from before the
    /// rewrite; it waits for no reader, stopping short of the end of the log
    /// is not a failure, and the log is not truncated.
    pub fn reclaim(&mut self) -> Result<(), StoreError> {
        let sql = StoreError::sqlite(self.store, &self.path);
        self.conn.execute_batch("VACUUM").map_err(&sql)?;
        // (busy, frames in the log, frames checkpointed): a short or busy
        // checkpoint leaves the shortening to SQLite's next one.
        let _: (i64, i64, i64) = self
            .conn
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(&sql)?;
        Ok(())
    }
}

/// A connection opened for writing, at the current schema version, with the
/// writer lock held for as long as it lives.
#[derive(Debug)]
pub struct StateDb {
    db: StoreDb,
}

impl StateDb {
    /// Takes the writer lock and opens `home`'s database, creating it and
    /// applying every pending migration.
    pub fn open(home: &Home) -> Result<Self, StateDbError> {
        Ok(Self {
            db: StoreDb::open(&Store::state(home))?,
        })
    }

    pub fn path(&self) -> &Path {
        self.db.path()
    }

    pub fn conn(&self) -> &Connection {
        self.db.conn()
    }

    /// Begins one write transaction: everything written through it
    /// commits together or not at all.
    pub fn transaction(&mut self) -> Result<Transaction<'_>, StateDbError> {
        Ok(self.db.transaction()?)
    }

    /// Runs `change` in one transaction and commits it; an error rolls
    /// everything back.
    pub fn write<T, E: From<StateDbError>>(
        &mut self,
        change: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let path = self.path().to_path_buf();
        let tx = self.transaction()?;
        let value = change(&tx)?;
        tx.commit().map_err(StateDbError::sqlite(&path))?;
        Ok(value)
    }

    /// Drops, in one transaction, whichever history tables an earlier
    /// version left here (version 32); the history store calls it once it
    /// holds their rows.
    pub fn remove_history(&mut self) -> Result<(), StateDbError> {
        let path = self.path().to_path_buf();
        self.write(|tx| {
            tx.execute_batch(_schema::DROP_HISTORY)
                .map_err(StateDbError::sqlite(&path))
        })
    }

    /// Reclaims the database's free pages under the writer lock this handle
    /// holds, as [`StoreDb::reclaim`] does for any store.
    pub fn reclaim(&mut self) -> Result<(), StateDbError> {
        Ok(self.db.reclaim()?)
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
        let store = Store::state(home);
        match store.read(home.is_state_read_only())? {
            None => Ok(None),
            Some(Read::Current(conn)) => Ok(Some(Self {
                conn,
                path: store.file,
            })),
            Some(Read::Behind) => {
                drop(StateDb::open(home)?);
                Self::open(home)
            }
        }
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
fn protect(path: &Path) -> Result<(), StoreError> {
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

fn configure(conn: &Connection, store: &'static str, path: &Path) -> Result<(), StoreError> {
    let sql = StoreError::sqlite(store, path);
    conn.busy_timeout(BUSY_TIMEOUT).map_err(&sql)?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(&sql)?;
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(&sql)?;
    Ok(())
}

fn stored_version(conn: &Connection, store: &'static str, path: &Path) -> Result<u32, StoreError> {
    conn.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
        .map_err(StoreError::sqlite(store, path))
}

/// Where a connection's migrations land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Applied {
    /// The store's file moves to the new version.
    OnDisk,
    /// A snapshot moves in memory; the file is unchanged.
    InMemory,
}

/// Applies each of `store`'s migrations past the stored version, each in
/// its own transaction that sets `user_version` too. A store whose file
/// moved records each version it reached.
fn migrate_store(conn: &mut Connection, store: &Store, applied: Applied) -> Result<(), StoreError> {
    let path = &store.file;
    let sql = StoreError::sqlite(store.name, path);
    let found = stored_version(conn, store.name, path)?;
    let known = store.version();
    if found > known {
        return Err(StoreError::Newer {
            store: store.name,
            path: path.clone(),
            found,
            known,
        });
    }
    for (index, migration) in store.migrations.iter().enumerate().skip(found as usize) {
        let version = index as u32 + 1;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(&sql)?;
        match migration {
            Migration::Sql(script) => tx.execute_batch(script),
            Migration::Step(step) => step(&tx),
        }
        .map_err(&sql)?;
        tx.pragma_update(None, "user_version", version)
            .map_err(&sql)?;
        tx.commit().map_err(&sql)?;
        if applied == Applied::OnDisk {
            tracing::info!(
                event = "state_db_migrated",
                store = store.name,
                path = %path.display(),
                version,
                "a store moved to a new schema version"
            );
        }
    }
    Ok(())
}

/// The state database's stored schema version, for its migration tests.
#[cfg(test)]
fn user_version(conn: &Connection, path: &Path) -> Result<u32, StateDbError> {
    Ok(stored_version(conn, STATE_DATABASE, path)?)
}

/// Applies the state database's pending migrations, for its migration tests.
#[cfg(test)]
fn migrate(conn: &mut Connection, path: &Path) -> Result<(), StateDbError> {
    let store = Store {
        file: path.to_path_buf(),
        ..Store::state(&Home::at_user_home(PathBuf::from("/")))
    };
    Ok(migrate_store(conn, &store, Applied::OnDisk)?)
}

/// Inspect an existing SQLite store without opening a disk connection or
/// creating sidecars. The in-memory connection permits queries only.
pub fn inspect_file(path: &Path) -> Result<Option<Connection>, StateDbError> {
    refuse_database_links(STATE_DATABASE, path)?;
    if !path.exists() {
        return Ok(None);
    }
    let conn = _snapshot::read(STATE_DATABASE, path)?;
    conn.pragma_update(None, "query_only", true)
        .map_err(StateDbError::sqlite(path))?;
    Ok(Some(conn))
}

fn refuse_database_links(store: &'static str, path: &Path) -> Result<(), StoreError> {
    refuse_link(store, path)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        refuse_link(store, &_snapshot::sidecar(path, suffix))?;
    }
    Ok(())
}

/// Refuses a symbolic link at `path`, a file of `store`: a link would put
/// one store, or one lock, under two lock domains. A missing file is fine.
pub fn refuse_link(store: &'static str, path: &Path) -> Result<(), StoreError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(StoreError::Symlink { path: path.into() }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(StoreError::invalid(store, path, error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentSelection, Effort, HarnessKind};
    use crate::ids::{ProfileName, ProviderInstanceId};

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
    fn an_earlier_schema_with_a_retained_announcement_index_can_upgrade_again() {
        let path = Path::new(":memory:");
        for version in [9, 11, 12] {
            let mut conn = Connection::open_in_memory().unwrap();
            conn.pragma_update(None, "foreign_keys", true).unwrap();
            // The last schema retaining announcement intents and their index.
            for migration in _schema::MIGRATIONS.iter().take(29) {
                let tx = conn.transaction().unwrap();
                match migration {
                    _schema::Migration::Sql(script) => tx.execute_batch(script),
                    _schema::Migration::Step(step) => step(&tx),
                }
                .unwrap();
                tx.commit().unwrap();
            }

            // The historical reservation fixture restores the pre-link schema.
            // All three fixture versions restore poll deferrals without a cause.
            if version == 9 {
                conn.execute_batch(
                    "DROP TABLE conversation_relay_positions; DROP TABLE conversation_links;",
                )
                .unwrap();
            }
            conn.execute_batch("ALTER TABLE poll_deferrals DROP COLUMN cause;")
                .unwrap();
            conn.execute_batch("CREATE TABLE runtime_starts (runtime TEXT NOT NULL,semantic_key TEXT NOT NULL,project TEXT,kind TEXT NOT NULL,subject TEXT NOT NULL,checkout TEXT NOT NULL,brief TEXT NOT NULL,request TEXT NOT NULL,begun_at TEXT NOT NULL,reported INTEGER NOT NULL,resolved_at TEXT,resolved_by TEXT,resolution TEXT,state TEXT NOT NULL,PRIMARY KEY(runtime,semantic_key)); ALTER TABLE runtime_bindings DROP COLUMN generation; ALTER TABLE runtime_bindings DROP COLUMN start_key; ALTER TABLE runtime_bindings DROP COLUMN replaced_session; ALTER TABLE runtime_bindings DROP COLUMN legacy_modes; ALTER TABLE connected_conversations DROP COLUMN display_title; DROP TABLE runtime_generations;").unwrap();
            conn.pragma_update(None, "user_version", version).unwrap();
            conn.execute(
                "INSERT INTO consumer_sources VALUES ('consumer', 'former', 'abandoned', 1)",
                [],
            )
            .unwrap();

            let parked = serde_json::json!({"parked": {
                "destination": "instrukt:events:dlq", "entry_id": "1-0", "announced": false,
                "announcement_source": "former", "announcement_receipt": "old-receipt",
                "announcement_payload": "{}", "announcement_abandoned": true
            }});
            conn.execute("INSERT INTO consumer_outcomes VALUES('consumer','former','instrukt:events','1-0',?1,'2026-11-05T00:00:00Z')", [parked.to_string()]).unwrap();
            migrate(&mut conn, path).unwrap_or_else(|error| panic!("schema {version}: {error}"));
            assert_eq!(user_version(&conn, path).unwrap(), schema_version());
            let retained: bool = conn.query_row(
                "SELECT prepared FROM consumer_sources WHERE consumer = 'consumer' AND source = 'former' AND state = 'abandoned'",
                [],
                |row| row.get(0),
            ).unwrap();
            assert!(retained);
            let indexes: u32 = conn.query_row(
                "SELECT count(*) FROM pragma_index_list('consumer_outcomes') WHERE name = 'consumer_open_announcements' AND partial = 1",
                [],
                |row| row.get(0),
            ).unwrap();
            assert_eq!(indexes, 0);
            let outcome: String = conn
                .query_row(
                    "SELECT outcome FROM consumer_outcomes WHERE consumer='consumer'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&outcome).unwrap(),
                serde_json::json!({"parked": {
                    "destination": "instrukt:events:dlq", "entry_id": "1-0", "announced": false
                }})
            );
        }
    }

    #[test]
    fn version_22_admits_an_abandoned_source_and_keeps_every_stored_source() {
        let mut conn = Connection::open_in_memory().unwrap();
        for migration in _schema::MIGRATIONS.iter().take(21) {
            let tx = conn.transaction().unwrap();
            match migration {
                _schema::Migration::Sql(script) => tx.execute_batch(script),
                _schema::Migration::Step(step) => step(&tx),
            }
            .unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", 21).unwrap();
        // The shape before version 22 refuses the new state.
        assert!(conn.execute("INSERT INTO consumer_sources(consumer,source,state,prepared) VALUES('c','gone','abandoned',1)", []).is_err());
        conn.execute_batch("INSERT INTO consumer_sources(consumer,source,state,prepared) VALUES('procedure-activation','cluster','retiring',1);INSERT INTO consumer_sources(consumer,source,state,prepared) VALUES('procedure-activation','local','active',0);INSERT INTO consumer_sources(consumer,source,state,prepared) VALUES('procedure-activation','old','retired',1);").unwrap();
        migrate(&mut conn, Path::new(":memory:")).unwrap();
        conn.execute(
            "UPDATE consumer_sources SET state='abandoned' WHERE source='cluster'",
            [],
        )
        .unwrap();
        let kept: Vec<(String, String, bool)> = conn
            .prepare("SELECT source,state,prepared FROM consumer_sources ORDER BY source")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        insta::assert_debug_snapshot!(kept, @r#"
        [
            (
                "cluster",
                "abandoned",
                true,
            ),
            (
                "local",
                "active",
                false,
            ),
            (
                "old",
                "retired",
                true,
            ),
        ]
        "#);
        assert!(
            conn.execute(
                "UPDATE consumer_sources SET state='unknown' WHERE source='local'",
                []
            )
            .is_err()
        );
    }

    #[test]
    fn version_32_keeps_every_history_table_while_one_holds_a_row_and_drops_them_when_all_are_empty()
     {
        let path = Path::new(":memory:");
        let at_31 = || {
            let mut conn = Connection::open_in_memory().unwrap();
            for migration in _schema::MIGRATIONS.iter().take(31) {
                let tx = conn.transaction().unwrap();
                match migration {
                    _schema::Migration::Sql(script) => tx.execute_batch(script),
                    _schema::Migration::Step(step) => step(&tx),
                }
                .unwrap();
                tx.commit().unwrap();
            }
            conn.pragma_update(None, "user_version", 31).unwrap();
            conn
        };
        let history = |conn: &Connection| -> [bool; 5] {
            [
                "history_conversations",
                "history_fts",
                "history_projection_records",
                "history_source_positions",
                "history_sources",
            ]
            .map(|table| {
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    [table],
                    |row| row.get(0),
                )
                .unwrap()
            })
        };
        // One retained row keeps every history table, the row included.
        let mut retained = at_31();
        retained
            .execute(
                "INSERT INTO history_sources(source,position,imported) VALUES('teleclaude:kept',1,1)",
                [],
            )
            .unwrap();
        migrate(&mut retained, path).unwrap();
        assert_eq!(history(&retained), [true; 5]);
        let kept: (i64, bool) = retained
            .query_row(
                "SELECT position,imported FROM history_sources WHERE source='teleclaude:kept'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kept, (1, true));
        // Tables that hold nothing are removed, the full-text index included.
        let mut empty = at_31();
        migrate(&mut empty, path).unwrap();
        assert_eq!(history(&empty), [false; 5]);
    }

    #[test]
    fn version_36_carries_a_stored_antigravity_effort_as_thought_level_and_keeps_other_reservations()
     {
        let path = Path::new(":memory:");
        let mut conn = Connection::open_in_memory().unwrap();
        let at_35 = Store {
            file: path.to_path_buf(),
            migrations: &_schema::MIGRATIONS[..35],
            ..Store::state(&Home::at_user_home(PathBuf::from("/")))
        };
        migrate_store(&mut conn, &at_35, Applied::OnDisk).unwrap();
        assert_eq!(user_version(&conn, path).unwrap(), 35);
        let request = |key: &str, selection: serde_json::Value| {
            serde_json::json!({
                "preferences": [{"agent": "antigravity", "tier": "strong"}],
                "selection": selection,
                "title": format!("standing · {key}"),
                "reservation": format!("token-{key}"),
                "key": key,
                "brief": "retained-input",
                "checkout": "/checkout",
                "start_allowed": true,
            })
        };
        let stored = [
            request(
                "antigravity-effort",
                serde_json::json!({"profile": "antigravity", "kind": "antigravity", "instance": "antigravity", "model": "gemini-pro-agent", "effort": "high", "options": {"thinking_level": "high"}}),
            ),
            request(
                "codex-effort",
                serde_json::json!({"profile": "codex", "kind": "codex", "instance": "codex", "model": "gpt-6-astra", "effort": "high", "options": {"reasoningEffort": "high", "serviceTier": "flex"}}),
            ),
            request(
                "antigravity-plain",
                serde_json::json!({"profile": "antigravity", "kind": "antigravity", "instance": "antigravity", "model": "gemini-pro-agent", "effort": null, "options": {}}),
            ),
            // An ACP agent may carry its own scalar `thinking_level`; only Antigravity's is renamed.
            request(
                "acp-thinking-level",
                serde_json::json!({"profile": "acp", "kind": "acpRegistry", "instance": "acp", "model": "acp-model", "effort": null, "options": {"thinking_level": "high"}}),
            ),
        ];
        for row in &stored {
            conn.execute(
                "INSERT INTO manual_standing_reservations(responsibility,token,caller,home,declaration,request) VALUES (?1,?2,'caller','/home','{}',?3)",
                rusqlite::params![row["key"].as_str().unwrap(), row["reservation"].as_str().unwrap(), row.to_string()],
            )
            .unwrap();
        }
        migrate(&mut conn, path).unwrap();
        assert_eq!(user_version(&conn, path).unwrap(), schema_version());
        let loaded = |key: &str| -> serde_json::Value {
            let text: String = conn
                .query_row(
                    "SELECT request FROM manual_standing_reservations WHERE responsibility=?1",
                    [key],
                    |row| row.get(0),
                )
                .unwrap();
            serde_json::from_str(&text).unwrap()
        };
        let selection = |value: &serde_json::Value| -> AgentSelection {
            serde_json::from_value(value["selection"].clone()).unwrap()
        };
        let without_selection = |value: &serde_json::Value| {
            let mut rest = value.clone();
            rest.as_object_mut().unwrap().remove("selection");
            rest
        };
        let converted = loaded("antigravity-effort");
        assert_eq!(
            selection(&converted),
            AgentSelection {
                profile: ProfileName::parse("antigravity").unwrap(),
                kind: HarnessKind::Antigravity,
                instance: ProviderInstanceId::parse("antigravity").unwrap(),
                model: "gemini-pro-agent".into(),
                effort: Some(Effort::High),
                options: [("thought_level".to_string(), serde_json::json!("high"))].into(),
            }
        );
        assert_eq!(without_selection(&converted), without_selection(&stored[0]));
        for (key, original) in [
            ("codex-effort", &stored[1]),
            ("antigravity-plain", &stored[2]),
            ("acp-thinking-level", &stored[3]),
        ] {
            let kept = loaded(key);
            assert_eq!(selection(&kept), selection(original));
            assert_eq!(without_selection(&kept), without_selection(original));
        }
    }

    #[test]
    fn version_20_records_when_an_answer_piece_was_posted_and_keeps_stored_pieces() {
        let mut conn = Connection::open_in_memory().unwrap();
        for migration in _schema::MIGRATIONS.iter().take(19) {
            let tx = conn.transaction().unwrap();
            match migration {
                _schema::Migration::Sql(script) => tx.execute_batch(script),
                _schema::Migration::Step(step) => step(&tx),
            }
            .unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", 19).unwrap();
        conn.execute_batch("INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,posted_at) VALUES('acme','slack','C1/1','turn-1',0,'On Friday.','2026-11-18T10:00:00Z');INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text) VALUES('acme','slack','C1/2','turn-2',0,'Ada reviews it.');").unwrap();
        migrate(&mut conn, Path::new(":memory:")).unwrap();
        // Stored pieces keep their state and record no attempt.
        let kept: Vec<(String, Option<String>, Option<String>)> = conn
            .prepare("SELECT conversation,posted_at,attempted_at FROM conversation_answers ORDER BY conversation")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            kept,
            [
                (
                    "C1/1".to_owned(),
                    Some("2026-11-18T10:00:00Z".to_owned()),
                    None
                ),
                ("C1/2".to_owned(), None, None),
            ]
        );
        conn.execute("UPDATE conversation_answers SET attempted_at='2026-11-18T10:01:00Z' WHERE conversation='C1/2'", []).unwrap();
    }

    #[test]
    fn delivery_receipts_upgrade_the_previous_schema_and_can_be_reapplied() {
        let path = Path::new(":memory:");
        let mut conn = Connection::open_in_memory().unwrap();
        // Use main's real version 24, even if later migrations are added.
        let before = 24;
        for migration in _schema::MIGRATIONS
            .iter()
            .take(usize::try_from(before).unwrap())
        {
            let tx = conn.transaction().unwrap();
            match migration {
                _schema::Migration::Sql(script) => tx.execute_batch(script),
                _schema::Migration::Step(step) => step(&tx),
            }
            .unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", before).unwrap();
        conn.execute_batch(
            "INSERT INTO conversation_messages(company,connector,conversation,message,written_us,written_at,arrived_at,state,text)
             VALUES ('acme','slack','C1/1','1.000001',1,'2026-11-18T10:00:00Z','2026-11-18T10:00:00Z','sent','Delivered'),
                    ('acme','slack','C1/1','1.000002',2,'2026-11-18T10:00:01Z','2026-11-18T10:00:01Z','pending','Waiting');",
        ).unwrap();
        migrate(&mut conn, path).unwrap();
        conn.execute(
            r#"UPDATE conversation_messages SET context_delivery_ids='["1.000000"]', send_unknown_at='2026-11-18T10:05:00Z', dispatch_command_id='retained-command', dispatch_write_at='2026-11-18T10:00:00Z' WHERE message='1.000001'"#,
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO conversation_context_receipts(company,connector,conversation,message,delivery,retired_at)
             VALUES ('acme','slack','C1/1','1.000000','1.000001','2026-11-18T10:00:00Z')",
            [],
        ).unwrap();
        conn.execute_batch("CREATE TABLE runtime_starts (runtime TEXT NOT NULL,semantic_key TEXT NOT NULL,project TEXT,kind TEXT NOT NULL,subject TEXT NOT NULL,checkout TEXT NOT NULL,brief TEXT NOT NULL,request TEXT NOT NULL,begun_at TEXT NOT NULL,reported INTEGER NOT NULL,resolved_at TEXT,resolved_by TEXT,resolution TEXT,state TEXT NOT NULL,PRIMARY KEY(runtime,semantic_key)); ALTER TABLE runtime_bindings DROP COLUMN generation; ALTER TABLE runtime_bindings DROP COLUMN start_key; ALTER TABLE runtime_bindings DROP COLUMN replaced_session; ALTER TABLE runtime_bindings DROP COLUMN legacy_modes; ALTER TABLE connected_conversations DROP COLUMN display_title; DROP TABLE runtime_generations;").unwrap();
        conn.pragma_update(None, "user_version", before).unwrap();
        migrate(&mut conn, path).unwrap();

        assert_eq!(user_version(&conn, path).unwrap(), schema_version());
        let receipts: Vec<(String, Option<String>)> = conn
            .prepare(
                "SELECT message,retired_at FROM conversation_context_receipts
             WHERE company='acme' AND connector='slack' AND conversation='C1/1' ORDER BY message",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            receipts,
            [
                (
                    "1.000000".to_owned(),
                    Some("2026-11-18T10:00:00Z".to_owned())
                ),
                ("1.000001".to_owned(), None),
            ]
        );
        let delivery: (String, String, String, String) = conn
            .query_row(
                "SELECT context_delivery_ids,send_unknown_at,dispatch_command_id,dispatch_write_at FROM conversation_messages WHERE message='1.000001'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            delivery,
            (
                r#"["1.000000"]"#.to_owned(),
                "2026-11-18T10:05:00Z".to_owned(),
                "retained-command".to_owned(),
                "2026-11-18T10:00:00Z".to_owned()
            )
        );
    }

    #[test]
    fn version_23_keeps_every_stored_reply_as_a_not_own_one() {
        // The shape before version 23.
        let before = 22;
        let mut conn = Connection::open_in_memory().unwrap();
        for migration in _schema::MIGRATIONS.iter().take(before) {
            let tx = conn.transaction().unwrap();
            match migration {
                _schema::Migration::Sql(script) => tx.execute_batch(script),
                _schema::Migration::Step(step) => step(&tx),
            }
            .unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", u32::try_from(before).unwrap())
            .unwrap();
        conn.execute("INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,attempted_at) VALUES('acme','slack','C1/1','turn-1',0,'Kept answer','2026-11-18T10:00:00Z')", []).unwrap();
        conn.execute_batch(
            "INSERT INTO conversation_messages(company,connector,conversation,message,written_us,written_at,arrived_at,state,text,turn,unavailable_since,turn_update)
             VALUES ('acme','slack','C1/1','pending-piece',1,'2026-11-18T10:00:00Z','2026-11-18T10:00:00Z','answered','First','turn-1','2026-11-18T10:00:00Z','Working'),
                    ('acme','slack','C1/1','posted-piece',2,'2026-11-18T10:00:01Z','2026-11-18T10:00:01Z','answered','Second','turn-2','2026-11-18T10:00:01Z','Working');
             INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,posted_at)
             VALUES ('acme','slack','C1/1','turn-2',0,'Posted answer','2026-11-18T10:00:02Z');",
        ).unwrap();
        conn.execute_batch("INSERT INTO conversation_refusals(company,connector,conversation,sender,received_at,replied_at) VALUES('acme','slack','C1/1','UOTHER','2026-11-18T10:00:00Z','2026-11-18T10:00:02Z');INSERT INTO conversation_refusals(company,connector,conversation,sender,received_at,failure) VALUES('acme','slack','D1/2','UGUEST','2026-11-18T10:01:00Z','Slack chat.postMessage: timeout');").unwrap();
        migrate(&mut conn, Path::new(":memory:")).unwrap();
        let attempt = conn
            .query_row(
                "SELECT attempted_at,provider_post_id FROM conversation_answers WHERE turn='turn-1'",
                [],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                )),
            )
            .unwrap();
        assert_eq!(attempt, ("2026-11-18T10:00:00Z".to_owned(), None));
        let positions: Vec<(i64, i64)> = conn
            .prepare("SELECT m.admission_position,a.admission_position FROM conversation_messages m JOIN conversation_answers a USING(company,connector,conversation,turn) ORDER BY m.admission_position")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(positions, [(1, 1), (2, 2)]);
        let intervals: Vec<(Option<String>, Option<String>)> = conn
            .prepare("SELECT unavailable_since,turn_update FROM conversation_messages ORDER BY written_us")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            intervals,
            [
                (
                    Some("2026-11-18T10:00:00Z".to_owned()),
                    Some("Working".to_owned())
                ),
                (None, None)
            ]
        );
        type Row = (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let kept: Vec<Row> = conn
            .prepare("SELECT sender,reason,attempted_at,replied_at,failure FROM conversation_refusals ORDER BY rowid")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        insta::assert_debug_snapshot!(kept, @r#"
        [
            (
                "UOTHER",
                "not_own",
                None,
                Some(
                    "2026-11-18T10:00:02Z",
                ),
                None,
            ),
            (
                "UGUEST",
                "not_own",
                None,
                None,
                Some(
                    "Slack chat.postMessage: timeout",
                ),
            ),
        ]
        "#);
        // The widened state retains an unserved message without changing its
        // position in the notice's coverage.
        conn.execute(
            "UPDATE conversation_messages SET state='failed' WHERE message='pending-piece'",
            [],
        )
        .unwrap();
        let failed: (String, i64) = conn
            .query_row(
                "SELECT state,admission_position FROM conversation_messages WHERE message='pending-piece'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(failed, ("failed".to_owned(), 1));
        // A sender retains the unavailable attempt beside their fixed reply.
        conn.execute("INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at,attempted_at) VALUES('acme','slack','C1/1','UOTHER','unavailable','2026-11-18T10:03:00Z','2026-11-18T10:04:00Z')", []).unwrap();
    }

    #[test]
    fn version_24_keeps_failed_messages_and_their_notice_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        for migration in _schema::MIGRATIONS.iter().take(23) {
            let tx = conn.transaction().unwrap();
            match migration {
                _schema::Migration::Sql(script) => tx.execute_batch(script),
                _schema::Migration::Step(step) => step(&tx),
            }
            .unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", 23).unwrap();
        conn.execute_batch(
            "INSERT INTO conversation_messages(company,connector,conversation,message,written_us,written_at,arrived_at,
              state,text,boundary,content,attempted_at,admission_position,unavailable_since,superseded_at)
             VALUES ('acme','slack','C1/1','failed',1,'2026-11-18T10:00:00Z','2026-11-18T10:00:00Z',
                       'failed','Question','boundary','Framed question','old-accepted-send',1,'still-unavailable',NULL),
                    ('acme','slack','C1/1','superseded',2,'2026-11-18T10:00:00Z','2026-11-18T10:00:00Z',
                       'failed','Question','superseded-boundary','Framed question','historical-send',2,NULL,'retained-supersession'),
                    ('acme','slack','C1/1','sent',3,'2026-11-18T10:00:00Z','2026-11-18T10:00:00Z',
                       'sent','Question','sent-boundary','Framed question','unconfirmed-send',3,NULL,NULL);",
        ).unwrap();
        // Retain the version-24 initialization, then carry an old moved-source finding forward.
        let tx = conn.transaction().unwrap();
        match &_schema::MIGRATIONS[23] {
            _schema::Migration::Sql(script) => tx.execute_batch(script),
            _schema::Migration::Step(step) => step(&tx),
        }
        .unwrap();
        tx.pragma_update(None, "user_version", 24).unwrap();
        tx.commit().unwrap();
        conn.execute_batch("INSERT INTO connected_conversations(company,connector,conversation,semantic_key,session,source_identity,position_json)
            VALUES('acme','slack','C1/1','binding','t3:bound','adopted-source','{}');
            UPDATE conversation_messages SET source_uncertain=1,failure='retained finding' WHERE message='sent';").unwrap();
        migrate(&mut conn, Path::new(":memory:")).unwrap();
        let uncertain: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('conversation_messages') WHERE name='source_uncertain'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(uncertain, 0);
        let binding: (Option<String>, String, String, String) = conn.query_row(
            "SELECT c.source_identity,c.session,c.position_json,m.failure FROM connected_conversations c JOIN conversation_messages m USING(company,connector,conversation) WHERE m.message='sent'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap();
        assert_eq!(binding, (None, "t3:bound".into(), "{}".into(), "retained finding; native input unmatched after an earlier source move; original source identity unavailable".into()));
        let retained = conn
            .prepare(
                "SELECT state,boundary,admission_position,unavailable_since,superseded_at,attempted_at,retry_at,failed_turns
                 FROM conversation_messages ORDER BY admission_position",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            retained,
            [
                (
                    "failed".into(),
                    "boundary".into(),
                    1,
                    Some("still-unavailable".into()),
                    None,
                    None,
                    None,
                    0
                ),
                (
                    "failed".into(),
                    "superseded-boundary".into(),
                    2,
                    None,
                    Some("retained-supersession".into()),
                    Some("historical-send".into()),
                    None,
                    0
                ),
                (
                    "sent".into(),
                    "sent-boundary".into(),
                    3,
                    None,
                    None,
                    Some("unconfirmed-send".into()),
                    None,
                    0
                ),
            ]
        );
    }

    #[test]
    fn version_21_keeps_stored_messages_with_nothing_open() {
        let mut conn = Connection::open_in_memory().unwrap();
        for migration in _schema::MIGRATIONS.iter().take(20) {
            let tx = conn.transaction().unwrap();
            match migration {
                _schema::Migration::Sql(script) => tx.execute_batch(script),
                _schema::Migration::Step(step) => step(&tx),
            }
            .unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", 20).unwrap();
        conn.execute_batch("INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,written_us,written_at,arrived_at,state,text) VALUES('acme','slack','C1/1','1.1','USTEVEN','Steven',1,'2026-11-18T10:00:00Z','2026-11-18T10:00:01Z','sent','When do we ship?');").unwrap();
        migrate(&mut conn, Path::new(":memory:")).unwrap();
        // A stored message keeps its stage, with no admission time, no open
        // interval, no update and no turn end.
        type Row = (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let message: Row = conn
            .query_row(
                "SELECT state,admitted_at,unavailable_since,turn_update,turn_ended_at FROM conversation_messages",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(message, ("sent".to_owned(), None, None, None, None));
    }

    #[test]
    fn version_19_adds_the_runners_own_state_and_keeps_the_relays() {
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
        let seeded = "T3 is unreachable";
        conn.execute("INSERT INTO conversation_relay(id,failure,updated_at) VALUES(1,?1,'2026-11-18T10:00:00Z')", [seeded]).unwrap();
        migrate(&mut conn, Path::new(":memory:")).unwrap();
        // The relay's recorded failure is kept as it was stored.
        let kept: i64 = conn
            .query_row(
                "SELECT count(*) FROM conversation_relay WHERE id=1 AND failure=?1",
                [seeded],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1);
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
        conn.execute(
            "INSERT INTO runner_admission(id,at,outcome) VALUES(1,'2026-11-18T10:00:00Z','kept')",
            [],
        )
        .unwrap();
        assert!(
            conn.execute("INSERT INTO runner_admission(id,at,outcome) VALUES(1,'2026-11-18T10:01:00Z','lost') ON CONFLICT(id) DO UPDATE SET outcome=excluded.outcome", [])
                .is_err(),
            "an outcome is replaced or kept"
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
    fn version_34_upgrades_v33_conversations_and_refusals_without_loss() {
        let mut conn = Connection::open_in_memory().unwrap();
        for migration in _schema::MIGRATIONS.iter().take(33) {
            let tx = conn.transaction().unwrap();
            match migration {
                _schema::Migration::Sql(script) => tx.execute_batch(script),
                _schema::Migration::Step(step) => step(&tx),
            }
            .unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", 33).unwrap();
        conn.execute_batch(
            "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,opening,session,source_identity,display_title)
             VALUES ('acme','slack','C1/1.1','instrukt:conversation:acme:slack:C1/1.1','1.1','t3:sess-1','src-1','acme — #general');
             INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,sender_email,written_us,written_at,arrived_at,state,text,admission_position)
             VALUES ('acme','slack','C1/1.1','1.1','U1','Alex','alex@example.com',1,'2026-11-20T10:00:00Z','2026-11-20T10:00:01Z','answered','Hello',1);
             INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,posted_at,admission_position)
             VALUES ('acme','slack','C1/1.1','t1',0,'Hi there','2026-11-20T10:00:02Z',1);
             INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at,replied_at)
             VALUES ('acme','slack','C1/1.1','UGUEST','not_own','2026-11-20T10:00:00Z','2026-11-20T10:00:01Z'),
                    ('acme','slack','C1/1.1','U1','unavailable','2026-11-20T10:01:00Z',NULL);",
        )
        .unwrap();

        migrate(&mut conn, Path::new(":memory:")).unwrap();
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, schema_version());

        let conv: (String, String, String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT role, session_scope, reply_mode, opening_conversation, display_title FROM connected_conversations WHERE conversation='C1/1.1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            conv,
            (
                "company-agent".to_owned(),
                "conversation".to_owned(),
                "turn_output".to_owned(),
                None,
                Some("acme — #general".to_owned()),
            )
        );

        let display_and_thread: (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT conversation_display, thread FROM conversation_messages WHERE message='1.1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(display_and_thread, (None, None));

        let refusals: Vec<(String, String, Option<String>)> = conn
            .prepare("SELECT sender, reason, replied_at FROM conversation_refusals ORDER BY sender, reason")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            refusals,
            vec![
                ("U1".into(), "unavailable".into(), None),
                (
                    "UGUEST".into(),
                    "not_own".into(),
                    Some("2026-11-20T10:00:01Z".into())
                ),
            ]
        );

        // `not_admin` is now admitted by the CHECK constraint; invalid reasons are rejected.
        conn.execute(
            "INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at) VALUES('acme','slack','C1/1.1','UNONADMIN','not_admin','2026-11-20T10:02:00Z')",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at) VALUES('acme','slack','C1/1.1','UBAD','bogus','2026-11-20T10:02:00Z')",
                [],
            )
            .is_err()
        );

        // Re-running v34 and v35 is idempotent and preserves every row.
        let tx = conn.transaction().unwrap();
        if let Some(_schema::Migration::Step(step)) = _schema::MIGRATIONS.get(33) {
            step(&tx).unwrap();
        }
        if let Some(_schema::Migration::Step(step)) = _schema::MIGRATIONS.get(34) {
            step(&tx).unwrap();
        }
        tx.commit().unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM conversation_refusals", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 3);
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
