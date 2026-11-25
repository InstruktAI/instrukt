#![allow(clippy::unwrap_used, reason = "temporary source fixtures")]
use crate::common::TestEnv;
use instrukt_core::state_db::rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::os::unix::fs::OpenOptionsExt as _;
use std::{
    path::{Path, PathBuf},
    process::Child,
    time::Duration,
};

/// How a write to T3's database waits while the other work streams: the
/// stream takes the database for a moment every millisecond, so the write is
/// tried again every millisecond, for up to 20 seconds, rather than on a
/// backoff the stream could outpace.
pub fn retry_soon(attempt: i32) -> bool {
    std::thread::sleep(Duration::from_millis(1));
    attempt < 20_000
}

pub struct T3Store {
    pub file: PathBuf,
    db: Connection,
}
impl T3Store {
    /// A computer whose T3 holds the seeded conversations: its machine id and
    /// global configuration, then T3's database.
    pub fn new(env: &TestEnv) -> Self {
        computer(env, "aaaaaaaaaaaaaaaa");
        std::fs::write(
            env.instrukt_home().join("instrukt.yaml"),
            "schema_version: 1\nrunner: {runtime: t3}\n",
        )
        .unwrap();
        Self::create(env)
    }
    /// T3 creates its database with the seeded conversations; nothing of
    /// instrukt's configuration changes.
    pub fn create(env: &TestEnv) -> Self {
        let file = env.home().join(".t3/userdata/statev2.sqlite");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut db = Connection::open(&file).unwrap();
        db.busy_handler(Some(retry_soon)).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL;
CREATE TABLE effect_sql_migrations(migration_id INTEGER PRIMARY KEY,name TEXT);
CREATE TABLE IF NOT EXISTS orchestration_command_receipts(command_id TEXT PRIMARY KEY,aggregate_kind TEXT,aggregate_id TEXT,command_type TEXT,status TEXT);
INSERT INTO effect_sql_migrations VALUES(79,'fixture');
CREATE TABLE orchestration_v2_projection_threads(thread_id TEXT PRIMARY KEY,project_id TEXT,title TEXT,created_at TEXT,updated_at TEXT,archived_at TEXT,deleted_at TEXT,payload_json TEXT);
CREATE TABLE orchestration_v2_projection_messages(message_id TEXT PRIMARY KEY,thread_id TEXT,role TEXT,streaming INTEGER,created_at TEXT,updated_at TEXT,payload_json TEXT);
CREATE TABLE orchestration_v2_projection_runs(run_id TEXT PRIMARY KEY,thread_id TEXT,status TEXT,requested_at TEXT,completed_at TEXT,payload_json TEXT);
CREATE TABLE orchestration_v2_projection_subagents(subagent_id TEXT PRIMARY KEY,thread_id TEXT,child_thread_id TEXT,updated_at TEXT,payload_json TEXT);
CREATE TABLE orchestration_v2_projection_turn_items(turn_item_id TEXT PRIMARY KEY,thread_id TEXT,type TEXT,status TEXT,updated_at TEXT,payload_json TEXT);").unwrap();
        let store = Self { file, db };
        store.thread("parent", None, None, "2026-11-08T10:00:00.000Z");
        store.thread("child", Some("parent"), None, "2026-11-08T10:00:00.000Z");
        store.message(
            "parent",
            "message-parent",
            "Keep the jewels searchable.",
            "2026-11-08T10:00:00.000Z",
            false,
        );
        store.message(
            "child",
            "message-child",
            "Keep the jewels searchable.",
            "2026-11-08T10:00:00.000Z",
            false,
        );
        store
    }
    pub fn thread(&self, id: &str, parent: Option<&str>, archive: Option<&str>, at: &str) {
        let payload = json!({"id":id,"projectId":"project","title":"Local jewels","lineage":{"parentThreadId":parent,"relationshipToParent":parent.map(|_|"subagent")},"createdAt":at,"updatedAt":at,"archivedAt":archive,"deletedAt":null});
        self.db.execute("INSERT OR REPLACE INTO orchestration_v2_projection_threads VALUES(?1,'project','Local jewels',?2,?2,?3,NULL,?4)",params![id,at,archive,payload.to_string()]).unwrap();
    }
    #[allow(dead_code, reason = "used by the conversation presence scenarios")]
    pub fn unproject_thread(&self, id: &str) {
        self.db
            .execute(
                "DELETE FROM orchestration_v2_projection_threads WHERE thread_id=?1",
                [id],
            )
            .unwrap();
    }
    pub fn message(&self, thread: &str, id: &str, text: &str, at: &str, streaming: bool) {
        let payload = json!({"id":id,"threadId":thread,"role":"assistant","text":text,"createdAt":at,"updatedAt":at,"streaming":streaming,"runId":null,"nodeId":null,"attachments":[],"createdBy":"agent","creationSource":"provider"});
        self.db.execute("INSERT INTO orchestration_v2_projection_messages VALUES(?1,?2,'assistant',?3,?4,?4,?5) ON CONFLICT(message_id) DO UPDATE SET streaming=excluded.streaming,updated_at=excluded.updated_at,payload_json=excluded.payload_json",params![id,thread,streaming,at,payload.to_string()]).unwrap();
    }
    pub fn native_records(&self) {
        let at = "2026-11-08T10:00:00.000Z";
        for (id, role, text, actor, origin) in [
            (
                "native-user-message",
                "user",
                "Useritemtoken text.",
                "user",
                "web",
            ),
            (
                "native-assistant-message",
                "assistant",
                "Assistantitemtoken text.",
                "agent",
                "provider",
            ),
        ] {
            let mut payload = json!({"id":id,"threadId":"child","runId":"native-run","nodeId":null,"role":role,"text":text,"createdAt":at,"updatedAt":at,"streaming":false,"attachments":[],"createdBy":actor,"creationSource":origin});
            if role == "user" {
                // Composer context T3 stores beside what the person wrote:
                // terminal lines they attached and a review comment on a diff.
                payload["context"] = json!({"version":1,"records":[
                    {"version":1,"contextId":"terminal-fixture","kind":"terminal","label":"Terminal 1 lines 1-2","terminalId":"terminal-1","terminalLabel":"Terminal 1","lineStart":1,"lineEnd":2,"text":"Terminalonlytoken printed."},
                    {"version":1,"contextId":"review-comment-fixture","kind":"review-comment","label":"src/lib.rs lines 3-4","sectionId":"working-tree","sectionTitle":"Working tree","filePath":"src/lib.rs","startIndex":3,"endIndex":4,"rangeLabel":"Lines 3-4","text":"Keep this comment.","diff":"+Diffonlytoken added line"}
                ]});
            }
            self.db.execute("INSERT INTO orchestration_v2_projection_messages VALUES(?1,'child',?2,0,?3,?3,?4)",params![id,role,at,payload.to_string()]).unwrap();
        }
        let run = json!({"id":"native-run","threadId":"child","ordinal":1,"providerInstanceId":"fixture","modelSelection":{"instanceId":"fixture","model":"fixture"},"providerThreadId":null,"userMessageId":"native-user-message","rootNodeId":null,"activeAttemptId":null,"status":"completed","requestedAt":at,"startedAt":at,"completedAt":at,"checkpointId":null,"contextHandoffId":null});
        self.db.execute("INSERT INTO orchestration_v2_projection_runs VALUES('native-run','child','completed',?1,?1,?2)",params![at,run.to_string()]).unwrap();
        let subagent = json!({"id":"native-subagent","threadId":"child","runId":"native-run","parentNodeId":"root","origin":"provider_native","createdBy":"agent","driver":"codex","providerInstanceId":"fixture","providerThreadId":null,"childThreadId":null,"nativeTaskRef":null,"prompt":"Fixture work","title":null,"model":null,"status":"completed","result":"Subagenttoken result.","startedAt":at,"completedAt":at,"updatedAt":at});
        self.db.execute("INSERT INTO orchestration_v2_projection_subagents VALUES('native-subagent','child',NULL,?1,?2)",params![at,subagent.to_string()]).unwrap();
        for (ordinal, kind, text) in [
            (0, "user_message", "Useritemtoken text."),
            (1, "assistant_message", "Assistantitemtoken text."),
            (2, "command_execution", "Toolitemtoken output."),
            (3, "reasoning", "Reasonitemtoken text."),
        ] {
            let id = format!("native-item-{ordinal}");
            let mut item = json!({"id":id,"threadId":"child","runId":"native-run","nodeId":null,"providerThreadId":null,"providerTurnId":null,"nativeItemRef":null,"parentItemId":null,"ordinal":ordinal,"status":"completed","title":null,"startedAt":at,"completedAt":at,"updatedAt":at,"type":kind});
            if kind == "command_execution" {
                item["input"] = json!("fixture command");
                item["output"] = json!(text);
                item["exitCode"] = json!(0);
            } else {
                item["text"] = json!(text);
            }
            if kind == "user_message" {
                item["messageId"] = json!("native-user-message");
                item["createdBy"] = json!("user");
                item["creationSource"] = json!("web");
                item["inputIntent"] = json!("turn_start");
                item["attachments"] = json!([]);
            }
            if kind == "assistant_message" {
                item["messageId"] = json!("native-assistant-message");
                item["streaming"] = json!(false);
            }
            if kind == "reasoning" {
                item["streaming"] = json!(false);
            }
            self.db.execute("INSERT INTO orchestration_v2_projection_turn_items VALUES(?1,'child',?2,'completed',?3,?4)",params![id,kind,at,item.to_string()]).unwrap();
        }
    }
    /// T3 changes a row of each of its run, subagent and turn-item tables
    /// for `child`, and nothing else of it.
    #[allow(dead_code, reason = "used by the history scenarios")]
    pub fn rewrite_runs_subagents_and_turn_items(&self) {
        let at = "2026-11-08T11:00:00.000Z";
        self.db.execute("UPDATE orchestration_v2_projection_runs SET completed_at=?1,payload_json=json_set(payload_json,'$.completedAt',?1) WHERE thread_id='child'",[at]).unwrap();
        self.db.execute("UPDATE orchestration_v2_projection_subagents SET updated_at=?1,payload_json=json_set(payload_json,'$.result','Rewrittentoken result.','$.updatedAt',?1) WHERE thread_id='child'",[at]).unwrap();
        self.db.execute("UPDATE orchestration_v2_projection_turn_items SET updated_at=?1,payload_json=json_set(payload_json,'$.output','Rewrittentoken output.','$.updatedAt',?1) WHERE thread_id='child' AND type='command_execution'",[at]).unwrap();
    }
    /// T3's run, subagent and turn-item tables are gone.
    #[allow(dead_code, reason = "used by the history scenarios")]
    pub fn remove_runs_subagents_and_turn_items(&self) {
        self.db.execute_batch("DROP TABLE orchestration_v2_projection_runs; DROP TABLE orchestration_v2_projection_subagents; DROP TABLE orchestration_v2_projection_turn_items;").unwrap();
    }
    #[allow(
        dead_code,
        reason = "completed turns are used by the shared relay scenarios"
    )]
    pub fn turn(&self, thread: &str, id: &str, order: u64, human: &str, agent: &str) {
        self.terminal_turn(thread, id, order, human, Some(agent), "completed", false);
    }

    #[allow(
        dead_code,
        reason = "failed turns are used by the conversation scenarios"
    )]
    pub fn failed_turn(&self, thread: &str, id: &str, order: u64, human: &str) {
        self.terminal_turn(thread, id, order, human, None, "failed", false);
    }

    #[allow(dead_code, reason = "used by the conversation answer scenarios")]
    pub fn streaming_turn(&self, thread: &str, id: &str, order: u64, human: &str, agent: &str) {
        self.terminal_turn(thread, id, order, human, Some(agent), "completed", true);
    }

    #[allow(dead_code, reason = "used by the conversation answer scenarios")]
    pub fn failed_turn_with_progress(
        &self,
        thread: &str,
        id: &str,
        order: u64,
        human: &str,
        progress: &str,
    ) {
        self.terminal_turn(thread, id, order, human, Some(progress), "failed", false);
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "native fixture dimensions are independent"
    )]
    fn terminal_turn(
        &self,
        thread: &str,
        id: &str,
        order: u64,
        human: &str,
        agent: Option<&str>,
        status: &str,
        streaming_reply: bool,
    ) {
        let at = "2026-11-08T10:00:00.000Z";
        let tx = self.db.unchecked_transaction().unwrap();
        let user = format!("{id}-user");
        let reply = format!("{id}-reply");
        for (message, role, text, ordinal) in std::iter::once((&user, "user", human, 0))
            .chain(agent.map(|text| (&reply, "assistant", text, 1)))
        {
            let streaming = role == "assistant" && streaming_reply;
            let native = json!({"id":message,"threadId":thread,"runId":id,"role":role,"text":text,"streaming":streaming,"createdAt":at,"updatedAt":at,"createdBy":if role=="user"{"user"}else{"agent"},"creationSource":if role=="user"{"web"}else{"provider"},"attachments":[]});
            tx.execute(
                "INSERT INTO orchestration_v2_projection_messages VALUES(?1,?2,?3,?4,?5,?5,?6)",
                params![message, thread, role, streaming, at, native.to_string()],
            )
            .unwrap();
            let item_id = format!("{id}-item-{ordinal}");
            let kind = if role == "user" {
                "user_message"
            } else {
                "assistant_message"
            };
            let item = json!({"id":item_id,"threadId":thread,"runId":id,"type":kind,"messageId":message,"ordinal":ordinal,"status":"completed","streaming":false,"text":text,"createdAt":at,"updatedAt":at,"completedAt":at});
            tx.execute("INSERT INTO orchestration_v2_projection_turn_items VALUES(?1,?2,?3,'completed',?4,?5)",params![item_id,thread,kind,at,item.to_string()]).unwrap();
        }
        let run = json!({"id":id,"threadId":thread,"ordinal":order,"providerInstanceId":"fixture","modelSelection":{"instanceId":"fixture","model":"fixture"},"userMessageId":user,"status":status,"requestedAt":at,"completedAt":at});
        tx.execute(
            "INSERT INTO orchestration_v2_projection_runs VALUES(?1,?2,?3,?4,?4,?5)",
            params![id, thread, status, at, run.to_string()],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    /// The messages whose id begins with `prefix` leave T3's projection, kept
    /// aside until [`T3Store::show_messages`] puts them back.
    #[allow(dead_code, reason = "used by the conversation scenarios")]
    pub fn hide_messages(&self, prefix: &str) {
        let pattern = format!("{prefix}%");
        self.db.execute_batch("CREATE TABLE IF NOT EXISTS hidden_messages AS SELECT * FROM orchestration_v2_projection_messages WHERE 0;").unwrap();
        self.db.execute("INSERT INTO hidden_messages SELECT * FROM orchestration_v2_projection_messages WHERE message_id LIKE ?1", [&pattern]).unwrap();
        self.db
            .execute(
                "DELETE FROM orchestration_v2_projection_messages WHERE message_id LIKE ?1",
                [&pattern],
            )
            .unwrap();
    }
    /// Puts back the messages [`T3Store::hide_messages`] kept aside; they
    /// stay kept aside, so putting them back again is harmless.
    #[allow(dead_code, reason = "used by the conversation scenarios")]
    pub fn show_messages(&self) {
        self.db.execute_batch("INSERT OR REPLACE INTO orchestration_v2_projection_messages SELECT * FROM hidden_messages;").unwrap();
    }
    /// Keep the WAL writer's exclusive lock across commits, so a native
    /// wake can reach a relay that cannot read the just-written data.
    #[allow(dead_code, reason = "used by the conversation scenarios")]
    pub fn exclude_readers(&self) {
        self.db
            .execute_batch("PRAGMA locking_mode=EXCLUSIVE")
            .unwrap();
    }

    /// Return to shared WAL access without writing another projection row.
    #[allow(dead_code, reason = "used by the conversation scenarios")]
    pub fn let_readers_in(&self) {
        self.db
            .execute_batch(
                "PRAGMA locking_mode=NORMAL; BEGIN; SELECT 1 FROM effect_sql_migrations; COMMIT",
            )
            .unwrap();
    }

    /// The agent writes `text` in the running turn `run` of `thread`: an
    /// assistant message T3 records in that turn, `ordinal` among its items.
    #[allow(dead_code, reason = "used by the conversation scenarios")]
    pub fn agent_writes(&self, thread: &str, run: &str, ordinal: u64, text: &str) {
        let at = "2026-11-08T10:00:00.000Z";
        let id = format!("{run}-written-{ordinal}");
        let message = json!({"id":id,"threadId":thread,"runId":run,"role":"assistant","text":text,"streaming":false,"createdAt":at,"updatedAt":at,"createdBy":"agent","creationSource":"provider","attachments":[]});
        let item = json!({"id":id,"threadId":thread,"runId":run,"type":"assistant_message","messageId":id,"ordinal":ordinal,"status":"completed","streaming":false,"text":text,"createdAt":at,"updatedAt":at,"completedAt":at});
        let tx = self.db.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO orchestration_v2_projection_messages VALUES(?1,?2,'assistant',0,?3,?3,?4)",
            params![id, thread, at, message.to_string()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO orchestration_v2_projection_turn_items VALUES(?1,?2,'assistant_message','completed',?3,?4)",
            params![id, thread, at, item.to_string()],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    /// Other work on the computer: `count` finished threads beside the
    /// scenario's own, each saying `said`, and one more whose answer is
    /// still streaming; [`Elsewhere::stream`] keeps rewriting it until
    /// [`Elsewhere::settle`], after which it says `said` too.
    #[allow(dead_code, reason = "shared by the source-reading scenarios")]
    pub fn elsewhere(&self, count: usize, said: &str) -> Elsewhere {
        let tx = self.db.unchecked_transaction().unwrap();
        let threads = (0..count)
            .map(|n| (format!("elsewhere-{n:04}"), false))
            .chain([(STREAMING.to_owned(), true)]);
        for (thread, streaming) in threads {
            let payload = json!({"id":thread,"projectId":"project","title":"Elsewhere","lineage":{"parentThreadId":null,"relationshipToParent":null},"createdAt":ELSEWHERE_AT,"updatedAt":ELSEWHERE_AT,"archivedAt":null,"deletedAt":null});
            tx.execute("INSERT INTO orchestration_v2_projection_threads VALUES(?1,'project','Elsewhere',?2,?2,NULL,NULL,?3)",params![thread,ELSEWHERE_AT,payload.to_string()]).unwrap();
            let message = if streaming {
                STREAMING_ANSWER.to_owned()
            } else {
                format!("{thread}-message")
            };
            let payload = json!({"id":message,"threadId":thread,"role":"assistant","text":said,"createdAt":ELSEWHERE_AT,"updatedAt":ELSEWHERE_AT,"streaming":streaming,"runId":null,"nodeId":null,"attachments":[],"createdBy":"agent","creationSource":"provider"});
            tx.execute("INSERT INTO orchestration_v2_projection_messages VALUES(?1,?2,'assistant',?3,?4,?4,?5)",params![message,thread,streaming,ELSEWHERE_AT,payload.to_string()]).unwrap();
        }
        tx.commit().unwrap();
        Elsewhere {
            file: self.file.clone(),
            said: said.to_owned(),
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            writer: None,
            deltas: 0,
        }
    }
    /// T3's server descriptor, served by the test with the content it had.
    /// Every read of T3's durable source opens the descriptor first, so while
    /// the gate is held each source read becomes a point the test sees, and
    /// can hold.
    #[allow(dead_code, reason = "shared by the source-reading scenarios")]
    pub fn gate(&self) -> Gate {
        let state = self.file.parent().unwrap();
        Gate::serve(
            state.join("server-runtime.json"),
            state.parent().unwrap().join("descriptor-gate"),
        )
    }
    pub fn remove_threads(&self) {
        self.db.execute_batch("DELETE FROM orchestration_v2_projection_messages;DELETE FROM orchestration_v2_projection_threads;").unwrap();
    }
    pub fn active_child(&self, append: bool) {
        if append {
            self.message(
                "child",
                "hidden-update",
                "Unreported child progress.",
                "2026-11-08T11:00:00.000Z",
                false,
            );
        }
    }
    /// T3's database removed: the connection closed and the database, its
    /// WAL and its shared memory deleted.
    pub fn remove(self) {
        let Self { file, db } = self;
        db.close().unwrap();
        for suffix in ["", "-wal", "-shm"] {
            let path = std::path::PathBuf::from(format!("{}{suffix}", file.display()));
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    pub fn complete(&self) {
        self.message(
            "child",
            "completion",
            "Finished the next turn.",
            "2026-11-08T11:00:00.000Z",
            false,
        );
    }
    pub fn archived_after_gap(&self) {
        self.thread(
            "archived-gap",
            None,
            Some("2026-11-08T12:00:00.000Z"),
            "2026-11-08T12:00:00.000Z",
        );
        self.message(
            "archived-gap",
            "archived-message",
            "Archivedgap treasure.",
            "2026-11-08T12:00:00.000Z",
            false,
        );
    }
    pub fn break_shape(&self) {
        self.db.execute_batch("ALTER TABLE orchestration_v2_projection_messages RENAME COLUMN payload_json TO incompatible_payload;").unwrap();
    }
    pub fn restore_shape(&self) {
        self.db.execute_batch("ALTER TABLE orchestration_v2_projection_messages RENAME COLUMN incompatible_payload TO payload_json;").unwrap();
    }
    pub fn release_and_migration(&self) {
        self.db
            .execute("UPDATE effect_sql_migrations SET migration_id=1000", [])
            .unwrap();
        std::fs::write(
            self.file.parent().unwrap().join("server-runtime.json"),
            json!({"serverVersion":"future-compatible-release"}).to_string(),
        )
        .unwrap();
    }
}

const ELSEWHERE_AT: &str = "2026-11-08T09:00:00.000Z";
const STREAMING: &str = "elsewhere-streaming";
const STREAMING_ANSWER: &str = "elsewhere-answer";

/// The other work of [`T3Store::elsewhere`]; dropping it settles its stream.
pub struct Elsewhere {
    file: PathBuf,
    said: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    writer: Option<std::thread::JoinHandle<()>>,
    deltas: u64,
}
#[allow(dead_code, reason = "shared by the source-reading scenarios")]
impl Elsewhere {
    /// The streaming answer keeps changing, as a provider's deltas do,
    /// until [`Self::settle`].
    pub fn stream(&mut self) {
        if self.writer.is_some() {
            return;
        }
        self.stop.store(false, std::sync::atomic::Ordering::Relaxed);
        let stopping = self.stop.clone();
        let file = self.file.clone();
        let said = self.said.clone();
        self.writer = Some(std::thread::spawn(move || {
            let db = Connection::open(&file).unwrap();
            let mut n = 0_u64;
            while !stopping.load(std::sync::atomic::Ordering::Relaxed) {
                n += 1;
                delta(&db, &format!("{said} {n}"), true);
                // Paces the stream like a provider's deltas.
                std::thread::sleep(Duration::from_millis(1));
            }
        }));
    }
    /// One more delta of the streaming answer, now.
    pub fn nudge(&mut self) {
        self.deltas += 1;
        let db = Connection::open(&self.file).unwrap();
        delta(&db, &format!("{} nudged {}", self.said, self.deltas), true);
    }
    /// Stops the stream; its answer is finished and says what the others say.
    pub fn settle(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(writer) = self.writer.take() {
            writer.join().unwrap();
        }
        let db = Connection::open(&self.file).unwrap();
        delta(&db, &self.said, false);
    }
}
impl Drop for Elsewhere {
    /// The stream always stops and its writer is reaped; a failing test
    /// only skips the finished answer, and nothing here panics again while
    /// it unwinds.
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(writer) = self.writer.take() {
                let _ = writer.join();
            }
        } else {
            self.settle();
        }
    }
}
fn delta(db: &Connection, text: &str, streaming: bool) {
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    let payload = json!({"id":STREAMING_ANSWER,"threadId":STREAMING,"role":"assistant","text":text,"createdAt":ELSEWHERE_AT,"updatedAt":ELSEWHERE_AT,"streaming":streaming,"runId":null,"nodeId":null,"attachments":[],"createdBy":"agent","creationSource":"provider"});
    db.execute(
        "UPDATE orchestration_v2_projection_messages SET streaming=?1,payload_json=?2 WHERE message_id=?3",
        params![streaming, payload.to_string(), STREAMING_ANSWER],
    )
    .unwrap();
}

#[derive(Default)]
struct Gating {
    /// Held source reads served so far.
    served: usize,
    /// Whether a source read is waiting at the gate.
    waiting: bool,
    /// Whether reads wait at the gate until admitted.
    held: bool,
    /// Reads admitted while the gate is held.
    admitted: usize,
    /// `served` when the gate was last held.
    armed_at: usize,
    /// The pipe the server is opening for the next held read, while it waits
    /// for a reader of it.
    opening: Option<PathBuf>,
    stop: bool,
}

/// See [`T3Store::gate`]. Open, the descriptor is a regular file and every
/// read passes at once. Held, the descriptor is a link to a named pipe of the
/// next read's own, so the process reading stands still at the start of a
/// source read with everything before it done, until it is admitted. The
/// pipes and the link they are reached through live in a directory of their
/// own, so T3's state directory, which the source watches, changes only when
/// the gate is held or opened. Dropping it restores the descriptor as it was.
///
/// A held read ends on its pipe's end of file, and macOS can leave a reader
/// asleep in `read` when the pipe's last writer closes. After each held read
/// the server therefore opens the pipe for writing and closes it again for as
/// long as a reader still holds the pipe: each close signals end of file once
/// more, and the read counts as served only once no reader holds the pipe.
pub struct Gate {
    descriptor: PathBuf,
    pipes: PathBuf,
    content: Option<Vec<u8>>,
    /// What every read of the descriptor receives.
    served: Vec<u8>,
    shared: std::sync::Arc<(std::sync::Mutex<Gating>, std::sync::Condvar)>,
    server: Option<std::thread::JoinHandle<()>>,
}
/// The `n`th held read's named pipe.
fn make_pipe(pipes: &std::path::Path, n: usize) -> PathBuf {
    let pipe = pipes.join(format!("pipe-{n}"));
    nix::unistd::mkfifo(
        &pipe,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    pipe
}
/// The descriptor, through the current link, points to `pipe` from now on.
fn point(pipes: &std::path::Path, pipe: &std::path::Path) {
    let link = pipes.join("next");
    std::os::unix::fs::symlink(pipe, &link).unwrap();
    std::fs::rename(&link, pipes.join("current")).unwrap();
}
/// The descriptor becomes a link through the current link, in one rename.
fn link_descriptor(pipes: &std::path::Path, descriptor: &std::path::Path) {
    let next = pipes.join("descriptor-link");
    std::os::unix::fs::symlink(pipes.join("current"), &next).unwrap();
    std::fs::rename(&next, descriptor).unwrap();
}
/// The descriptor becomes a regular file holding `content`, in one rename.
fn file_descriptor(pipes: &std::path::Path, descriptor: &std::path::Path, content: &[u8]) {
    let next = pipes.join("descriptor-file");
    std::fs::write(&next, content).unwrap();
    std::fs::rename(&next, descriptor).unwrap();
}
/// Whether a reader still holds `pipe`. Opening it for writing without
/// waiting succeeds only then, and closing that writer signals end of file to
/// the reader once more.
fn reader_holds(pipe: &std::path::Path) -> bool {
    standing_writer(pipe).is_some()
}
/// A writer of `pipe` when a reader already stands at it, opened without
/// waiting; `None` when no reader is there.
fn standing_writer(pipe: &std::path::Path) -> Option<std::fs::File> {
    let opened = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(pipe);
    if opened.as_ref().err().and_then(std::io::Error::raw_os_error)
        == Some(nix::errno::Errno::ENXIO as i32)
    {
        return None;
    }
    Some(opened.unwrap())
}
#[allow(dead_code, reason = "shared by the source-reading scenarios")]
impl Gate {
    fn serve(descriptor: PathBuf, pipes: PathBuf) -> Self {
        let content = std::fs::read(&descriptor).ok();
        let served = content.clone().unwrap_or_else(|| b"{}".to_vec());
        std::fs::create_dir_all(&pipes).unwrap();
        file_descriptor(&pipes, &descriptor, &served);
        let shared = std::sync::Arc::new((
            std::sync::Mutex::new(Gating::default()),
            std::sync::Condvar::new(),
        ));
        let serving = shared.clone();
        let path = pipes.clone();
        let bytes = served.clone();
        let mut pipe = make_pipe(&path, 0);
        point(&path, &pipe);
        let server = std::thread::spawn(move || {
            let (lock, changed) = &*serving;
            for n in 1.. {
                // Held, the server waits for the next read at its pipe. Open,
                // it still serves a read that reached the pipe before the
                // gate opened: several reads can queue while it is held, and
                // opening lets every one of them through.
                let standing = {
                    let mut gating = lock.lock().unwrap();
                    loop {
                        if gating.stop {
                            return;
                        }
                        if gating.held {
                            gating.opening = Some(pipe.clone());
                            break None;
                        }
                        if let Some(writer) = standing_writer(&pipe) {
                            break Some(writer);
                        }
                        gating = changed
                            .wait_timeout(gating, Duration::from_millis(10))
                            .unwrap()
                            .0;
                    }
                };
                // Blocks until a reader opens this read's pipe.
                let mut writer = match standing {
                    Some(writer) => writer,
                    None => std::fs::OpenOptions::new().write(true).open(&pipe).unwrap(),
                };
                // Later readers wait at the next read's pipe.
                let next = make_pipe(&path, n);
                point(&path, &next);
                let mut gating = lock.lock().unwrap();
                gating.opening = None;
                gating.waiting = true;
                changed.notify_all();
                while gating.held && gating.admitted == 0 && !gating.stop {
                    gating = changed.wait(gating).unwrap();
                }
                if gating.admitted > 0 {
                    gating.admitted -= 1;
                }
                drop(gating);
                // A reader that ended while it waited is no longer there.
                let _ = std::io::Write::write_all(&mut writer, &bytes);
                drop(writer);
                while reader_holds(&pipe) {
                    std::thread::yield_now();
                }
                std::fs::remove_file(&pipe).unwrap();
                let mut gating = lock.lock().unwrap();
                gating.waiting = false;
                gating.served += 1;
                changed.notify_all();
                drop(gating);
                pipe = next;
            }
        });
        Self {
            descriptor,
            pipes,
            content,
            served,
            shared,
            server: Some(server),
        }
    }
    fn until(&self, what: &str, done: impl Fn(&Gating) -> bool) {
        let (lock, changed) = &*self.shared;
        let (gating, timeout) = changed
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(20), |gating| {
                !done(gating)
            })
            .unwrap();
        drop(gating);
        assert!(!timeout.timed_out(), "timed out waiting for {what}");
    }
    /// A server waiting for a reader at a pipe nobody reads any more is
    /// given a reader of its own, held until the server has opened the pipe.
    fn release(changed: &std::sync::Condvar, mut gating: std::sync::MutexGuard<'_, Gating>) {
        let Some(pipe) = gating.opening.clone() else {
            return;
        };
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
            .open(&pipe);
        while gating.opening.is_some() {
            gating = changed
                .wait(gating)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        drop(gating);
        drop(reader);
    }
    /// Later source reads wait at the gate.
    pub fn hold(&self) {
        let (lock, changed) = &*self.shared;
        let mut gating = lock.lock().unwrap();
        gating.held = true;
        gating.armed_at = gating.served;
        drop(gating);
        changed.notify_all();
        link_descriptor(&self.pipes, &self.descriptor);
    }
    /// Waits until a source read stands at the held gate.
    pub fn held(&self) {
        self.until("a source read at the gate", |gating| {
            gating.held && gating.waiting
        });
    }
    /// Lets the source read standing at the held gate through, and waits
    /// until that read has ended.
    pub fn admit(&self) {
        self.held();
        let served = {
            let (lock, changed) = &*self.shared;
            let mut gating = lock.lock().unwrap();
            gating.admitted += 1;
            changed.notify_all();
            gating.served
        };
        self.until("the admitted source read", |gating| gating.served > served);
    }
    /// One step of admission, decided under the gate's own lock: whether
    /// `stop` holds, given how many reads were served since the gate was last
    /// held. When it does not and a read stands at the gate, that read is let
    /// through and the step waits until it has ended. A read that arrives once
    /// `stop` holds is never let through: it stays held. Returns whether
    /// `stop` held.
    pub fn admit_unless(&self, stop: impl Fn(usize) -> bool) -> bool {
        let served = {
            let (lock, changed) = &*self.shared;
            let mut gating = lock.lock().unwrap();
            if stop(gating.served - gating.armed_at) {
                return true;
            }
            if !(gating.held && gating.waiting) {
                return false;
            }
            gating.admitted += 1;
            changed.notify_all();
            gating.served
        };
        self.until("the admitted source read", |gating| gating.served > served);
        false
    }
    /// Every source read passes at once again: later reads find the
    /// descriptor a regular file.
    pub fn open(&self) {
        file_descriptor(&self.pipes, &self.descriptor, &self.served);
        let (lock, changed) = &*self.shared;
        let mut gating = lock.lock().unwrap();
        gating.held = false;
        changed.notify_all();
        Self::release(changed, gating);
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        {
            let (lock, changed) = &*self.shared;
            let mut gating = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            gating.stop = true;
            gating.held = false;
            changed.notify_all();
            Self::release(changed, gating);
        }
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
        let _ = std::fs::remove_file(&self.descriptor);
        let _ = std::fs::remove_dir_all(&self.pipes);
        if let Some(content) = &self.content {
            let restored = std::fs::write(&self.descriptor, content);
            // While a failing test unwinds, a second panic would abort it.
            if !std::thread::panicking() {
                restored.unwrap();
            }
        }
    }
}

pub fn computer(env: &TestEnv, id: &str) {
    std::fs::create_dir_all(env.instrukt_home().join("state")).unwrap();
    std::fs::write(env.instrukt_home().join("state/machine-id"), id).unwrap();
}
pub fn mirror(env: &TestEnv) -> PathBuf {
    let file = env.home().join(".teleclaude/mirrors.db");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let db = Connection::open(&file).unwrap();
    db.execute_batch("PRAGMA user_version=11;
CREATE TABLE mirrors(id INTEGER PRIMARY KEY,computer TEXT,title TEXT,conversation_text TEXT,metadata TEXT,native_session_id TEXT,source_identity TEXT,session INTEGER,project_path TEXT,timestamp_start TEXT,timestamp_end TEXT);
CREATE TABLE mirror_sessions(id INTEGER PRIMARY KEY,session_id TEXT,initiator_session_id TEXT);
CREATE TABLE mirror_sources(id INTEGER PRIMARY KEY,mirror INTEGER,source_identity TEXT,native_session_id TEXT,agent TEXT,transcript_path TEXT,position INTEGER,archival_text TEXT);
CREATE TABLE mirror_messages(source INTEGER,ordinal INTEGER,entry_index INTEGER,block_ordinal INTEGER,timestamp TEXT,role TEXT,text TEXT);
INSERT INTO mirror_sessions VALUES(1,'archive-session',NULL);
INSERT INTO mirrors VALUES(1,'old-computer','Retained archive','The archive jewels.','{}','native-session','source-alias',1,'/project','2026-01-01T00:00:00Z',NULL);
INSERT INTO mirror_sources VALUES(1,1,'source-alias','native-session','claude','/gone/transcript',0,'The archive jewels.');").unwrap();
    file
}
/// The ids of the mirrors `mirrors` adds beside the archive mirror, id 1:
/// more than one import batch, from below zero through zero and on past it,
/// each carrying [`BATCHED_TERM`]; those up to zero carry
/// [`NONPOSITIVE_TERM`] as well.
pub fn batched_ids() -> impl Iterator<Item = i64> {
    (-5..=125).filter(|id| *id != 1)
}
pub const BATCHED_TERM: &str = "Batchtoken";
pub const NONPOSITIVE_TERM: &str = "Lowtoken";
/// A mirror after the first batch the importer cannot read: it has no title.
pub const UNREADABLE: i64 = 125;

/// The archive mirror store of [`mirror`] with the [`batched_ids`] mirrors
/// beside it, [`UNREADABLE`] among them.
pub fn mirrors(env: &TestEnv) -> PathBuf {
    let file = mirror(env);
    let db = Connection::open(&file).unwrap();
    for id in batched_ids() {
        let title = (id != UNREADABLE).then(|| format!("Batched {id}"));
        let low = if id <= 0 { NONPOSITIVE_TERM } else { "" };
        db.execute(
            "INSERT INTO mirrors(id,computer,title,conversation_text,metadata) VALUES(?1,'old-computer',?2,?3,'{}')",
            params![id, title, format!("{BATCHED_TERM} {low} mirror {id}.")],
        )
        .unwrap();
    }
    file
}

/// Gives mirror `id` a title, or takes it away so the importer cannot read it.
pub fn retitle(file: &Path, id: i64, title: Option<&str>) {
    Connection::open(file)
        .unwrap()
        .execute(
            "UPDATE mirrors SET title=?2 WHERE id=?1",
            params![id, title],
        )
        .unwrap();
}

pub fn configure_cluster(env: &TestEnv, url: &str) {
    std::fs::write(env.instrukt_home().join("instrukt.yaml"),format!("schema_version: 1\nmodules: {{events: false, todos: false}}\nrunner: {{runtime: t3}}\nredis:\n  cluster:\n    url: {url}\n")).unwrap();
}
pub struct Runner(Child);
impl Drop for Runner {
    fn drop(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}
impl Runner {
    /// The runner process this fixture started, still running and the same
    /// process (its pid).
    pub fn running(&mut self) -> Option<u32> {
        self.0.try_wait().unwrap().is_none().then(|| self.0.id())
    }
}
pub fn runner(env: &TestEnv) -> Runner {
    let mut child = env
        .command(
            std::path::Path::new(env!("CARGO_BIN_EXE_instrukt")),
            env.home(),
        )
        .arg("run")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // The query worker publishes readiness only after opening its request queue.
    let config: Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(env.instrukt_home().join("instrukt.yaml")).unwrap(),
    )
    .unwrap();
    let mut conn = redis::Client::open(config["redis"]["cluster"]["url"].as_str().unwrap())
        .unwrap()
        .get_connection()
        .unwrap();
    crate::common::wait_until("history responder startup", Duration::from_secs(15), || {
        if redis::cmd("EXISTS")
            .arg(format!(
                "instrukt:history:ready:{}",
                std::fs::read_to_string(env.instrukt_home().join("state/machine-id"))
                    .unwrap()
                    .trim()
            ))
            .query::<bool>(&mut conn)
            .unwrap()
        {
            return true;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "history runner stopped before readiness"
        );
        false
    });
    Runner(child)
}
