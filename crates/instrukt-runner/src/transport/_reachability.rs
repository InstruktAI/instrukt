//! Dedicated background SQLite store for per-Redis last observed reachability
//! ([`Home::redis_reachability_db_file`] under
//! [`Home::redis_reachability_lock_file`]; `project/instrukt/spec/domain/runner`,
//! "Runner background state").

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use instrukt_core::home::Home;
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, params};
use instrukt_core::state_db::{
    Migration, StateDb, StateDbError, StateRead, Store, StoreDb, StoreLock,
};

use crate::state::{self, StateError};

/// Bounded wait for [`Home::redis_reachability_lock_file`].
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

const V1: &str = r#"
CREATE TABLE redis_reachability (
    source TEXT PRIMARY KEY,
    reachable INTEGER NOT NULL
);
CREATE TABLE reachability_move (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    done INTEGER NOT NULL
);
"#;

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1)];

pub(crate) fn store(home: &Home) -> Store {
    Store {
        name: "the Redis reachability store",
        file: home.redis_reachability_db_file(),
        lock: StoreLock::Exclusive(home.redis_reachability_lock_file(), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

fn move_done(conn: &Connection, path: &Path) -> Result<bool, StateError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM reachability_move WHERE id = 1 AND done = 1)",
        [],
        |row| row.get(0),
    )
    .map_err(state::sql(path))
}

fn mark_move_done(db: &mut StoreDb) -> Result<(), StateError> {
    db.write(|tx| {
        let path = state::db_path(tx);
        tx.execute(
            "INSERT INTO reachability_move(id, done) VALUES (1, 1) ON CONFLICT(id) DO UPDATE SET done = 1",
            [],
        )
        .map_err(state::sql(&path))?;
        Ok::<_, StateError>(())
    })
}

fn legacy_table_exists(conn: &Connection, path: &Path) -> Result<bool, StateError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'redis_reachability')",
        [],
        |row| row.get(0),
    )
    .map_err(state::sql(path))
}

fn read_source(conn: &Connection, path: &Path, source: &str) -> Result<Option<bool>, StateError> {
    conn.query_row(
        "SELECT reachable FROM redis_reachability WHERE source = ?1",
        [source],
        |row| row.get(0),
    )
    .optional()
    .map_err(state::sql(path))
}

fn all_sources(conn: &Connection, path: &Path) -> Result<Vec<String>, StateError> {
    let mut statement = conn
        .prepare("SELECT source FROM redis_reachability")
        .map_err(state::sql(path))?;
    let rows = statement
        .query_map([], |row| row.get(0))
        .map_err(state::sql(path))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(state::sql(path))
}

/// Transfers any legacy `redis_reachability` rows from the state database into
/// `db`, removes the legacy table from the state database, and marks the move
/// complete. Never acquires [`Home::writer_lock_file`] or creates the state
/// database when no legacy table exists.
fn move_in(home: &Home, db: &mut StoreDb) -> Result<(), StateError> {
    if move_done(db.conn(), db.path())? {
        return Ok(());
    }
    let legacy = match StateRead::open(home)? {
        Some(read) => {
            if legacy_table_exists(read.conn(), read.path())? {
                let mut statement = read
                    .conn()
                    .prepare("SELECT source, reachable FROM redis_reachability")
                    .map_err(state::sql(read.path()))?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
                    })
                    .map_err(state::sql(read.path()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(state::sql(read.path()))?;
                Some(rows)
            } else {
                None
            }
        }
        None => None,
    };
    if let Some(rows) = legacy {
        db.write(|tx| {
            let path = state::db_path(tx);
            for (source, reachable) in &rows {
                tx.execute(
                    "INSERT OR IGNORE INTO redis_reachability(source, reachable) VALUES (?1, ?2)",
                    params![source, reachable],
                )
                .map_err(state::sql(&path))?;
            }
            Ok::<_, StateError>(())
        })?;
        let mut state_db = StateDb::open(home)?;
        state_db.remove_redis_reachability()?;
    }
    mark_move_done(db)
}

