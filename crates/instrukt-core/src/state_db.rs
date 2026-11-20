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
    fn an_earlier_schema_with_a_retained_announcement_index_can_upgrade_again() {
        let path = Path::new(":memory:");
        for version in [9, 11, 12] {
            let mut conn = Connection::open_in_memory().unwrap();
            conn.pragma_update(None, "foreign_keys", true).unwrap();
            migrate(&mut conn, path).unwrap();

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
            conn.pragma_update(None, "user_version", version).unwrap();
            conn.execute(
                "INSERT INTO consumer_sources VALUES ('consumer', 'former', 'abandoned', 1)",
                [],
            )
            .unwrap();

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
            assert_eq!(indexes, 1);
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
                "SELECT attempted_at,latest_attempted_at,post_id,legacy_read_at,delivery_unknown_at FROM conversation_answers WHERE turn='turn-1'",
                [],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                )),
            )
            .unwrap();
        assert_eq!(
            attempt,
            ("2026-11-18T10:00:00Z".to_owned(), None, None, None, None)
        );
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
            .prepare("SELECT sender,reason,state,replied_at,failure FROM conversation_refusals ORDER BY rowid")
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
        // A sender has a notice beside a fixed reply; a notice keeps its
        // decision, and a fixed reply none.
        conn.execute("INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at,state) VALUES('acme','slack','C1/1','UOTHER','unavailable','2026-11-18T10:03:00Z','owed')", []).unwrap();
        assert!(
            conn.execute(
                "UPDATE conversation_refusals SET state='posting' WHERE reason='unavailable'",
                []
            )
            .is_err()
        );
        assert!(conn.execute("INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at) VALUES('acme','slack','C1/1','USTEVEN','unavailable','2026-11-18T10:03:00Z')", []).is_err());
        assert!(conn.execute("INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at,state) VALUES('acme','slack','C1/1','USTEVEN','not_own','2026-11-18T10:03:00Z','owed')", []).is_err());
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
        migrate(&mut conn, Path::new(":memory:")).unwrap();
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
    fn a_database_newer_than_this_build_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", schema_version() + 1)
            .unwrap();
        let refused = migrate(&mut conn, Path::new(":memory:"));
        assert!(matches!(refused, Err(StateDbError::Newer { .. })));
    }
}
