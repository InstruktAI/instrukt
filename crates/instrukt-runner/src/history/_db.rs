//! The history store `state/history.db`
//! (`project/instrukt/spec/feature/runner/history`, History store): its own
//! SQLite file on the state database's machinery, written only under the
//! history lock, the store's write-isolation unit. Opening it for writing
//! first completes moving the history earlier builds kept in the state
//! database; once that move is recorded complete no history read or write
//! opens the state database or takes its writer lock.
use super::HistoryError;
use instrukt_core::{
    home::Home,
    state_db::{
        Migration, Read, StateDb, StateRead, Store, StoreDb, StoreLock,
        rusqlite::{Connection, OptionalExtension, Transaction, params},
    },
};
use std::time::{Duration, Instant};

/// How long a history writer waits for another one holding the history lock.
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// The most rows one batch of the move copies.
const BATCH_ROWS: usize = 128;

/// The most retained bytes one batch of the move copies; a batch always
/// takes at least one row.
const BATCH_BYTES: usize = 32 * 1024 * 1024;

/// Version 1: the final shape earlier builds kept in the state database,
/// with a TeleClaude position that is absent until a first batch commits,
/// and the record of how far the move of that history has progressed.
const V1: &str = r#"
CREATE TABLE history_sources (
 source TEXT PRIMARY KEY, position INTEGER, imported INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE history_conversations (
 id INTEGER PRIMARY KEY, source TEXT NOT NULL, native_id TEXT NOT NULL, computer TEXT NOT NULL,
 title TEXT NOT NULL, parent TEXT, aliases TEXT NOT NULL, document TEXT NOT NULL,
 body TEXT NOT NULL, limitations TEXT NOT NULL, UNIQUE(source,native_id)
);
CREATE VIRTUAL TABLE history_fts USING fts5(title,body,content='history_conversations',content_rowid='id');
CREATE TRIGGER history_insert AFTER INSERT ON history_conversations BEGIN
 INSERT INTO history_fts(rowid,title,body) VALUES(new.id,new.title,new.body);
END;
CREATE TRIGGER history_delete AFTER DELETE ON history_conversations BEGIN
 INSERT INTO history_fts(history_fts,rowid,title,body) VALUES('delete',old.id,old.title,old.body);
END;
CREATE TRIGGER history_update AFTER UPDATE ON history_conversations BEGIN
 INSERT INTO history_fts(history_fts,rowid,title,body) VALUES('delete',old.id,old.title,old.body);
 INSERT INTO history_fts(rowid,title,body) VALUES(new.id,new.title,new.body);
END;
CREATE TABLE history_projection_records (
 source TEXT NOT NULL, projection TEXT NOT NULL, native_id TEXT NOT NULL,
 thread_id TEXT NOT NULL, updated_at TEXT NOT NULL, payload_json TEXT NOT NULL,
 PRIMARY KEY(source,projection,native_id)
);
CREATE INDEX history_projection_threads ON history_projection_records(source,thread_id);
CREATE TABLE history_source_positions (source TEXT PRIMARY KEY,position_json TEXT NOT NULL);
CREATE TABLE history_move (
 id INTEGER PRIMARY KEY CHECK(id=1),
 state TEXT NOT NULL CHECK(state IN ('copying','removal_pending','complete')),
 conversations INTEGER, records INTEGER
);
"#;

/// Every migration the history store ships; its schema version is their count.
const MIGRATIONS: &[Migration] = &[Migration::Sql(V1)];

fn store(home: &Home) -> Store {
    Store {
        name: "the history store",
        file: home.history_db_file(),
        lock: StoreLock::Exclusive(home.history_lock_file(), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

/// Takes the history lock and opens the store for writing, completing the
/// move of earlier history first. The lock is held for as long as the
/// handle lives; a second handle waits for it, in this process too.
pub(super) fn open(home: &Home) -> Result<StoreDb, HistoryError> {
    let mut db = StoreDb::open(&store(home))?;
    move_in(home, &mut db)?;
    Ok(db)
}

/// Opens the store for reading without the history lock. An inspection home
/// reads an existing store as it is and never creates, migrates or moves
/// it; `None` means it holds no current store. Any other home runs the
/// write path first while the move is not recorded complete, creating the
/// store when absent, and always returns it.
pub(super) fn read(home: &Home) -> Result<Option<Connection>, HistoryError> {
    let store = store(home);
    if home.is_state_read_only() {
        return Ok(match store.read(true)? {
            Some(Read::Current(conn)) => Some(conn),
            Some(Read::Behind) | None => None,
        });
    }
    if let Some(Read::Current(conn)) = store.read(false)?
        && progress(&conn)? == Some(Progress::Complete)
    {
        return Ok(Some(conn));
    }
    drop(open(home)?);
    read(home)
}

/// The normalized records the store retained of conversation `thread` from
/// `source`: one keyed read of a store opened read-only, which migrates and
/// moves nothing; `None` when no current store exists.
pub(super) fn retained_records(
    home: &Home,
    source: &str,
    thread: &str,
) -> Result<Option<Vec<crate::runtime::conversations::ConversationRecord>>, HistoryError> {
    let Some(Read::Current(conn)) = store(home).read(true)? else {
        return Ok(None);
    };
    let mut statement = conn.prepare(
        "SELECT payload_json FROM history_projection_records WHERE source=?1 AND thread_id=?2",
    )?;
    let payloads = statement
        .query_map(params![source, thread], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let records = payloads
        .iter()
        .map(|payload| serde_json::from_str(payload))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(records))
}

/// How far moving earlier history in has progressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    /// Copying, after the last committed conversation id and projection
    /// record rowid of the state database.
    Copying {
        conversations: Option<i64>,
        records: Option<i64>,
    },
    /// Every row is copied; the state database's tables are to be dropped
    /// and its space reclaimed.
    RemovalPending,
    Complete,
}

fn progress(conn: &Connection) -> Result<Option<Progress>, HistoryError> {
    let row: Option<(String, Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT state,conversations,records FROM history_move WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    row.map(|(state, conversations, records)| match state.as_str() {
        "copying" => Ok(Progress::Copying {
            conversations,
            records,
        }),
        "removal_pending" => Ok(Progress::RemovalPending),
        "complete" => Ok(Progress::Complete),
        other => Err(HistoryError::Source(format!(
            "the history store records an unknown move state {other}"
        ))),
    })
    .transpose()
}

fn record(tx: &Transaction<'_>, progress: Progress) -> Result<(), HistoryError> {
    let (state, conversations, records) = match progress {
        Progress::Copying {
            conversations,
            records,
        } => ("copying", conversations, records),
        Progress::RemovalPending => ("removal_pending", None, None),
        Progress::Complete => ("complete", None, None),
    };
    tx.execute(
        "INSERT INTO history_move(id,state,conversations,records) VALUES(1,?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state,conversations=excluded.conversations,records=excluded.records",
        params![state, conversations, records],
    )?;
    Ok(())
}

fn record_complete(db: &mut StoreDb) -> Result<(), HistoryError> {
    db.write(|tx| record(tx, Progress::Complete))
}

/// Completes moving the history an earlier build kept in the state
/// database, resuming wherever a stopped move left it.
fn move_in(home: &Home, db: &mut StoreDb) -> Result<(), HistoryError> {
    let started = Instant::now();
    let mut cursor = match progress(db.conn())? {
        Some(Progress::Complete) => return Ok(()),
        Some(Progress::RemovalPending) => return remove(home, db, started),
        Some(Progress::Copying {
            conversations,
            records,
        }) => (conversations, records),
        None => (None, None),
    };
    // Brings an older state database to its current schema under the
    // writer lock; a current one is read without it.
    let Some(state) = StateRead::open(home)? else {
        // Created at its current schema: no history tables, and refused by
        // the builds that wrote them.
        drop(StateDb::open(home)?);
        return record_complete(db);
    };
    if !holds_history(state.conn())? {
        drop(state);
        return record_complete(db);
    }
    while let Some(next) = copy_conversations(state.conn(), db, cursor)? {
        cursor.0 = Some(next);
    }
    while let Some(next) = copy_records(state.conn(), db, cursor)? {
        cursor.1 = Some(next);
    }
    copy_bookkeeping(state.conn(), db)?;
    // No connection that read the state database outlives the copy, so the
    // checkpoint after reclaiming is never held back by the move itself.
    drop(state);
    remove(home, db, started)
}

fn holds_history(state: &Connection) -> Result<bool, HistoryError> {
    Ok(state.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name IN ('history_conversations','history_fts','history_projection_records','history_source_positions','history_sources'))",
        [],
        |r| r.get(0),
    )?)
}

/// One retained conversation row of the state database, copied unchanged.
struct ConversationRow {
    id: i64,
    source: String,
    native_id: String,
    computer: String,
    title: String,
    parent: Option<String>,
    aliases: String,
    document: String,
    body: String,
    limitations: String,
}

impl ConversationRow {
    fn bytes(&self) -> usize {
        [
            &self.source,
            &self.native_id,
            &self.computer,
            &self.title,
            &self.aliases,
            &self.document,
            &self.body,
            &self.limitations,
        ]
        .iter()
        .map(|s| s.len())
        .sum::<usize>()
            + self.parent.as_ref().map_or(0, String::len)
    }
}

/// Copies the next batch of conversations after `cursor.0` and commits it
/// with its cursor; `None` when none remains. The read ends before the
/// write begins.
fn copy_conversations(
    state: &Connection,
    db: &mut StoreDb,
    cursor: (Option<i64>, Option<i64>),
) -> Result<Option<i64>, HistoryError> {
    const COLUMNS: &str = "SELECT id,source,native_id,computer,title,parent,aliases,document,body,limitations FROM history_conversations";
    let batch = {
        let mut stmt = match cursor.0 {
            None => state.prepare(&format!("{COLUMNS} ORDER BY id LIMIT ?1"))?,
            Some(_) => state.prepare(&format!("{COLUMNS} WHERE id>?2 ORDER BY id LIMIT ?1"))?,
        };
        let map = |r: &instrukt_core::state_db::rusqlite::Row<'_>| {
            Ok(ConversationRow {
                id: r.get(0)?,
                source: r.get(1)?,
                native_id: r.get(2)?,
                computer: r.get(3)?,
                title: r.get(4)?,
                parent: r.get(5)?,
                aliases: r.get(6)?,
                document: r.get(7)?,
                body: r.get(8)?,
                limitations: r.get(9)?,
            })
        };
        let rows = match cursor.0 {
            None => stmt.query_map(params![BATCH_ROWS as i64], map)?,
            Some(after) => stmt.query_map(params![BATCH_ROWS as i64, after], map)?,
        };
        bounded(rows, ConversationRow::bytes)?
    };
    let Some(last) = batch.last().map(|row| row.id) else {
        return Ok(None);
    };
    db.write(|tx| -> Result<(), HistoryError> {
        for row in &batch {
            tx.execute(
                "INSERT INTO history_conversations(id,source,native_id,computer,title,parent,aliases,document,body,limitations) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![row.id, row.source, row.native_id, row.computer, row.title, row.parent, row.aliases, row.document, row.body, row.limitations],
            )?;
        }
        record(
            tx,
            Progress::Copying {
                conversations: Some(last),
                records: cursor.1,
            },
        )
    })?;
    Ok(Some(last))
}

/// One normalized record row of the state database, copied unchanged.
struct RecordRow {
    rowid: i64,
    source: String,
    projection: String,
    native_id: String,
    thread_id: String,
    updated_at: String,
    payload_json: String,
}

impl RecordRow {
    fn bytes(&self) -> usize {
        self.source.len()
            + self.projection.len()
            + self.native_id.len()
            + self.thread_id.len()
            + self.updated_at.len()
            + self.payload_json.len()
    }
}

/// Copies the next batch of projection records after `cursor.1` and
/// commits it with its cursor; `None` when none remains.
fn copy_records(
    state: &Connection,
    db: &mut StoreDb,
    cursor: (Option<i64>, Option<i64>),
) -> Result<Option<i64>, HistoryError> {
    const COLUMNS: &str = "SELECT rowid,source,projection,native_id,thread_id,updated_at,payload_json FROM history_projection_records";
    let batch = {
        let mut stmt = match cursor.1 {
            None => state.prepare(&format!("{COLUMNS} ORDER BY rowid LIMIT ?1"))?,
            Some(_) => {
                state.prepare(&format!("{COLUMNS} WHERE rowid>?2 ORDER BY rowid LIMIT ?1"))?
            }
        };
        let map = |r: &instrukt_core::state_db::rusqlite::Row<'_>| {
            Ok(RecordRow {
                rowid: r.get(0)?,
                source: r.get(1)?,
                projection: r.get(2)?,
                native_id: r.get(3)?,
                thread_id: r.get(4)?,
                updated_at: r.get(5)?,
                payload_json: r.get(6)?,
            })
        };
        let rows = match cursor.1 {
            None => stmt.query_map(params![BATCH_ROWS as i64], map)?,
            Some(after) => stmt.query_map(params![BATCH_ROWS as i64, after], map)?,
        };
        bounded(rows, RecordRow::bytes)?
    };
    let Some(last) = batch.last().map(|row| row.rowid) else {
        return Ok(None);
    };
    db.write(|tx| -> Result<(), HistoryError> {
        for row in &batch {
            tx.execute(
                "INSERT INTO history_projection_records(source,projection,native_id,thread_id,updated_at,payload_json) VALUES(?1,?2,?3,?4,?5,?6)",
                params![row.source, row.projection, row.native_id, row.thread_id, row.updated_at, row.payload_json],
            )?;
        }
        record(
            tx,
            Progress::Copying {
                conversations: cursor.0,
                records: Some(last),
            },
        )
    })?;
    Ok(Some(last))
}

