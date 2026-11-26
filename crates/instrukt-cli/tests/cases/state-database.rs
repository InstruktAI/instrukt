//! `project/instrukt/spec/feature/work/state-database`: the work record and
//! the runner's state in one transactional database.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nix::fcntl::{FcntlArg, fcntl};
use nix::libc;

use instrukt_core::home::Home;
use instrukt_core::state_db::rusqlite::Connection;
use instrukt_core::state_db::{StateDb, StateDbError, StateRead};

use crate::common;
use crate::fixtures::work as support;
use crate::fixtures::workflow;
use workflow::Crew;

// spec: project/instrukt/spec/feature/work/state-database UC-SD2
#[test]
fn a_documentation_transfer_failure_leaves_the_last_completed_state() {
    an_owned_v1_database_keeps_its_existing_binding_when_attempt_records_are_introduced();
    let crew = Crew::new();
    let project = &crew.project;
    project.repo.write(
        "docs/project/spec/feature/depth.md",
        "---\ndescription: List depth.\n---\n\n# Depth — Spec\n\nLists have depth.\n",
    );
    project.repo.write(
        "docs/project/spec/feature/depth.list-depth.md",
        "---\ndescription: List depth.\n---\n\n# Depth — Spec\n\nLists keep their depth everywhere.\n",
    );
    project.repo.commit("plan list-depth's documentation");
    project.create(
        "list-depth",
        "feature",
        "Lists keep their depth everywhere.",
    );
    let before = project.ok(["show", "list-depth"]);
    let retained = retain_historical_starts(&project.env);

    // Grouping saves the parent and child in one transaction, then transfers
    // pending documentation before committing. An existing destination stops
    // that transfer deterministically, after the record writes. A directory
    // works even for a user who could overwrite a read-only file.
    std::fs::create_dir_all(
        project
            .repo
            .path()
            .join("docs/project/spec/feature/depth.nested-lists.md"),
    )
    .unwrap();
    let refused = project
        .todo([
            "create",
            "nested-lists",
            "Keep the depth of nested lists when parsing Markdown.",
            "--kind",
            "change",
            "--approved",
            "--group",
            "list-depth",
        ])
        .failure();
    insta::assert_snapshot!(
        String::from_utf8_lossy(&refused.get_output().stderr),
        @"instrukt: `list-depth` cannot become a group: its pending documentation cannot move to `nested-lists` (docs/project/spec/feature/depth.nested-lists.md: a companion of nested-lists is already there)"
    );

    let after = project.ok(["show", "list-depth"]);
    assert_retained_starts(&project.env, &retained);
    let absent = project.todo(["show", "nested-lists"]).failure();
    insta::assert_snapshot!(
        String::from_utf8_lossy(&absent.get_output().stderr),
        @"instrukt: demo has no todo `nested-lists`; list the project's work with `instrukt todo list`"
    );
    assert_eq!(
        after, before,
        "list-depth is as the last completed change left it"
    );
}

fn an_owned_v1_database_keeps_its_existing_binding_when_attempt_records_are_introduced() {
    let project = support::Project::new();
    let file = project.env.instrukt_home().join("state/instrukt.db");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let db = Connection::open(&file).unwrap();
    db.execute_batch(include_str!("../fixtures/state-v1.sql"))
        .unwrap();
    let id = "11111111111111111111111111111111";
    db.execute("INSERT INTO todos(id,project,slug,kind,status,created_at,created_by,revision,catch_up,sequence) VALUES (?1,'demo','legacy','change','active','2026-01-01T00:00:00Z','{\"person\":\"Test Developer\"}',1,'once',1)",[id]).unwrap();
    db.execute(
        "UPDATE todos SET agent='codex',thinking='slow' WHERE id=?1",
        [id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO artifacts(todo_id,name) VALUES (?1,'input')",
        [id],
    )
    .unwrap();
    let content = "Preserve the existing work record.";
    db.execute("INSERT INTO artifact_versions(todo_id,name,sequence,digest,writer,created_at,content,derived_from,grounding) VALUES (?1,'input',1,?2,'{\"person\":\"Test Developer\"}','2026-01-01T00:00:00Z',?3,'[]',?4)",instrukt_core::state_db::rusqlite::params![id,instrukt_todos::evidence::ContentDigest::of(content).to_string(),content,serde_json::to_string(&instrukt_todos::evidence::Grounding::default()).unwrap()]).unwrap();
    db.execute("INSERT INTO session_bindings(todo_id,role,session) VALUES (?1,'prepare-orchestrator','t3:legacy-driver')",[id]).unwrap();
    drop(db);
    let shown = project.ok(["show", "legacy"]);
    insta::assert_snapshot!("migrated-v1-binding", support::redact(&shown));
    let db = Connection::open(&file).unwrap();
    let version: u32 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, instrukt_core::state_db::schema_version());
}

