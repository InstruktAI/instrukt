//! The fake T3 host boundary shared by runner and work tests.
#![allow(clippy::unwrap_used, dead_code, reason = "isolated test setup")]
#[path = "t3.rs"]
mod t3;
use crate::common::TestEnv;
use std::path::Path;
pub use t3::FakeT3;

pub struct FakeRuntime {
    pub name: String,
    fake: FakeT3,
}
impl FakeRuntime {
    pub fn new(env: &TestEnv, _name: &str, _link: bool) -> Self {
        Self {
            name: "t3".into(),
            fake: FakeT3::create(env),
        }
    }
    pub fn open_session(&self, id: &str) {
        let json = serde_json::json!({"threadId":id,"projectId":"test","title":"Developer thread","checkout":"test","status":"waiting","providerInstanceId":"codex","model":"gpt-test","worktreePath":null,"branch":null,"pendingRequestCount":0,"archived":false,"messages":[]});
        std::fs::write(
            self.state_dir().join(format!("{id}.json")),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
    }
    pub fn start_without_printing_ids(&self) {
        self.fake.flag("silent-start", true);
    }
    pub fn kill_runner_on_next_start(&self) {
        self.fake.flag("die-after-start", true);
    }
    pub fn set_status(&self, id: &str, value: &str) {
        let native = match value {
            "ended ok" => "completed",
            "ended failed" => "failed",
            "needs-input" => "waiting",
            other => other,
        };
        self.fake.set_status(id, native);
    }
    pub fn finish(&self, id: &str, status: &str, text: &str) {
        self.fake.finish(id, status, text);
    }
    pub fn consume(&self, id: &str) {
        self.fake.consume(id);
    }
    pub fn notifications(&self) -> Vec<String> {
        std::fs::read_to_string(self.state_dir().join("notifications"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    pub fn input_log(&self, id: &str) -> String {
        self.fake.input_log(id)
    }
    pub fn message_count(&self, id: &str) -> usize {
        self.fake.message_count(id)
    }
    pub fn calls(&self) -> Vec<String> {
        self.fake.calls()
    }
    pub fn opening(&self, id: &str) -> String {
        self.fake.message(id, 0)
    }
    pub fn started_count(&self) -> usize {
        self.fake.threads().len()
    }
    pub fn sent_count(&self, id: &str) -> usize {
        self.fake.message_count(id) - 1
    }
    pub fn sent(&self, id: &str, n: usize) -> String {
        self.fake.message(id, n)
    }
    pub fn state_dir(&self) -> &Path {
        self.fake.state_dir()
    }
}
pub fn global_layer(env: &TestEnv, modules: &str, runtimes: &[&FakeRuntime]) {
    global_layer_with(env, modules, runtimes, "");
}
pub fn global_layer_with(env: &TestEnv, modules: &str, runtimes: &[&FakeRuntime], extra: &str) {
    let file = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let notify = runtimes
        .first()
        .map(|rt| {
            format!(
                "  notify: >-\n    printf '%s\\n' >> '{}'\n",
                rt.state_dir().join("notifications").display()
            )
        })
        .unwrap_or_default();
    let redis = if modules.contains("events: true") {
        format!("redis:\n  cluster:\n    url: '{}'\n", env.redis_url())
    } else {
        String::new()
    };
    std::fs::write(
        file,
        format!(
            "schema_version: 1\nmodules: {{{modules}}}\nrunner:\n  runtime: t3\n{notify}{redis}{extra}"
        ),
    )
    .unwrap();
}

/// A native launch result supplied to instrukt's record-only entry boundary.
pub struct NativePlacement<'a> {
    pub cwd: &'a Path,
    pub slug: &'a str,
    pub role: &'a str,
    pub session: &'a str,
}
pub fn placement_evidence(env: &TestEnv, request: &NativePlacement<'_>) -> String {
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let (id, project): (String, String) = db
        .query_row(
            "SELECT id,project FROM todos WHERE project='demo' AND slug=?1 AND status='active'",
            [request.slug],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    use instrukt_core::state_db::rusqlite::OptionalExtension as _;
    let active:Option<String>=db.query_row("SELECT semantic_key FROM session_bindings WHERE todo_id=?1 AND role=?2 AND released_at IS NULL ORDER BY attempt DESC LIMIT 1",instrukt_core::state_db::rusqlite::params![id,request.role],|row|row.get(0)).optional().unwrap();
    let number: u64 = db
        .query_row(
            "SELECT COALESCE(MAX(attempt),0)+1 FROM session_bindings WHERE todo_id=?1 AND role=?2",
            instrukt_core::state_db::rusqlite::params![id, request.role],
            |row| row.get(0),
        )
        .unwrap();
    let key = active
        .unwrap_or_else(|| instrukt_runner::keyed::todo_key(&project, &id, request.role, number));
    let head = env.git(request.cwd, ["rev-parse", "HEAD"]);
    let branch = env.git(request.cwd, ["branch", "--show-current"]);
    serde_json::json!({"session":format!("t3:{}",request.session),"key":key,"worktree":request.cwd,"branch":branch,"head":head,"source":"native launch/recorded-workspace fixture plus Git HEAD"}).to_string()
}
