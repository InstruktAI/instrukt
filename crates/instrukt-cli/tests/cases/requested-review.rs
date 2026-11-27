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
        project.rt.folder("s1"),
        std::fs::canonicalize(project.repo.path()).unwrap(),
        "the requested reviewer runs in the registered checkout's runtime project"
    );
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
        "the dispute wakes the reviewer that recorded the verdict"
    );
    let commits = [(head.as_str(), "HEAD"), (base.as_str(), "BASE")];
    insta::assert_snapshot!(
        "requested-review-dispute-input",
        project.redact(&project.rt.sent("s1", 1), &commits, &[])
    );
    project.rt.consume("s1");
    project.rt.set_status("s1", "ended ok");

    let fixed = project.commit_in(&worktree, "fix.txt");
    let rereview = project
        .ask(
            &worktree,
            "feature",
            &fixed,
            &project.report("fix.md", None),
            &["--prior-report", prior.to_str().unwrap(), "--json"],
        )
        .success();
    assert_eq!(
        (project.rt.started_count(), project.rt.sent_count("s1")),
        (1, 2),
        "the fixing commit wakes the reviewer that recorded the earlier verdict"
    );
    let rereview_json: serde_json::Value = serde_json::from_str(&stdout(&rereview)).unwrap();
    assert_eq!(
        rereview_json["re_review"],
        serde_json::json!({
            "reached_reviewer": {
                "earlier": {
                    "head": head,
                    "traced_by": "digest"
                }
            }
        })
    );
    insta::assert_snapshot!(
        "requested-review-rereview-input",
        project.redact(
            &project.rt.sent("s1", 2),
            &[(fixed.as_str(), "FIXED"), (base.as_str(), "BASE")],
            &[]
        )
    );
    project.rt.consume("s1");
    project.rt.set_status("s1", "ended ok");

    project.rt.archive("s1");
    let fixed2 = project.commit_in(&worktree, "fix2.txt");
    let fresh = project
        .ask(
            &worktree,
            "feature",
            &fixed2,
            &project.report("fix2.md", None),
            &["--prior-report", prior.to_str().unwrap(), "--json"],
        )
        .success();
    assert_eq!(
        project.rt.started_count(),
        2,
        "an archived earlier reviewer starts a fresh reviewer for the fixing head"
    );
    let fresh_json: serde_json::Value = serde_json::from_str(&stdout(&fresh)).unwrap();
    assert_eq!(
        fresh_json["re_review"],
        serde_json::json!({
            "fresh_reviewer": {
                "reason": "reviewer_gone",
                "earlier": {
                    "head": head,
                    "traced_by": "digest"
                }
            }
        })
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

// spec: project/instrukt/spec/feature/runner/requested-review UC-RQ4
#[test]
fn a_developers_agent_learns_its_reviews_outcome_without_asking() {
    let mut project = Reviewed::new();
    project.rt.open_session("developer-agent");
    let (worktree, head) = project.worktree("demo-feature", "feature", "one.txt");
    let base = project.repo.git(["merge-base", &head, "main"]);
    let report = project.report("one.md", None);
    let commits = [(head.as_str(), "HEAD"), (base.as_str(), "BASE")];

    project
        .ask(&worktree, "feature", &head, &report, &[])
        .success();
    assert_eq!(project.rt.started_count(), 2, "developer + one reviewer");
    let reviewer = project.rt.native_id("s2");

    project
        .record_at(&reviewer, &head, "needs_work", Some("2026-11-27T10:00:00Z"))
        .success();
    project.tick_at("2026-11-27T10:01:00Z".parse().unwrap());
    assert_eq!(
        project.rt.message_count("developer-agent"),
        1,
        "first recorded verdict is delivered to the asking session"
    );
    insta::assert_snapshot!(
        "requested-review-verdict-notice",
        project.redact(&project.rt.input_log("developer-agent"), &commits, &["s2"])
    );

    project.rt.consume("developer-agent");
    project
        .record_at(&reviewer, &head, "approve", Some("2026-11-27T10:02:00Z"))
        .success();
    project.tick_at("2026-11-27T10:03:00Z".parse().unwrap());
    assert_eq!(
        project.rt.message_count("developer-agent"),
        2,
        "revised verdict version is delivered to the asking session"
    );
    insta::assert_snapshot!(
        "requested-review-verdict-notice-revised",
        project.redact(&project.rt.input_log("developer-agent"), &commits, &["s2"])
    );
}