// Historical completed records belong to UC-SD2's last completed state.
fn retain_historical_starts(env: &common::TestEnv) -> serde_json::Value {
    let file = env.instrukt_home().join("state/instrukt.db");
    let db = Connection::open(&file).unwrap();
    db.execute_batch("DROP TABLE agent_quota; DROP TABLE conversation_relay_positions; DROP TABLE conversation_links; ALTER TABLE poll_deferrals DROP COLUMN cause; CREATE TABLE runtime_starts (runtime TEXT NOT NULL,semantic_key TEXT NOT NULL,project TEXT,kind TEXT NOT NULL,subject TEXT NOT NULL,checkout TEXT NOT NULL,brief TEXT NOT NULL,request TEXT NOT NULL,begun_at TEXT NOT NULL,reported INTEGER NOT NULL,resolved_at TEXT,resolved_by TEXT,resolution TEXT,state TEXT NOT NULL,PRIMARY KEY(runtime,semantic_key)); ALTER TABLE runtime_bindings DROP COLUMN generation; ALTER TABLE runtime_bindings DROP COLUMN start_key; ALTER TABLE runtime_bindings DROP COLUMN replaced_session; ALTER TABLE runtime_bindings DROP COLUMN legacy_modes; ALTER TABLE connected_conversations DROP COLUMN display_title; DROP TABLE runtime_generations; PRAGMA user_version=4;")
        .unwrap();
    crate::common::recreate_pre_v40_listing_copies(&db);
    let checkout = env.git_repo("retained");
    let old = serde_json::json!({"key":"retained-key","brief":"retained-input","checkout":checkout.path(),"worktree":null,"selection":{"family":"gemini","provider":"account-two","model":"chosen-model","effort":"high","options":{"thinking_level":"high"}},"preferences":[{"agent":"gemini","tier":"strong"}]});
    db.execute("INSERT INTO runtime_starts(runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state) VALUES('t3','retained-key','p','run','s',?1,'retained-input',?2,'2030-01-01T00:00:00Z',0,'retry_allowed')",instrukt_core::state_db::rusqlite::params![checkout.path().to_str().unwrap(),old.to_string()]).unwrap();
    let cursor = serde_json::json!({"key":"cursor-key","brief":"cursor-input","checkout":checkout.path(),"selection":{"family":"cursor","provider":"cursor-account","model":"open-model","effort":"high","options":{"effort":"high","reasoning":"medium","fastMode":true}}});
    db.execute("INSERT INTO runtime_starts(runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state) VALUES('t3','cursor-key','p','run','s',?1,'cursor-input',?2,'2030-01-01T00:00:00Z',0,'retry_allowed')",instrukt_core::state_db::rusqlite::params![checkout.path().to_str().unwrap(),cursor.to_string()]).unwrap();
    drop(db);
    old
}

