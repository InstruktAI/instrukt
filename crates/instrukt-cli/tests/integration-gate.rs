//! `local/projects/instrukt/spec/feature/workflow/integration-gate`: what
//! instrukt contributes to integration. The standing integrator — here the
//! fake runtime's session — is woken with finalized work, with its exact
//! candidate's verification result read from GitHub, with the decision that
//! the project has no CI taken from the canonical branch's head, and with
//! direct commits waiting on local main. The test plays the integrator's
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

/// The integrator's session, started for the first event.
const INTEGRATOR: &str = "s1";

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
    project
        .repo
        .git(["checkout", "--quiet", "-b", "tidy-imports/work"]);
    project.repo.write("src/lib.rs", "// sorted\n");
    project.repo.commit("refactor: sort the imports");
    project.todo(["stamp", "tidy-imports", "--finalize-ready"]);
    project.repo.git(["checkout", "--quiet", "main"]);
    project.tick();
    assert_eq!(project.rt.started_count(), 1, "the integrator started");
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

// spec: local/projects/instrukt/spec/feature/workflow/integration-gate UC-IG1
#[test]
fn finalized_work_starts_the_standing_integrator_with_its_handoff() {
    let project = finalized(true);
    insta::assert_snapshot!(
        "integrator-started-for-finalized-work",
        work::redact(&project.rt.opening(INTEGRATOR))
    );
}

// spec: local/projects/instrukt/spec/feature/workflow/integration-gate UC-IG1
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
    insta::assert_snapshot!(
        "green-candidate",
        project
            .rt
            .sent(INTEGRATOR, 1)
            .replace(&candidate, "<candidate>")
    );
}

// spec: local/projects/instrukt/spec/feature/workflow/integration-gate UC-IG1
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

// spec: local/projects/instrukt/spec/feature/workflow/integration-gate UC-IG1
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
    project
        .repo
        .git(["checkout", "--quiet", "tidy-imports/work"]);
    project.repo.write("src/lib.rs", "// sorted, fixed\n");
    let repaired = project.repo.commit("fix: keep the module compiling");
    project.repo.git(["checkout", "--quiet", "main"]);
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
    insta::assert_snapshot!(
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

// spec: local/projects/instrukt/spec/feature/workflow/integration-gate UC-IG1
#[test]
fn without_ci_on_the_canonical_head_the_integrator_is_told_to_verify_locally() {
    let project = finalized(false);
    let candidate = work_head(&project);
    push_candidate(&project, &candidate);

    project.tick();

    assert_eq!(project.rt.sent_count(INTEGRATOR), 1);
    insta::assert_snapshot!(
        "no-ci",
        project
            .rt
            .sent(INTEGRATOR, 1)
            .replace(&candidate, "<candidate>")
    );
}

// spec: local/projects/instrukt/spec/feature/workflow/integration-gate UC-IG3
#[test]
fn direct_commits_on_local_main_wake_the_integrator_to_take_them_through_verification() {
    let project = Tracked::integrating();
    let pushed = project.repo.head();
    project.repo.write("NOTES.md", "a direct change\n");
    let direct = project.repo.commit("docs: a direct note");

    project.tick();

    assert_eq!(project.rt.started_count(), 1);
    insta::assert_snapshot!(
        "main-ahead",
        project
            .rt
            .opening(INTEGRATOR)
            .replace(&direct, "<local main>")
            .replace(&pushed, "<remote main>")
    );
}
