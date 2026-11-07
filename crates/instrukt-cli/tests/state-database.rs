//! `project/instrukt/spec/feature/work/state-database`: the work record and
//! the runner's state in one transactional database.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod support;
#[path = "support/workflow.rs"]
mod workflow;

use std::process::Stdio;
use std::time::{Duration, Instant};

use instrukt_core::state_db::rusqlite::{Connection, ErrorCode};
use workflow::Crew;

/// How many unchanged snippets the documentation holds: enough that moving
/// the planned documentation — the last step of the grouping, inside its
/// transaction — takes long enough to be stopped part-way.
const BULK_SNIPPETS: usize = 400;

/// Whether another writer holds the database's write transaction open now:
/// a write transaction of our own is refused at once. Ours is given back
/// unused when it was granted.
fn write_held(probe: &Connection) -> bool {
    match probe.execute_batch("BEGIN IMMEDIATE") {
        Ok(()) => {
            probe.execute_batch("ROLLBACK").unwrap();
            false
        }
        Err(err) => {
            let busy = err.sqlite_error_code() == Some(ErrorCode::DatabaseBusy);
            assert!(busy, "probing the state database failed: {err}");
            true
        }
    }
}

// spec: project/instrukt/spec/feature/work/state-database UC-SD2
#[test]
fn a_change_spanning_several_records_stopped_part_way_leaves_the_last_completed_state() {
    let crew = Crew::new();
    let project = &crew.project;
    let filler = "Unchanged prose of the current contract.\n".repeat(2_000);
    for n in 0..BULK_SNIPPETS {
        project.repo.write(
            &format!("docs/project/spec/feature/bulk/s{n:03}.md"),
            &format!("---\ndescription: Bulk {n}.\n---\n\n# Bulk {n} — Spec\n\n{filler}"),
        );
    }
    project.repo.write(
        "docs/project/spec/feature/depth.md",
        "---\ndescription: List depth.\ndelivered_by: [list-depth]\n---\n\n# Depth — Spec\n\nLists keep their depth.\n",
    );
    project.repo.commit("plan list-depth's documentation");
    project.create(
        "list-depth",
        "feature",
        "Lists keep their depth everywhere.",
    );
    let before = project.ok(["show", "list-depth"]);

    // Filing nested-lists under list-depth is one change across two
    // records: list-depth becomes its group's holder and nested-lists is
    // filed under it.
    let mut filing = project
        .env
        .instrukt_process(project.repo.path())
        .args([
            "todo",
            "create",
            "nested-lists",
            "Keep the depth of nested lists when parsing Markdown.",
            "--kind",
            "change",
            "--approved",
            "--group",
            "list-depth",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let probe = Connection::open(project.env.instrukt_home().join("state/instrukt.db")).unwrap();
    probe.busy_timeout(Duration::ZERO).unwrap();
    // Stop it once its write transaction has stayed open across two looks
    // well apart: the grouping's records are written and not yet committed.
    // The transfer only reads regular files, so no FIFO can hold it open.
    // If scheduling misses this window, report the coverage gap below.
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut held_since: Option<Instant> = None;
    let stopped = loop {
        if let Some(status) = filing.try_wait().unwrap() {
            break Err(status);
        }
        match (write_held(&probe), held_since) {
            (true, Some(since)) if since.elapsed() >= Duration::from_millis(40) => {
                filing.kill().unwrap();
                filing.wait().unwrap();
                break Ok(());
            }
            (true, None) => held_since = Some(Instant::now()),
            (true, Some(_)) => {}
            (false, _) => held_since = None,
        }
        assert!(
            Instant::now() < deadline,
            "the filing never held its write open"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    drop(probe);
    if let Err(status) = stopped {
        assert!(status.success(), "the filing failed: {status}");
        eprintln!(
            "SKIP UC-SD2 crash check: the filing committed before an open transaction could be stopped"
        );
        return;
    }

    let after = project.ok(["show", "list-depth"]);
    // Nothing of the interrupted filing is present: no nested-lists.
    project.todo(["show", "nested-lists"]).failure();

    assert_eq!(
        after, before,
        "list-depth is as the last completed change left it"
    );
}

// spec: project/instrukt/spec/feature/work/state-database UC-SD2
#[test]
fn an_owned_v1_database_keeps_its_existing_binding_when_attempt_records_are_introduced() {
    let project = support::Project::new();
    let file = project.env.instrukt_home().join("state/instrukt.db");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let db = Connection::open(&file).unwrap();
    db.execute_batch(include_str!("fixtures/state-v1.sql"))
        .unwrap();
    let id = "11111111111111111111111111111111";
    db.execute("INSERT INTO todos(id,project,slug,kind,status,created_at,created_by,revision,catch_up,sequence) VALUES (?1,'demo','legacy','change','active','2026-01-01T00:00:00Z','{\"person\":\"Test Developer\"}',1,'once',1)",[id]).unwrap();
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