fn assert_retained_starts(env: &common::TestEnv, old: &serde_json::Value) {
    let file = env.instrukt_home().join("state/instrukt.db");
    let db = Connection::open(file).unwrap();
    let (encoded, unresolved): (String, bool) = db
        .query_row(
            "SELECT request,accepted=0 FROM runtime_bindings WHERE semantic_key='retained-key'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let current: instrukt_runner::runtime::RuntimeStart = serde_json::from_str(&encoded).unwrap();
    assert!(unresolved);
    assert_eq!(current.key, old["key"].as_str().unwrap());
    assert_eq!(current.brief, old["brief"].as_str().unwrap());
    assert_eq!(
        current.checkout,
        std::path::Path::new(old["checkout"].as_str().unwrap())
    );
    assert!(current.worktree.is_none());
    let selection = current.selection.unwrap();
    assert_eq!(
        selection.profile,
        instrukt_core::ids::ProfileName::parse("antigravity").unwrap()
    );
    assert_eq!(
        selection.kind,
        instrukt_core::config::HarnessKind::Antigravity
    );
    assert_eq!(
        selection.instance,
        instrukt_core::ids::ProviderInstanceId::parse(
            old["selection"]["provider"].as_str().unwrap()
        )
        .unwrap()
    );
    assert_eq!(selection.model, old["selection"]["model"].as_str().unwrap());
    assert_eq!(selection.effort, Some(instrukt_core::config::Effort::High));
    assert!(selection.options.is_empty());
    assert_eq!(current.preferences[0].agent, selection.profile);
    assert_eq!(
        current.preferences[0].tier,
        Some(instrukt_core::config::Tier::Strong)
    );
    let encoded: String = db
        .query_row(
            "SELECT request FROM runtime_bindings WHERE semantic_key='cursor-key'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let cursor: instrukt_runner::runtime::RuntimeStart = serde_json::from_str(&encoded).unwrap();
    let cursor = cursor.selection.unwrap();
    assert_eq!(cursor.effort, Some(instrukt_core::config::Effort::High));
    assert_eq!(
        cursor.options,
        [("fastMode".into(), serde_json::json!(true))].into()
    );
}

// spec: project/instrukt/spec/feature/work/state-database UC-SD3
#[test]
fn linked_state_shares_one_writer_and_refuses_database_file_links() {
    use std::os::unix::fs::symlink;
    use std::time::Duration;
    let project = support::Project::new();
    let env = &project.env;
    let state = env.instrukt_home().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let a = env.path("state-home-a");
    let b = env.path("state-home-b");
    symlink(env.instrukt_home(), &a).unwrap();
    std::fs::create_dir(&b).unwrap();
    symlink(&state, b.join("state")).unwrap();
    for name in ["instrukt.yaml", "agents.yaml"] {
        if env.instrukt_home().join(name).exists() {
            std::fs::copy(env.instrukt_home().join(name), b.join(name)).unwrap();
        }
    }
    let launch = |home: &std::path::Path, slug: &str| {
        env.instrukt_process(project.repo.path())
            .env("INSTRUKT_HOME", home)
            .args([
                "todo",
                "create",
                slug,
                "Retain the work.",
                "--kind",
                "change",
                "--approved",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let first = launch(&a, "linked-home").wait_with_output().unwrap();
    assert!(first.status.success(), "{first:?}");
    let status = env
        .instrukt_process(project.repo.path())
        .env("INSTRUKT_HOME", &a)
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");

    let events = env
        .instrukt_process(project.repo.path())
        .env("INSTRUKT_HOME", &a)
        .args(["events", "status", "--json"])
        .output()
        .unwrap();
    assert!(events.status.success(), "{events:?}");
    assert!(!state.join("connector-read-back.sqlite").exists());

    let lock = instrukt_core::fsx::LockGuard::acquire(
        &a.join("state/writer.lock"),
        Duration::from_secs(2),
    )
    .unwrap();
    let mut writer = launch(&b, "linked-state");
    std::thread::sleep(Duration::from_millis(200));
    assert!(writer.try_wait().unwrap().is_none());
    drop(lock);
    let written = writer.wait_with_output().unwrap();
    assert!(written.status.success(), "{written:?}");
    let sidecar = state.join("instrukt.db-shm");
    let outside = env.path("sidecar-target");
    std::fs::write(&outside, "unchanged").unwrap();
    symlink(&outside, &sidecar).unwrap();
    let refused = launch(&b, "linked-sidecar").wait_with_output().unwrap();
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("link the instrukt home or state/ directory instead")
    );
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "unchanged");
    std::fs::rename(sidecar, state.join("rejected-sidecar")).unwrap();

    let db = state.join("instrukt.db");
    let original = state.join("original.db");
    std::fs::rename(&db, &original).unwrap();
    symlink(original, &db).unwrap();
    let refused = launch(&b, "linked-file").wait_with_output().unwrap();
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains(
            db.canonicalize()
                .unwrap()
                .parent()
                .unwrap()
                .join("instrukt.db")
                .to_str()
                .unwrap()
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains("link the instrukt home or state/ directory instead"),
        "{stderr}"
    );
}

// spec: project/instrukt/spec/feature/work/state-database UC-SD4
#[test]
fn status_reports_older_and_newer_state_without_writing_it() {
    let project = support::Project::new();
    let env = &project.env;
    let state = env.instrukt_home().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let file = state.join("instrukt.db");
    let db = Connection::open(&file).unwrap();
    db.execute_batch(include_str!("../fixtures/state-v1.sql"))
        .unwrap();
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA wal_autocheckpoint=0;
         INSERT INTO events(seq,id,name,payload,envelope,logged_at)
             VALUES(1,'retained-event','fixture','{}','{}','2026-01-01T00:00:00Z');
         INSERT INTO procedure_activations(event_id,procedure,exact_home,envelope,declaration,accepted_at,state,failure)
             VALUES('retained-event','fixture','fixture','{}','{}','2026-01-01T00:00:00Z','failed','retained activation failure');"
    ).unwrap();
    let bytes = || {
        ["instrukt.db", "instrukt.db-wal", "instrukt.db-shm"]
            .map(|name| std::fs::read(state.join(name)).unwrap())
    };
    let status = |args: &[&str]| {
        let out = env.run(project.repo.path(), args).success();
        serde_json::from_slice::<serde_json::Value>(&out.get_output().stdout).unwrap()
    };
    let scan_store = state.join("connector-read-back.sqlite");
    {
        let scans = Connection::open(&scan_store).unwrap();
        scans.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE scans(company TEXT, connector TEXT, failure TEXT, retry_at INTEGER);
             CREATE TABLE seen(company TEXT, connector TEXT, conversation TEXT, message TEXT, written_us INTEGER, missed_link TEXT);
             INSERT INTO scans VALUES('fixture','slack','retained scan failure',NULL);"
        ).unwrap();
    }
    let scan_bytes = std::fs::read(&scan_store).unwrap();
    let scan_sidecars =
        ["sqlite-wal", "sqlite-shm"].map(|suffix| scan_store.with_extension(suffix));
    assert!(scan_sidecars.iter().all(|path| !path.exists()));
    let older = bytes();
    assert!(status(&["status", "--json"])["machine"]["state_database"].is_null());
    let events = status(&["events", "status", "--json"]);
    assert!(events["state_database"].is_null());
    assert_eq!(
        events["activation_failures"][0]["failure"],
        "retained activation failure"
    );
    assert_eq!(
        events["conversations"]["read_back_failures"][0]["cause"],
        "retained scan failure"
    );
    assert_eq!(std::fs::read(scan_store).unwrap(), scan_bytes);
    assert!(scan_sidecars.iter().all(|path| !path.exists()));
    assert_eq!(bytes(), older);
    let current = instrukt_core::state_db::schema_version();
    db.pragma_update(None, "user_version", current + 1).unwrap();
    let newer = bytes();
    let machine = status(&["status", "--json"])["machine"].clone();
    let reason = machine["state_database"]["reason"].as_str().unwrap();
    assert!(reason.contains(instrukt_core::INSTALLER_COMMAND));
    insta::assert_snapshot!(format!("configured: {}\nstate: {}", machine["configured"], reason.replace(file.to_str().unwrap(), "[DB]").replace(&(current + 1).to_string(), "[NEWER]").replace(&current.to_string(), "[CURRENT]")), @r"
    configured: true
    state: the state database at [DB] is at schema version [NEWER], newer than the [CURRENT] this instrukt knows; upgrade with `curl -fsSL https://github.com/InstruktAI/instrukt/releases/latest/download/install.sh | sh`
    ");
    let events = status(&["events", "status", "--json"]);
    assert_eq!(events["state_database"]["reason"], reason);
    assert!(events["admission"].is_object());
    assert_eq!(bytes(), newer);
    let refused = project
        .todo([
            "create",
            "newer-state",
            "Write work.",
            "--kind",
            "change",
            "--approved",
        ])
        .failure();
    assert!(
        String::from_utf8_lossy(&refused.get_output().stderr)
            .contains(instrukt_core::INSTALLER_COMMAND)
    );
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        current + 1
    );
}

/// How long a command that must not wait on another may take. The writer
/// lock's own wait is a minute; only a command that waited on it, or on
/// GitHub, comes near this.
const UNBLOCKED: Duration = Duration::from_secs(20);

/// Runs `command` to completion within `bound`; `None` when it is still
/// running then (it is left to finish on its own).
fn finished_within(mut command: Command, bound: Duration) -> Option<Output> {
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || done.send(child.wait_with_output().unwrap()));
    finished.recv_timeout(bound).ok()
}

