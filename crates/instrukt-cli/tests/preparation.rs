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
// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG5
// spec: project/instrukt/spec/feature/docs/pending-documentation UC-PD2
#[test]
fn reviewed_preparation_integrates_before_work_and_pending_reads_use_its_own_tree() {
    let crew = Crew::new();
    let project = &crew.project;
    let slug = "docs-first";
    let global = project.env.instrukt_home().join("instrukt.yaml");
    let mut text = std::fs::read_to_string(&global).unwrap();
    text.push_str(&format!(
        "modules: {{events: true}}\nredis:\n  cluster:\n    url: '{}'\n",
        project.env.redis_url()
    ));
    std::fs::write(global, text).unwrap();
    crew.flavor.repo.write("docs/domain/crew/procedure/integrate-preparation.md","---\ndescription: Integrate reviewed preparation immediately.\nrole: integrator\nsubscriptions: [workflow.preparation_reviewed]\nsubscription_home: project\n---\n# Integrate preparation — Procedure\n\n## Goal\n\nIntegrate reviewed preparation docs before build.\n\n## Preconditions\n\nThe exact reviewed preparation head is available.\n\n## Steps\n\nMerge through the candidate gate, fold pending docs, then remove the prepare worktree and local/remote branch.\n\n## Outputs\n\nPublished docs on main.\n\n## Recovery\n\nRetain failed integration or cleanup as owed work.\n");
    crew.flavor
        .republish(&project.env, "integrator: publish preparation subscription");
    let config = project
        .repo
        .read("instrukt.yaml")
        .replace("todos: true", "todos: true\n  events: true");
    project.repo.write("instrukt.yaml", &config);
    project
        .repo
        .commit("integrator: enable preparation delivery");
    project.repo.git(["push", "--quiet", "origin", "main"]);
    project.env.run(crew.root(), ["sync"]).success();
    project.create(slug, "change", "Publish the contract before building it.");
    crew.write(
        slug,
        "implementation-plan",
        "Implement the published contract.",
    );
    crew.write(slug, "plan-review-findings", "Approved plan.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);
    let writer = crew.worker(slug, "planner", Some("crew/procedure/plan"));
    let prepare = std::path::Path::new(writer["worktree"].as_str().unwrap());
    let file = prepare.join("docs/project/spec/feature/early-contract.md");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file,"---\ndescription: Contract available before its implementation.\ndelivered_by: [docs-first]\n---\n# Early contract — Spec\n\n## What it is\n\nAvailable before build.\n\n## Canonical fields\n\nA documented value.\n\n## Known caveats\n\nNone.\n\n## See Also\n\n").unwrap();
    project.env.git(prepare, ["add", "docs"]);
    project.env.git(
        prepare,
        [
            "commit",
            "--quiet",
            "-m",
            "docs: publish the early contract",
        ],
    );
    project.env.git(
        prepare,
        ["push", "--quiet", "origin", "HEAD:docs-first/prepare"],
    );
    let stamp = |session: &str, args: &[&str]| {
        project
            .env
            .instrukt(prepare)
            .env("T3CODE_THREAD_ID", session.strip_prefix("t3:").unwrap())
            .args(["todo", "stamp", slug])
            .args(args)
            .assert()
    };
    stamp(writer["session"].as_str().unwrap(), &["--role-complete"]).success();
    assert!(
        !crew
            .root()
            .join("docs/project/spec/feature/early-contract.md")
            .exists()
    );
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
    insta::assert_snapshot!(
        "preparation-only-pending-contract",
        support::redact(&String::from_utf8_lossy(&read.get_output().stdout).replace(
            project.env.path("").to_str().unwrap().trim_end_matches('/'),
            "[TMP]"
        ))
    );
    project
        .env
        .instrukt(crew.root())
        .args(["todo", "work", slug])
        .assert()
        .failure();
    let reviewer = crew.worker(
        slug,
        "commit-reviewer",
        Some("crew/procedure/review-commits"),
    );
    crew.write(
        slug,
        "preparation-review-findings",
        "All preparation lanes approve this exact head.",
    );
    stamp(
        reviewer["session"].as_str().unwrap(),
        &["--verdict", "approve", "--phase", "preparation_head"],
    )
    .success();
    project.env.run(crew.root(), ["tick"]).success();
    let before = crew.runtime.started_count();
    assert!(before >= 3);
    let integration = project.env.path("integration-docs");
    project.repo.git([
        "worktree",
        "add",
        "--quiet",
        "-b",
        "integration-docs",
        integration.to_str().unwrap(),
        "main",
    ]);
    project.env.git(
        &integration,
        [
            "merge",
            "--quiet",
            "--no-ff",
            "docs-first/prepare",
            "-m",
            "integrate reviewed preparation",
        ],
    );
    project
        .env
        .run(&integration, ["docs", "fold", slug])
        .success();
    project.env.git(&integration, ["add", "docs"]);
    project.env.git(
        &integration,
        [
            "commit",
            "--quiet",
            "-m",
            "docs: activate reviewed preparation",
        ],
    );
    project
        .repo
        .git(["merge", "--quiet", "--ff-only", "integration-docs"]);
    project.repo.git(["push", "--quiet", "origin", "main"]);
    project.env.run(crew.root(), ["sync"]).success();
    project
        .repo
        .git(["worktree", "remove", prepare.to_str().unwrap()]);
    project.repo.git(["branch", "-d", "docs-first/prepare"]);
    project.repo.git([
        "push",
        "--quiet",
        "origin",
        "--delete",
        "docs-first/prepare",
    ]);
    let current = project
        .env
        .run(
            crew.root(),
            ["docs", "get", "project/demo/spec/feature/early-contract"],
        )
        .success();
    insta::assert_snapshot!(
        "integrated-before-build",
        support::redact(
            &String::from_utf8_lossy(&current.get_output().stdout).replace(
                project.env.path("").to_str().unwrap().trim_end_matches('/'),
                "[TMP]"
            )
        )
    );
    project
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
    crew.todo(["update", slug, "--unclaim"]);
    let placement = project
        .env
        .instrukt(crew.root())
        .args(["todo", "work", slug])
        .assert()
        .success();
    let placement: serde_json::Value =
        serde_json::from_slice(&placement.get_output().stdout).unwrap();
    assert_eq!(
        placement["placement"]["base"].as_str(),
        Some(project.repo.head().as_str())
    );
    assert!(!prepare.exists());
    assert_eq!(crew.runtime.started_count(), before);
}
