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

/// Runs a change that must be refused; returns what it said, and the
/// todo's record before and after, which must be equal.
fn refused_unchanged(project: &Project, slug: &str, change: &[&str]) -> String {
    let before = project.ok(["show", slug]);
    let mut args = vec!["update", slug];
    args.extend_from_slice(change);
    let refusal = project.todo(&args).failure();
    let after = project.ok(["show", slug]);
    assert_eq!(
        after, before,
        "the refused change left the record as it was"
    );
    // What the developer is told; diagnostics carry the real time.
    String::from_utf8_lossy(&refusal.get_output().stderr)
        .lines()
        .filter(|line| line.starts_with("instrukt: "))
        .collect::<Vec<_>>()
        .join("\n")
}

// spec: local/projects/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_prerequisite_that_already_waits_on_the_todo_is_refused_naming_the_cycle() {
    let project = Project::new();
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok(["update", "nested-lists", "--after", "parser-depth"]);

    let refused = refused_unchanged(&project, "parser-depth", &["--after", "nested-lists"]);

    insta::assert_snapshot!("direct-cycle", refused);
}

// spec: local/projects/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_prerequisite_that_waits_on_the_todo_through_others_is_refused_naming_the_cycle() {
    let project = Project::new();
    project.create("lexer-depth", "change", "Count list depth in the lexer.");
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok(["update", "nested-lists", "--after", "parser-depth"]);
    project.ok(["update", "parser-depth", "--after", "lexer-depth"]);

    let refused = refused_unchanged(&project, "lexer-depth", &["--after", "nested-lists"]);

    insta::assert_snapshot!("indirect-cycle", refused);
}

// spec: local/projects/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn grouping_a_todo_under_its_own_prerequisite_is_refused_naming_the_cycle() {
    let project = Project::new();
    project.create(
        "list-depth",
        "feature",
        "Lists keep their depth everywhere.",
    );
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok(["update", "nested-lists", "--after", "list-depth"]);

    let refused = refused_unchanged(&project, "nested-lists", &["--group", "list-depth"]);

    insta::assert_snapshot!("grouping-cycle", refused);
}

// spec: local/projects/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_prerequisite_that_waits_through_a_group_on_its_own_child_is_refused_naming_the_cycle() {
    let project = Project::new();
    project.create(
        "list-depth",
        "feature",
        "Lists keep their depth everywhere.",
    );
    project.create(
        "nested-lists",
        "change",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.create("renderer", "change", "Render lists at their depth.");
    project.ok(["update", "nested-lists", "--group", "list-depth"]);
    project.ok(["update", "nested-lists", "--after", "renderer"]);

    // The group waits on its child, which waits on the renderer.
    let refused = refused_unchanged(&project, "renderer", &["--after", "list-depth"]);

    insta::assert_snapshot!("cycle-through-a-group", refused);
}

// spec: local/projects/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_cycle_through_a_prerequisite_changed_on_another_computer_is_refused() {
    let project = Project::new();
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project
        .other_computer()
        .ok(["update", "parser-depth", "--after", "nested-lists"]);

    let refused = refused_unchanged(&project, "nested-lists", &["--after", "parser-depth"]);

    insta::assert_snapshot!("cycle-through-a-remote-change", refused);
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
