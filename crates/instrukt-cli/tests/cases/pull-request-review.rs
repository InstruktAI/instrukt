//! `project/instrukt/spec/feature/runner/pull-request-review`: the current
//! head of every ready pull request from a project's trusted authors is
//! reviewed independently on instrukt's own initiative. The runner's tick
//! observes the fake GitHub's pull requests and asks for each owed review;
//! the review procedure's session starts on the fake runtime.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::fixtures::reviewed::{REPOSITORY, Reviewed};

// spec: project/instrukt/spec/feature/runner/pull-request-review UC-RV1
#[test]
fn a_ready_pull_request_head_is_reviewed_once_and_each_new_head_in_its_own_right() {
    let mut project = Reviewed::opted_in();
    let first = project.push("feature", "one.txt");
    project.open("feature", false);
    project.push("sketch", "draft.txt");
    let sketch = project.open("sketch", true);

    project.tick();
    assert_eq!(
        project.rt.started_count(),
        1,
        "the ready pull request's head is reviewed; the draft is not"
    );

    project
        .record(&project.rt.native_id("s1"), &first, "approve")
        .success();
    project.rt.consume("s1");
    project.rt.set_status("s1", "ended ok");
    // Well past any wait before a commit is asked for again: the turn that
    // received the request has ended, and the head is observed again.
    let later = jiff::Timestamp::now() + jiff::SignedDuration::from_hours(5);
    project.tick_at(later);
    project.tick_at(later + jiff::SignedDuration::from_secs(30));
    assert_eq!(
        (project.rt.started_count(), project.rt.sent_count("s1")),
        (1, 0),
        "observing the reviewed head again starts no second review"
    );

    project.push("feature", "two.txt");
    project.tick_at(later + jiff::SignedDuration::from_mins(1));
    assert_eq!(
        project.rt.started_count() + project.rt.sent_count("s1"),
        2,
        "the new head is asked for in its own right"
    );

    project.github.set_pull_draft(REPOSITORY, sketch, false);
    project.tick_at(later + jiff::SignedDuration::from_mins(2));
    assert_eq!(
        project.rt.started_count() + project.rt.sent_count("s1"),
        3,
        "the draft is reviewed once it is marked ready"
    );
}

/// The project layer's revision, as `instrukt config get --revision` prints
/// it.
fn revision(project: &Reviewed) -> String {
    let out = project
        .env
        .run(project.root(), ["config", "get", "--revision"])
        .success();
    String::from_utf8(out.get_output().stdout.clone())
        .unwrap()
        .trim()
        .to_owned()
}

/// Switches automatic review on through `instrukt config patch`.
fn opt_in(project: &Reviewed) -> assert_cmd::assert::Assert {
    project
        .env
        .instrukt(project.root())
        .args([
            "config",
            "patch",
            "--revision",
            &revision(project),
            "--yaml",
            "review: {automatic: true}",
        ])
        .assert()
}

// spec: project/instrukt/spec/feature/runner/pull-request-review UC-RV2
#[test]
fn opting_in_is_admitted_with_a_publishing_reviewer_and_refused_without_one() {
    let project = Reviewed::new();
    project.push("feature", "one.txt");
    project.open("feature", false);
    project.grant_reviewer(false);

    let refused = opt_in(&project).failure();
    let stderr = String::from_utf8(refused.get_output().stderr.clone()).unwrap();
    insta::assert_snapshot!(
        "opting-in-without-commit-status-write",
        stderr.replace(project.root().to_str().unwrap(), "[ROOT]")
    );

    project.grant_reviewer(true);
    opt_in(&project).success();
    project.tick();
    assert_eq!(
        project.rt.started_count(),
        1,
        "the project's pull requests are observed once the setting is admitted"
    );
}

// spec: project/instrukt/spec/feature/runner/pull-request-review UC-RV3
#[test]
fn outside_contributions_and_dependency_updates_start_no_review() {
    let project = Reviewed::opted_in();
    let pull = |head: &str, login: &str, association: &str, fork: Option<&str>| {
        project.push(head, &format!("{head}.txt"));
        project.github.seed_pull(
            REPOSITORY,
            crate::common::github::PullSeed {
                title: format!("Change {head}"),
                head: head.to_owned(),
                base: "main".into(),
                draft: false,
                login: Some(login.to_owned()),
                association: Some(association.to_owned()),
                fork: fork.map(str::to_owned),
            },
        );
    };
    pull("from-fork", "dev", "MEMBER", Some("stranger/demo"));
    pull("outside", "passer-by", "CONTRIBUTOR", None);
    pull("deps", "renovate[bot]", "NONE", None);

    project.tick();

    assert_eq!(project.rt.started_count(), 0, "no review is started");
}
