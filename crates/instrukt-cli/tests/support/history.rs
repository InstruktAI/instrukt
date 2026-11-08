#![allow(clippy::unwrap_used, reason = "temporary source fixtures")]
use crate::common::TestEnv;
use instrukt_core::state_db::rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Child,
    time::{Duration, Instant},
};

pub struct T3Store {
    pub file: PathBuf,
    db: Connection,
}
impl T3Store {
    pub fn new(env: &TestEnv) -> Self {
        computer(env, "aaaaaaaaaaaaaaaa");
        std::fs::write(
            env.instrukt_home().join("instrukt.yaml"),
            "schema_version: 1\nrunner: {runtime: t3}\n",
        )
        .unwrap();
        let file = env.home().join(".t3/userdata/statev2.sqlite");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let db = Connection::open(&file).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL;
CREATE TABLE effect_sql_migrations(migration_id INTEGER PRIMARY KEY,name TEXT);
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
    pub fn message(&self, thread: &str, id: &str, text: &str, at: &str, streaming: bool) {
        let payload = json!({"id":id,"threadId":thread,"role":"assistant","text":text,"createdAt":at,"updatedAt":at,"streaming":streaming});
        self.db.execute("INSERT OR REPLACE INTO orchestration_v2_projection_messages VALUES(?1,?2,'assistant',?3,?4,?4,?5)",params![id,thread,streaming,at,payload.to_string()]).unwrap();
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
pub fn configure_cluster(env: &TestEnv, url: &str) {
    std::fs::write(env.instrukt_home().join("instrukt.yaml"),format!("schema_version: 1\nmodules: {{events: true, todos: false}}\nrunner: {{runtime: t3}}\nredis:\n  cluster:\n    url: {url}\n")).unwrap();
}
pub struct Runner(Child);
impl Drop for Runner {
    fn drop(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
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
    let deadline = Instant::now() + Duration::from_secs(15);
    let config: Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(env.instrukt_home().join("instrukt.yaml")).unwrap(),
    )
    .unwrap();
    let mut conn = redis::Client::open(config["redis"]["cluster"]["url"].as_str().unwrap())
        .unwrap()
        .get_connection()
        .unwrap();
    while !redis::cmd("EXISTS")
        .arg(format!(
            "instrukt:history:ready:{}",
            std::fs::read_to_string(env.instrukt_home().join("state/machine-id"))
                .unwrap()
                .trim()
        ))
        .query::<bool>(&mut conn)
        .unwrap()
    {
        assert!(
            child.try_wait().unwrap().is_none(),
            "history runner stopped before readiness"
        );
        assert!(Instant::now() < deadline, "history responder did not start");
        std::thread::yield_now();
    }
    Runner(child)
}
