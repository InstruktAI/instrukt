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

use support::{Project, redact};

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
