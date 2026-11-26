//! `project/instrukt/spec/feature/runner/requested-review`: an authoring
//! session asks for the independent review of an exact committed worktree
//! head by naming its coordinates only, and instrukt starts the reviewer on
//! an assignment it rendered from them. The reviewer starts, and receives
//! later asks, on the fake runtime; automatic review's requests come from the
//! fake GitHub.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::fixtures::reviewed::Reviewed;

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stderr.clone()).unwrap()
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

// spec: project/instrukt/spec/feature/runner/requested-review UC-RQ1
#[test]
fn committed_work_is_reviewed_from_its_coordinates_alone() {
    let project = Reviewed::new();
    let (worktree, head) = project.worktree("demo-feature", "feature", "one.txt");
    let base = project.repo.git(["merge-base", &head, "main"]);
    let report = project.report("one.md", None);
    let commits = [(head.as_str(), "HEAD"), (base.as_str(), "BASE")];

    let asked = project
        .ask(&worktree, "feature", &head, &report, &[])
        .success();
    assert_eq!(project.rt.started_count(), 1, "one reviewer is started");
    assert_eq!(
        project.rt.worktree("s1"),
        Some(std::fs::canonicalize(&worktree).unwrap()),
        "the requested reviewer starts in the candidate's linked worktree"
    );
    assert_eq!(
        project.rt.branch("s1").as_deref(),
        Some("feature"),
        "the requested reviewer attaches to the candidate's branch"
    );
    insta::assert_snapshot!(
        "requested-review-reply",
        project.redact(&stdout(&asked), &commits, &["s1"])
    );
    insta::assert_snapshot!(
        "requested-review-opening-input",
        project.redact(&project.rt.opening("s1"), &commits, &[])
    );

    project
        .ask(&worktree, "feature", &head, &report, &[])
        .success();
    assert_eq!(
        project.rt.started_count(),
        1,
        "the same ask again starts nothing new"
    );

    std::fs::write(worktree.join("scratch.txt"), "draft\n").unwrap();
    let dirty = project
        .ask(&worktree, "feature", &head, &report, &[])
        .failure();
    insta::assert_snapshot!(
        "requested-review-dirty-worktree",
        project.redact(&stderr(&dirty), &commits, &[])
    );
    std::fs::remove_file(worktree.join("scratch.txt")).unwrap();

    let moved = project.commit_in(&worktree, "two.txt");
    let stale = project
        .ask(&worktree, "feature", &head, &report, &[])
        .failure();
    insta::assert_snapshot!(
        "requested-review-branch-moved",
        project.redact(
            &stderr(&stale),
            &[(moved.as_str(), "MOVED"), (head.as_str(), "HEAD")],
            &[]
        )
    );
    assert_eq!(
        project.rt.started_count(),
        1,
        "a refused ask starts no reviewer"
    );
}

// spec: project/instrukt/spec/feature/runner/requested-review UC-RQ2
#[test]
fn findings_on_an_unchanged_head_are_disputed_with_the_reviewer_that_recorded_them() {
    let project = Reviewed::new();
    let (worktree, head) = project.worktree("demo-feature", "feature", "one.txt");
    let base = project.repo.git(["merge-base", &head, "main"]);
    let report = project.report("one.md", None);
    project
        .ask(&worktree, "feature", &head, &report, &[])
        .success();
    let reviewer = project.rt.native_id("s1");
    project.record(&reviewer, &head, "needs_work").success();
    project
        .ask(&worktree, "feature", &head, &report, &[])
        .success();
    assert_eq!(
        (project.rt.started_count(), project.rt.sent_count("s1")),
        (1, 0),
        "the same ask while the turn it opened goes on returns that ask"
    );
    project.rt.consume("s1");
    project.rt.set_status("s1", "ended ok");

    let prior = project.report("one-findings.md", Some("# Review\n\nVerdict: needs_work\n"));
    let dispositions = project.report("one-dispositions.md", Some("F1: rejected\n"));
    let disputed = project.report("one-dispute.md", None);
    project
        .ask(
            &worktree,
            "feature",
            &head,
            &disputed,
            &[
                "--prior-report",
                prior.to_str().unwrap(),
                "--disposition",
                dispositions.to_str().unwrap(),
            ],
        )
        .success();
    assert_eq!(
        (project.rt.started_count(), project.rt.sent_count("s1")),
        (1, 1),
        "the dispute reaches the reviewer that recorded the verdict"
    );
    let commits = [(head.as_str(), "HEAD"), (base.as_str(), "BASE")];
    insta::assert_snapshot!(
        "requested-review-dispute-input",
        project.redact(&project.rt.sent("s1", 1), &commits, &[])
    );

    project.record(&reviewer, &head, "approve").success();
    let shown = project
        .as_session(&reviewer)
        .args(["review", "show", &head])
        .assert()
        .success();
    // The run's clock decides the timestamps `review show` prints.
    insta::with_settings!({filters => vec![(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z", "[TIME]")]}, {
        insta::assert_snapshot!(
            "requested-review-revised-verdict",
            project.redact(&stdout(&shown), &commits, &["s1"])
        );
    });

    let fixed = project.commit_in(&worktree, "fix.txt");
    project
        .ask(
            &worktree,
            "feature",
            &fixed,
            &project.report("fix.md", None),
            &["--prior-report", prior.to_str().unwrap()],
        )
        .success();
    assert_eq!(
        project.rt.started_count(),
        2,
        "the fixing commit is reviewed in its own right"
    );
}

// spec: project/instrukt/spec/feature/runner/requested-review UC-RQ3
#[test]
fn a_pull_request_head_under_automatic_review_is_not_reviewed_twice() {
    let project = Reviewed::opted_in();
    let (worktree, head) = project.worktree("demo-feature", "feature", "one.txt");
    project.repo.git(["push", "--quiet", "origin", "feature"]);
    project
        .github
        .set_branch(crate::fixtures::reviewed::REPOSITORY, "feature", &head);
    project.open("feature", false);
    project.tick();
    assert_eq!(
        project.rt.started_count(),
        1,
        "automatic review asked for the head"
    );

    let refused = project
        .ask(
            &worktree,
            "feature",
            &head,
            &project.report("one.md", None),
            &[],
        )
        .failure();
    insta::assert_snapshot!(
        "requested-review-request-in-flight",
        project.redact(&stderr(&refused), &[(head.as_str(), "HEAD")], &["s1"])
    );
    assert_eq!(
        project.rt.started_count(),
        1,
        "no second reviewer is started"
    );
}
