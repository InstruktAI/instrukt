//! `project/instrukt/spec/feature/work/state-database`: the work record and
//! the runner's state in one transactional database.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use crate::fixtures::work as support;
use crate::fixtures::workflow;
use instrukt_core::state_db::rusqlite::Connection;
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
    db.execute_batch("DROP TABLE history_projection_records; DROP TABLE history_source_positions; DROP TABLE history_sources; DROP TABLE history_conversations; DROP TABLE history_fts; DROP TABLE agent_quota; DROP TABLE conversation_relay_positions; DROP TABLE conversation_links; ALTER TABLE poll_deferrals DROP COLUMN cause; PRAGMA user_version=4;")
        .unwrap();
    let checkout = env.git_repo("retained");
    let old = serde_json::json!({"key":"retained-key","brief":"retained-input","checkout":checkout.path(),"worktree":null,"selection":{"family":"gemini","provider":"account-two","model":"chosen-model","effort":"high","options":{"thinking_level":"high"}},"preferences":[{"agent":"gemini","tier":"strong"}]});
    db.execute("INSERT INTO runtime_starts(runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state) VALUES('t3','retained-key','p','run','s',?1,'retained-input',?2,'2030-01-01T00:00:00Z',0,'unresolved')",instrukt_core::state_db::rusqlite::params![checkout.path().to_str().unwrap(),old.to_string()]).unwrap();
    let cursor = serde_json::json!({"key":"cursor-key","brief":"cursor-input","checkout":checkout.path(),"selection":{"family":"cursor","provider":"cursor-account","model":"open-model","effort":"high","options":{"effort":"high","reasoning":"medium","fastMode":true}}});
    db.execute("INSERT INTO runtime_starts(runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state) VALUES('t3','cursor-key','p','run','s',?1,'cursor-input',?2,'2030-01-01T00:00:00Z',0,'unresolved')",instrukt_core::state_db::rusqlite::params![checkout.path().to_str().unwrap(),cursor.to_string()]).unwrap();
    drop(db);
    old
}

fn assert_retained_starts(env: &common::TestEnv, old: &serde_json::Value) {
    let file = env.instrukt_home().join("state/instrukt.db");
    let db = Connection::open(file).unwrap();
    let (encoded, unresolved): (String, bool) = db.query_row("SELECT request,state='unresolved' FROM runtime_starts WHERE semantic_key='retained-key'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
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
            "SELECT request FROM runtime_starts WHERE semantic_key='cursor-key'",
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
    let older = bytes();
    assert!(status(&["status", "--json"])["machine"]["state_database"].is_null());
    let events = status(&["events", "status", "--json"]);
    assert!(events["state_database"].is_null());
    assert_eq!(
        events["activation_failures"][0]["failure"],
        "retained activation failure"
    );
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
