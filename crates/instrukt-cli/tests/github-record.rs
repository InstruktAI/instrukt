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
use support::{Project, REPOSITORY, redact};

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
        "implementation_plan",
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
        "review_findings",
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
