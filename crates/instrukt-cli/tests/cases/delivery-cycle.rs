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
    crew.tier_0_skip(SLUG);
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
fn the_runner_removes_a_delivered_worktree_and_branch_once_no_uncommitted_change_holds_them() {
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
    crew.verified(SLUG, "pass");
    stamp(&crew, &finalizer, &["--finalize-ready"]).success();
    let candidate = crew.project.env.git(&finalizer.path, ["rev-parse", "HEAD"]);
    crew.project
        .repo
        .git(["merge", "--quiet", "--ff-only", "add-depth/work"]);
    crew.project.repo.git(["push", "--quiet", "origin", "main"]);
    let identity = crew
        .todo(["show", SLUG])
        .lines()
        .find_map(|line| line.strip_prefix("identity: ").map(str::to_owned))
        .unwrap();
    crew.todo(["deliver", SLUG, "--commit", &candidate]);
    crew.project.env.run(crew.root(), ["sync"]).success();
    // The retained session bindings and attempts, as the record shows them.
    let retained = || {
        let shown = crew.todo(["show", &format!("{SLUG}@{identity}")]);
        let start = shown.find("\nsessions:\n").unwrap();
        let end = shown[start + 1..].find("\nartifacts:").unwrap() + start + 1;
        shown[start..end].to_owned()
    };
    let bound = retained();
    let cleanup = || {
        let out = crew
            .project
            .env
            .run(crew.root(), ["todo", "work", SLUG])
            .success();
        shown(&crew, &out.get_output().stdout)
    };
    let threads = crew.runtime.thread_calls();
    let tick = || crew.project.env.run(crew.root(), ["tick"]).success();

    std::fs::write(finalizer.path.join("uncommitted.txt"), "preserve").unwrap();
    tick();
    assert!(finalizer.path.join("uncommitted.txt").is_file());
    insta::assert_snapshot!("cleanup-dirty-worktree-owed", cleanup());

    std::fs::remove_file(finalizer.path.join("uncommitted.txt")).unwrap();
    tick();
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
    assert!(
        crew.project
            .env
            .git(&crew.origin, ["branch", "--list", "add-depth/work"])
            .is_empty()
    );
    assert_eq!(crew.runtime.thread_calls(), threads);
    assert_eq!(retained(), bound);
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
    let concurrent = crew
        .project
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
    let concurrent_err = String::from_utf8_lossy(&concurrent.get_output().stderr);
    assert!(
        concurrent_err.contains("outstanding role attempt blocks"),
        "expected interference lock refusal, got: {concurrent_err}"
    );

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
    let unreviewed = crew
        .project
        .env
        .instrukt(&builder_path)
        .env("T3CODE_THREAD_ID", "lead-1")
        .args(["todo", "stamp", slug, "--finalize-ready"])
        .assert()
        .failure();
    let unreviewed_err = String::from_utf8_lossy(&unreviewed.get_output().stderr);
    assert!(
        unreviewed_err
            .contains("independent reviewer's APPROVE stamp bound to this exact commit SHA"),
        "expected exact-SHA independent review refusal, got: {unreviewed_err}"
    );

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

    // The handoff gate's second half: no verification disposition admits it yet.
    let undisposed = crew
        .project
        .env
        .instrukt(&builder_path)
        .env("T3CODE_THREAD_ID", "lead-1")
        .args(["todo", "stamp", slug, "--finalize-ready"])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "fast-track-disposition-owed",
        shown(&crew, &undisposed.get_output().stderr)
    );
    // The orchestrator judges the bounded change Tier 0 and records the skip.
    assert_eq!(crew.workspace(slug), builder_path);
    crew.tier_0_skip(slug);

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

    // Kickback at the same head resets finalize_stamped so the handoff no longer stands.
    crew.project
        .env
        .instrukt(&builder_path)
        .env("T3CODE_THREAD_ID", "lead-1")
        .args([
            "todo",
            "stamp",
            slug,
            "--kickback",
            "--reason",
            "CI failed on integrate",
        ])
        .assert()
        .success();
    let after_kickback: serde_json::Value =
        serde_json::from_str(&crew.todo(["show", slug, "--json"])).unwrap();
    assert_eq!(
        after_kickback["ledger"]["gates"]["integrate_review_gate"]["finalize_stamped"],
        false
    );
    assert!(after_kickback["ledger"]["finalize_ready_commit"].is_null());

    // Shipped sdlc and general directive docs contain no retired two-phase or state-machine contradictions.
    let repo_docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let work_docs = repo_docs.join("domain/sdlc/procedure/lifecycle/work");
    let build_md = std::fs::read_to_string(work_docs.join("build.md")).unwrap();
    assert!(!build_md.contains(
        "`instrukt todo artifacts read {slug} implementation-plan` exists and is readable."
    ));
    assert!(build_md.contains("fast track"));
    let review_md = std::fs::read_to_string(work_docs.join("review-build.md")).unwrap();
    assert!(!review_md.contains("`implementation-plan` exists for every kind."));
    assert!(!review_md.contains("`quality-checklist` exists with Build and Review sections."));
    let finalize_md = std::fs::read_to_string(work_docs.join("finalize.md")).unwrap();
    assert!(
        !finalize_md.contains("- `instrukt todo artifacts read {slug} quality-checklist` exists.")
    );
    let forbidden = [
        "<slug>/prepare",
        "state machine routes",
        "integrated to main before build",
        "integrated as soon as possible before build",
        "separate preparation and work worktrees",
        "when reviewed preparation integrates",
        "preparation folding",
        "work-delivery folding",
        "work-phase doc freeze",
        "prepare orchestrator",
        "work orchestrator",
        "prepare-orchestrator",
        "work-orchestrator",
        "next workflow call",
    ];
    let mut stack = vec![repo_docs.join("domain/sdlc"), repo_docs.join("general")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                let text = std::fs::read_to_string(&path).unwrap();
                let normalized = text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_ascii_lowercase();
                for needle in forbidden {
                    assert!(
                        !normalized.contains(needle),
                        "shipped directive doc {} still contains retired phrase {needle:?}",
                        path.display()
                    );
                }
            }
        }
    }
}