/// Takes rows up to [`BATCH_BYTES`] of retained bytes, at least one.
fn bounded<T>(
    rows: impl Iterator<Item = instrukt_core::state_db::rusqlite::Result<T>>,
    bytes: impl Fn(&T) -> usize,
) -> Result<Vec<T>, HistoryError> {
    let mut batch = Vec::new();
    let mut taken = 0;
    for row in rows {
        let row = row?;
        let size = bytes(&row);
        if !batch.is_empty() && taken + size > BATCH_BYTES {
            break;
        }
        taken += size;
        batch.push(row);
    }
    Ok(batch)
}

/// Copies both bookkeeping tables unchanged and records the copy finished
/// and the removal pending, in one transaction.
fn copy_bookkeeping(state: &Connection, db: &mut StoreDb) -> Result<(), HistoryError> {
    let positions = state
        .prepare("SELECT source,position_json FROM history_source_positions ORDER BY source")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let sources = state
        .prepare("SELECT source,position,imported FROM history_sources ORDER BY source")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    db.write(|tx| -> Result<(), HistoryError> {
        for (source, position) in &positions {
            tx.execute(
                "INSERT INTO history_source_positions(source,position_json) VALUES(?1,?2)",
                params![source, position],
            )?;
        }
        for (source, position, imported) in &sources {
            tx.execute(
                "INSERT INTO history_sources(source,position,imported) VALUES(?1,?2,?3)",
                params![source, position, imported],
            )?;
        }
        record(tx, Progress::RemovalPending)
    })
}

