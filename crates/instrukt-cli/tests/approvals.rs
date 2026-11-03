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

use support::{Project, REPOSITORY, redact};

/// Writes the prd version `text` of `nested-lists` and records its
/// independent review's approval.
fn reviewed_prd(project: &Project, text: &str) {
    project.ok(["artifacts", "write", "nested-lists", "prd", text]);
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd-review-findings",
        "No findings.",
    ]);
    project.ok([
        "stamp",
        "nested-lists",
        "--verdict",
        "approve",
        "--phase",
        "prd_review",
    ]);
}

/// The digest prefix the todo's issue shows beside its open prd gate.
fn shown_prd_prefix(project: &Project) -> String {
    let issue = project.github.issue(REPOSITORY, 1).unwrap();
    let body = issue["body"].as_str().unwrap();
    let line = body
        .lines()
        .find(|l| l.starts_with("**Awaiting approval:** prd"))
        .unwrap();
    line.split('`').nth(1).unwrap().to_owned()
}

// spec: local/projects/instrukt/spec/feature/work/approvals UC-AP1
#[test]
fn an_approval_on_github_applies_to_the_evidence_it_names_attributed_to_the_approver() {
    let project = Project::new();
    project.repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: 1\nproject: demo\nmodules:\n  todos: true\ngithub:\n  repository: {REPOSITORY}\n  approvers: [maintainer]\n"
        ),
    );
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    reviewed_prd(&project, "Lists keep their depth through parse and render.");
    let earlier = shown_prd_prefix(&project);
    reviewed_prd(
        &project,
        "Lists keep their depth, and numbering restarts per level.",
    );
    let current = shown_prd_prefix(&project);

    let github = &project.github;
    github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        &format!("/instrukt approve prd {earlier}"),
    );
    github.seed_comment(
        REPOSITORY,
        1,
        "passer-by",
        "NONE",
        &format!("/instrukt approve prd {current}"),
    );
    github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        &format!("/instrukt approve prd {current}"),
    );
    let shown = project.ok(["show", "nested-lists"]);
    let answers: Vec<String> = github
        .comments(REPOSITORY, 1)
        .iter()
        .filter(|c| c["user"]["login"] == "instrukt-test")
        .filter_map(|c| c["body"].as_str())
        .filter(|b| b.starts_with('@'))
        .map(str::to_owned)
        .collect();

    insta::assert_snapshot!(
        "approved-on-github",
        redact(&format!("{shown}---\n{}", answers.join("\n")))
            .replace(&earlier, "<earlier prefix>")
            .replace(&current, "<current prefix>")
    );
}

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
    // The required review is complete.
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd-review-findings",
        "No findings.",
    ]);
    project.ok([
        "stamp",
        "nested-lists",
        "--verdict",
        "approve",
        "--phase",
        "prd_review",
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
