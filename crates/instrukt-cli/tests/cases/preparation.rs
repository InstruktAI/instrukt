//! Preparation workspace placement, pending documentation reads, and
//! unchanged-head attempt completion (`docs/project/spec/feature/workflow/preparation.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::fixtures::work as support;
use crate::fixtures::workflow;
use instrukt_todos::evidence::artifact_names;
use support::redact;
use workflow::Crew;

// spec: project/instrukt/spec/feature/docs/pending-documentation UC-PD2
#[test]
fn pending_docs_reads_use_the_todo_worktree_and_fall_back_after_delivery() {
    let crew = Crew::new();
    let project = &crew.project;
    let slug = "docs-first";
    project.repo.write(
        "docs/project/spec/feature/live-parent.md",
        "---\ndescription: Live parent snippet with a pending companion.\n---\n# Live parent — Spec\n\n## What it is\n\nLive parent body.\n\n## Canonical fields\n\nLive value.\n\n## Known caveats\n\nNone.\n\n## See Also\n\n",
    );
    project.repo.commit("seed live parent doc");
    project.repo.git(["push", "--quiet", "origin", "main"]);
    project.env.run(crew.root(), ["sync"]).success();
    project.create(
        slug,
        "feature",
        "Publish the contract in the single worktree.",
    );
    let writer = crew.worker(slug, "planner", Some("crew/procedure/plan"));
    let tree = std::path::Path::new(writer["worktree"].as_str().unwrap());
    let file = tree.join("docs/project/spec/feature/early-contract.md");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        "---\ndescription: Contract authored in single worktree.\ndelivered_by: [docs-first]\n---\n# Early contract — Spec\n\n## What it is\n\nAvailable from todo worktree.\n\n## Canonical fields\n\nA documented value.\n\n## Known caveats\n\nNone.\n\n## See Also\n\n",
    )
    .unwrap();
    project.env.git(tree, ["add", "docs"]);
    project.env.git(
        tree,
        ["commit", "--quiet", "-m", "docs: add pending contract"],
    );
    // Pending read via --target resolves from the todo's single <slug>/work worktree.
    let read = project
        .env
        .run(
            crew.root(),
            [
                "docs",
                "get",
                "project/demo/spec/feature/early-contract",
                "--target",
                slug,
            ],
        )
        .success();
    assert!(
        String::from_utf8_lossy(&read.get_output().stdout)
            .contains("Available from todo worktree.")
    );
}

// spec: project/instrukt/spec/feature/workflow/preparation UC-PR3
#[test]
fn a_revision_that_changes_no_repository_file_proceeds_to_its_review() {
    let crew = Crew::new();
    let slug = "unchanged-revision";
    crew.project
        .create(slug, "change", "Keep list depth inside block quotes.");
    crew.next("prepare", slug);
    // An earlier planner attempt committed its doc change on the prepare
    // branch and completed.
    let first = crew.worker(slug, "planner", Some("crew/procedure/plan"));
    let tree = std::path::Path::new(first["worktree"].as_str().unwrap());
    let session = first["session"]
        .as_str()
        .unwrap()
        .strip_prefix("t3:")
        .unwrap();
    std::fs::create_dir_all(tree.join("docs/project/spec")).unwrap();
    std::fs::write(tree.join("docs/project/spec/quoted-lists.md"),"---\ndescription: Quoted lists keep their depth.\n---\n# Quoted lists — Spec\n\n## What it is\n\nLists inside block quotes.\n\n## Canonical fields\n\nDepth survives quoting.\n").unwrap();
    crew.project.env.git(tree, ["add", "docs"]);
    crew.project.env.git(
        tree,
        ["commit", "--quiet", "-m", "docs: specify quoted lists"],
    );
    for args in [
        &[
            "todo",
            "artifacts",
            "write",
            slug,
            artifact_names::IMPLEMENTATION_PLAN,
            "1. Parse quotes first.",
        ][..],
        &["todo", "stamp", slug, "--role-complete"][..],
    ] {
        crew.project
            .env
            .instrukt(tree)
            .env("T3CODE_THREAD_ID", session)
            .args(args)
            .assert()
            .success();
    }
    crew.write(
        slug,
        artifact_names::PLAN_REVIEW_FINDINGS,
        "Nested quotes are unplanned.",
    );
    crew.stamp(slug, &["--verdict", "needs_work", "--phase", "plan_review"]);

    // The revision answers the findings in the plan alone: no repository
    // file changes, and completion is recorded at the unchanged head.
    let revision = crew.worker(slug, "planner", Some("crew/procedure/plan"));
    let tree = std::path::Path::new(revision["worktree"].as_str().unwrap());
    let session = revision["session"]
        .as_str()
        .unwrap()
        .strip_prefix("t3:")
        .unwrap();
    for args in [
        &[
            "todo",
            "artifacts",
            "write",
            slug,
            artifact_names::IMPLEMENTATION_PLAN,
            "1. Parse quotes first, nested quotes included.",
        ][..],
        &["todo", "stamp", slug, "--role-complete"][..],
    ] {
        crew.project
            .env
            .instrukt(tree)
            .env("T3CODE_THREAD_ID", session)
            .args(args)
            .assert()
            .success();
    }

    // The revised plan's independent review is owed, and its reviewer is
    // admitted.
    let next = crew.next("prepare", slug);
    crew.worker(slug, "plan-reviewer", Some("crew/procedure/review-plan"));
    insta::assert_snapshot!("unchanged-revision-review", redact(&next));
}
