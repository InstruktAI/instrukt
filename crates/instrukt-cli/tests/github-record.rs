//! `local/projects/instrukt/spec/feature/work/github-record`: the local
//! record keeps accepted work through GitHub outages, and a delivered
//! todo's evidence can be followed on GitHub.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/work.rs"]
mod support;

use serde_json::Value;
use support::{Project, REPOSITORY, redact, stdout};

const INTENT: &str = "Keep the depth of nested lists when parsing Markdown.";

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR1
#[test]
fn progress_recorded_without_credentials_stays_local_and_says_why_it_is_pending() {
    let mut project = Project::new();
    let gh_config = project.env.path("gh-config");
    std::fs::create_dir_all(&gh_config).unwrap();
    project
        .env
        .remove_env("GITHUB_TOKEN")
        .set_env("GH_CONFIG_DIR", gh_config.to_str().unwrap());

    let recorded = project.create("nested-lists", "feature", INTENT);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "recorded-without-credentials",
        redact(&format!("{recorded}---\n{shown}"))
    );
    assert!(
        project.github.requests().is_empty(),
        "nothing reached GitHub"
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR1
#[test]
fn reflection_held_back_by_rate_limiting_completes_once_github_accepts_requests() {
    let project = Project::new();
    // GitHub limits the next two requests; its window has already reset.
    project.github.rate_limit_next(2, 0);

    let limited = project.create("nested-lists", "feature", INTENT);
    let still_limited = project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth through parse and render.",
    ]);
    let accepted = project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth, and numbering restarts per level.",
    ]);

    insta::assert_snapshot!(
        "rate-limited-then-reflected",
        redact(&format!("{limited}---\n{still_limited}---\n{accepted}"))
    );
    let reflected: Vec<(String, usize)> = project
        .github
        .artifacts(REPOSITORY, 1)
        .iter()
        .map(|a| {
            (
                a["artifact"]["name"].as_str().unwrap().to_owned(),
                a["versions"].as_array().unwrap().len(),
            )
        })
        .collect();
    assert_eq!(
        reflected,
        [("input".to_owned(), 1), ("prd".to_owned(), 2)],
        "every accepted version reached GitHub"
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR3
#[test]
fn a_todo_changed_elsewhere_is_refreshed_before_progress_is_recorded() {
    let project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    other_computer_holds(&project, "waiting for the renderer rewrite");

    let refused = project
        .todo([
            "artifacts",
            "write",
            "nested-lists",
            "prd",
            "Lists keep their depth through parse and render.",
        ])
        .failure();
    let refreshed = project.ok(["refresh", "nested-lists"]);
    let recorded = project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth through parse and render.",
    ]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "refused-then-refreshed",
        redact(&format!(
            "{}---\n{refreshed}---\n{recorded}---\n{shown}",
            String::from_utf8_lossy(&refused.get_output().stderr)
        ))
    );
    assert!(
        stdout(&refused).is_empty(),
        "the refused change reports nothing as recorded"
    );
}

/// Another computer records a hold on todo #1: its instrukt writes the
/// next revision into the record comment, as GitHub then serves it.
fn other_computer_holds(project: &Project, reason: &str) {
    let (id, next) = next_revision_holding(project, reason);
    project.github.edit_comment(REPOSITORY, id, &next);
}