// spec: project/instrukt/spec/feature/work/state-database UC-SD5
#[test]
fn local_work_proceeds_while_a_github_reflection_is_in_flight() {
    const FIRST: &str = "Started the depth parser.";
    const SECOND: &str = "Joined the nested-list cases.";
    let mut project = support::Project::new();
    // One application clock for every command, so what the listing and the
    // todo show does not depend on the time the test takes.
    project.env.set_env("INSTRUKT_NOW", "2030-01-06T09:00:00Z");
    project.create(
        "list-depth",
        "feature",
        "Lists keep their depth everywhere.",
    );
    let todo = |args: &[&str]| {
        let mut command = project.env.instrukt_process(project.repo.path());
        command.arg("todo").args(args);
        command
    };
    let read = || finished_within(todo(&["artifacts", "read", "list-depth", "log"]), UNBLOCKED);

    // The coming reflection stops at its artifact upload, and again at the
    // issue fields once GitHub holds the artifact version.
    let upload = project.github.pause_once("POST", "/issues/1/artifacts");
    let fields = project.github.pause_once("PATCH", "/issues/1");
    let first = todo(&["artifacts", "write", "list-depth", "log", FIRST])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert!(
        upload.wait_until_received(Duration::from_secs(30)),
        "the first command's reflection never reached GitHub"
    );

    // Reads complete while a local write holds the writer lock.
    let writer = instrukt_core::fsx::LockGuard::acquire(
        &project.env.instrukt_home().join("state/writer.lock"),
        Duration::from_secs(5),
    )
    .unwrap();
    let listing = finished_within(todo(&["list"]), UNBLOCKED)
        .expect("listing the work waited on the writer lock or on GitHub");
    assert!(listing.status.success(), "{listing:?}");
    let listing = String::from_utf8(listing.stdout).unwrap();
    let current = read().expect("reading an artifact waited on the writer lock or on GitHub");
    assert!(current.status.success(), "{current:?}");
    let current = String::from_utf8(current.stdout).unwrap();
    drop(writer);

    // GitHub accepts the first artifact version, with newly assigned
    // coordinates, and the reflection stops at the issue update before this
    // computer records anything of it.
    drop(upload);
    assert!(
        fields.wait_until_received(Duration::from_secs(30)),
        "the first command's issue update never reached GitHub"
    );

    // A local change to the same todo commits while the reflection is in
    // flight.
    let second = todo(&["artifacts", "write", "list-depth", "log", SECOND])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + UNBLOCKED;
    loop {
        let current = read().expect("reading an artifact waited on the writer lock or on GitHub");
        if String::from_utf8(current.stdout).unwrap() == format!("{SECOND}\n") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the local change waited on the in-flight reflection"
        );
    }

    // This computer holds a revision past the one GitHub shows; reading the
    // todo waits on nothing.
    let during = finished_within(todo(&["show", "list-depth"]), UNBLOCKED)
        .expect("showing the todo waited on the in-flight reflection");
    assert!(during.status.success(), "{during:?}");
    let during = String::from_utf8(during.stdout).unwrap();
    drop(fields);
    for command in [first, second] {
        let output = command.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    // Both changes reached GitHub intact.
    let listed = project.ok(["artifacts", "list", "list-depth"]);
    let after = project.ok(["show", "list-depth"]);
    let on_github: Vec<String> = project
        .github
        .artifacts(support::REPOSITORY, 1)
        .iter()
        .filter(|a| a["artifact"]["name"] == "log")
        .flat_map(|a| a["versions"].as_array().unwrap().clone())
        .map(|v| v["body"].as_str().unwrap().to_owned())
        .collect();
    // GitHub lists an artifact's versions newest first.
    let on_github = on_github.join("\n");
    insta::assert_snapshot!(
        "reflection-in-flight",
        support::redact(&format!(
            "{listing}---\n{current}---\n{during}---\n{listed}---\n{after}---\n{on_github}\n"
        ))
    );
}