/// A built candidate of `SLUG`, published and independently approved at its
/// exact head: the builder, whose attempt is released.
fn approved_candidate(crew: &Crew) -> Worker {
    let builder = worker(crew, "builder", Some("crew/procedure/build"));
    commit(crew, &builder, "built.txt");
    publish(crew, &builder);
    stamp(crew, &builder, &["--build-complete"]).success();
    let reviewer = worker(crew, "build-reviewer", Some("crew/procedure/review-build"));
    crew.write(SLUG, "review-findings", "Approve the exact candidate.");
    stamp(
        crew,
        &reviewer,
        &["--verdict", "approve", "--phase", "build_review"],
    )
    .success();
    builder
}

/// `todo stamp SLUG <args>` by the claim-holding orchestrator `lead-1`,
/// from the shared worktree.
fn lead(crew: &Crew, args: &[&str]) -> assert_cmd::assert::Assert {
    crew.project
        .env
        .instrukt(&crew.workspace(SLUG))
        .env("T3CODE_THREAD_ID", "lead-1")
        .args(["todo", "stamp", SLUG])
        .args(args)
        .assert()
}

/// A refusal's text, with temporary paths and commits redacted.
fn stderr(crew: &Crew, assert: &assert_cmd::assert::Assert) -> String {
    shown(crew, &assert.get_output().stderr)
}

fn ledger(crew: &Crew) -> serde_json::Value {
    let shown: serde_json::Value =
        serde_json::from_str(&crew.todo(["show", SLUG, "--json"])).unwrap();
    shown["ledger"].clone()
}

// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC10
#[test]
fn only_an_independent_verifiers_pass_accounts_for_live_behavior_at_handoff() {
    let crew = prepared();
    let builder = approved_candidate(&crew);
    let head = crew.project.env.git(&builder.path, ["rev-parse", "HEAD"]);

    // No disposition is current for the approved commit: readiness is refused.
    let refused = lead(&crew, &["--finalize-ready"]).failure();
    insta::assert_snapshot!("disposition-owed", stderr(&crew, &refused));
    assert_eq!(
        ledger(&crew)["gates"]["integrate_review_gate"]["status"],
        "blocked"
    );

    // A pass from the orchestrator or from the change's writer records nothing.
    crew.write(SLUG, "verification-findings", "Live run passed.");
    let before = crew.todo(["show", SLUG]);
    let orchestrator = lead(&crew, &["--verification", "pass"]).failure();
    insta::assert_snapshot!("orchestrator-pass-refused", stderr(&crew, &orchestrator));
    let writer = stamp(&crew, &builder, &["--verification", "pass"]).failure();
    insta::assert_snapshot!("writer-pass-refused", stderr(&crew, &writer));
    assert_eq!(crew.todo(["show", SLUG]), before);

    // An independent verifier's pass releases its attempt and admits handoff.
    let verifier = worker(&crew, "verifier", Some("crew/procedure/verify"));
    assert_eq!(verifier.path, builder.path);
    let first = stamp(&crew, &verifier, &["--verification", "pass"]).success();
    let replay = stamp(&crew, &verifier, &["--verification", "pass"]).success();
    assert_eq!(first.get_output().stdout, replay.get_output().stdout);
    let verified = ledger(&crew);
    assert!(verified["gates"]["active_workspace_attempt"].is_null());
    assert_eq!(verified["verification"]["disposition"], "pass");
    assert_eq!(verified["verification"]["commit"], head.as_str());
    assert_eq!(verified["verification"]["role"], "verifier");
    assert_eq!(verified["verification"]["binding"], "current");
    assert_eq!(
        verified["gates"]["integrate_review_gate"]["status"],
        "ready"
    );
    lead(&crew, &["--finalize-ready"]).success();
    assert_eq!(ledger(&crew)["finalize_ready_commit"], head.as_str());
}

// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC11
#[test]
fn a_verifiers_fail_is_not_waved_through_by_a_skip_or_a_deferral() {
    let crew = prepared();
    approved_candidate(&crew);
    crew.project
        .create("verify-parser-live", "change", "Verify the parser live.");
    crew.verified(SLUG, "fail");
    let before = crew.todo(["show", SLUG]);
    let skip = lead(&crew, &["--verification", "tier_0_skip"]).failure();
    insta::assert_snapshot!("skip-after-fail-refused", stderr(&crew, &skip));
    let deferral = lead(
        &crew,
        &[
            "--verification",
            "hotfix_deferred",
            "--follow-up",
            "verify-parser-live",
        ],
    )
    .failure();
    insta::assert_snapshot!("deferral-after-fail-refused", stderr(&crew, &deferral));
    let readiness = lead(&crew, &["--finalize-ready"]).failure();
    insta::assert_snapshot!("readiness-after-fail-refused", stderr(&crew, &readiness));
    assert_eq!(crew.todo(["show", SLUG]), before);
    let limited = ledger(&crew);
    assert_eq!(limited["verifier_limit"]["disposition"], "fail");
    assert_eq!(
        limited["gates"]["integrate_review_gate"]["status"],
        "blocked"
    );
}

