//! `instrukt todo prepare`
//! (`docs/project/spec/feature/workflow/preparation.md`): a feature is
//! released for implementation only once its derivations, reviews and
//! approval are current, and review findings return to their author.
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

// spec: project/instrukt/spec/feature/workflow/preparation UC-PR1
#[test]
fn a_feature_is_released_only_after_current_reviews_and_approval() {
    let crew = Crew::new();
    let slug = "nested-lists";
    crew.project.create(
        slug,
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    let mut steps = Vec::new();
    // Picking the todo up claims it for this session; intake is owed.
    steps.push(crew.next("prepare", slug));

    crew.write(slug, artifact_names::PRD, "Lists keep their depth.");
    steps.push(crew.next("prepare", slug));

    // The review finds work: the findings return to the PRD's author.
    crew.write(
        slug,
        artifact_names::PRD_REVIEW_FINDINGS,
        "Numbering is unspecified.",
    );
    crew.stamp(slug, &["--verdict", "needs_work", "--phase", "prd_review"]);
    steps.push(crew.next("prepare", slug));

    // The revision is a new PRD: the earlier verdict no longer binds it.
    crew.write(
        slug,
        artifact_names::PRD,
        "Lists keep their depth; numbering restarts per level.",
    );
    steps.push(crew.next("prepare", slug));

    crew.write(slug, artifact_names::PRD_REVIEW_FINDINGS, "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "prd_review"]);
    steps.push(crew.next("prepare", slug));

    crew.stamp(
        slug,
        &["--prd-approved", "--by", "human", "--session", "t3:lead-1"],
    );
    steps.push(crew.next("prepare", slug));

    // The plan declares the code it is grounded on.
    crew.write(
        slug,
        artifact_names::IMPLEMENTATION_PLAN,
        "1. Track depth in the parser.\n\n```referenced-paths\nREADME.md\n```\n",
    );
    crew.write(slug, artifact_names::PLAN_REVIEW_FINDINGS, "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);
    steps.push(crew.next("prepare", slug));

    // A committed change to that code reaches the canonical branch: release
    // is owed again only after the drift is analyzed.
    let readme = crew.root().join("README.md");
    std::fs::write(&readme, "# demo\n\nLists nest.\n").unwrap();
    crew.project.repo.commit("document nesting");
    crew.project
        .env
        .git(crew.root(), ["push", "--quiet", "origin", "main"]);
    steps.push(crew.next("prepare", slug));
    crew.stamp(
        slug,
        &[
            "--drift-reviewed",
            artifact_names::IMPLEMENTATION_PLAN,
            "cosmetic",
            "the README only documents nesting",
        ],
    );
    steps.push(crew.next("prepare", slug));

    // That judgment settled the evidence it assessed, not later code:
    // another change to the README owes drift analysis again.
    std::fs::write(&readme, "# demo\n\nLists nest, and quotes too.\n").unwrap();
    crew.project.repo.commit("document quoted nesting");
    crew.project
        .env
        .git(crew.root(), ["push", "--quiet", "origin", "main"]);
    steps.push(crew.next("prepare", slug));
    crew.stamp(
        slug,
        &[
            "--drift-reviewed",
            artifact_names::IMPLEMENTATION_PLAN,
            "cosmetic",
            "the README still only documents nesting",
        ],
    );
    steps.push(crew.next("prepare", slug));

    // The developer refines the intent: the PRD and plan derived from the
    // earlier input are drift evidence, analyzed before release.
    crew.write(
        slug,
        artifact_names::INPUT,
        "Keep the depth of nested lists, including in block quotes.",
    );
    steps.push(crew.next("prepare", slug));

    // The analysis judges both cosmetic: the same evidence is settled.
    for artifact in [artifact_names::PRD, artifact_names::IMPLEMENTATION_PLAN] {
        crew.stamp(
            slug,
            &[
                "--drift-reviewed",
                artifact,
                "cosmetic",
                "block quotes are lists too",
            ],
        );
    }
    steps.push(crew.next("prepare", slug));

    // Rewriting the approved PRD unbinds its review and approval: release
    // is no longer owed.
    crew.write(
        slug,
        artifact_names::PRD,
        "Lists keep their depth and indentation.",
    );
    steps.push(crew.next("prepare", slug));

    insta::assert_snapshot!("feature-preparation", redact(&steps.join("=====\n")));
}

// spec: project/instrukt/spec/feature/workflow/preparation UC-PR1
#[test]
fn a_feature_grounded_on_unresolved_references_is_released_only_after_drift_analysis() {
    let crew = Crew::new();
    let slug = "quoted-lists";
    crew.project
        .create(slug, "feature", "Keep list depth inside block quotes.");
    crew.next("work", slug);
    // The PRD depends on a missing snippet. The plan will create its cited file.
    crew.write(
        slug,
        artifact_names::PRD,
        "Quoted lists keep their depth, as @~/.instrukt/docs/crew/policy/quoting.md requires.",
    );
    crew.write(slug, artifact_names::PRD_REVIEW_FINDINGS, "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "prd_review"]);
    crew.stamp(
        slug,
        &["--prd-approved", "--by", "human", "--session", "t3:lead-1"],
    );
    crew.write(
        slug,
        artifact_names::IMPLEMENTATION_PLAN,
        "1. Parse quotes first.\n\n```referenced-paths\nsrc/quote.rs\n```\n",
    );
    crew.write(slug, artifact_names::PLAN_REVIEW_FINDINGS, "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);
    let mut steps = vec![crew.next("prepare", slug)];

    // Settling the PRD's unresolved snippet settles the feature's drift while the
    // planned path remains fresh.
    crew.stamp(
        slug,
        &[
            "--drift-reviewed",
            artifact_names::PRD,
            "cosmetic",
            "the snippet names work still to come",
        ],
    );
    steps.push(crew.next("prepare", slug));

    insta::assert_snapshot!("unresolved-grounding", redact(&steps.join("=====\n")));
}

// spec: project/instrukt/spec/feature/workflow/preparation UC-PR1
#[test]
fn a_judged_dependency_still_binds_after_its_prerequisite_edge_is_removed() {
    let crew = Crew::new();
    crew.project
        .create("parse-lists", "feature", "Parse nested lists.");
    crew.write(
        "parse-lists",
        artifact_names::PRD,
        "Lists parse with depth.",
    );
    // The feature is prepared through its intent's review and approval,
    // then its plan's review.
    let slug = "render-lists";
    crew.project
        .create(slug, "feature", "Render nested lists with their depth.");
    crew.next("work", slug);
    crew.write(
        slug,
        artifact_names::PRD,
        "Rendered lists keep their depth.",
    );
    crew.write(slug, artifact_names::PRD_REVIEW_FINDINGS, "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "prd_review"]);
    crew.stamp(
        slug,
        &["--prd-approved", "--by", "human", "--session", "t3:lead-1"],
    );
    crew.write(
        slug,
        artifact_names::IMPLEMENTATION_PLAN,
        "1. Indent by depth.",
    );
    crew.write(slug, artifact_names::PLAN_REVIEW_FINDINGS, "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);
    let mut steps = vec![crew.next("prepare", slug)];

    // The feature comes to depend on `parse-lists`; drift analysis assesses
    // the plan against that prerequisite's PRD.
    crew.todo(["update", slug, "--after", "parse-lists"]);
    crew.stamp(
        slug,
        &[
            "--drift-reviewed",
            artifact_names::IMPLEMENTATION_PLAN,
            "cosmetic",
            "rendering follows the parsed depth as planned",
        ],
    );
    steps.push(crew.next("prepare", slug));

    // The edge is dropped, then the PRD the judgment assessed changes: the
    // judgment no longer settles the plan.
    crew.todo(["update", slug, "--after", ""]);
    crew.write(
        "parse-lists",
        artifact_names::PRD,
        "Lists parse with depth; quotes reset it.",
    );
    steps.push(crew.next("prepare", slug));

    insta::assert_snapshot!("judged-dependency", redact(&steps.join("=====\n")));
}

// spec: project/instrukt/spec/feature/workflow/preparation UC-PR1
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

// spec: project/instrukt/spec/feature/workflow/preparation UC-PR1
#[test]
fn rejected_plan_review_surfaces_open_findings_in_factual_ledger_until_revised_and_approved() {
    let crew = Crew::new();
    let slug = "reject-plan";
    crew.project
        .create(slug, "change", "Correct the rejected plan.");
    crew.next("work", slug);
    crew.write(slug, artifact_names::IMPLEMENTATION_PLAN, "Initial plan.");
    crew.write(
        slug,
        artifact_names::PLAN_REVIEW_FINDINGS,
        "Missing edge case handling.",
    );
    crew.stamp(slug, &["--verdict", "needs_work", "--phase", "plan_review"]);

    let rejected_json: serde_json::Value =
        serde_json::from_str(&crew.todo(["show", slug, "--json"])).unwrap();
    assert_eq!(
        rejected_json["ledger"]["preparation_reviews"][0]["open_findings"],
        true
    );

    crew.write(
        slug,
        artifact_names::IMPLEMENTATION_PLAN,
        "Revised plan with edge case handling.",
    );
    crew.write(slug, artifact_names::PLAN_REVIEW_FINDINGS, "Approved.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);

    let approved_json: serde_json::Value =
        serde_json::from_str(&crew.todo(["show", slug, "--json"])).unwrap();
    assert_eq!(
        approved_json["ledger"]["preparation_reviews"][0]["open_findings"],
        false
    );
    assert_eq!(
        approved_json["ledger"]["preparation_reviews"][0]["current"],
        true
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