/// The stage variable under which this test binary, run again, observes
/// the database's lock from another process; it carries the database path.
const LOCK_OBSERVER: &str = "INSTRUKT_STATE_DB_LOCK_OBSERVER";

// spec: project/instrukt/spec/feature/work/state-database UC-SD6
#[test]
fn a_process_holding_the_database_keeps_its_lock_while_it_inspects_it() {
    if let Some(file) = std::env::var_os(LOCK_OBSERVER) {
        println!(
            "observed-lock {}",
            observe_lock(std::path::Path::new(&file))
        );
        return;
    }
    let project = support::Project::new();
    project.create("held-database", "change", "Keep the database whole.");
    let home = Home::at_user_home(project.env.home().to_path_buf());
    let file = home.state_db_file();
    let todos = |db: &Connection| -> i64 {
        db.query_row("SELECT count(*) FROM todos", [], |row| row.get(0))
            .unwrap()
    };

    // This process holds the database open, as the runner does: a
    // connection that has run a query keeps its read lock between reads.
    let held = StateRead::open(&home).unwrap().unwrap();
    assert_eq!(todos(held.conn()), 1);

    // It reads the database for inspection, as the runner's workers do.
    let inspected = StateRead::open(&home.for_inspection()).unwrap().unwrap();
    assert_eq!(todos(inspected.conn()), 1);
    drop(inspected);

    // Another process asks the kernel whether a write lock could be taken
    // where SQLite's readers hold theirs; a process never sees its own locks.
    let observer = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "state_database::a_process_holding_the_database_keeps_its_lock_while_it_inspects_it",
            "--nocapture",
        ])
        .env(LOCK_OBSERVER, &file)
        .output()
        .unwrap();
    assert!(observer.status.success(), "{observer:?}");
    let stdout = String::from_utf8(observer.stdout).unwrap();
    let observed = stdout
        .lines()
        .find_map(|line| line.strip_prefix("observed-lock "))
        .unwrap_or_else(|| panic!("the observer reported no lock: {stdout}"))
        .split(' ')
        .map(|number| number.parse::<i64>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        observed,
        [i64::from(libc::F_RDLCK), i64::from(std::process::id())],
        "another process finds this process's read lock on {} in place",
        file.display()
    );

    // Another instrukt command writes the database and exits.
    project.create("second-write", "change", "Write while it is held.");

    // The holder goes on reading, seeing that write, and writing.
    assert_eq!(todos(held.conn()), 2);
    let mut writer = StateDb::open(&home).unwrap();
    let path = writer.path().to_path_buf();
    writer
        .write(|tx| {
            tx.execute(
                "UPDATE todos SET revision = revision WHERE slug = 'second-write'",
                [],
            )
            .map_err(StateDbError::sqlite(&path))
        })
        .unwrap();
    drop(writer);
    assert_eq!(todos(held.conn()), 2);
    // The integrity check reports each problem it finds as a row of its own.
    let problems: i64 = held
        .conn()
        .query_row(
            "SELECT count(*) FROM pragma_integrity_check WHERE integrity_check <> 'ok'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(problems, 0, "the database passes its integrity check");
}

