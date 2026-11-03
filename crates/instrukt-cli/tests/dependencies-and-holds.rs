//! `local/projects/instrukt/spec/feature/work/dependencies-and-holds`:
//! prerequisites recorded for a delivery, and dispatch holds with an
//! inspectable reason.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/work.rs"]
mod support;

use support::{Project, redact};

// spec: local/projects/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_dependent_todo_retains_every_prerequisite_of_its_delivery() {
    let project = Project::new();
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.create("renderer-depth", "change", "Render lists at their depth.");
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );

    let recorded = project.ok([
        "update",
        "nested-lists",
        "--after",
        "parser-depth,renderer-depth",
    ]);
    let shown = project.ok(["show", "nested-lists"]);
    let listed = project.ok(["list"]);

    insta::assert_snapshot!(
        "prerequisites",
        redact(&format!("{recorded}---\n{shown}---\n{listed}"))
    );
}

// spec: local/projects/instrukt/spec/feature/work/dependencies-and-holds UC-DH2
#[test]
fn a_held_todo_shows_why_it_waits_and_keeps_its_recorded_work() {
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

    let held = project.ok([
        "update",
        "nested-lists",
        "--hold",
        "--reason",
        "waiting for the renderer rewrite to land",
    ]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!("held", redact(&format!("{held}---\n{shown}")));
}
