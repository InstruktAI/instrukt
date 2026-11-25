//! The history store `state/history.db`
//! (`project/instrukt/spec/feature/runner/history`, History store): its own
//! SQLite file on the state database's machinery, written only under the
//! history lock, the store's write-isolation unit. Opening it for writing
//! brings a store of an earlier schema version forward, reclaims the space
//! that freed, then completes moving the history earlier builds kept in the
//! state database; once that move is recorded complete no history read or
//! write opens the state database or takes its writer lock.
use super::{
    _earlier::{self, Current, Earlier},
    _store::{self, Conversation},
    HistoryError, RetainedMessage,
};
use instrukt_core::{
    home::Home,
    state_db::{
        Migration, Read, StateDb, StateRead, Store, StoreDb, StoreLock,
        rusqlite::{self, Connection, OptionalExtension, Transaction, params},
    },
};
use std::time::{Duration, Instant};

/// How long a history writer waits for another one holding the history lock.
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// The most conversations one batch of the move, or of the migration,
/// converts.
const BATCH_ROWS: usize = 128;

/// The most retained bytes one batch of the move copies; a batch always
/// takes at least one conversation.
const BATCH_BYTES: usize = 32 * 1024 * 1024;

/// Version 1: the final shape earlier builds kept in the state database,
/// with an import position that is absent until a first batch commits,
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

/// Version 2's tables: each retained text once, and the index reading its
/// text from them through a view, one entry per conversation.
const V2: &str = r#"
CREATE TABLE history_messages (
 id INTEGER PRIMARY KEY, conversation INTEGER NOT NULL REFERENCES history_conversations(id),
 native_id TEXT NOT NULL, role TEXT, text TEXT NOT NULL, created_at TEXT, updated_at TEXT,
 place TEXT, record TEXT, UNIQUE(conversation,native_id)
);
CREATE VIEW history_search(id,title,body) AS
 SELECT c.id,c.title,coalesce((SELECT group_concat(m.text,char(10)||char(10) ORDER BY m.created_at,m.native_id)
  FROM history_messages m WHERE m.conversation=c.id),'')
 FROM history_conversations c;
CREATE TABLE history_reclaim (id INTEGER PRIMARY KEY CHECK(id=1));
"#;

/// Every migration the history store ships; its schema version is their count.
const MIGRATIONS: &[Migration] = &[Migration::Sql(V1), Migration::Step(version_2)];

/// Version 2: every retained text stored once. In one transaction: a copy
/// from the state database an earlier version left unfinished is removed,
/// to start again; each conversation is converted by the kind of its
/// source; the earlier records and searchable text are removed; the index
/// is rebuilt; and the space that freed is recorded to be reclaimed.
fn version_2(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    let held: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM history_conversations) OR EXISTS(SELECT 1 FROM history_projection_records)",
        [],
        |r| r.get(0),
    )?;
    tx.execute_batch(
        "DROP TRIGGER history_insert; DROP TRIGGER history_delete; DROP TRIGGER history_update; DROP TABLE history_fts;",
    )?;
    let copying: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM history_move WHERE state='copying')",
        [],
        |r| r.get(0),
    )?;
    if copying {
        tx.execute_batch(
            "DELETE FROM history_projection_records; DELETE FROM history_conversations; DELETE FROM history_move;",
        )?;
    }
    tx.execute_batch(V2)?;
    let mut after = None;
    loop {
        let batch = {
            let mut statement = tx.prepare_cached(&format!(
                "{} WHERE ?1 IS NULL OR id>?1 ORDER BY id LIMIT ?2",
                _earlier::EARLIER
            ))?;
            statement
                .query_map(params![after, BATCH_ROWS as i64], Earlier::row)?
                .collect::<Result<Vec<_>, _>>()?
        };
        let Some(last) = batch.last().map(|c| c.id) else {
            break;
        };
        for earlier in &batch {
            let Current { document, texts } = _earlier::convert(tx, earlier)?;
            tx.execute(
                "UPDATE history_conversations SET document=?2 WHERE id=?1",
                params![earlier.id, document.to_string()],
            )?;
            _store::write_texts(tx, earlier.id, &texts)?;
        }
        after = Some(last);
    }
    tx.execute_batch(
        "DROP TABLE history_projection_records;
         ALTER TABLE history_conversations DROP COLUMN body;
         ALTER TABLE history_move DROP COLUMN records;
         CREATE VIRTUAL TABLE history_fts USING fts5(title,body,content='history_search',content_rowid='id');
         INSERT INTO history_fts(history_fts) VALUES('rebuild');",
    )?;
    if held {
        tx.execute("INSERT INTO history_reclaim(id) VALUES(1)", [])?;
    }
    Ok(())
}

