//! `local/projects/instrukt/spec/feature/work/approvals`: approvals
//! attributed to their observed source and bound to the evidence approved;
//! intent checked by its first other recipient.
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

use instrukt_core::config::CURRENT_SCHEMA_VERSION;
use support::{Project, REPOSITORY, redact};
use workflow::Crew;

// spec: local/projects/instrukt/spec/feature/work/approvals UC-AP3
#[test]
fn an_earlier_approval_carries_to_the_revised_outcome_only_under_a_current_review() {
    let project = Project::new();
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    reviewed_prd(&project, "Lists keep their depth through parse and render.");
    // Stamp #5: the developer's approval of that outcome.
    project.ok([
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

    let unreviewed = project
        .todo([
            "stamp",
            "nested-lists",
            "--prd-approved",
            "--carry-from",
            "5",
        ])
        .failure();
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd-review-findings",
        "No findings on the revision.",
    ]);
    project.ok([
        "stamp",
        "nested-lists",
        "--verdict",
        "approve",
        "--phase",
        "prd_review",
    ]);
    let carried = project.ok([
        "stamp",
        "nested-lists",
        "--prd-approved",
        "--carry-from",
        "5",
    ]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "carried",
        redact(&format!(
            "{}---\n{carried}---\n{shown}",
            String::from_utf8_lossy(&unreviewed.get_output().stderr)
        ))
    );
}

/// The `crew` project with `approvals.prd` set to `approver`.
fn prd_approved_by(crew: &Crew, approver: &str) {
    crew.project.repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: {CURRENT_SCHEMA_VERSION}\nproject: demo\nmodules:\n  todos: true\ngithub:\n  repository: {REPOSITORY}\nworkflow:\n  domain: crew\napprovals:\n  prd: {approver}\n"
        ),
    );
}

// spec: local/projects/instrukt/spec/feature/work/approvals UC-AP4
#[test]
fn a_peer_approves_where_the_project_allows_it_with_its_session_and_reason() {
    let mut crew = Crew::new();
    let slug = "nested-lists";
    prd_approved_by(&crew, "peer");
    crew.project.create(
        slug,
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    crew.next("prepare", slug);
    crew.write(slug, "prd", "Lists keep their depth.");
    crew.write(slug, "prd-review-findings", "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "prd_review"]);
    let approve = ["stamp", slug, "--prd-approved", "--by", "peer"];

    // In the peer's session but stating no reason; then with no session.
    crew.project.env.set_env("AGENT_SESSION", "peer-2");
    let unreasoned = crew.project.todo(approve).failure();
    crew.project.env.remove_env("AGENT_SESSION");
    let sessionless = crew
        .project
        .todo(
            approve
                .iter()
                .copied()
                .chain(["--reason", "it follows the input"]),
        )
        .failure();

    crew.project.env.set_env("AGENT_SESSION", "peer-2");
    let approved = crew.todo(
        approve
            .iter()
            .copied()
            .chain(["--reason", "it follows the input"]),
    );
    crew.project.env.set_env("AGENT_SESSION", "lead-1");
    let allowed = crew.next("prepare", slug);
    // The project reserves the gate for the developer.
    prd_approved_by(&crew, "human");
    let reserved = crew.next("prepare", slug);
    let shown = crew.todo(["show", slug]);

    insta::assert_snapshot!(
        "peer-approval",
        redact(&format!(
            "{}---\n{}---\n{approved}---\npeer allowed:\n{allowed}---\nhuman required:\n{reserved}---\n{shown}",
            String::from_utf8_lossy(&unreasoned.get_output().stderr),
            String::from_utf8_lossy(&sessionless.get_output().stderr),
        ))
    );
}

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
