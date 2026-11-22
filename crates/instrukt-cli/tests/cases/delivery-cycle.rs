//! Native role records exercise one shared work-phase tree through the real CLI.
#![allow(clippy::unwrap_used, reason = "isolated CLI setup")]
use crate::fixtures::work as support;
use crate::fixtures::workflow;
use std::path::PathBuf;
use workflow::Crew;
const SLUG: &str = "add-depth";
struct Worker {
    id: String,
    path: PathBuf,
    assignment: serde_json::Value,
}
fn prepared() -> Crew {
    let crew = Crew::new();
    crew.project
        .create(SLUG, "change", "Keep list depth when parsing.");
    crew.write(SLUG, "implementation-plan", "Track depth in the parser.");
    crew.write(SLUG, "plan-review-findings", "No findings.");
    crew.stamp(SLUG, &["--verdict", "approve", "--phase", "plan_review"]);
    crew.next("work", SLUG);
    crew
}
fn worker(crew: &Crew, role: &str, procedure: Option<&str>) -> Worker {
    let assignment = crew.worker(SLUG, role, procedure);
    Worker {
        id: assignment["session"].as_str().unwrap().into(),
        path: assignment["worktree"].as_str().unwrap().into(),
        assignment,
    }
}
fn stamp(crew: &Crew, w: &Worker, args: &[&str]) -> assert_cmd::assert::Assert {
    crew.project
        .env
        .instrukt(&w.path)
        .env("T3CODE_THREAD_ID", w.id.strip_prefix("t3:").unwrap())
        .args(["todo", "stamp", SLUG])
        .args(args)
        .assert()
}
fn commit(crew: &Crew, w: &Worker, name: &str) {
    std::fs::write(w.path.join(name), name).unwrap();
    crew.project.env.git(&w.path, ["add", name]);
    crew.project
        .env
        .git(&w.path, ["commit", "--quiet", "-m", "role work"]);
}
fn publish(crew: &Crew, w: &Worker) {
    crew.project.env.git(
        &w.path,
        ["push", "--quiet", "origin", "HEAD:add-depth/work"],
    );
    crew.project.repo.git(["fetch", "--quiet", "origin"]);
}
fn setup(crew: &Crew) -> Worker {
    let w = worker(crew, "setup", None);
    commit(crew, &w, "setup.txt");
    stamp(crew, &w, &["--build-started"]).success();
    crew.write(
        SLUG,
        "quality-checklist",
        "Verify shared work and independent review.",
    );
    publish(crew, &w);
    stamp(crew, &w, &["--role-complete"]).success();
    w
}
fn shown(crew: &Crew, bytes: &[u8]) -> String {
    support::redact(
        &String::from_utf8_lossy(bytes).replace(
            crew.project
                .env
                .path("")
                .to_str()
                .unwrap()
                .trim_end_matches('/'),
            "[TMP]",
        ),
    )
}
// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC4
#[test]
fn setup_completion_releases_its_attempt_and_builder_inherits_the_same_tree() {
    let crew = prepared();
    let setup = setup(&crew);
    let builder = worker(&crew, "builder", Some("crew/procedure/build"));
    assert_eq!(setup.path, builder.path);
    assert!(builder.path.join("setup.txt").is_file());
    insta::assert_snapshot!(crew.project.env.git(&builder.path,["branch","--show-current"]),@"add-depth/work");
}
// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC2
#[test]
fn completion_and_release_are_atomic_and_replay_keeps_one_completion() {
    let crew = prepared();
    setup(&crew);
    let builder = worker(&crew, "builder", Some("crew/procedure/build"));
    commit(&crew, &builder, "fix.txt");
    let first = stamp(&crew, &builder, &["--build-complete"]).success();
    let replay = stamp(&crew, &builder, &["--build-complete"]).success();
    assert_eq!(first.get_output().stdout, replay.get_output().stdout);
    let reviewer = worker(&crew, "build-reviewer", Some("crew/procedure/review-build"));
    assert_eq!(builder.path, reviewer.path);
    assert!(reviewer.path.join("fix.txt").is_file());
    let record = crew.todo(["show", SLUG]);
    insta::assert_snapshot!("completed-shared-attempt", shown(&crew, record.as_bytes()));
}
// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC5
#[test]
fn deferrals_release_only_with_current_dispositions() {
    let crew = prepared();
    setup(&crew);
    let deferrer = worker(&crew, "deferrer", Some("crew/procedure/deferrals"));
    stamp(&crew, &deferrer, &["--deferrals-processed"]).failure();
    crew.write(
        SLUG,
        "deferrals",
        "Defer table support; retained for later work.",
    );
    let before = crew.project.env.git(&deferrer.path, ["rev-parse", "HEAD"]);
    stamp(&crew, &deferrer, &["--deferrals-processed"]).success();
    let finalizer = worker(&crew, "finalizer", Some("crew/procedure/finalize"));
    assert_eq!(deferrer.path, finalizer.path);
    assert_eq!(
        before,
        crew.project.env.git(&finalizer.path, ["rev-parse", "HEAD"])
    );
}
// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC7
#[test]
fn stopped_clean_failure_preserves_commits_and_dirty_work_stays_outstanding() {
    let crew = prepared();
    let failed = worker(&crew, "setup", None);
    commit(&crew, &failed, "retained.txt");
    let evidence = serde_json::json!({"session":failed.id,"stopped":true,"source":"native cancelled task result","worktree":failed.path,"branch":"add-depth/work","head":crew.project.env.git(&failed.path,["rev-parse","HEAD"])});
    let resolve = || {
        crew.project
            .env
            .instrukt(crew.root())
            .args([
                "todo",
                "attempt",
                SLUG,
                "--role",
                "setup",
                "--attempt",
                "1",
                "--failed",
                "--reason",
                "native child stopped",
                "--evidence",
                "-",
            ])
            .write_stdin(evidence.to_string())
            .assert()
    };
    std::fs::write(failed.path.join("unfinished.txt"), "recover in place").unwrap();
    resolve().failure();
    std::fs::remove_file(failed.path.join("unfinished.txt")).unwrap();
    resolve().success();
    resolve().success();
    let successor = worker(&crew, "setup", None);
    assert_eq!(failed.path, successor.path);
    assert!(successor.path.join("retained.txt").is_file());
    assert_eq!(successor.assignment["attempt"].as_u64(), Some(2));
}
// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC1
// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC6
#[test]
fn finalizer_release_precedes_independent_current_review_and_readiness() {
    let crew = prepared();
    setup(&crew);
    let builder = worker(&crew, "builder", Some("crew/procedure/build"));
    commit(&crew, &builder, "built.txt");
    stamp(&crew, &builder, &["--build-complete"]).success();
    let review = worker(&crew, "build-reviewer", Some("crew/procedure/review-build"));
    crew.write(SLUG, "review-findings", "Approve the built candidate.");
    stamp(
        &crew,
        &review,
        &["--verdict", "approve", "--phase", "build_review"],
    )
    .success();
    let finalizer = worker(&crew, "finalizer", Some("crew/procedure/finalize"));
    commit(&crew, &finalizer, "synchronized.txt");
    publish(&crew, &finalizer);
    stamp(&crew, &finalizer, &["--role-complete"]).success();
    stamp(&crew, &finalizer, &["--finalize-ready"]).failure();
    let review = worker(&crew, "build-reviewer", Some("crew/procedure/review-build"));
    crew.write(
        SLUG,
        "review-findings",
        "Independently approve the final candidate.",
    );
    stamp(
        &crew,
        &review,
        &["--verdict", "approve", "--phase", "build_review"],
    )
    .success();
    stamp(&crew, &finalizer, &["--finalize-ready"]).success();
    let before = crew.todo(["show", SLUG]);
    stamp(&crew, &finalizer, &["--finalize-ready"]).success();
    assert_eq!(before, crew.todo(["show", SLUG]));
    assert_eq!(review.path, finalizer.path);
}
// spec: project/instrukt/spec/feature/work/todos UC-TD7
#[test]
fn binding_accepts_exact_native_results_and_refuses_changed_identity_or_placement() {
    let crew = prepared();
    let w = worker(&crew, "setup", None);
    let calls = crew.runtime.calls().len();
    let bind = |id: &str, branch: &str| {
        crew.project
            .env
            .instrukt(crew.root())
            .args([
                "todo",
                "bind",
                SLUG,
                "--role",
                "setup",
                "--attempt",
                "1",
                "--session",
                id,
                "--worktree",
                w.path.to_str().unwrap(),
                "--branch",
                branch,
                "--base",
                w.assignment["base"].as_str().unwrap(),
            ])
            .assert()
    };
    bind(&w.id, "add-depth/work").success();
    bind("t3:another", "add-depth/work").failure();
    bind(&w.id, "another/work").failure();
    assert_eq!(crew.runtime.calls().len(), calls);
}
// spec: project/instrukt/spec/feature/work/todos UC-TD8
#[test]
fn interrupted_native_binding_reuses_its_reservation_until_no_start_is_recorded() {
    let crew = prepared();
    let calls = crew.runtime.calls().len();
    let reserve = || {
        crew.project
            .env
            .instrukt(crew.root())
            .args(["todo", "attempt", SLUG, "--role", "setup", "--brief", "-"])
            .write_stdin("Prepare the shared worktree.")
            .assert()
            .success()
    };
    let first: serde_json::Value = serde_json::from_slice(&reserve().get_output().stdout).unwrap();
    let retry: serde_json::Value = serde_json::from_slice(&reserve().get_output().stdout).unwrap();
    assert_eq!(first["attempt"], retry["attempt"]);
    assert_eq!(first["semantic_key"], retry["semantic_key"]);
    assert_eq!(retry["dispatch_allowed"].as_bool(), Some(false));
    crew.project
        .env
        .instrukt(crew.root())
        .args([
            "todo",
            "attempt",
            SLUG,
            "--role",
            "setup",
            "--attempt",
            "1",
            "--not-started",
            "--reason",
            "native task API confirmed rejection before creation",
        ])
        .assert()
        .success();
    let retry: serde_json::Value = serde_json::from_slice(&reserve().get_output().stdout).unwrap();
    assert_eq!(retry["attempt"], first["attempt"]);
    assert_eq!(retry["dispatch_allowed"].as_bool(), Some(true));
    assert_eq!(crew.runtime.calls().len(), calls);
}

// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC8
#[test]
fn delivered_cleanup_retains_each_failed_or_incomplete_deletion() {
    let crew = prepared();
    setup(&crew);
    let builder = worker(&crew, "builder", Some("crew/procedure/build"));
    commit(&crew, &builder, "built.txt");
    publish(&crew, &builder);
    stamp(&crew, &builder, &["--build-complete"]).success();
    let reviewer = worker(&crew, "build-reviewer", Some("crew/procedure/review-build"));
    crew.write(SLUG, "review-findings", "Approve the exact candidate.");
    stamp(
        &crew,
        &reviewer,
        &["--verdict", "approve", "--phase", "build_review"],
    )
    .success();
    let finalizer = worker(&crew, "finalizer", Some("crew/procedure/finalize"));
    stamp(&crew, &finalizer, &["--role-complete"]).success();
    stamp(&crew, &finalizer, &["--finalize-ready"]).success();
    let candidate = crew.project.env.git(&finalizer.path, ["rev-parse", "HEAD"]);
    crew.project
        .repo
        .git(["merge", "--quiet", "--ff-only", "add-depth/work"]);
    crew.project.repo.git(["push", "--quiet", "origin", "main"]);
    crew.todo(["deliver", SLUG, "--commit", &candidate]);
    let cleanup = || {
        let out = crew
            .project
            .env
            .run(crew.root(), ["todo", "work", SLUG])
            .success();
        shown(&crew, &out.get_output().stdout)
    };
    std::fs::write(finalizer.path.join("uncommitted.txt"), "preserve").unwrap();
    crew.project
        .env
        .git_command(crew.root())
        .args(["worktree", "remove", finalizer.path.to_str().unwrap()])
        .assert()
        .failure();
    insta::assert_snapshot!("cleanup-dirty-worktree-owed", cleanup());
    std::fs::remove_file(finalizer.path.join("uncommitted.txt")).unwrap();
    crew.project
        .repo
        .git(["worktree", "remove", finalizer.path.to_str().unwrap()]);
    insta::assert_snapshot!("cleanup-branches-still-owed", cleanup());
    crew.project.repo.git(["branch", "-d", "add-depth/work"]);
    insta::assert_snapshot!("cleanup-origin-still-owed", cleanup());
    crew.project
        .repo
        .git(["push", "--quiet", "origin", "--delete", "add-depth/work"]);
    insta::assert_snapshot!("cleanup-complete", cleanup());
    assert!(!finalizer.path.exists());
    assert_eq!(
        crew.project
            .repo
            .git([
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads/add-depth/work",
                "refs/remotes/origin/add-depth/work"
            ])
            .lines()
            .count(),
        0
    );
}

// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC9
#[test]
fn fast_track_bug_reaches_finalize_ready_without_plan_setup_or_finalizer_and_enforces_three_invariants()
 {
    let crew = Crew::new();
    let slug = "quick-fix";
    crew.project
        .create(slug, "change", "Fix off-by-one in small parser helper.");
    // Enter single <slug>/work worktree directly: no implementation-plan, no setup, no quality-checklist!
    let ledger_out = crew.next("work", slug);
    assert!(ledger_out.contains("interference lock: free"));
    assert!(ledger_out.contains("prd approval: not required"));

    // Dispatch builder directly in <slug>/work.
    let builder_assignment = crew.worker(slug, "builder", Some("crew/procedure/build"));
    let builder_id = builder_assignment["session"].as_str().unwrap().to_owned();
    let builder_path = PathBuf::from(builder_assignment["worktree"].as_str().unwrap());

    // Invariant 1 (no interference): a second concurrent workspace attempt is refused.
    crew.project
        .env
        .instrukt(crew.root())
        .args([
            "todo",
            "attempt",
            slug,
            "--role",
            "build-reviewer",
            "--procedure",
            "crew/procedure/review-build",
            "--args",
            slug,
            "--brief",
            "-",
        ])
        .write_stdin("Concurrent review attempt.")
        .assert()
        .failure();

    // Commit fix, push, and complete builder atomically.
    std::fs::write(
        builder_path.join("fix.rs"),
        "pub fn ok() -> bool { true }\n",
    )
    .unwrap();
    crew.project.env.git(&builder_path, ["add", "fix.rs"]);
    crew.project.env.git(
        &builder_path,
        ["commit", "--quiet", "-m", "fix: off-by-one"],
    );
    crew.project.env.git(
        &builder_path,
        ["push", "--quiet", "origin", "HEAD:quick-fix/work"],
    );
    crew.project.repo.git(["fetch", "--quiet", "origin"]);
    crew.project
        .env
        .instrukt(&builder_path)
        .env("T3CODE_THREAD_ID", builder_id.strip_prefix("t3:").unwrap())
        .args(["todo", "stamp", slug, "--build-complete"])
        .assert()
        .success();

    // Invariant 2 (no self-approval): orchestrator cannot finalize without independent APPROVE.
    crew.project
        .env
        .instrukt(&builder_path)
        .env("T3CODE_THREAD_ID", "lead-1")
        .args(["todo", "stamp", slug, "--finalize-ready"])
        .assert()
        .failure();

    // Independent build-reviewer approves exact HEAD.
    let reviewer_assignment =
        crew.worker(slug, "build-reviewer", Some("crew/procedure/review-build"));
    let reviewer_id = reviewer_assignment["session"].as_str().unwrap().to_owned();
    let reviewer_path = PathBuf::from(reviewer_assignment["worktree"].as_str().unwrap());
    crew.write(slug, "review-findings", "Independent full-depth approval.");
    crew.project
        .env
        .instrukt(&reviewer_path)
        .env("T3CODE_THREAD_ID", reviewer_id.strip_prefix("t3:").unwrap())
        .args([
            "todo",
            "stamp",
            slug,
            "--verdict",
            "approve",
            "--phase",
            "build_review",
        ])
        .assert()
        .success();

    // Claim-holding orchestrator stamps --finalize-ready directly (no separate finalizer needed).
    crew.project
        .env
        .instrukt(&builder_path)
        .env("T3CODE_THREAD_ID", "lead-1")
        .args(["todo", "stamp", slug, "--finalize-ready"])
        .assert()
        .success();

    let shown_json: serde_json::Value =
        serde_json::from_str(&crew.todo(["show", slug, "--json"])).unwrap();
    assert_eq!(
        shown_json["ledger"]["gates"]["integrate_review_gate"]["status"],
        "ready"
    );
    assert_eq!(
        shown_json["ledger"]["gates"]["integrate_review_gate"]["finalize_stamped"],
        true
    );
}

// spec: project/instrukt/spec/feature/workflow/preparation UC-PR4
#[test]
fn feature_blocks_builder_until_configured_prd_approval_is_recorded() {
    let crew = Crew::new();
    let slug = "gated-feature";
    crew.project
        .create(slug, "feature", "Add multi-level heading anchor links.");
    crew.next("work", slug);
    crew.write(slug, "prd", "Headings receive stable slug anchors.");
    crew.write(slug, "prd-review-findings", "PRD reviewed and approved.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "prd_review"]);

    // Invariant 3: without human PRD approval, builder reservation is refused.
    crew.project
        .env
        .instrukt(crew.root())
        .args([
            "todo",
            "attempt",
            slug,
            "--role",
            "builder",
            "--procedure",
            "crew/procedure/build",
            "--args",
            slug,
            "--brief",
            "-",
        ])
        .write_stdin("Start building before approval.")
        .assert()
        .failure();

    // Once human PRD approval is stamped, builder reservation succeeds immediately.
    crew.stamp(
        slug,
        &["--prd-approved", "--by", "human", "--session", "t3:lead-1"],
    );
    let builder_assignment = crew.worker(slug, "builder", Some("crew/procedure/build"));
    assert_eq!(builder_assignment["role"], "builder");
}
