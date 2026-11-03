//! `local/projects/instrukt/spec/feature/work/approvals`: approvals
//! attributed to their observed source and bound to the evidence approved;
//! intent checked by its first other recipient.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/work.rs"]
mod support;

use support::{Project, redact};

// spec: local/projects/instrukt/spec/feature/work/approvals UC-AP1
#[test]
fn an_approval_names_its_source_and_stays_bound_to_the_approved_evidence() {
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

    let approved = project.ok([
        "stamp",
        "nested-lists",
        "--prd-approved",
        "--by",
        "human",
        "--session",
        "thread-host:thread-7",
    ]);
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth, and numbering restarts per level.",
    ]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!("approved", redact(&format!("{approved}---\n{shown}")));
}

// spec: local/projects/instrukt/spec/feature/work/approvals UC-AP2
#[test]
fn the_first_other_recipient_checks_the_recorded_intent_against_its_conversation() {
    let mut project = Project::with_runtime(Some("AGENT_SESSION"));
    project.env.set_env("AGENT_SESSION", "author-1");
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );

    project.env.set_env("AGENT_SESSION", "checker-2");
    let checked = project.ok(["stamp", "nested-lists", "--intent-checked"]);
    project.env.set_env("AGENT_SESSION", "builder-3");
    let received = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "intent-checked",
        redact(&format!("{checked}---\n{received}"))
    );
}
