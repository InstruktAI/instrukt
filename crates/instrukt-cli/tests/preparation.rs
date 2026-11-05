//! `instrukt todo prepare`
//! (`docs/project/spec/feature/workflow/preparation.md`): a feature is
//! released for implementation only once its derivations, reviews and
//! approval are current, and review findings return to their author.
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
        &[
            "--prd-approved",
            "--by",
            "human",
            "--session",
            "agent:lead-1",
        ],
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
    crew.todo(["prepare", slug]);
    // The PRD names a snippet that does not exist; the plan names a path
    // the repository does not have. Neither resolves.
    crew.write(
        slug,
        artifact_names::PRD,
        "Quoted lists keep their depth, as @~/.instrukt/docs/crew/policy/quoting.md requires.",
    );
    crew.write(slug, artifact_names::PRD_REVIEW_FINDINGS, "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "prd_review"]);
    crew.stamp(
        slug,
        &[
            "--prd-approved",
            "--by",
            "human",
            "--session",
            "agent:lead-1",
        ],
    );
    crew.write(
        slug,
        artifact_names::IMPLEMENTATION_PLAN,
        "1. Parse quotes first.\n\n```referenced-paths\nsrc/quote.rs\n```\n",
    );
    crew.write(slug, artifact_names::PLAN_REVIEW_FINDINGS, "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);
    let mut steps = vec![crew.next("prepare", slug)];

    // The analysis judges the unresolved provenance cosmetic: release.
    for artifact in [artifact_names::PRD, artifact_names::IMPLEMENTATION_PLAN] {
        crew.stamp(
            slug,
            &[
                "--drift-reviewed",
                artifact,
                "cosmetic",
                "the references name work still to come",
            ],
        );
    }
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
    crew.todo(["prepare", slug]);
    crew.write(
        slug,
        artifact_names::PRD,
        "Rendered lists keep their depth.",
    );
    crew.write(slug, artifact_names::PRD_REVIEW_FINDINGS, "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "prd_review"]);
    crew.stamp(
        slug,
        &[
            "--prd-approved",
            "--by",
            "human",
            "--session",
            "agent:lead-1",
        ],
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
