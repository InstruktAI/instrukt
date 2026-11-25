//! `project/instrukt/spec/feature/runner/fleet-attention`: a project's
//! stuck work wakes the standing procedure subscribed to fleet attention,
//! through the real binary's ticks, its local event log and the fake T3
//! host; healthy, person-waiting and released work wake no one.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::fixtures::documented;
use crate::fixtures::flavor::Tracked;
use crate::fixtures::runtime::{self, FakeT3};

const PROCEDURE: &str = "docs/domain/acme/procedure/attend.md";

/// A developer's project with todos and events on, whose published
/// standing procedure subscribes to fleet attention in the project.
fn attended() -> Tracked {
    let project = Tracked::with_global_modules(true, "todos: true, events: true");
    project.repo.write(
        PROCEDURE,
        &documented::snippet(
            "procedure",
            "Attend",
            "Attend to the project's fleet.",
            "role: attender\nsubscription_home: project\nsubscriptions:\n  - runner.attention.*\n",
            &[],
            "Read the current record and attend to the condition.\n",
        ),
    );
    project.repo.commit("docs: attend to the fleet");
    documented::sync(&project.env, &project.repo);
    if !project.repo.git(["status", "--porcelain"]).is_empty() {
        project
            .repo
            .commit("fixture: include generated publication files");
    }
    project
}

fn change(project: &Tracked, slug: &str) {
    project.todo([
        "create",
        slug,
        "Improve the parser.",
        "--kind",
        "change",
        "--approved",
    ]);
}

fn db(project: &Tracked) -> instrukt_core::state_db::rusqlite::Connection {
    instrukt_core::state_db::rusqlite::Connection::open(
        project.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap()
}

/// Every logged fleet-attention envelope, oldest first.
fn attention(project: &Tracked) -> Vec<serde_json::Value> {
    let db = db(project);
    let mut statement = db
        .prepare("SELECT envelope FROM events WHERE name LIKE 'runner.attention.%' ORDER BY seq")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|envelope| serde_json::from_str(&envelope.unwrap()).unwrap())
        .collect()
}

/// A developer's own session takes `todo` up in its retained worktree,
/// from the session `from` that released it.
fn take_up(project: &Tracked, fake: &FakeT3, todo: &str, from: &str) {
    let worktree = fake.worktree(from);
    project
        .env
        .instrukt(&worktree)
        .env("T3CODE_THREAD_ID", fake.native_id(from))
        .args(["todo", "update", todo, "--unclaim"])
        .assert()
        .success();
    let start = project
        .env
        .command(std::path::Path::new("t3"), project.repo.path())
        .args([
            "thread",
            "start",
            "--title",
            &format!("todo-orchestrator · {todo}"),
            "--",
            project.repo.path().to_str().unwrap(),
            "Developer takes the work up.",
        ])
        .output()
        .unwrap();
    assert!(start.status.success());
    let second = String::from_utf8(start.stdout).unwrap().trim().to_owned();
    let branch = project.env.git(&worktree, ["branch", "--show-current"]);
    fake.change(&second, "worktreePath", worktree.to_str().unwrap().into());
    fake.change(&second, "branch", branch.into());
    let proof = runtime::placement_evidence(
        &project.env,
        &runtime::NativePlacement {
            cwd: &worktree,
            slug: todo,
            role: "todo-orchestrator",
            session: &second,
        },
    );
    project
        .env
        .instrukt(&worktree)
        .env("T3CODE_THREAD_ID", &second)
        .args(["todo", "work", todo, "--placement-evidence", "-"])
        .write_stdin(proof)
        .assert()
        .success();
}

/// The session holding `todo`'s execution, as the runner watches it.
fn holder(project: &Tracked, todo: &str) -> String {
    db(project)
        .query_row(
            "SELECT e.session FROM executions e JOIN todos t ON t.id = e.todo_id WHERE t.slug = ?1",
            [todo],
            |row| row.get(0),
        )
        .unwrap()
}

/// The sessions each attention event was delivered to.
fn deliveries(project: &Tracked) -> Vec<(String, String)> {
    let db = db(project);
    let mut statement = db
        .prepare(
            "SELECT d.event_id, d.target FROM standing_deliveries d
             JOIN events e ON e.id = d.event_id
             WHERE e.name LIKE 'runner.attention.%' AND d.state = 'delivered'
             ORDER BY d.rowid",
        )
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

// spec: project/instrukt/spec/feature/runner/fleet-attention UC-FA1
#[test]
fn a_developers_stuck_work_is_attended_to_without_their_watching() {
    let project = attended();
    let fake = FakeT3::create(&project.env);
    // Three dispatched todos: `gate` reaches a person's approval, `handed`
    // is released by its session to a developer's, and `tokens` keeps
    // running.
    for slug in ["gate", "handed", "tokens"] {
        change(&project, slug);
    }
    project.tick();
    project.tick();
    let (gate, handed, tokens) = ("s1", "s2", "s3");

    project.todo(["artifacts", "write", "gate", "prd", "Keep parser depth."]);
    project.todo([
        "artifacts",
        "write",
        "gate",
        "prd-review-findings",
        "Approve current requirements.",
    ]);
    project.todo([
        "stamp",
        "gate",
        "--verdict",
        "approve",
        "--phase",
        "prd_review",
    ]);
    take_up(&project, &fake, "handed", handed);
    fake.set_status(gate, "waiting");
    fake.set_status(handed, "waiting");
    project.tick();
    project.tick();
    let started = fake.threads().len();
    let quiet = (attention(&project), started);

    let waiting = holder(&project, "tokens");
    fake.set_status(tokens, "waiting");
    project.tick();
    project.tick();
    let woken = (attention(&project), deliveries(&project));
    project.tick();
    let again = (attention(&project).len(), deliveries(&project).len());

    assert_eq!(quiet, (Vec::new(), started));
    let (events, delivered) = woken;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(
        (
            event["type"].as_str(),
            event["subject"].as_str(),
            event["data"]["session"].as_str(),
        ),
        (
            Some("runner.attention.input_waiting"),
            Some("todo:tokens"),
            Some(waiting.as_str()),
        )
    );
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].0, event["id"].as_str().unwrap());
    assert_eq!(fake.threads().len(), started + 1);
    assert_eq!(again, (1, 1));
}