/// In one hold of the writer lock: drops whichever history tables the
/// state database still holds and reclaims the space they freed; then
/// records the move complete, logging it with the time since `started`. A
/// failure leaves the removal pending for the next write-open.
fn remove(home: &Home, db: &mut StoreDb, started: Instant) -> Result<(), HistoryError> {
    let mut state = StateDb::open(home)?;
    state.remove_history()?;
    state.reclaim()?;
    drop(state);
    record_complete(db)?;
    tracing::info!(
        event = "history_moved",
        duration_ms = started.elapsed().as_millis() as u64,
        "earlier history moved out of the state database"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_core::state_db::rusqlite::OpenFlags;
    use std::path::Path;

    /// The history tables as the state database held them at version 31.
    const LEGACY: &str = r#"
CREATE TABLE history_sources (
 source TEXT PRIMARY KEY, position INTEGER NOT NULL DEFAULT 0, imported INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE history_conversations (
 id INTEGER PRIMARY KEY, source TEXT NOT NULL, native_id TEXT NOT NULL, computer TEXT NOT NULL,
 title TEXT NOT NULL, parent TEXT, aliases TEXT NOT NULL, document TEXT NOT NULL,
 body TEXT NOT NULL, limitations TEXT NOT NULL, UNIQUE(source,native_id)
);
CREATE VIRTUAL TABLE history_fts USING fts5(title,body,content='history_conversations',content_rowid='id');
CREATE TRIGGER history_insert AFTER INSERT ON history_conversations BEGIN
 INSERT INTO history_fts(rowid,title,body) VALUES(new.id,new.title,new.body);
END;
CREATE TRIGGER history_delete AFTER DELETE ON history_conversations BEGIN
 INSERT INTO history_fts(history_fts,rowid,title,body) VALUES('delete',old.id,old.title,old.body);
END;
CREATE TRIGGER history_update AFTER UPDATE ON history_conversations BEGIN
 INSERT INTO history_fts(history_fts,rowid,title,body) VALUES('delete',old.id,old.title,old.body);
 INSERT INTO history_fts(rowid,title,body) VALUES(new.id,new.title,new.body);
END;
CREATE TABLE history_projection_records (
 source TEXT NOT NULL, projection TEXT NOT NULL, native_id TEXT NOT NULL,
 thread_id TEXT NOT NULL, updated_at TEXT NOT NULL, payload_json TEXT NOT NULL,
 PRIMARY KEY(source,projection,native_id)
);
CREATE INDEX history_projection_threads ON history_projection_records(source,thread_id);
CREATE TABLE history_source_positions (source TEXT PRIMARY KEY,position_json TEXT NOT NULL);
"#;

    /// A home whose current state database holds `conversations` retained
    /// conversations of about `size` bytes each, two normalized records, a
    /// runtime position, a completed TeleClaude import and an inert `t3:` row.
    fn upgraded(conversations: u32, size: u32) -> (tempfile::TempDir, Home) {
        let dir = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        drop(StateDb::open(&home).unwrap());
        let db = raw(&home.state_db_file());
        db.execute_batch(LEGACY).unwrap();
        db.execute(
            "INSERT INTO history_conversations(source,native_id,computer,title,parent,aliases,document,body,limitations)
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?1)
             SELECT 't3:legacy','thread-'||i,'computer','Moved '||i,NULL,'[]','{}','Movedtoken '||i||' '||hex(zeroblob(?2)),'[]' FROM n",
            params![conversations, size / 2],
        )
        .unwrap();
        db.execute_batch(
            "INSERT INTO history_projection_records VALUES('t3:legacy','message','m1','thread-1','2026-11-01','{}'),('t3:legacy','message','m2','thread-2','2026-11-02','{}');
             INSERT INTO history_source_positions VALUES('t3:legacy','{\"version\":2}');
             INSERT INTO history_sources VALUES('teleclaude:done',1,1),('t3:legacy',42,0);",
        )
        .unwrap();
        (dir, home)
    }

    fn raw(file: &Path) -> Connection {
        instrukt_core::fsx::open_database(file, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// Copies every retained row and records the removal pending, as a move
    /// that stopped before removing anything leaves the store.
    fn copied(home: &Home) {
        let mut db = StoreDb::open(&store(home)).unwrap();
        let state = StateRead::open(home).unwrap().unwrap();
        let mut cursor = (None, None);
        while let Some(next) = copy_conversations(state.conn(), &mut db, cursor).unwrap() {
            cursor.0 = Some(next);
        }
        while let Some(next) = copy_records(state.conn(), &mut db, cursor).unwrap() {
            cursor.1 = Some(next);
        }
        copy_bookkeeping(state.conn(), &mut db).unwrap();
    }

    #[test]
    fn a_move_stopped_while_copying_continues_after_its_last_committed_batch() {
        // More conversations than one batch takes.
        let total = BATCH_ROWS as u32 + 2;
        let (_dir, home) = upgraded(total, 16);
        // The move stopped once its first batch had committed.
        let last = {
            let mut db = StoreDb::open(&store(&home)).unwrap();
            let state = StateRead::open(&home).unwrap().unwrap();
            copy_conversations(state.conn(), &mut db, (None, None))
                .unwrap()
                .unwrap()
        };
        assert!(last < i64::from(total));
        // The rows it copied change in the state database afterwards.
        raw(&home.state_db_file())
            .execute(
                "UPDATE history_conversations SET body='Changedtoken' WHERE id<=?1",
                [last],
            )
            .unwrap();
        let db = open(&home).unwrap();
        assert_eq!(progress(db.conn()).unwrap(), Some(Progress::Complete));
        let moved = (
            count(db.conn(), "SELECT count(*) FROM history_conversations"),
            count(
                db.conn(),
                "SELECT count(*) FROM history_conversations WHERE body='Changedtoken'",
            ),
            count(
                db.conn(),
                "SELECT count(*) FROM history_fts WHERE history_fts MATCH 'Movedtoken'",
            ),
            count(db.conn(), "SELECT count(*) FROM history_projection_records"),
            count(db.conn(), "SELECT count(*) FROM history_source_positions"),
        );
        assert_eq!(moved, (i64::from(total), 0, i64::from(total), 2, 1));
        let sources: Vec<(String, Option<i64>, i64)> = db
            .conn()
            .prepare("SELECT source,position,imported FROM history_sources ORDER BY source")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            sources,
            [
                ("t3:legacy".to_owned(), Some(42), 0),
                ("teleclaude:done".to_owned(), Some(1), 1)
            ]
        );
    }

    #[test]
    fn a_pending_removal_reclaims_pages_a_committed_drop_freed() {
        let (_dir, home) = upgraded(40, 32 * 1024);
        copied(&home);
        // The drop committed and the process stopped before reclaiming.
        StateDb::open(&home).unwrap().remove_history().unwrap();
        let file = home.state_db_file();
        let noted = std::fs::metadata(&file).unwrap().len();
        // Kept open across the reopen, idle, so the move's close is never the
        // last one and only reclaiming's own checkpoint can shorten the file.
        let idle = raw(&file);
        let free = count(&idle, "SELECT freelist_count FROM pragma_freelist_count");
        let page = count(&idle, "SELECT page_size FROM pragma_page_size");
        assert!(free > 0);
        let db = open(&home).unwrap();
        assert_eq!(progress(db.conn()).unwrap(), Some(Progress::Complete));
        let length = std::fs::metadata(&file).unwrap().len();
        let pages = count(&idle, "SELECT page_count FROM pragma_page_count");
        assert_eq!(
            count(&idle, "SELECT freelist_count FROM pragma_freelist_count"),
            0
        );
        assert!(length <= noted - u64::try_from(free * page).unwrap());
        assert_eq!(length, u64::try_from(pages * page).unwrap());
    }

    #[test]
    fn a_reader_on_an_earlier_snapshot_does_not_keep_the_move_from_completing() {
        let (_dir, home) = upgraded(40, 32 * 1024);
        copied(&home);
        let file = home.state_db_file();
        let reader = raw(&file);
        reader.execute_batch("BEGIN").unwrap();
        assert_eq!(
            count(&reader, "SELECT count(*) FROM history_conversations"),
            40
        );
        let db = open(&home).unwrap();
        assert_eq!(progress(db.conn()).unwrap(), Some(Progress::Complete));
        let current = raw(&file);
        let removed = (
            count(
                &current,
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'history%'",
            ),
            count(&current, "SELECT freelist_count FROM pragma_freelist_count"),
        );
        assert_eq!(removed, (0, 0));
        reader.execute_batch("COMMIT").unwrap();
    }
}