/// The lock, as `<type> <pid>`, that keeps a write lock off SQLite's
/// shared-reader range of the database at `file` (`SHARED_FIRST`,
/// `0x40000002`, 510 bytes).
fn observe_lock(file: &std::path::Path) -> String {
    let database = std::fs::File::open(file).unwrap();
    let mut lock = libc::flock {
        l_type: libc::F_WRLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0x4000_0002,
        l_len: 510,
        l_pid: 0,
    };
    fcntl(&database, FcntlArg::F_GETLK(&mut lock)).unwrap();
    format!("{} {}", lock.l_type, lock.l_pid)
}

#[test]
fn golden_v33_database_upgrades_cleanly_to_v34_preserving_conversations_and_refusals() {
    let env = common::TestEnv::new();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf()).for_writing();
    let state = env.instrukt_home().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let file = state.join("instrukt.db");
    {
        let db = Connection::open(&file).unwrap();
        db.execute_batch(include_str!("../fixtures/state-v33.sql"))
            .unwrap();
        db.execute_batch(
            "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,opening,session,source_identity,display_title)
             VALUES ('acme','slack','C1/1700.1','instrukt:conversation:acme:slack:C1%2F1700.1','1700.1','t3:legacy-conv','src-legacy','acme — #general');
             INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,sender_email,written_us,written_at,arrived_at,state,text,admission_position)
             VALUES ('acme','slack','C1/1700.1','1700.1','U1','Steven','steven@example.com',1,'2026-11-20T10:00:00Z','2026-11-20T10:00:01Z','answered','Ship date?',1);
             INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,posted_at,admission_position)
             VALUES ('acme','slack','C1/1700.1','turn-1',0,'Friday.','2026-11-20T10:00:02Z',1);
             INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at,replied_at)
             VALUES ('acme','slack','C1/1700.1','UGUEST','not_own','2026-11-20T10:00:00Z','2026-11-20T10:00:01Z'),
                    ('acme','slack','C1/1700.1','U1','unavailable','2026-11-20T10:00:30Z',NULL);",
        )
        .unwrap();
        let version: u32 = db
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 33);
    }

    // Opening the database through StateDb applies v34 forward migration cleanly.
    let db = instrukt_core::state_db::StateDb::open(&home).unwrap();
    drop(db);

    let session = instrukt_core::ids::SessionRef::parse("t3:legacy-conv").unwrap();
    let assignment = instrukt_company::conversations::conversation_assignment(&home, &session)
        .unwrap()
        .unwrap();
    assert_eq!(assignment.role.as_str(), "company-agent");
    assert_eq!(
        assignment.company.as_ref().map(|slug| slug.as_str()),
        Some("acme")
    );
    let binding = instrukt_company::conversations::conversation_binding(&home, &session)
        .unwrap()
        .unwrap();
    assert_eq!(binding.connector, "slack");
    assert_eq!(binding.conversation, "C1/1700.1");
    assert_eq!(binding.role.as_str(), "company-agent");
    assert_eq!(
        binding.session_scope,
        instrukt_core::config::SessionScope::Conversation
    );
    assert_eq!(
        binding.reply_mode,
        instrukt_core::config::ReplyMode::TurnOutput
    );

    // Existing refusals survive verbatim and `not_admin` is now accepted.
    let raw = Connection::open(&file).unwrap();
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, instrukt_core::state_db::schema_version());
    raw.execute(
        "INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at)
         VALUES ('acme','slack','C1/1700.1','U2','not_admin','2026-11-20T10:05:00Z')",
        [],
    )
    .unwrap();
    let reasons: Vec<String> = raw
        .prepare("SELECT reason FROM conversation_refusals ORDER BY reason")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(reasons, vec!["not_admin", "not_own", "unavailable"]);
}

