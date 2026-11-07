//! `project/instrukt/spec/feature/workflow/integration-gate`: what
//! instrukt contributes to integration. The standing integrator — here the
//! fake runtime's session — is woken with finalized work, with its exact
//! candidate's verification result read from GitHub, with the decision that
//! the project has no CI taken from the canonical branch's head. Preparation
//! head approval is covered by preparation.rs. The test plays the integrator's
//! git steps between wake-ups, as the integrate procedure prescribes.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/flavor.rs"]
mod flavor;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod work;

use common::github::{CheckRun, Conclusion, StatusState};
use flavor::{REPOSITORY, Tracked};
macro_rules! log_snapshot {
    ($name:literal,$value:expr $(,)?) => {
        insta::assert_snapshot!($name, work::redact(&$value));
    };
}

/// The integrator's session, started for the first event.
const INTEGRATOR: &str = "s3";

/// A project with CI on its canonical head (when `ci`), whose developer
/// finalized `tidy-imports` on its work branch; the first wake-up started
/// the standing integrator, which then parked.
fn finalized(ci: bool) -> Tracked {
    let project = Tracked::integrating();
    if ci {
        project
            .github
            .set_status(REPOSITORY, &project.repo.head(), "ci", StatusState::Success);
    }
    project.todo([
        "create",
        "tidy-imports",
        "Sort the imports of every module.",
        "--kind",
        "change",
        "--approved",
    ]);
    project.todo([
        "artifacts",
        "write",
        "tidy-imports",
        "implementation-plan",
        "Sort imports and preserve behavior.",
    ]);
    project.todo([
        "artifacts",
        "write",
        "tidy-imports",
        "plan-review-findings",
        "Approved plan.",
    ]);
    project
        .env
        .instrukt(project.repo.path())
        .env("T3CODE_THREAD_ID", "plan-reviewer")
        .args([
            "todo",
            "stamp",
            "tidy-imports",
            "--verdict",
            "approve",
            "--phase",
            "plan_review",
        ])
        .assert()
        .success();
    project
        .env
        .instrukt(project.repo.path())
        .env("T3CODE_THREAD_ID", "lead")
        .args(["todo", "work", "tidy-imports"])
        .assert()
        .success();
    let start = |procedure: &str| {
        let (role, id) = if procedure.ends_with("finalize") {
            ("finalizer", "s1")
        } else {
            ("build-reviewer", "s2")
        };
        project.native_worker("tidy-imports", role, procedure, "lead", id)
    };
    let finalizer = start("acme/procedure/finalize");
    let path = std::path::Path::new(finalizer["worktree"].as_str().unwrap());
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(path.join("src/lib.rs"), "// sorted\n").unwrap();
    project.env.git(path, ["add", "src/lib.rs"]);
    project.env.git(
        path,
        ["commit", "--quiet", "-m", "refactor: sort the imports"],
    );
    project.env.git(
        path,
        ["push", "--quiet", "origin", "HEAD:tidy-imports/work"],
    );
    project.repo.git(["fetch", "--quiet", "origin"]);
    project
        .env
        .instrukt(path)
        .env("T3CODE_THREAD_ID", "s1")
        .args(["todo", "stamp", "tidy-imports", "--role-complete"])
        .assert()
        .success();
    let reviewer = start("acme/procedure/review-build");
    project.todo([
        "artifacts",
        "write",
        "tidy-imports",
        "review-findings",
        "Approve the finalized imports.",
    ]);
    project
        .env
        .instrukt(std::path::Path::new(reviewer["worktree"].as_str().unwrap()))
        .env("T3CODE_THREAD_ID", "s2")
        .args([
            "todo",
            "stamp",
            "tidy-imports",
            "--verdict",
            "approve",
            "--phase",
            "build_review",
        ])
        .assert()
        .success();
    project
        .env
        .instrukt(path)
        .env("T3CODE_THREAD_ID", "s1")
        .args(["todo", "stamp", "tidy-imports", "--finalize-ready"])
        .assert()
        .success();
    project.tick();
    assert_eq!(
        project.rt.started_count(),
        3,
        "the finalizer, reviewer and integrator started"
    );
    project.rt.set_status(INTEGRATOR, "ended ok");
    project
}

/// The integrator pushes `commit` to `integrate` as its candidate.
fn push_candidate(project: &Tracked, commit: &str) {
    project.repo.git([
        "push",
        "--quiet",
        "--force",
        "origin",
        &format!("{commit}:refs/heads/integrate"),
    ]);
}