/// The record comment of todo #1 and the envelope of its next revision
/// with a hold for `reason`: what an instrukt elsewhere writes, or what a
/// forger copies.
fn next_revision_holding(project: &Project, reason: &str) -> (u64, String) {
    const MARKER: &str = "<!-- instrukt:work-record:v1:";
    const CLOSE: &str = " -->";
    let comment = project
        .github
        .comments(REPOSITORY, 1)
        .into_iter()
        .find(|c| c["body"].as_str().unwrap().contains(MARKER))
        .unwrap();
    let body = comment["body"].as_str().unwrap();
    let line = body.lines().find(|l| l.starts_with(MARKER)).unwrap();
    let mut record: Value =
        serde_json::from_str(&line[MARKER.len()..line.len() - CLOSE.len()]).unwrap();
    record["revision"] = Value::from(record["revision"].as_u64().unwrap() + 1);
    record["hold"] = serde_json::json!({
        "source": "manual",
        "reason": reason,
        "by": {"person": {"name": "Other Developer"}},
        "at": "2026-11-03T12:00:00Z",
    });
    let next = format!(
        "[instrukt record · nested-lists · rev {}]\n{MARKER}{record}{CLOSE}\n",
        record["revision"]
    );
    (comment["id"].as_u64().unwrap(), next)
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR4
#[test]
fn records_posted_without_write_access_never_change_the_work_and_collaborators_still_do() {
    let project = Project::new();
    project.create("nested-lists", "feature", INTENT);

    let (_, forged) = next_revision_holding(&project, "forged: stop all work");
    project
        .github
        .seed_comment(REPOSITORY, 1, "passer-by", "NONE", &forged);
    project
        .github
        .seed_comment(REPOSITORY, 1, "drive-by", "CONTRIBUTOR", &forged);
    let after_forgery = project.ok(["refresh", "nested-lists"]);
    let unchanged = project.ok(["show", "nested-lists"]);

    let (_, agreed) = next_revision_holding(&project, "waiting for the renderer rewrite");
    project
        .github
        .seed_comment(REPOSITORY, 1, "teammate", "COLLABORATOR", &agreed);
    let after_collaborator = project.ok(["refresh", "nested-lists"]);
    let applied = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "untrusted-ignored-trusted-applied",
        redact(&format!(
            "{after_forgery}---\n{unchanged}---\n{after_collaborator}---\n{applied}"
        ))
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR1
#[test]
fn progress_recorded_while_github_is_unreachable_stays_local_and_pending() {
    let mut project = Project::new();
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    let issue = 1;
    let reflected_before = project.github.artifacts(REPOSITORY, issue);

    project.github.stop();
    let recorded = project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth through parse and render.",
    ]);
    let shown = project.ok(["show", "nested-lists"]);
    let read = project.ok(["artifacts", "read", "nested-lists", "prd"]);

    insta::assert_snapshot!(
        "recorded-while-unreachable",
        redact(&format!("{recorded}---\n{shown}---\n{read}"))
    );
    assert_eq!(
        project.github.artifacts(REPOSITORY, issue),
        reflected_before,
        "nothing reached GitHub"
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR2
#[test]
fn a_delivered_todo_shows_its_intent_artifacts_decisions_and_commit_on_github() {
    let project = Project::new();
    let repo = &project.repo;
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth through parse and render.",
    ]);
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "implementation-plan",
        "1. Track depth in the parser.\n2. Render it.",
    ]);
    repo.git(["checkout", "--quiet", "-b", "nested-lists"]);
    repo.write("parser.rs", "fn parse() {}\n");
    let built = repo.commit("Track list depth");
    project
        .github
        .set_branch(REPOSITORY, "nested-lists", &built);
    project.ok(["stamp", "nested-lists", "--build-started"]);
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "review-findings",
        "No findings.",
    ]);
    project.ok([
        "stamp",
        "nested-lists",
        "--verdict",
        "approve",
        "--phase",
        "build_review",
    ]);

    project.ok(["deliver", "nested-lists", "--commit", &built]);

    insta::assert_snapshot!("delivery-on-github", redact(&github_view(&project)));
}

/// What a collaborator sees on GitHub: each issue and pull request with
/// its state, body, comments, artifacts and reviews.
fn github_view(project: &Project) -> String {
    let mut out = String::new();
    for item in project.github.issues(REPOSITORY) {
        let number = item["number"].as_u64().unwrap();
        let kind = if item.get("pull_request").is_some() {
            "pull request"
        } else {
            "issue"
        };
        out.push_str(&format!(
            "{kind} #{number} {} [{} {}]\n",
            item["title"].as_str().unwrap(),
            item["state"].as_str().unwrap(),
            item["state_reason"].as_str().unwrap_or("-"),
        ));
        out.push_str(&format!("labels: {}\n", labels(&item)));
        out.push_str(&format!("{}\n", item["body"].as_str().unwrap_or_default()));
        for comment in project.github.comments(REPOSITORY, number) {
            let body = comment["body"].as_str().unwrap();
            for line in body.lines().filter(|line| !line.starts_with("<!--")) {
                out.push_str(&format!("comment: {line}\n"));
            }
        }
        for artifact in project.github.artifacts(REPOSITORY, number) {
            out.push_str(&format!(
                "artifact {} ({})\n",
                artifact["artifact"]["name"].as_str().unwrap(),
                artifact["artifact"]["type"].as_str().unwrap()
            ));
            let mut versions: Vec<&Value> =
                artifact["versions"].as_array().unwrap().iter().collect();
            versions.reverse();
            for version in versions {
                out.push_str(&format!(
                    "  v{}: {}\n",
                    version["version"],
                    version["body"].as_str().unwrap().replace('\n', " / ")
                ));
            }
        }
    }
    let reviews = project
        .github
        .requests_to("POST", "/reviews")
        .into_iter()
        .map(|r| r.json()["body"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    for review in reviews {
        out.push_str(&format!("review: {review}\n"));
    }
    out
}

fn labels(item: &Value) -> String {
    item["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}
