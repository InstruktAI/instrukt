//! `local/projects/instrukt/spec/feature/work/github-record`: the local
//! record keeps accepted work through GitHub outages, a delivered todo's
//! evidence can be followed on GitHub, a change made against a todo
//! changed elsewhere is refused until refreshed, and records posted without
//! write access never change the work.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/work.rs"]
mod support;

use common::github::CannedResponse;
use std::collections::BTreeSet;

use serde_json::Value;
use support::{Project, REPOSITORY, redact, stdout};

const INTENT: &str = "Keep the depth of nested lists when parsing Markdown.";
const PRD: &str = "Lists keep their depth through parse and render.";

const RECORD_MARKER: &str = "<!-- instrukt:work-record:v2:";
const CLOSE: &str = " -->";

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR1
#[test]
fn progress_recorded_while_github_is_unreachable_stays_local_and_pending() {
    let mut project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    let reflected_before = project.github.artifacts(REPOSITORY, 1);

    project.github.stop();
    let recorded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    let shown = project.ok(["show", "nested-lists"]);
    let read = project.ok(["artifacts", "read", "nested-lists", "prd"]);

    insta::assert_snapshot!(
        "recorded-while-unreachable",
        redact(&format!("{recorded}---\n{shown}---\n{read}"))
    );
    assert_eq!(
        project.github.artifacts(REPOSITORY, 1),
        reflected_before,
        "nothing reached GitHub"
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR1
#[test]
fn progress_recorded_without_credentials_stays_local_and_says_why_it_is_pending() {
    let mut project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    let requests_before = project.github.requests().len();
    let gh_config = project.env.path("gh-config");
    std::fs::create_dir_all(&gh_config).unwrap();
    project
        .env
        .remove_env("GITHUB_TOKEN")
        .set_env("GH_CONFIG_DIR", gh_config.to_str().unwrap());

    let recorded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "recorded-without-credentials",
        redact(&format!("{recorded}---\n{shown}"))
    );
    assert_eq!(
        project.github.requests().len(),
        requests_before,
        "nothing reached GitHub"
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR1
#[test]
fn reflection_held_back_by_rate_limiting_completes_once_github_accepts_requests() {
    let project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    // GitHub limits the next two requests; its window has already reset.
    project.github.rate_limit_next(2, 0);

    let limited = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    let still_limited = project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth, and numbering restarts per level.",
    ]);
    let accepted = project.ok(["artifacts", "write", "nested-lists", "log", "Started."]);

    insta::assert_snapshot!(
        "rate-limited-then-reflected",
        redact(&format!(
            "{limited}---\n{still_limited}---\n{accepted}---\n{}",
            retained_artifacts(&project)
        ))
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR1
#[test]
fn a_revision_github_accepted_whose_answer_was_lost_completes_without_a_conflict() {
    let project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    // GitHub accepts the next revision, but its answer never arrives.
    project.github.respond_once(
        "POST",
        "/issues/1/comments",
        CannedResponse {
            status: 502,
            headers: Vec::new(),
            body: serde_json::json!({"message": "Bad Gateway"}),
        },
    );
    let interrupted = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    let lost = project.github.requests_to("POST", "/issues/1/comments");
    let landed = lost.last().unwrap().json()["body"]
        .as_str()
        .unwrap()
        .to_owned();
    project
        .github
        .seed_comment(REPOSITORY, 1, "instrukt-test", "COLLABORATOR", &landed);

    let resumed = project.ok(["artifacts", "write", "nested-lists", "log", "Started."]);

    insta::assert_snapshot!(
        "answer-lost-then-reflected",
        redact(&format!(
            "{interrupted}---\n{resumed}---\n{}",
            revision_headers(&project)
        ))
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR2
#[test]
fn a_delivered_todo_shows_its_intent_artifacts_decisions_and_commit_on_github() {
    let project = Project::new();
    let repo = &project.repo;
    project.create("nested-lists", "feature", INTENT);
    project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
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

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR3
#[test]
fn a_todo_changed_elsewhere_is_refreshed_before_progress_is_recorded() {
    let project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    let elsewhere = project.other_computer();
    elsewhere.ok([
        "update",
        "nested-lists",
        "--hold",
        "--reason",
        "waiting for the renderer rewrite",
    ]);

    let refused = project
        .todo(["artifacts", "write", "nested-lists", "prd", PRD])
        .failure();
    let refreshed = project.ok(["refresh", "nested-lists"]);
    let recorded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
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

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR3
#[test]
fn of_two_computers_recording_at_once_the_later_is_told_to_refresh() {
    let project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    let elsewhere = project.other_computer();
    elsewhere.ok(["artifacts", "write", "nested-lists", "log", "Joined."]);
    project.ok(["show", "nested-lists"]);
    // What the other computer reads while this one writes.
    let before_write = project.github.comments(REPOSITORY, 1);

    let first = project.ok([
        "update",
        "nested-lists",
        "--hold",
        "--reason",
        "waiting for the renderer rewrite",
    ]);
    // The other computer's reads race the write: they still see the
    // revision both built on.
    project.github.respond_once(
        "GET",
        "/issues/1",
        CannedResponse {
            status: 304,
            headers: Vec::new(),
            body: Value::Null,
        },
    );
    project.github.respond_once(
        "GET",
        "/issues/1/comments",
        CannedResponse {
            status: 200,
            headers: Vec::new(),
            body: Value::Array(before_write),
        },
    );
    let second = elsewhere.ok([
        "update",
        "nested-lists",
        "--hold",
        "--reason",
        "waiting for a review",
    ]);
    let refused = elsewhere
        .todo(["artifacts", "write", "nested-lists", "prd", PRD])
        .failure();
    elsewhere.ok(["refresh", "nested-lists"]);
    let after_refresh = elsewhere.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "concurrent-writers",
        redact(&format!(
            "{first}---\n{second}---\n{}---\n{after_refresh}",
            String::from_utf8_lossy(&refused.get_output().stderr)
        ))
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR4
#[test]
fn records_posted_without_write_access_never_change_the_work_and_collaborators_still_do() {
    let project = Project::new();
    project.create("nested-lists", "feature", INTENT);

    let forged = next_revision_holding(&project, "forged: stop all work");
    project
        .github
        .seed_comment(REPOSITORY, 1, "passer-by", "NONE", &forged);
    project
        .github
        .seed_comment(REPOSITORY, 1, "drive-by", "CONTRIBUTOR", &forged);
    let unchanged = project.ok(["show", "nested-lists"]);

    let agreed = next_revision_holding(&project, "waiting for the renderer rewrite");
    project
        .github
        .seed_comment(REPOSITORY, 1, "teammate", "COLLABORATOR", &agreed);
    let applied = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "untrusted-ignored-trusted-applied",
        redact(&format!("{unchanged}---\n{applied}"))
    );
}

/// The envelope of todo #1's next revision, built on its latest one, with
/// a hold for `reason`: what an instrukt elsewhere writes, or what a forger
/// copies.
fn next_revision_holding(project: &Project, reason: &str) -> String {
    let latest = project
        .github
        .comments(REPOSITORY, 1)
        .into_iter()
        .rev()
        .filter(|c| c["author_association"] == "COLLABORATOR")
        .find_map(|c| {
            let body = c["body"].as_str()?.to_owned();
            let line = body
                .lines()
                .find(|l| l.starts_with(RECORD_MARKER))?
                .to_owned();
            Some(line)
        })
        .unwrap();
    let envelope: Value =
        serde_json::from_str(&latest[RECORD_MARKER.len()..latest.len() - CLOSE.len()]).unwrap();
    let mut record = envelope["record"].clone();
    let base = record["revision"].as_u64().unwrap();
    record["revision"] = Value::from(base + 1);
    record["hold"] = serde_json::json!({
        "source": "manual",
        "reason": reason,
        "by": {"person": {"name": "Other Developer"}},
        "at": "2026-11-03T12:00:00Z",
    });
    let next = serde_json::json!({"base": base, "record": record});
    format!(
        "[instrukt record · nested-lists · rev {} on rev {base}]\n{RECORD_MARKER}{next}{CLOSE}\n",
        base + 1
    )
}

/// Every artifact GitHub retains for issue #1, each version's body in
/// version order, the current one last.
fn retained_artifacts(project: &Project) -> String {
    let mut out = String::new();
    for artifact in project.github.artifacts(REPOSITORY, 1) {
        out.push_str(&format!(
            "{}\n",
            artifact["artifact"]["name"].as_str().unwrap()
        ));
        let mut versions: Vec<&Value> = artifact["versions"].as_array().unwrap().iter().collect();
        versions.reverse();
        for version in versions {
            out.push_str(&format!(
                "  v{}: {}\n",
                version["version"],
                version["body"].as_str().unwrap()
            ));
        }
        out.push_str(&format!(
            "  current: {}\n",
            artifact["artifact"]["body"].as_str().unwrap()
        ));
    }
    out
}

/// The first line of every record comment on issue #1, in order: the
/// revision chain.
fn revision_headers(project: &Project) -> String {
    project
        .github
        .comments(REPOSITORY, 1)
        .iter()
        .filter_map(|c| c["body"].as_str()?.lines().next().map(str::to_owned))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a collaborator sees on GitHub: each issue and pull request with
/// its state, body, revision chain and the latest revision's decisions,
/// artifacts and reviews.
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
        let comments = project.github.comments(REPOSITORY, number);
        for comment in &comments {
            let first = comment["body"].as_str().unwrap().lines().next().unwrap();
            out.push_str(&format!("comment: {first}\n"));
        }
        if let Some(latest) = comments.last() {
            for line in latest["body"]
                .as_str()
                .unwrap()
                .lines()
                .filter(|l| l.starts_with("- "))
            {
                out.push_str(&format!("decision {line}\n"));
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

/// Points the project's layer at board `board`.
fn choose_board(project: &Project, board: &str) {
    let layer = project.repo.path().join("instrukt.yaml");
    let text = std::fs::read_to_string(&layer).unwrap();
    let kept: String = text
        .lines()
        .filter(|line| !line.starts_with("board:"))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&layer, format!("{kept}board: {board}\n")).unwrap();
}

/// The node id of the project's issue `number`.
fn issue_node(project: &Project, number: u64) -> String {
    project.github.issue(REPOSITORY, number).unwrap()["node_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The issues on board `<owner>/<number>`, by node id.
fn on_board(project: &Project, owner: &str, number: u64) -> BTreeSet<String> {
    project
        .github
        .project_items(owner, number)
        .into_iter()
        .map(|(node, _)| node)
        .collect()
}

/// The record status the reflection wrote into issue `node`'s body: the
/// value its `Status` entry carries.
fn issue_status(project: &Project, node: &str) -> String {
    let issue = project
        .github
        .issues(REPOSITORY)
        .into_iter()
        .find(|issue| issue["node_id"] == node)
        .unwrap();
    let body = issue["body"].as_str().unwrap();
    body.lines()
        .find_map(|line| line.split_once("**Status:** "))
        .map(|(_, status)| status.trim().to_owned())
        .unwrap()
}

/// Checks board `<owner>/<number>` against what reflection wrote to the
/// issues, given its fields before reflection (`before`): every item's
/// field values are its issue's record status, under one field; the
/// fields before survive in place, and the only field that gains options
/// is that one, gaining new options that include every status its items
/// carry that it lacked.
fn assert_board_follows_issues(
    project: &Project,
    owner: &str,
    number: u64,
    before: &[(String, Vec<String>)],
) {
    let items = project.github.project_items(owner, number);
    let mut owned_field = BTreeSet::new();
    let mut carried = BTreeSet::new();
    for (node, values) in &items {
        assert_eq!(values.len(), 1, "item {node} carries exactly one field");
        let (field, status) = &values[0];
        assert_eq!(status, &issue_status(project, node), "item {node}");
        owned_field.insert(field.clone());
        carried.insert(status.clone());
    }
    let [owned_field] = owned_field
        .into_iter()
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();

    let after = project.github.project_fields(owner, number);
    // Fields the board had keep their place, names and options.
    for ((name, options), (kept, now)) in before.iter().zip(&after) {
        assert_eq!(kept, name);
        assert_eq!(&now[..options.len()], options.as_slice());
        if kept != &owned_field {
            assert_eq!(now, options, "field {kept} is untouched");
        }
    }
    // Nothing is added beside the owned field, made only when it lacked.
    let had_owned = before.iter().any(|(name, _)| name == &owned_field);
    assert_eq!(after.len(), before.len() + usize::from(!had_owned));
    let (_, options) = after.iter().find(|(name, _)| name == &owned_field).unwrap();
    let earlier = before
        .iter()
        .find(|(name, _)| name == &owned_field)
        .map_or(0, |(_, options)| options.len());
    let appended: BTreeSet<&String> = options[earlier..].iter().collect();
    assert_eq!(appended.len(), options.len() - earlier, "no option twice");
    for status in &carried {
        assert!(
            options.contains(status),
            "the field offers carried status {status}"
        );
        if !options[..earlier].contains(status) {
            assert!(appended.contains(status));
        }
    }
    assert!(
        appended
            .iter()
            .all(|option| !options[..earlier].contains(option)),
        "appended options are new"
    );
}

// spec: local/projects/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn reflected_todos_appear_on_the_chosen_board_and_follow_a_change_of_board() {
    let project = Project::new();
    let github = &project.github;
    // The developer's board, with columns and fields of its own: its
    // `Record status` already has one of instrukt's options and one of
    // theirs.
    github.add_project("acme", 7).add_project("acme", 8);
    github.add_project_field("acme", 7, "Status", Some(&["Todo", "Done"]));
    github.add_project_field("acme", 7, "Estimate", None);
    github.add_project_field("acme", 7, "Record status", Some(&["active", "blocked"]));
    let first_before = github.project_fields("acme", 7);
    let second_before = github.project_fields("acme", 8);
    choose_board(&project, "acme/7");

    project.create("nested-lists", "feature", INTENT);
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.ok(["freeze", "parser-depth"]);
    let every_todo = BTreeSet::from([issue_node(&project, 1), issue_node(&project, 2)]);

    assert_eq!(on_board(&project, "acme", 7), every_todo);
    assert_board_follows_issues(&project, "acme", 7, &first_before);

    // The developer chooses another board; the next reflection puts the
    // work there, with its current status.
    choose_board(&project, "acme/8");
    project.ok(["update", "nested-lists", "--hold", "--reason", "waiting"]);
    project.ok(["refresh", "parser-depth"]);

    assert_eq!(on_board(&project, "acme", 8), every_todo);
    assert_board_follows_issues(&project, "acme", 8, &second_before);
}