/// Completes any pending transfer of legacy `redis_reachability` rows out of
/// the state database. Lock-free no-op when read-only, already moved, or when
/// no legacy table exists.
pub(crate) fn ensure_migrated(home: &Home) -> Result<(), StateError> {
    if home.is_inspection() {
        return Ok(());
    }
    if let Some(conn) = store(home).read().map_err(StateDbError::from)?
        && move_done(&conn, &home.redis_reachability_db_file())?
    {
        return Ok(());
    }
    let has_legacy = match StateRead::open(home)? {
        Some(read) => legacy_table_exists(read.conn(), read.path())?,
        None => false,
    };
    if !has_legacy {
        return Ok(());
    }
    let mut db = StoreDb::open(&store(home)).map_err(StateDbError::from)?;
    move_in(home, &mut db)
}

/// Records `source`'s latest reachability observation for a background writer.
/// Steady-state observations perform a lock-free read and write nothing;
/// transitions acquire `in_process` and [`Home::redis_reachability_lock_file`],
/// re-check under the lock, persist the new state, and emit one transition log.
pub(crate) fn record(home: &Home, source: &str, reachable: bool, in_process: &Mutex<()>) {
    if home.is_inspection() {
        return;
    }
    if let Ok(Some(conn)) = store(home).read() {
        let path = home.redis_reachability_db_file();
        if move_done(&conn, &path).unwrap_or(false)
            && read_source(&conn, &path, source).ok().flatten() == Some(reachable)
        {
            return;
        }
    }
    let _guard = in_process.lock().unwrap_or_else(PoisonError::into_inner);
    let result: Result<(), StateError> = (|| {
        let mut db = StoreDb::open(&store(home)).map_err(StateDbError::from)?;
        move_in(home, &mut db)?;
        if read_source(db.conn(), db.path(), source)? == Some(reachable) {
            return Ok(());
        }
        db.write(|tx| {
            let path = state::db_path(tx);
            tx.execute(
                "INSERT INTO redis_reachability(source, reachable) VALUES (?1, ?2) ON CONFLICT(source) DO UPDATE SET reachable = excluded.reachable",
                params![source, reachable],
            )
            .map_err(state::sql(&path))?;
            Ok::<_, StateError>(())
        })?;
        if reachable {
            tracing::info!(event = "redis_reachable", source = %source);
        } else {
            tracing::warn!(event = "redis_unreachable", source = %source);
        }
        Ok(())
    })();
    if let Err(error) = result {
        tracing::warn!(event = "redis_reachability_not_recorded", error = %error);
    }
}

/// Prunes rows for logical Redis endpoints no longer present in `active_sources`
/// while preserving transition state for every active endpoint. Lock-free no-op
/// when read-only, when the store does not exist yet, or when no obsolete row
/// is present.
pub(crate) fn retire_unconfigured(home: &Home, active_sources: &[&str]) -> Result<(), StateError> {
    if home.is_inspection() {
        return Ok(());
    }
    let Some(conn) = store(home).read().map_err(StateDbError::from)? else {
        return Ok(());
    };
    let path = home.redis_reachability_db_file();
    let stored = all_sources(&conn, &path)?;
    if stored
        .iter()
        .all(|source| active_sources.contains(&source.as_str()))
    {
        return Ok(());
    }
    drop(conn);
    let mut db = StoreDb::open(&store(home)).map_err(StateDbError::from)?;
    db.write(|tx| {
        let path = state::db_path(tx);
        let current = all_sources(tx, &path)?;
        for source in current {
            if !active_sources.contains(&source.as_str()) {
                tx.execute(
                    "DELETE FROM redis_reachability WHERE source = ?1",
                    [&source],
                )
                .map_err(state::sql(&path))?;
            }
        }
        Ok::<_, StateError>(())
    })
}