// spec: project/instrukt/spec/feature/workflow/preparation UC-PR4
#[test]
fn feature_blocks_builder_until_configured_prd_approval_is_recorded() {
    let crew = Crew::new();
    let slug = "gated-feature";
    crew.project
        .create(slug, "feature", "Add multi-level heading anchor links.");
    let wt = crew.workspace(slug);
    crew.next("work", slug);
    crew.write(slug, "prd", "Headings receive stable slug anchors.");
    crew.write(slug, "prd-review-findings", "PRD reviewed and approved.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "prd_review"]);

    // Invariant 3: without human PRD approval, builder, fixer, finalizer, and --finalize-ready are refused.
    for (role, proc_id) in [
        ("builder", "crew/procedure/build"),
        ("fixer", "crew/procedure/fix"),
        ("finalizer", "crew/procedure/finalize"),
    ] {
        let refused = crew
            .project
            .env
            .instrukt(crew.root())
            .args([
                "todo",
                "attempt",
                slug,
                "--role",
                role,
                "--procedure",
                proc_id,
                "--args",
                slug,
                "--brief",
                "-",
            ])
            .write_stdin("Start before PRD approval.")
            .assert()
            .failure();
        let err = String::from_utf8_lossy(&refused.get_output().stderr);
        assert!(
            err.contains("PRD approval is required"),
            "role {role} should be refused for missing PRD approval, got: {err}"
        );
    }
    let finalize_refused = crew
        .project
        .env
        .instrukt(&wt)
        .env("T3CODE_THREAD_ID", "lead-1")
        .args(["todo", "stamp", slug, "--finalize-ready"])
        .assert()
        .failure();
    let finalize_err = String::from_utf8_lossy(&finalize_refused.get_output().stderr);
    assert!(
        finalize_err.contains("PRD approval is required"),
        "expected PRD approval refusal on --finalize-ready, got: {finalize_err}"
    );

    // Once human PRD approval is stamped, builder reservation succeeds immediately.
    crew.stamp(
        slug,
        &["--prd-approved", "--by", "human", "--session", "t3:lead-1"],
    );
    let builder_assignment = crew.worker(slug, "builder", Some("crew/procedure/build"));
    assert_eq!(builder_assignment["role"], "builder");

    // Non-feature todo carrying a prd artifact is also gated by approvals.prd (fresh session).
    let crew2 = Crew::new();
    let change_slug = "change-with-prd";
    crew2
        .project
        .create(change_slug, "change", "Refactor with explicit PRD.");
    crew2.next("work", change_slug);
    crew2.write(change_slug, "prd", "Explicit PRD for change todo.");
    let change_refused = crew2
        .project
        .env
        .instrukt(crew2.root())
        .args([
            "todo",
            "attempt",
            change_slug,
            "--role",
            "builder",
            "--procedure",
            "crew/procedure/build",
            "--args",
            change_slug,
            "--brief",
            "-",
        ])
        .write_stdin("Start change carrying PRD before approval.")
        .assert()
        .failure();
    assert!(
        String::from_utf8_lossy(&change_refused.get_output().stderr)
            .contains("PRD approval is required")
    );
}

#[test]
fn task_preparer_child_attempt_completes_without_worktree_and_task_firing_requires_approval() {
    let crew = Crew::new();
    let slug = "daily-check";
    crew.project
        .create(slug, "task", "Check daily build health.");
    // Deliberate pickup on a task creates no worktree.
    let out = crew
        .project
        .env
        .instrukt(crew.root())
        .args(["todo", "work", slug])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("task approval: pending human approval"));

    // Non-task-preparer roles are refused on worktree-less task todos.
    let wrong_role = crew
        .project
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
        .write_stdin("Builder should not reserve on a task todo.")
        .assert()
        .failure();
    assert!(
        String::from_utf8_lossy(&wrong_role.get_output().stderr)
            .contains("only task-preparer may reserve a worktree-less attempt on a task todo")
    );

    // Reserve and bind a worktree-less task-preparer child attempt.
    let reserved = crew
        .project
        .env
        .instrukt(crew.root())
        .args([
            "todo",
            "attempt",
            slug,
            "--role",
            "task-preparer",
            "--procedure",
            "crew/procedure/prepare-task",
            "--args",
            slug,
            "--brief",
            "-",
            "--json",
        ])
        .write_stdin("Draft task-plan.")
        .assert()
        .success();
    let assignment: serde_json::Value =
        serde_json::from_slice(&reserved.get_output().stdout).unwrap();
    assert!(assignment.get("worktree").is_none());
    assert!(assignment.get("branch").is_none());
    assert!(assignment.get("base").is_none());

    crew.project
        .env
        .instrukt(crew.root())
        .args([
            "todo",
            "bind",
            slug,
            "--role",
            "task-preparer",
            "--attempt",
            &assignment["attempt"].as_u64().unwrap().to_string(),
            "--session",
            "t3:task-prep-1",
        ])
        .assert()
        .success();

    // Child drafts task-plan and stamps --role-complete without a worktree.
    crew.project
        .env
        .instrukt(crew.root())
        .env("T3CODE_THREAD_ID", "task-prep-1")
        .args([
            "todo",
            "artifacts",
            "write",
            slug,
            "task-plan",
            "Run daily health check.",
        ])
        .assert()
        .success();
    crew.project
        .env
        .instrukt(crew.root())
        .env("T3CODE_THREAD_ID", "task-prep-1")
        .args(["todo", "stamp", slug, "--role-complete"])
        .assert()
        .success();

    // Invariant 3: --task-firing-started is refused before approvals.task is satisfied.
    let unapproved_firing = crew
        .project
        .env
        .instrukt(crew.root())
        .args(["todo", "stamp", slug, "--task-firing-started"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&unapproved_firing.get_output().stderr);
    assert!(
        err.contains("task approval is required"),
        "expected task approval refusal, got: {err}"
    );
}

#[test]
fn todo_show_json_matches_todo_work_json_for_prerequisite_grounding_and_legacy_prepare_branch_carries_forward()
 {
    let crew = Crew::new();
    // 1. Prerequisite grounding parity between `todo show --json` and `todo work --json`.
    crew.project
        .create("prereq-base", "change", "Base prerequisite change.");
    crew.project
        .env
        .instrukt(crew.root())
        .args([
            "todo",
            "create",
            "dep-child",
            "Depends on prereq-base.",
            "--kind",
            "change",
            "--after",
            "prereq-base",
            "--approved",
        ])
        .assert()
        .success();
    crew.write(
        "dep-child",
        "implementation-plan",
        "Plan grounded on prereq-base.",
    );
    let show_json: serde_json::Value =
        serde_json::from_str(&crew.todo(["show", "dep-child", "--json"])).unwrap();
    let work_out = crew
        .project
        .env
        .instrukt(&crew.workspace("dep-child"))
        .args([
            "todo",
            "work",
            "dep-child",
            "--json",
            "--placement-evidence",
            "-",
        ])
        .write_stdin(crate::fixtures::runtime::placement_evidence(
            &crew.project.env,
            &crate::fixtures::runtime::NativePlacement {
                cwd: &crew.workspace("dep-child"),
                slug: "dep-child",
                role: "todo-lead",
                session: "lead-1",
            },
        ))
        .assert()
        .success();
    let work_json: serde_json::Value =
        serde_json::from_slice(&work_out.get_output().stdout).unwrap();
    assert_eq!(
        show_json["ledger"]["artifacts"], work_json["ledger"]["artifacts"],
        "todo show --json and todo work --json must agree on artifact drift/grounding"
    );

    // 2. Legacy `<slug>/prepare` branch commits carry forward when placing `<slug>/work`.
    let legacy_slug = "legacy-prep";
    crew.project.create(
        legacy_slug,
        "feature",
        "In-flight feature with legacy prepare commits.",
    );
    let prep_wt = crew.project.env.path("legacy-prep-wt");
    crew.project.repo.git([
        "worktree",
        "add",
        "--quiet",
        "-b",
        "legacy-prep/prepare",
        prep_wt.to_str().unwrap(),
        "main",
    ]);
    std::fs::write(prep_wt.join("pending-doc.md"), "Prepared spec.\n").unwrap();
    crew.project.env.git(&prep_wt, ["add", "pending-doc.md"]);
    crew.project
        .env
        .git(&prep_wt, ["commit", "--quiet", "-m", "docs: pending spec"]);
    let prep_head = crew.project.env.git(&prep_wt, ["rev-parse", "HEAD"]);
    crew.project.env.git(
        &prep_wt,
        ["push", "--quiet", "origin", "HEAD:legacy-prep/prepare"],
    );
    crew.project
        .repo
        .git(["worktree", "remove", prep_wt.to_str().unwrap()]);

    // Deliberate `todo work` placement for `legacy-prep` derives `from: origin/legacy-prep/prepare` at `prep_head`.
    let placement_out = crew
        .project
        .env
        .instrukt(crew.root())
        .args(["todo", "work", legacy_slug, "--json"])
        .assert()
        .success();
    let placement: serde_json::Value =
        serde_json::from_slice(&placement_out.get_output().stdout).unwrap();
    assert_eq!(placement["placement"]["branch"], "legacy-prep/work");
    assert_eq!(placement["placement"]["from"], "origin/legacy-prep/prepare");
    assert_eq!(placement["placement"]["base"], prep_head.trim());

    // 3. An ALREADY-MERGED leftover `<slug>/prepare` branch falls through to fresh `origin/main` (not behind main).
    let merged_slug = "already-merged-prep";
    crew.project.create(
        merged_slug,
        "feature",
        "Feature whose prepare branch already merged.",
    );
    crew.project
        .repo
        .git(["branch", "already-merged-prep/prepare", "main"]);
    crew.project.repo.git([
        "push",
        "--quiet",
        "origin",
        "already-merged-prep/prepare:already-merged-prep/prepare",
    ]);
    // Advance main one commit past the already-merged prepare branch.
    std::fs::write(crew.root().join("newer-on-main.txt"), "newer\n").unwrap();
    crew.project.repo.git(["add", "newer-on-main.txt"]);
    crew.project
        .repo
        .git(["commit", "--quiet", "-m", "chore: advance main"]);
    let new_main_head = crew.project.repo.git(["rev-parse", "HEAD"]);
    crew.project.repo.git(["push", "--quiet", "origin", "main"]);

    let merged_placement_out = crew
        .project
        .env
        .instrukt(crew.root())
        .args(["todo", "work", merged_slug, "--json"])
        .assert()
        .success();
    let merged_placement: serde_json::Value =
        serde_json::from_slice(&merged_placement_out.get_output().stdout).unwrap();
    assert_eq!(
        merged_placement["placement"]["branch"],
        "already-merged-prep/work"
    );
    assert_eq!(merged_placement["placement"]["from"], "origin/main");
    assert_eq!(merged_placement["placement"]["base"], new_main_head.trim());
}

#[test]
fn legacy_prepare_driver_retires_and_retained_work_driver_is_adopted_on_dispatch() {
    use instrukt_core::state_db::rusqlite::Connection;

    // 1. Unreleased legacy `prepare-orchestrator` driver binding does not block dispatch and is retired.
    let project = crate::fixtures::flavor::Tracked::new(true);
    project.todo([
        "create",
        "retire-prep",
        "In-flight legacy prepare todo.",
        "--kind",
        "change",
        "--approved",
    ]);
    let db_path = project.env.instrukt_home().join("state/instrukt.db");
    {
        let db = Connection::open(&db_path).unwrap();
        let todo_id: String = db
            .query_row("SELECT id FROM todos WHERE slug='retire-prep'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let key = format!("instrukt:todo:demo/{todo_id}:prepare-orchestrator:1");
        let evidence = serde_json::json!({
            "role": "prepare-orchestrator",
            "session": null,
            "worktree": null,
            "runtime": "t3",
            "native_placement": null,
            "assignment": "driver",
            "occurrence": null,
            "attempt": 1,
            "semantic_key": key,
            "native_assignment": null,
            "lane": "prepare",
            "start_from": null,
            "no_start_reason": null,
            "branch": "retire-prep/prepare",
            "base_head": null,
            "completion_sequence": null,
            "completed_head": null,
            "failure_reason": null,
            "failure_head": null,
            "failure_workspace": null,
            "failed_at": null,
            "released_at": null
        })
        .to_string();
        db.execute(
            "INSERT INTO session_bindings(todo_id,role,attempt,runtime,semantic_key,branch,evidence) \
             VALUES (?1,'prepare-orchestrator',1,'t3',?2,'retire-prep/prepare',?3)",
            instrukt_core::state_db::rusqlite::params![todo_id, key, evidence],
        )
        .unwrap();
    }
    project.tick();
    assert_eq!(
        project.rt.started_count(),
        1,
        "todo-orchestrator should dispatch despite legacy unbound prepare-orchestrator binding"
    );
    {
        let db = Connection::open(&db_path).unwrap();
        let released: Option<String> = db
            .query_row(
                "SELECT released_at FROM session_bindings WHERE role='prepare-orchestrator'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            released.is_some(),
            "legacy prepare-orchestrator driver binding should be retired on takeover"
        );
    }

    // 2. Retained legacy `work-orchestrator` driver on existing `<slug>/work` worktree is adopted in place.
    let project2 = crate::fixtures::flavor::Tracked::new(true);
    project2.todo([
        "create",
        "adopt-work",
        "In-flight legacy work todo.",
        "--kind",
        "change",
        "--approved",
    ]);
    // First dispatch creates a real worktree, T3 thread, and runtime_bindings row;
    // we then rewrite its recorded role/key to the pre-upgrade `work-orchestrator`
    // shape, end the turn, and release the claim so the next tick must adopt it in place.
    project2.tick();
    assert_eq!(project2.rt.started_count(), 1);
    let db_path2 = project2.env.instrukt_home().join("state/instrukt.db");
    let (session_id, old_key, legacy_key) = {
        let db = Connection::open(&db_path2).unwrap();
        let (todo_id, session, old_key, evidence): (String, String, String, String) = db
            .query_row(
                "SELECT t.id, b.session, b.semantic_key, b.evidence \
                 FROM todos t JOIN session_bindings b ON b.todo_id = t.id \
                 WHERE t.slug='adopt-work'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        let legacy_key = format!("instrukt:todo:demo/{todo_id}:work-orchestrator:1");
        let mut parsed: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        parsed["role"] = serde_json::json!("work-orchestrator");
        parsed["semantic_key"] = serde_json::json!(legacy_key);
        db.execute(
            "UPDATE session_bindings SET role='work-orchestrator', semantic_key=?1, evidence=?2 WHERE todo_id=?3",
            instrukt_core::state_db::rusqlite::params![legacy_key, parsed.to_string(), todo_id],
        )
        .unwrap();
        let req_json: Option<String> = db
            .query_row(
                "SELECT request FROM runtime_bindings WHERE semantic_key=?1",
                instrukt_core::state_db::rusqlite::params![old_key],
                |row| row.get(0),
            )
            .unwrap();
        let updated_req = req_json.map(|raw| {
            let mut req: serde_json::Value = serde_json::from_str(&raw).unwrap();
            req["key"] = serde_json::json!(legacy_key);
            req.to_string()
        });
        db.execute(
            "UPDATE runtime_bindings SET semantic_key=?1, request=?2 WHERE semantic_key=?3",
            instrukt_core::state_db::rusqlite::params![legacy_key, updated_req, old_key],
        )
        .unwrap();
        db.execute(
            "UPDATE todos SET claim_machine=NULL, claim_session=NULL, claim_at=NULL WHERE id=?1",
            instrukt_core::state_db::rusqlite::params![todo_id],
        )
        .unwrap();
        let bare_id = session.strip_prefix("t3:").unwrap().to_owned();
        (bare_id, old_key, legacy_key)
    };
    let _ = (old_key, legacy_key);
    project2
        .rt
        .finish(&session_id, "completed", "paused before upgrade");
    let before_messages = project2.rt.message_count(&session_id);

    project2.tick();
    assert_eq!(
        project2.rt.started_count(),
        1,
        "retained legacy work-orchestrator session should be resumed rather than starting a new thread: {:?}",
        project2.rt.calls()
    );
    assert!(
        project2.rt.message_count(&session_id) > before_messages,
        "adopted legacy work-orchestrator session should receive the wakeup prompt; calls={:?}",
        project2.rt.calls()
    );
    {
        let db = Connection::open(&db_path2).unwrap();
        let claim_session: Option<String> = db
            .query_row(
                "SELECT claim_session FROM todos WHERE slug='adopt-work'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            claim_session.as_deref(),
            Some(format!("t3:{session_id}").as_str()),
            "adopted legacy work-orchestrator session must be accepted into the todo claim"
        );
    }
}