fn store(home: &Home) -> Store {
    Store {
        name: "the history store",
        file: home.history_db_file(),
        lock: StoreLock::Exclusive(home.history_lock_file(), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

/// Takes the history lock and opens the store for writing: brought to the
/// current schema version, the space that freed reclaimed, and the move of
/// earlier history completed, in that order. The lock is held for as long
/// as the handle lives; a second handle waits for it, in this process too.
pub(super) fn open(home: &Home) -> Result<StoreDb, HistoryError> {
    let mut db = StoreDb::open(&store(home))?;
    reclaim(&mut db)?;
    move_in(home, &mut db)?;
    Ok(db)
}

/// Reclaims the space a migration freed while that is recorded pending,
/// and records it done once the rewrite has committed. A failure leaves it
/// pending for the next write-open.
fn reclaim(db: &mut StoreDb) -> Result<(), HistoryError> {
    let pending: bool =
        db.conn()
            .query_row("SELECT EXISTS(SELECT 1 FROM history_reclaim)", [], |r| {
                r.get(0)
            })?;
    if !pending {
        return Ok(());
    }
    let started = Instant::now();
    db.reclaim()?;
    db.write(|tx| -> Result<(), HistoryError> {
        tx.execute("DELETE FROM history_reclaim", [])?;
        Ok(())
    })?;
    tracing::info!(
        event = "history_reclaimed",
        duration_ms = started.elapsed().as_millis() as u64,
        "the space a history store migration freed is reclaimed"
    );
    Ok(())
}

/// Opens the store for reading without the history lock. An inspection home
/// reads an existing store as it is, an earlier schema version brought
/// forward in memory only, and never creates, migrates on disk or moves it;
/// `None` means it holds no store. Any other home runs the write path first
/// while the store is at an earlier schema version or the move is not
/// recorded complete, creating the store when absent, and always returns it.
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

/// The messages that carry a role of conversation `thread` from `source`,
/// in order: one keyed read of a snapshot taken without the history lock,
/// which changes nothing on disk; a store at an earlier schema version is
/// brought forward in memory for it. `None` when no store exists or it has
/// not retained the conversation.
pub(super) fn retained_messages(
    home: &Home,
    source: &str,
    thread: &str,
) -> Result<Option<Vec<RetainedMessage>>, HistoryError> {
    let Some(Read::Current(conn)) = store(home).read(true)? else {
        return Ok(None);
    };
    _store::messages(&conn, source, thread)
}

/// How far moving earlier history in has progressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    /// Copying, after the last committed conversation id of the state
    /// database.
    Copying {
        conversations: Option<i64>,
    },
    /// Every row is copied; the state database's tables are to be dropped
    /// and its space reclaimed.
    RemovalPending,
    Complete,
}

fn progress(conn: &Connection) -> Result<Option<Progress>, HistoryError> {
    let row: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT state,conversations FROM history_move WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(state, conversations)| match state.as_str() {
        "copying" => Ok(Progress::Copying { conversations }),
        "removal_pending" => Ok(Progress::RemovalPending),
        "complete" => Ok(Progress::Complete),
        other => Err(HistoryError::Source(format!(
            "the history store records an unknown move state {other}"
        ))),
    })
    .transpose()
}

fn record(tx: &Transaction<'_>, progress: Progress) -> Result<(), HistoryError> {
    let (state, conversations) = match progress {
        Progress::Copying { conversations } => ("copying", conversations),
        Progress::RemovalPending => ("removal_pending", None),
        Progress::Complete => ("complete", None),
    };
    tx.execute(
        "INSERT INTO history_move(id,state,conversations) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state,conversations=excluded.conversations",
        params![state, conversations],
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
        Some(Progress::Copying { conversations }) => conversations,
        None => None,
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
        cursor = Some(next);
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

/// Converts the next batch of the state database's conversations after
/// `cursor`, with the records the conversion reads, and commits it in the
/// current shape with its cursor; `None` when none remains. The read ends
/// before the write begins.
fn copy_conversations(
    state: &Connection,
    db: &mut StoreDb,
    cursor: Option<i64>,
) -> Result<Option<i64>, HistoryError> {
    let batch = {
        let mut statement = state.prepare(&format!(
            "{} WHERE ?1 IS NULL OR id>?1 ORDER BY id LIMIT ?2",
            _earlier::EARLIER
        ))?;
        let rows = statement.query_map(params![cursor, BATCH_ROWS as i64], Earlier::row)?;
        let mut batch = Vec::new();
        let mut taken = 0;
        for row in rows {
            let earlier = row?;
            let current = _earlier::convert(state, &earlier)?;
            let size = earlier.bytes().max(current.bytes());
            if !batch.is_empty() && taken + size > BATCH_BYTES {
                break;
            }
            taken += size;
            batch.push((earlier, current));
        }
        batch
    };
    let Some(last) = batch.last().map(|(earlier, _)| earlier.id) else {
        return Ok(None);
    };
    db.write(|tx| -> Result<(), HistoryError> {
        for (earlier, current) in batch {
            let conversation = Conversation {
                native_id: earlier.native_id,
                computer: earlier.computer,
                title: earlier.title,
                parent: earlier.parent,
                aliases: serde_json::from_str(&earlier.aliases)?,
                limitations: serde_json::from_str(&earlier.limitations)?,
                document: current.document,
                texts: current.texts,
            };
            _store::write_conversation(tx, &earlier.source, Some(earlier.id), &conversation)?;
        }
        record(
            tx,
            Progress::Copying {
                conversations: Some(last),
            },
        )
    })?;
    Ok(Some(last))
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
    use crate::runtime::conversations::Role;
    use instrukt_core::state_db::rusqlite::OpenFlags;
    use serde_json::Value;
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
    /// runtime conversations, each with its conversation record and one
    /// message of about `size` bytes, a run record, a runtime position, a
    /// completed import of an earlier archive and an inert `t3:` row.
    fn upgraded(conversations: u32, size: u32) -> (tempfile::TempDir, Home) {
        let dir = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        drop(StateDb::open(&home).unwrap());
        let db = raw(&home.state_db_file());
        db.execute_batch(LEGACY).unwrap();
        db.execute(
            "INSERT INTO history_conversations(source,native_id,computer,title,parent,aliases,document,body,limitations)
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?1)
             SELECT 't3:legacy','thread-'||i,'computer','Moved '||i,NULL,'[\"thread-'||i||'\"]','{}','','[]' FROM n",
            [conversations],
        )
        .unwrap();
        db.execute(
            "INSERT INTO history_projection_records
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?1)
             SELECT 't3:legacy','conversation','thread-'||i,'thread-'||i,'2026-11-01',
              json_object('kind','conversation','id','thread-'||i,'conversation_id','thread-'||i,'title','Moved '||i,'native',json_object('id','thread-'||i,'title','Moved '||i))
             FROM n
             UNION ALL
             SELECT 't3:legacy','message','message-'||i,'thread-'||i,'2026-11-01',
              json_object('kind','message','id','message-'||i,'conversation_id','thread-'||i,'role','assistant','text',NULL,'created_at','2026-11-01T00:00:00Z','updated_at','2026-11-01T00:00:00Z',
               'native',json_object('id','message-'||i,'threadId','thread-'||i,'role','assistant','text','Movedtoken '||i||' '||hex(zeroblob(?2))))
             FROM n
             UNION ALL
             SELECT 't3:legacy','run','run-'||i,'thread-'||i,'2026-11-01',
              json_object('kind','run','id','run-'||i,'conversation_id','thread-'||i,'text','Runtoken '||i,'native',json_object('id','run-'||i))
             FROM n",
            params![conversations, size / 2],
        )
        .unwrap();
        db.execute_batch(
            "INSERT INTO history_source_positions VALUES('t3:legacy','{\"version\":2}');
             INSERT INTO history_sources VALUES('archive:done',1,1),('t3:legacy',42,0);",
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

    fn found(home: &Home, query: &str) -> Vec<String> {
        let mut found: Vec<String> = _store::search(home, query, 1000)
            .unwrap()
            .into_iter()
            .map(|m| m.conversation)
            .collect();
        found.sort();
        found
    }

    /// Copies every retained conversation and records the removal pending,
    /// as a move that stopped before removing anything leaves the store.
    fn copied(home: &Home) {
        let mut db = StoreDb::open(&store(home)).unwrap();
        let state = StateRead::open(home).unwrap().unwrap();
        let mut cursor = None;
        while let Some(next) = copy_conversations(state.conn(), &mut db, cursor).unwrap() {
            cursor = Some(next);
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
            copy_conversations(state.conn(), &mut db, None)
                .unwrap()
                .unwrap()
        };
        assert!(last < i64::from(total));
        // The messages it copied change in the state database afterwards.
        raw(&home.state_db_file())
            .execute(
                "UPDATE history_projection_records SET payload_json=json_set(payload_json,'$.native.text','Changedtoken') WHERE projection='message' AND thread_id IN (SELECT native_id FROM history_conversations WHERE id<=?1)",
                [last],
            )
            .unwrap();
        let db = open(&home).unwrap();
        assert_eq!(progress(db.conn()).unwrap(), Some(Progress::Complete));
        let moved = (
            count(db.conn(), "SELECT count(*) FROM history_conversations"),
            count(db.conn(), "SELECT count(*) FROM history_messages"),
            count(
                db.conn(),
                "SELECT count(*) FROM history_messages WHERE text='Changedtoken'",
            ),
            count(
                db.conn(),
                "SELECT count(*) FROM history_fts WHERE history_fts MATCH 'Movedtoken'",
            ),
            count(
                db.conn(),
                "SELECT count(*) FROM history_fts WHERE history_fts MATCH 'Runtoken'",
            ),
            count(db.conn(), "SELECT count(*) FROM history_source_positions"),
        );
        assert_eq!(
            moved,
            (
                i64::from(total),
                i64::from(total),
                0,
                i64::from(total),
                0,
                1
            )
        );
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
                ("archive:done".to_owned(), Some(1), 1),
                ("t3:legacy".to_owned(), Some(42), 0)
            ]
        );
    }

    #[test]
    fn an_unfinished_copy_an_earlier_version_left_starts_again_from_the_first_conversation() {
        let total = 5;
        let (_dir, home) = upgraded(total, 16);
        // An earlier version copied two conversations and their records,
        // the first since changed in the state database, and stopped.
        {
            let file = home.history_db_file();
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            let earlier = Connection::open(&file).unwrap();
            earlier.execute_batch(V1).unwrap();
            earlier.pragma_update(None, "user_version", 1).unwrap();
            earlier
                .execute_batch(
                    "INSERT INTO history_conversations(id,source,native_id,computer,title,parent,aliases,document,body,limitations) VALUES
                      (1,'t3:legacy','thread-1','computer','Moved 1',NULL,'[]','{}','Stalecopytoken','[]'),
                      (2,'t3:legacy','thread-2','computer','Moved 2',NULL,'[]','{}','','[]');
                     INSERT INTO history_projection_records VALUES
                      ('t3:legacy','conversation','thread-1','thread-1','','{\"kind\":\"conversation\",\"native\":{\"id\":\"thread-1\"}}'),
                      ('t3:legacy','message','stale','thread-1','','{\"kind\":\"message\",\"role\":\"user\",\"native\":{\"id\":\"stale\",\"text\":\"Stalecopytoken\"}}'),
                      ('t3:legacy','conversation','thread-2','thread-2','','{\"kind\":\"conversation\",\"native\":{\"id\":\"thread-2\"}}');
                     INSERT INTO history_move(id,state,conversations,records) VALUES(1,'copying',2,3);",
                )
                .unwrap();
        }
        let db = open(&home).unwrap();
        assert_eq!(progress(db.conn()).unwrap(), Some(Progress::Complete));
        let ids: Vec<(i64, String)> = db
            .conn()
            .prepare("SELECT id,native_id FROM history_conversations ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let expected: Vec<(i64, String)> = (1..=i64::from(total))
            .map(|i| (i, format!("thread-{i}")))
            .collect();
        assert_eq!(ids, expected);
        assert_eq!(
            count(db.conn(), "SELECT count(*) FROM history_messages"),
            i64::from(total)
        );
        drop(db);
        assert!(found(&home, "Stalecopytoken").is_empty());
        assert_eq!(found(&home, "Movedtoken").len(), total as usize);
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

    /// Each runtime source's position, and each import's progress.
    type Bookkeeping = (Vec<(String, String)>, Vec<(String, Option<i64>, i64)>);

    fn bookkeeping(conn: &Connection) -> Bookkeeping {
        (
            conn.prepare(
                "SELECT source,position_json FROM history_source_positions ORDER BY source",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap(),
            conn.prepare("SELECT source,position,imported FROM history_sources ORDER BY source")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap(),
        )
    }

    /// One retained conversation of the frozen version 1 store, as its
    /// earlier row held it.
    struct Held {
        source: String,
        native_id: String,
        computer: String,
        parent: Option<String>,
        aliases: Vec<String>,
        document: Value,
    }

    #[test]
    fn a_version_1_store_comes_forward_with_each_text_once_and_nothing_tools_held() {
        let dir = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let file = home.history_db_file();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let earlier = Connection::open(&file).unwrap();
        earlier
            .execute_batch(include_str!("history-store-v1.sql"))
            .unwrap();
        let held: Vec<Held> = earlier
            .prepare("SELECT source,native_id,computer,parent,aliases,document FROM history_conversations ORDER BY id")
            .unwrap()
            .query_map([], |r| {
                Ok(Held {
                    source: r.get(0)?,
                    native_id: r.get(1)?,
                    computer: r.get(2)?,
                    parent: r.get(3)?,
                    aliases: serde_json::from_str(&r.get::<_, String>(4)?).unwrap(),
                    document: serde_json::from_str(&r.get::<_, String>(5)?).unwrap(),
                })
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let kept = bookkeeping(&earlier);
        drop(earlier);
        let [runtime, transcript, sources, other] = held.as_slice() else {
            panic!("the fixture holds four conversations");
        };

        drop(open(&home).unwrap());

        let db = raw(&file);
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            store(&home).version()
        );
        // Each conversation is found by its title and by a token of each of
        // its texts; text only tool, run and subagent records held is not.
        let by = |query: &str, conversation: &Held| {
            assert_eq!(
                found(&home, query),
                [conversation.native_id.as_str()],
                "{query}"
            );
        };
        by("Runtimetitle", runtime);
        by("Userwordtoken", runtime);
        by("Answertoken", runtime);
        by("Latertoken", runtime);
        by("Delegatedresulttoken", runtime);
        by("Transcripttitle", transcript);
        by("Transcripttoken", transcript);
        by("Sourcetitle", sources);
        by("Archivaltoken", sources);
        by("Sourcemessagetoken", sources);
        by("Sourcereplytoken", sources);
        by("Othertitle", other);
        by("Othertoken", other);
        assert!(found(&home, "Runtimetitle OR Mirrormessagetoken").len() == 1);
        assert!(
            found(
                &home,
                "Commandonlytoken OR Outputonlytoken OR Reasononlytoken OR Subagentonlytoken OR Runonlytoken OR Terminalonlytoken OR Diffonlytoken OR Htmlonlytoken OR Shelloutputtoken"
            )
            .is_empty()
        );
        // Each conversation keeps its computer, parent and archive aliases:
        // search reports them, and every earlier alias still finds its
        // conversation's document.
        for conversation in &held {
            let found = _store::search(
                &home,
                "Runtimetitle OR Transcripttitle OR Sourcetitle OR Othertitle",
                10,
            )
            .unwrap()
            .into_iter()
            .find(|m| m.conversation == conversation.native_id)
            .unwrap();
            assert_eq!(
                (&found.computer, &found.parent, &found.aliases),
                (
                    &conversation.computer,
                    &conversation.parent,
                    &conversation.aliases
                )
            );
            let by_id = _store::show(&home, &conversation.source, &conversation.native_id).unwrap();
            for alias in &conversation.aliases {
                assert_eq!(
                    _store::show(&home, &conversation.source, alias).unwrap(),
                    by_id,
                    "{alias}"
                );
            }
        }
        assert!(held.iter().any(|c| c.parent.is_some()));
        assert!(
            held.iter()
                .any(|c| c.aliases.iter().any(|a| *a != c.native_id))
        );
        // A mirror with a transcript is indexed by that transcript alone, once.
        let body = |native_id: &str| -> String {
            db.query_row(
                "SELECT s.body FROM history_search s JOIN history_conversations c ON c.id=s.id WHERE c.native_id=?1",
                [native_id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            Value::String(body(&transcript.native_id)),
            transcript.document["mirror"]["conversation_text"]
        );
        // Each shown mirror and imported document is its earlier document,
        // member for member and in order.
        for conversation in [transcript, sources, other] {
            let shown = _store::show(&home, &conversation.source, &conversation.native_id).unwrap();
            assert_eq!(shown.to_string(), conversation.document.to_string());
        }
        // The shown runtime document holds the earlier thread and messages,
        // in order, without the run, subagent and turn-item collections; a
        // message keeps its composer context without the terminal lines and
        // diff lines the composer captured.
        let shown = _store::show(&home, &runtime.source, &runtime.native_id).unwrap();
        let members: Vec<&String> = shown.as_object().unwrap().keys().collect();
        assert_eq!(members, ["messages", "thread"]);
        assert_eq!(shown["thread"], runtime.document["thread"]);
        let mut earlier_messages = runtime.document["messages"].as_array().unwrap().clone();
        // A background command's report is no message history keeps.
        let before = earlier_messages.len();
        earlier_messages.retain(|m| m["id"] != "m-command");
        assert_eq!(earlier_messages.len(), before - 1);
        let captured = earlier_messages
            .iter_mut()
            .find(|m| m.get("context").is_some())
            .unwrap();
        for (record, member) in [
            ("/context/records/0", "text"),
            ("/context/records/1", "diff"),
            ("/context/records/2/elements/0", "htmlPreview"),
            ("/context/records/3", "htmlPreview"),
        ] {
            let record = captured
                .pointer_mut(record)
                .and_then(Value::as_object_mut)
                .unwrap();
            assert!(record.shift_remove(member).is_some());
        }
        let shown_messages = shown["messages"].as_array().unwrap();
        assert_eq!(shown_messages.len(), earlier_messages.len());
        for message in shown_messages {
            assert!(earlier_messages.contains(message));
        }
        let mut in_order: Vec<&Value> = earlier_messages.iter().collect();
        in_order.sort_by_key(|m| {
            m["createdAt"]
                .as_str()
                .unwrap()
                .parse::<jiff::Timestamp>()
                .unwrap()
        });
        let shown_ids: Vec<&Value> = shown_messages.iter().map(|m| &m["id"]).collect();
        let earlier_ids: Vec<&Value> = in_order.iter().map(|m| &m["id"]).collect();
        assert_eq!(shown_ids, earlier_ids);
        // Review's read: the runtime conversation's messages under their
        // roles, in order, and nothing for a conversation not retained.
        let retained = retained_messages(&home, &runtime.source, &runtime.native_id)
            .unwrap()
            .unwrap();
        let expected: Vec<RetainedMessage> = in_order
            .iter()
            .map(|m| RetainedMessage {
                role: serde_json::from_value::<Role>(m["role"].clone()).unwrap(),
                text: m["text"].as_str().unwrap().to_owned(),
            })
            .collect();
        assert_eq!(retained, expected);
        assert_eq!(
            retained_messages(&home, &runtime.source, "not-retained").unwrap(),
            None
        );
        // An imported conversation's texts that carry a role come back under
        // it, in document order; a transcript or a searchable text carries
        // none.
        let mirror_messages: Vec<RetainedMessage> = sources.document["sources"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["text"].as_str().is_some_and(|text| !text.is_empty()))
            .map(|m| RetainedMessage {
                role: serde_json::from_value::<Role>(m["role"].clone()).unwrap(),
                text: m["text"].as_str().unwrap().to_owned(),
            })
            .collect();
        assert_eq!(mirror_messages.len(), 2);
        assert_eq!(
            retained_messages(&home, &sources.source, &sources.native_id).unwrap(),
            Some(mirror_messages)
        );
        for conversation in [transcript, other] {
            assert_eq!(
                retained_messages(&home, &conversation.source, &conversation.native_id).unwrap(),
                Some(Vec::new())
            );
        }
        // Each text is retained exactly once: no row's record, and no stored
        // document at a row's place, still holds it.
        let rows: Vec<(String, Option<String>, Option<String>, String)> = db
            .prepare("SELECT c.document,m.place,m.record,m.text FROM history_messages m JOIN history_conversations c ON c.id=m.conversation")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 4 + 1 + 3 + 1);
        for (document, place, record, text) in &rows {
            let document: Value = serde_json::from_str(document).unwrap();
            assert_eq!(
                rows.iter().filter(|(_, _, _, t)| t == text).count(),
                1,
                "{text}"
            );
            match (place, record) {
                (Some(place), Some(record)) => {
                    let record: Value = serde_json::from_str(record).unwrap();
                    assert_eq!(record.get("text"), None);
                    assert_eq!(document.pointer(place), Some(&Value::Array(Vec::new())));
                }
                (Some(place), None) => assert_eq!(document.pointer(place), Some(&Value::Null)),
                (None, _) => assert_eq!(document, Value::Object(Default::default())),
            }
        }
        // The earlier records and searchable text are gone; bookkeeping is
        // as it was; no free page remains; the index is intact.
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM sqlite_master WHERE name='history_projection_records'"
            ),
            0
        );
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM pragma_table_info('history_conversations') WHERE name='body'"
            ),
            0
        );
        assert_eq!(bookkeeping(&db), kept);
        assert_eq!(
            count(&db, "SELECT freelist_count FROM pragma_freelist_count"),
            0
        );
        assert_eq!(count(&db, "SELECT count(*) FROM history_reclaim"), 0);
        db.execute(
            "INSERT INTO history_fts(history_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .unwrap();
    }
}