/// The work branch's head.
fn work_head(project: &Tracked) -> String {
    project.repo.git(["rev-parse", "tidy-imports/work"])
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG1
#[test]
fn finalized_work_starts_the_standing_integrator_with_its_handoff() {
    let project = finalized(true);
    log_snapshot!(
        "integrator-started-for-finalized-work",
        work::redact(&project.rt.opening(INTEGRATOR))
    );
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG1
#[test]
fn a_green_candidate_wakes_the_integrator_naming_that_exact_commit() {
    let project = finalized(true);
    let candidate = work_head(&project);
    push_candidate(&project, &candidate);
    project.github.set_check_run(
        REPOSITORY,
        &candidate,
        "test",
        CheckRun::Completed(Conclusion::Success),
    );

    project.tick();

    assert_eq!(project.rt.sent_count(INTEGRATOR), 1);
    log_snapshot!(
        "green-candidate",
        project
            .rt
            .sent(INTEGRATOR, 1)
            .replace(&candidate, "<candidate>")
    );
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG1
#[test]
fn an_unfinished_candidate_wakes_no_one() {
    let project = finalized(true);
    let candidate = work_head(&project);
    push_candidate(&project, &candidate);
    project.tick();
    project
        .github
        .set_check_run(REPOSITORY, &candidate, "test", CheckRun::InProgress);
    project.tick();

    assert_eq!(
        project.rt.sent_count(INTEGRATOR),
        0,
        "nothing completed yet"
    );
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG1
#[test]
fn a_repaired_candidate_is_reported_by_its_own_result_never_the_earlier_ones() {
    let project = finalized(true);
    let failed = work_head(&project);
    push_candidate(&project, &failed);
    project.github.set_check_run(
        REPOSITORY,
        &failed,
        "test",
        CheckRun::Completed(Conclusion::Failure),
    );
    project.tick();
    assert_eq!(project.rt.sent_count(INTEGRATOR), 1);
    project.rt.set_status(INTEGRATOR, "ended ok");

    // The integrator makes a trivial fix and pushes the repaired candidate;
    // the failed one keeps its red result.
    let path = project.env.path("native-tidy-imports");
    std::fs::write(path.join("src/lib.rs"), "// sorted, fixed\n").unwrap();
    project.env.git(&path, ["add", "src/lib.rs"]);
    project.env.git(
        &path,
        ["commit", "--quiet", "-m", "fix: keep the module compiling"],
    );
    let repaired = project.env.git(&path, ["rev-parse", "HEAD"]);
    push_candidate(&project, &repaired);
    project.tick();
    assert_eq!(
        project.rt.sent_count(INTEGRATOR),
        1,
        "the repaired candidate is pending"
    );

    project.github.set_check_run(
        REPOSITORY,
        &repaired,
        "test",
        CheckRun::Completed(Conclusion::Success),
    );
    project.tick();

    assert_eq!(project.rt.sent_count(INTEGRATOR), 2);
    log_snapshot!(
        "red-then-repaired",
        format!(
            "{}\n---\n{}",
            project.rt.sent(INTEGRATOR, 1),
            project.rt.sent(INTEGRATOR, 2)
        )
        .replace(&failed, "<failed candidate>")
        .replace(&repaired, "<repaired candidate>")
    );
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG1
#[test]
fn without_ci_on_the_canonical_head_the_integrator_is_told_to_verify_locally() {
    let project = finalized(false);
    let candidate = work_head(&project);
    push_candidate(&project, &candidate);

    project.tick();

    assert_eq!(project.rt.sent_count(INTEGRATOR), 1);
    log_snapshot!(
        "no-ci",
        project
            .rt
            .sent(INTEGRATOR, 1)
            .replace(&candidate, "<candidate>")
    );
}

/// The integrator promotes `commit`: local main fast-forwards to it and
/// it is pushed as remote canonical main.
fn promote(project: &Tracked, commit: &str) {
    project.repo.git(["merge", "--quiet", "--ff-only", commit]);
    project.repo.git(["push", "--quiet", "origin", "main"]);
}

/// A commit on local main touching `file`, not yet pushed.
fn commit_on_main(project: &Tracked, file: &str) -> String {
    project.repo.write(file, "a delivery\n");
    project.repo.commit("feat: another delivery")
}

/// `finalized(true)` with its delivery promoted and main's CI on it red;
/// the integrator was woken with that failure.
fn failing_main() -> (Tracked, String) {
    let project = finalized(true);
    let delivered = work_head(&project);
    promote(&project, &delivered);
    project
        .github
        .set_status(REPOSITORY, &delivered, "ci", StatusState::Failure);
    project.tick();
    (project, delivered)
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG2
#[test]
fn a_failing_canonical_head_wakes_the_integrator_with_that_commit_once() {
    let (project, delivered) = failing_main();
    project.tick();

    assert_eq!(project.rt.sent_count(INTEGRATOR), 1, "reported once");
    log_snapshot!(
        "main-failing",
        project
            .rt
            .sent(INTEGRATOR, 1)
            .replace(&delivered, "<failing head>")
    );
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG2
#[test]
fn a_revert_is_woken_with_its_own_result_never_the_reverted_deliverys() {
    let (project, delivered) = failing_main();
    project.rt.set_status(INTEGRATOR, "ended ok");

    // The integrator pushes a revert of the delivery for integration.
    project
        .repo
        .git(["checkout", "--quiet", "-b", "revert-tidy-imports"]);
    project.repo.git(["revert", "--no-edit", &delivered]);
    let revert = project.repo.head();
    project.repo.git(["checkout", "--quiet", "main"]);
    push_candidate(&project, &revert);
    project.tick();
    assert_eq!(
        project.rt.sent_count(INTEGRATOR),
        1,
        "the revert's CI is still running"
    );

    project.github.set_check_run(
        REPOSITORY,
        &revert,
        "test",
        CheckRun::Completed(Conclusion::Success),
    );
    project.tick();

    assert_eq!(project.rt.sent_count(INTEGRATOR), 2);
    log_snapshot!(
        "revert-candidate",
        project
            .rt
            .sent(INTEGRATOR, 2)
            .replace(&revert, "<revert>")
            .replace(&delivered, "<reverted delivery>")
    );
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG2
#[test]
fn a_red_result_on_a_commit_main_has_moved_past_is_not_reported() {
    let project = finalized(true);
    let earlier = work_head(&project);
    promote(&project, &earlier);
    let latest = commit_on_main(&project, "later.md");
    project.repo.git(["push", "--quiet", "origin", "main"]);
    project
        .github
        .set_status(REPOSITORY, &latest, "ci", StatusState::Success);

    // The earlier commit's CI reports red only after main moved past it.
    project
        .github
        .set_status(REPOSITORY, &earlier, "ci", StatusState::Failure);
    project.tick();

    assert_eq!(project.rt.sent_count(INTEGRATOR), 0);
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG2
#[test]
fn each_new_failing_head_is_reported_afresh() {
    let (project, delivered) = failing_main();
    project.rt.set_status(INTEGRATOR, "ended ok");
    let next = commit_on_main(&project, "next.md");
    project.repo.git(["push", "--quiet", "origin", "main"]);
    project
        .github
        .set_status(REPOSITORY, &next, "ci", StatusState::Failure);

    project.tick();

    assert_eq!(project.rt.sent_count(INTEGRATOR), 2);
    log_snapshot!(
        "new-failing-head",
        project
            .rt
            .sent(INTEGRATOR, 2)
            .replace(&next, "<new failing head>")
            .replace(&delivered, "<earlier failing head>")
    );
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG2
#[test]
fn the_remotes_current_head_is_judged_even_when_this_checkout_never_fetched_it() {
    let project = finalized(true);
    let old_head = project.repo.head();
    // Another checkout pushes a new head of main; this one never fetches.
    let other = project
        .env
        .clone_repo(&project.env.path("demo-origin.git"), "other-checkout");
    other.write("OTHER.md", "a delivery from elsewhere\n");
    let new_head = other.commit("feat: a delivery from another checkout");
    other.git(["push", "--quiet", "origin", "main"]);
    project
        .github
        .set_status(REPOSITORY, &old_head, "ci", StatusState::Failure);
    project
        .github
        .set_status(REPOSITORY, &new_head, "ci", StatusState::Failure);

    project.tick();

    assert_eq!(project.rt.sent_count(INTEGRATOR), 1);
    log_snapshot!(
        "remote-current-head-failing",
        project
            .rt
            .sent(INTEGRATOR, 1)
            .replace(&new_head, "<remote's current head>")
            .replace(&old_head, "<this checkout's stale head>")
    );
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG4
#[test]
fn a_tick_publishes_contracts_integrated_on_another_computer() {
    use instrukt_core::{home::Home, ids::ProjectSlug, publication::PublishedRecord};
    use std::time::{Duration, Instant};
    let p = Tracked::new(false); // todos on, events off: maintenance still runs.
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_instrukt"),
        p.env.path("runtime-bin/instrukt"),
    )
    .unwrap();
    p.env.run(p.repo.path(), ["hooks", "install"]).success();
    let other = p
        .env
        .clone_repo(&p.env.path("demo-origin.git"), "integrator");
    other.write(
        "docs/project/concept/remote-contract.md",
        &documented::snippet(
            "concept",
            "Remote contract",
            "A reviewed integrated contract.",
            "",
            &[],
            "The new contract is available before build.\n",
        ),
    );
    let delivered = other.commit("integrator: publish reviewed contract");
    other.git(["push", "--quiet", "origin", "main"]);
    assert_ne!(p.repo.head(), delivered);
    p.tick();
    assert_eq!(p.repo.head(), delivered);
    let home = Home::at_user_home(p.env.home().to_path_buf());
    let slug = ProjectSlug::parse("demo").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while PublishedRecord::load(&home, &slug)
        .unwrap()
        .is_none_or(|r| r.commit.as_str() != delivered)
    {
        assert!(
            Instant::now() < deadline,
            "post-merge publication did not finish"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = p
        .env
        .run(
            p.repo.path(),
            ["docs", "get", "project/demo/concept/remote-contract"],
        )
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).replace(
        p.env.path("").to_str().unwrap().trim_end_matches('/'),
        "[TMP]",
    );
    insta::assert_snapshot!("contracts-from-other-computer", text);
    assert_eq!(p.rt.started_count(), 0);
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG4
#[test]
fn a_tick_preserves_a_dirty_canonical_checkout_and_retries_after_clearance() {
    let p = Tracked::new(false);
    let old = p.repo.head();
    let other = p
        .env
        .clone_repo(&p.env.path("demo-origin.git"), "integrator");
    other.write("integrated.txt", "green");
    let delivered = other.commit("integrator: promote candidate");
    other.git(["push", "--quiet", "origin", "main"]);
    p.repo.write("uncommitted.txt", "owned by another session");
    p.tick();
    assert_eq!(p.repo.head(), old);
    assert_eq!(
        std::fs::read(p.repo.path().join("uncommitted.txt")).unwrap(),
        b"owned by another session"
    );
    let log = std::fs::read_to_string(
        p.env
            .home()
            .join(".local/state/instrukt-ai/instrukt/tick.log"),
    )
    .unwrap();
    let lines = log
        .lines()
        .filter(|l| l.contains("event=canonical_checkout_sync_failed"))
        .map(|l| {
            l.split_once(' ')
                .unwrap()
                .1
                .replace(p.repo.path().to_str().unwrap(), "[PROJECT]")
        })
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("dirty-main-sync-refused", lines);
    std::fs::remove_file(p.repo.path().join("uncommitted.txt")).unwrap();
    p.tick();
    assert_eq!(p.repo.head(), delivered);
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG4
#[test]
fn a_tick_preserves_diverged_local_history_instead_of_forcing_main() {
    let p = Tracked::new(false);
    let other = p
        .env
        .clone_repo(&p.env.path("demo-origin.git"), "integrator");
    other.write("remote.txt", "green");
    let delivered = other.commit("integrator: promote candidate");
    other.git(["push", "--quiet", "origin", "main"]);
    p.repo.write("local.txt", "unexpected history");
    let local = p.repo.commit("fixture: unexpected local commit");
    p.tick();
    assert_eq!(p.repo.head(), local);
    assert_ne!(local, delivered);
    assert_eq!(p.repo.git(["rev-parse", "origin/main"]), delivered);
    let log = std::fs::read_to_string(
        p.env
            .home()
            .join(".local/state/instrukt-ai/instrukt/tick.log"),
    )
    .unwrap();
    let lines = log
        .lines()
        .filter(|l| l.contains("event=canonical_checkout_sync_failed"))
        .map(|l| {
            l.split_once(' ')
                .unwrap()
                .1
                .replace(p.repo.path().to_str().unwrap(), "[PROJECT]")
        })
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("diverged-main-sync-refused", lines);
}
