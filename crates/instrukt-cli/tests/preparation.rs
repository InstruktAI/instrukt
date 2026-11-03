//! `instrukt todo prepare`
//! (`docs/local/spec/feature/workflow/preparation.md`): a feature is
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

// spec: local/projects/instrukt/spec/feature/workflow/preparation UC-PR1
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
