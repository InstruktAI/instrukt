//! `instrukt todo work`
//! (`docs/local/spec/feature/workflow/delivery-cycle.md`): a prepared
//! change is built, reviewed, revised, its deferrals decided and finalized,
//! and handed to integration only on current approval and matching
//! finalization evidence.
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

use std::path::{Path, PathBuf};

use instrukt_todos::evidence::artifact_names;
use support::redact;
use workflow::Crew;

const SLUG: &str = "add-depth";

/// A change whose plan review approved its current plan.
fn prepared(crew: &Crew) {
    crew.project
        .create(SLUG, "change", "Keep list depth when parsing.");
    crew.write(
        SLUG,
        artifact_names::IMPLEMENTATION_PLAN,
        "1. Track depth in the parser.",
    );
    crew.write(SLUG, artifact_names::PLAN_REVIEW_FINDINGS, "No findings.");
    crew.stamp(SLUG, &["--verdict", "approve", "--phase", "plan_review"]);
}

/// Runs the build dispatch `todo work` owes: the runtime starts the
/// builder and returns the worktree path it set up, where the todo branch
/// is checked out.
fn dispatch_builder(crew: &Crew, command: &str) -> PathBuf {
    let args: Vec<&str> = command.split_whitespace().skip(1).collect();
    crew.project
        .env
        .instrukt(crew.root())
        .args(&args)
        .write_stdin("Build it.")
        .assert()
        .success();
    let worktree = crew
        .runtime
        .state_dir()
        .join("worktrees")
        .join(format!("{SLUG}/work"));
    crew.project.env.git(
        crew.root(),
        [
            "worktree",
            "add",
            "--quiet",
            "-b",
            SLUG,
            worktree.to_str().unwrap(),
            "main",
        ],
    );
    worktree
}

/// The command line `todo work` printed.
fn command_of(next: &str) -> String {
    next.lines()
        .find_map(|line| line.strip_prefix("command: "))
        .unwrap()
        .to_owned()
}

fn commit(crew: &Crew, dir: &Path, file: &str, message: &str) {
    std::fs::write(dir.join(file), message).unwrap();
    crew.project.env.git(dir, ["add", file]);
    crew.project
        .env
        .git(dir, ["commit", "--quiet", "-m", message]);
}

// spec: local/projects/instrukt/spec/feature/workflow/delivery-cycle UC-DC1
#[test]
fn a_change_is_handed_off_only_after_current_approval_deferrals_and_matching_finalization() {
    let crew = Crew::new();
    prepared(&crew);
    let mut steps = Vec::new();

    // The work lead picks the prepared change up; the build is owed.
    let next = crew.next("work", SLUG);
    let worktree = dispatch_builder(&crew, &command_of(&next));
    steps.push(next);

    commit(&crew, &worktree, "depth.txt", "track depth");
    crew.todo_in(&worktree, ["stamp", SLUG, "--build-started"]);
    crew.todo_in(&worktree, ["stamp", SLUG, "--build-complete"]);
    steps.push(crew.next("work", SLUG));

    // Changes requested: the findings go back to the builder.
    crew.write(
        SLUG,
        artifact_names::REVIEW_FINDINGS,
        "Depth resets on blank lines.",
    );
    crew.todo_in(
        &worktree,
        [
            "stamp",
            SLUG,
            "--verdict",
            "changes_requested",
            "--phase",
            "build_review",
        ],
    );
    steps.push(crew.next("work", SLUG));

    // The fix is committed and completed; a new review approves it.
    commit(
        &crew,
        &worktree,
        "depth.txt",
        "keep depth across blank lines",
    );
    crew.todo_in(&worktree, ["stamp", SLUG, "--build-complete"]);
    steps.push(crew.next("work", SLUG));
    crew.write(SLUG, artifact_names::REVIEW_FINDINGS, "No findings.");
    crew.todo_in(
        &worktree,
        [
            "stamp",
            SLUG,
            "--verdict",
            "approve",
            "--phase",
            "build_review",
        ],
    );
    steps.push(crew.next("work", SLUG));

    // A clean merge of upstream main keeps the approval current.
    commit(&crew, crew.root(), "NEWS.md", "upstream news");
    crew.project
        .env
        .git(crew.root(), ["push", "--quiet", "origin", "main"]);
    crew.project
        .env
        .git(&worktree, ["merge", "--quiet", "--no-edit", "origin/main"]);
    steps.push(crew.next("work", SLUG));

    // Deferred items are decided before finalization.
    crew.write(SLUG, artifact_names::DEFERRALS, "Tables keep depth too.");
    steps.push(crew.next("work", SLUG));
    crew.todo_in(&worktree, ["stamp", SLUG, "--deferrals-processed"]);

    // Finalized but not published: the evidence does not match yet.
    crew.todo_in(&worktree, ["stamp", SLUG, "--finalize-ready"]);
    steps.push(crew.next("work", SLUG));

    let head = format!("HEAD:{SLUG}");
    crew.project
        .env
        .git(&worktree, ["push", "--quiet", "origin", head.as_str()]);
    crew.project
        .env
        .git(crew.root(), ["fetch", "--quiet", "origin"]);
    steps.push(crew.next("work", SLUG));

    let root = crew.root().display().to_string();
    let state = crew.runtime.state_dir().display().to_string();
    let text = steps
        .join("=====\n")
        .replace(&state, "<runtime>")
        .replace(&root, "<project>");
    insta::assert_snapshot!("delivery-cycle", redact(&text));
}
