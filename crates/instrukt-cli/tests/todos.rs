//! `local/projects/instrukt/spec/feature/work/todos`: project work recorded
//! as todos with explicit kinds, and withdrawn work recovered under its
//! identity.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/work.rs"]
mod support;

use support::{Project, redact, stdout};

// spec: local/projects/instrukt/spec/feature/work/todos UC-TD1
#[test]
fn a_request_filed_with_its_kind_is_retained_as_a_todo_with_a_durable_identity() {
    let project = Project::new();

    let filed = project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!("recorded", redact(&format!("{filed}---\n{shown}")));
}

// spec: local/projects/instrukt/spec/feature/work/todos UC-TD2
#[test]
fn work_withdrawn_on_another_computer_is_restored_there_under_its_identity() {
    let project = Project::new();
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth through parse and render.",
    ]);
    project.ok(["remove", "nested-lists"]);

    let elsewhere = project.other_computer();
    let restored = elsewhere.ok(["reactivate", "nested-lists"]);
    let shown = elsewhere.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "restored-elsewhere",
        redact(&format!("{restored}---\n{shown}"))
    );
}

// spec: local/projects/instrukt/spec/feature/work/todos UC-TD2
#[test]
fn withdrawn_work_is_restored_with_its_intent_and_evidence_under_its_identity() {
    let project = Project::new();
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth through parse and render.",
    ]);
    let withdrawn = project.ok(["remove", "nested-lists"]);

    let restored = project.ok(["reactivate", "nested-lists"]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "restored",
        redact(&format!("{withdrawn}---\n{restored}---\n{shown}"))
    );
}

// spec: local/projects/instrukt/spec/feature/work/todos UC-TD2
#[test]
fn withdrawn_work_past_its_recovery_window_is_gone_and_cannot_be_restored() {
    let mut project = Project::new();
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok(["remove", "nested-lists"]);

    // Eight days later: past the seven-day recovery window.
    let later =
        jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() + 8 * 86_400).unwrap();
    project.env.set_env("INSTRUKT_NOW", &later.to_string());
    // Diagnostics carry the real time; the snapshot reads what the
    // developer is told.
    project.env.set_env("INSTRUKT_LOG", "warn");
    let restore = project.todo(["reactivate", "nested-lists"]).failure();
    let withdrawn = project.todo(["list", "--removed"]).success();

    insta::assert_snapshot!(
        "gone-after-the-window",
        redact(&format!(
            "{}---\n{}{}",
            String::from_utf8_lossy(&restore.get_output().stderr),
            String::from_utf8_lossy(&withdrawn.get_output().stderr),
            stdout(&withdrawn)
        ))
    );
}

// spec: local/projects/instrukt/spec/feature/work/todos UC-TD3
#[test]
fn filing_changing_preparing_and_working_on_a_todo_name_the_switch_while_tracking_is_off() {
    let project = Project::new();
    project.repo.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: demo\nmodules:\n  todos: false\n",
    );

    let refusals: Vec<String> = [
        vec![
            "create",
            "nested-lists",
            "Keep list depth.",
            "--kind",
            "feature",
            "--approved",
        ],
        vec!["update", "nested-lists", "--hold", "--reason", "waiting"],
        vec!["prepare", "nested-lists"],
        vec!["work", "nested-lists"],
    ]
    .into_iter()
    .map(|args| {
        let refused = project.todo(&args).failure();
        format!(
            "todo {}\n{}",
            args[0],
            String::from_utf8_lossy(&refused.get_output().stderr)
        )
    })
    .collect();

    insta::assert_snapshot!("tracking-off", refusals.join("---\n"));
    assert!(
        !project.env.instrukt_home().join("state/work").exists(),
        "nothing was recorded"
    );
}