/// The work tables' rows of `db`, each row as its columns' text, in key
/// order: what a migration must leave as it found it.
fn work_rows(db: &Connection) -> Vec<Vec<Option<String>>> {
    let mut rows = Vec::new();
    for query in [
        "SELECT * FROM todos ORDER BY id",
        "SELECT * FROM artifacts ORDER BY todo_id, name",
        "SELECT * FROM artifact_versions ORDER BY todo_id, sequence",
        "SELECT * FROM stamps ORDER BY todo_id, sequence",
        "SELECT * FROM holds ORDER BY todo_id, set_at",
    ] {
        let mut statement = db.prepare(query).unwrap();
        let columns = statement.column_count();
        let found = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|i| {
                        row.get::<_, instrukt_core::state_db::rusqlite::types::Value>(i)
                            .map(|value| match value {
                                instrukt_core::state_db::rusqlite::types::Value::Null => None,
                                other => Some(format!("{other:?}")),
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows.extend(found);
    }
    rows
}

/// Work as the shipped v35 writer (main 8bf7d63) stored it: a todo held for
/// review with its input and plan, a build-started stamp, the hold, and its
/// reflection to issue #1 and the project board. The claim columns, which
/// only a dispatch fills, carry a claim in the form that writer stores.
const V35_WORK: &str = r#"
INSERT INTO todos VALUES('d34b9b4b157da44424c3630d9592445e','demo','list-depth','change','active','2026-11-23T08:04:23.212917Z','{"session":"t3:lead-1"}',NULL,4,NULL,NULL,NULL,'once',NULL,NULL,'a1b2c3d4e5f60718293a4b5c6d7e8f90','t3:lead-1','2026-11-23T08:04:23.9Z','list-depth',NULL,NULL,NULL,NULL,NULL,NULL,3,'2026-11-23T08:04:24.979941Z');
INSERT INTO reflections VALUES('d34b9b4b157da44424c3630d9592445e',1,4,0,0,NULL,NULL,1013,1023,'W/"276c6835aac1d463"','the pull request waits for branch `list-depth` on GitHub',0,0,'acme/1','PVTI_1016','held',NULL);
INSERT INTO artifacts VALUES('d34b9b4b157da44424c3630d9592445e','input',1);
INSERT INTO artifacts VALUES('d34b9b4b157da44424c3630d9592445e','implementation-plan',2);
INSERT INTO artifact_versions VALUES('d34b9b4b157da44424c3630d9592445e','input',1,'sha256:c5c8a5c4312ab4a773f36943b2b451017b19e6f97e3cfd2b59ce2b5c38345ab4','{"session":"t3:lead-1"}','2026-11-23T08:04:23.212917Z','Lists keep their depth everywhere.','[]','{}',1);
INSERT INTO artifact_versions VALUES('d34b9b4b157da44424c3630d9592445e','implementation-plan',2,'sha256:ccbc8c07e0bbd0e6186a4c2778e51b3c0c90d989235ff97db55f5d6636548965','{"session":"t3:lead-1"}','2026-11-23T08:04:23.49263Z','1. Track depth in the parser.','[{"artifact":"input","digest":"sha256:c5c8a5c4312ab4a773f36943b2b451017b19e6f97e3cfd2b59ce2b5c38345ab4"},{"artifact":"prd","digest":null},{"artifact":"architecture","digest":null},{"artifact":"diagnosis","digest":null}]','{"head":"0bd0da14f58b73097505de14cf7e2ada3f78e478"}',1);
INSERT INTO stamps VALUES('d34b9b4b157da44424c3630d9592445e',3,'build_started','2026-11-23T08:04:24.347203Z','{"session":"t3:lead-1"}',NULL,NULL,NULL,'unknown','sha256:62343d00ad6e9b8485b7e99c926c0979d2198f38e7ad6659ad97065fee998299',NULL,'{"plan":"sha256:ccbc8c07e0bbd0e6186a4c2778e51b3c0c90d989235ff97db55f5d6636548965","base":"0bd0da14f58b73097505de14cf7e2ada3f78e478"}',NULL);
INSERT INTO holds VALUES('d34b9b4b157da44424c3630d9592445e','manual','Waits for the parser review.','{"session":"t3:lead-1"}','2026-11-23T08:04:24.097141Z',NULL,NULL);
"#;

#[test]
fn golden_v35_database_upgrades_to_v36_keeping_its_work_and_gaining_empty_withdrawal_intents() {
    let mut project = support::Project::new();
    // One application clock, so the listing's age of the record is fixed.
    project.env.set_env("INSTRUKT_NOW", "2026-11-23T09:00:00Z");
    // GitHub cannot be read: the commands answer from the migrated database.
    project.github.stop();
    let state = project.env.instrukt_home().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let file = state.join("instrukt.db");
    let before = {
        let db = Connection::open(&file).unwrap();
        db.execute_batch(include_str!("../fixtures/state-v35.sql"))
            .unwrap();
        db.execute_batch(V35_WORK).unwrap();
        let version: u32 = db
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 35);
        work_rows(&db)
    };

    // The real CLI migrates the database to v36 and reads the work through
    // the current work model: the todo, its claim and hold, its artifacts and
    // their versions, its stamp and its reflection.
    let listed = project.ok(["list"]);
    let raw = Connection::open(&file).unwrap();
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, instrukt_core::state_db::schema_version());
    assert_eq!(work_rows(&raw), before);
    let intents: u32 = raw
        .query_row("SELECT COUNT(*) FROM withdrawal_intents", [], |r| r.get(0))
        .unwrap();
    assert_eq!(intents, 0);
    let shown = project.ok(["show", "list-depth"]);
    let artifacts = project.ok(["artifacts", "list", "list-depth"]);
    let plan = project.ok(["artifacts", "read", "list-depth", "implementation-plan"]);
    insta::assert_snapshot!(
        "golden-v35-work",
        support::redact(&format!("{listed}---\n{shown}---\n{artifacts}---\n{plan}"))
    );

    // The new table holds an intent of an existing todo, refuses one of a
    // todo that does not exist, and goes with its todo.
    raw.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    raw.execute(
        "INSERT INTO withdrawal_intents(todo_id,branch,branch_head,worktrees,workspace_path,workspace,branch_state,started_at)
         VALUES ('d34b9b4b157da44424c3630d9592445e','list-depth',?1,'[]','/project/workspace/list-depth','present','pending','2026-11-21T09:00:00Z')",
        [&"e".repeat(40)],
    )
    .unwrap();
    let orphan = raw.execute(
        "INSERT INTO withdrawal_intents(todo_id,worktrees,workspace_path,workspace,started_at) VALUES ('ffffffffffffffffffffffffffffffff','[]','/project/workspace/gone','absent','2026-11-21T09:00:00Z')",
        [],
    );
    assert!(matches!(
        orphan,
        Err(instrukt_core::state_db::rusqlite::Error::SqliteFailure(
            instrukt_core::state_db::rusqlite::ffi::Error {
                code: instrukt_core::state_db::rusqlite::ErrorCode::ConstraintViolation,
                ..
            },
            _
        ))
    ));
    raw.execute(
        "DELETE FROM todos WHERE id = 'd34b9b4b157da44424c3630d9592445e'",
        [],
    )
    .unwrap();
    let left: u32 = raw
        .query_row("SELECT COUNT(*) FROM withdrawal_intents", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn golden_v37_database_upgrades_to_v38_keeping_its_work_and_gaining_its_lookups() {
    let mut project = support::Project::new();
    project.env.set_env("INSTRUKT_NOW", "2026-11-23T09:00:00Z");
    // GitHub cannot be read: the command answers from the migrated database.
    project.github.stop();
    let state = project.env.instrukt_home().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let file = state.join("instrukt.db");
    let before = {
        let db = Connection::open(&file).unwrap();
        db.execute_batch(include_str!("../fixtures/state-v37.sql"))
            .unwrap();
        // The v35 work rows: no shipped migration since changed their shape.
        db.execute_batch(V35_WORK).unwrap();
        let version: u32 = db
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 37);
        work_rows(&db)
    };

    // The real CLI migrates the database and reads the work through it.
    project.ok(["list"]);
    let raw = Connection::open(&file).unwrap();
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, instrukt_core::state_db::schema_version());
    assert_eq!(work_rows(&raw), before);

    // Unrelated records of every kind a standing lookup reads.
    raw.execute_batch(
        "CREATE TEMP TABLE other AS SELECT * FROM todos;
         UPDATE other SET id = 'e' || substr(id, 2), slug = 'other-a', branch = 'other-a/work';
         INSERT INTO todos SELECT * FROM other;
         UPDATE other SET id = 'f' || substr(id, 2), slug = 'other-b', branch = 'other-b/work';
         INSERT INTO todos SELECT * FROM other;
         INSERT INTO holds SELECT 'e' || substr(todo_id, 2), source, reason, set_by, set_at, released_at, released_by FROM holds;
         INSERT INTO holds SELECT 'f' || substr(todo_id, 2), source, reason, set_by, set_at, released_at, released_by FROM holds WHERE todo_id LIKE 'd%';
         INSERT INTO session_bindings(todo_id, role, attempt, semantic_key, worktree, branch, evidence) VALUES
             ('e34b9b4b157da44424c3630d9592445e', 'builder', 1, 'k-a', '/work/other-a', 'other-a/work', '{}'),
             ('f34b9b4b157da44424c3630d9592445e', 'builder', 1, 'k-b', '/work/other-b', 'other-b/work', '{}'),
             ('d34b9b4b157da44424c3630d9592445e', 'builder', 1, 'k-c', '/work/list-depth', 'list-depth', '{}');
         INSERT INTO github_label_actions VALUES
             ('demo', 1, 1, 'd34b9b4b157da44424c3630d9592445e', 'held', 'dev', '2026-11-23T08:00:00Z', 'accepted'),
             ('demo', 2, 2, 'e34b9b4b157da44424c3630d9592445e', 'held', 'dev', '2026-11-23T08:00:00Z', 'accepted'),
             ('demo', 3, 3, 'f34b9b4b157da44424c3630d9592445e', 'held', 'dev', '2026-11-23T08:00:00Z', 'accepted');",
    )
    .unwrap();

    // Each lookup worktree standing makes reaches its rows by a search,
    // never by stepping through the others.
    for lookup in [
        "SELECT todo_id FROM session_bindings WHERE worktree = ?1",
        "SELECT todo_id FROM session_bindings WHERE branch = ?1",
        "SELECT id FROM todos WHERE branch = ?1",
        "SELECT reason FROM holds WHERE todo_id = ?1",
        "SELECT action FROM github_label_actions WHERE todo_id = ?1",
    ] {
        let mut statement = raw.prepare(lookup).unwrap();
        let found = statement.query_map(["absent"], |_| Ok(())).unwrap().count();
        assert_eq!(found, 0);
        assert_eq!(
            statement.get_status(instrukt_core::state_db::rusqlite::StatementStatus::FullscanStep),
            0,
            "{lookup}"
        );
    }
}
