//! `project/instrukt/spec/feature/work/github-record`: the local
//! record keeps accepted work through GitHub outages, a delivered todo's
//! evidence can be followed on GitHub, progress that loses to a change made
//! elsewhere is not reflected until refreshed, records posted without write
//! access never change the work, todos need the project's code on GitHub,
//! and a todo is refreshed before work on it starts and before each command
//! acts on it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/flavor.rs"]
mod flavor;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod support;

use common::TestEnv;
use common::github::{CannedResponse, FakeGitHub};
use std::collections::BTreeSet;
use std::path::Path;

use instrukt_core::config::CURRENT_SCHEMA_VERSION;
use serde_json::Value;
use support::{Project, REPOSITORY, redact};

const INTENT: &str = "Keep the depth of nested lists when parsing Markdown.";
const PRD: &str = "Lists keep their depth through parse and render.";

const RECORD_MARKER: &str = "<!-- instrukt:work-record:v2:";
const CLOSE: &str = " -->";

// spec: project/instrukt/spec/feature/work/github-record UC-GR1
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

// spec: project/instrukt/spec/feature/work/github-record UC-GR1
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

// spec: project/instrukt/spec/feature/work/github-record UC-GR1
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

// spec: project/instrukt/spec/feature/work/github-record UC-GR1
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

// spec: project/instrukt/spec/feature/work/github-record UC-GR2
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

// spec: project/instrukt/spec/feature/work/github-record UC-GR3
#[test]
fn progress_that_loses_to_a_change_made_elsewhere_is_reflected_only_after_refreshing() {
    let mut project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    let elsewhere = project.other_computer();
    // This computer records progress while it cannot reach GitHub.
    let gh_config = project.env.path("gh-config");
    std::fs::create_dir_all(&gh_config).unwrap();
    project
        .env
        .remove_env("GITHUB_TOKEN")
        .set_env("GH_CONFIG_DIR", gh_config.to_str().unwrap());
    let recorded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    // Meanwhile the other computer's change reaches GitHub first.
    elsewhere.ok([
        "update",
        "nested-lists",
        "--hold",
        "--reason",
        "waiting for the renderer rewrite",
    ]);
    let chain_before = revision_headers(&project);

    project.env.set_env("GITHUB_TOKEN", "test-token");
    let reflected = project.ok(["show", "nested-lists"]);
    let refused = project
        .todo(["artifacts", "write", "nested-lists", "log", "Started."])
        .failure();
    assert_eq!(
        revision_headers(&project),
        chain_before,
        "the losing progress overwrote nothing on GitHub"
    );
    let refreshed = project.ok(["refresh", "nested-lists"]);
    let rerecorded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "lost-then-refreshed",
        redact(&format!(
            "{recorded}---\n{reflected}---\n{}---\n{refreshed}---\n{rerecorded}---\n{shown}",
            String::from_utf8_lossy(&refused.get_output().stderr)
        ))
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR3
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

// spec: project/instrukt/spec/feature/work/github-record UC-GR4
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

// spec: project/instrukt/spec/feature/work/github-record UC-GR6
#[test]
fn switching_todos_on_for_code_hosted_elsewhere_is_refused_in_plain_words() {
    let env = TestEnv::new();
    let repo = hosted_elsewhere(&env);
    let before = repo.read("instrukt.yaml");
    let revision = config_revision(&env, repo.path());

    let refused = env
        .run(
            repo.path(),
            [
                "config",
                "patch",
                "--revision",
                &revision,
                "--yaml",
                "modules: {todos: true}",
            ],
        )
        .failure();

    insta::assert_snapshot!("todos-need-github", told(&env, &refused));
    assert_eq!(
        repo.read("instrukt.yaml"),
        before,
        "the configuration is unchanged"
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR6
#[test]
fn todos_on_from_the_machine_for_code_hosted_elsewhere_refuse_and_validation_says_why() {
    let env = TestEnv::new();
    let global = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(global.parent().unwrap()).unwrap();
    std::fs::write(
        &global,
        format!("schema_version: {CURRENT_SCHEMA_VERSION}\nmodules:\n  todos: true\n"),
    )
    .unwrap();
    let repo = hosted_elsewhere(&env);

    let listed = env.run(repo.path(), ["todo", "list"]).failure();
    let validated = env.run(repo.path(), ["config", "validate"]).failure();

    insta::assert_snapshot!(
        "todo-commands-need-github",
        format!("{}---\n{}", told(&env, &listed), told(&env, &validated))
    );
}

/// A project whose code is kept on a host other than GitHub: its default
/// branch tracks a GitLab remote and `github.repository` is not set.
fn hosted_elsewhere(env: &TestEnv) -> common::Repo {
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        &format!("schema_version: {CURRENT_SCHEMA_VERSION}\nproject: demo\n"),
    );
    repo.commit("the developer's project");
    repo.git([
        "remote",
        "add",
        "origin",
        "https://gitlab.com/acme/demo.git",
    ]);
    repo.git(["config", "branch.main.remote", "origin"]);
    repo
}

fn config_revision(env: &TestEnv, cwd: &Path) -> String {
    let out = env.run(cwd, ["config", "get", "--revision"]).success();
    String::from_utf8(out.get_output().stdout.clone())
        .unwrap()
        .trim()
        .to_owned()
}

/// What a refused command told the developer, with the test's directory
/// shown as `[TMP]`.
fn told(env: &TestEnv, assert: &assert_cmd::assert::Assert) -> String {
    let base = env.path("").display().to_string();
    let output = assert.get_output();
    // Diagnostics carry the real time; what the developer is told does not.
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .lines()
    .filter(|line| !line.starts_with(|c: char| c.is_ascii_digit()))
    .map(|line| format!("{line}\n"))
    .collect::<String>()
    .replace(base.trim_end_matches('/'), "[TMP]")
}

/// The computer that held the todo before its claim moved.
const OTHER_MACHINE: &str = "c0ffee00c0ffee00c0ffee00c0ffee00";

// spec: project/instrukt/spec/feature/work/github-record UC-GR7
#[test]
fn a_todo_whose_claim_moves_here_starts_from_what_github_holds_now() {
    let project = flavor::Tracked::new(true);
    let collaborator = support::computer(&project.github);
    collaborator.ok([
        "create",
        "tidy-imports",
        "Sort the imports of every module.",
        "--kind",
        "change",
        "--approved",
    ]);
    // Another computer holds the todo.
    seed_revision(&project.github, |record| {
        record["claim"] =
            serde_json::json!({"machine": OTHER_MACHINE, "at": "2026-11-03T12:00:00Z"});
    });
    project.tick();
    assert!(
        project.rt.calls().is_empty(),
        "work held elsewhere is not dispatched here"
    );
    project.github.clear_requests();
    let listed = project.todo(["list"]);
    assert!(
        project.github.requests().is_empty(),
        "a listing reads no GitHub: {:?}",
        project.github.requests()
    );

    // A collaborator changes the todo while it is held, then the claim is
    // released.
    seed_revision(&project.github, |record| {
        record["hold_history"] = serde_json::json!([{
            "hold": {
                "source": "manual",
                "reason": "the import order is being agreed",
                "by": {"person": {"name": "Other Developer"}},
                "at": "2026-11-03T12:00:00Z"
            },
            "released": "2026-11-03T13:00:00Z",
            "by": {"person": {"name": "Other Developer"}}
        }]);
    });
    seed_revision(&project.github, |record| {
        record.as_object_mut().unwrap().remove("claim");
    });
    project.tick();

    assert_eq!(project.rt.calls().len(), 1, "{:?}", project.rt.calls());
    let shown = project.todo(["show", "tidy-imports"]);
    insta::assert_snapshot!(
        "claim-moved-here",
        redact(&format!(
            "{listed}---\n{shown}---\n{}",
            chain(&project.github)
        ))
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR7
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

    let recorded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "refreshed-before-progress",
        redact(&format!(
            "{recorded}---\n{shown}---\n{}",
            revision_headers(&project)
        ))
    );
}

/// Appends the next revision of todo #1 to its issue, built on its latest
/// one and changed by `change`: what an instrukt elsewhere writes.
fn seed_revision(github: &FakeGitHub, change: impl FnOnce(&mut Value)) {
    let latest = latest_envelope(github);
    let mut record = latest["record"].clone();
    let base = record["revision"].as_u64().unwrap();
    record["revision"] = Value::from(base + 1);
    change(&mut record);
    let slug = record["slug"].as_str().unwrap().to_owned();
    let next = serde_json::json!({"base": base, "record": record});
    github.seed_comment(
        REPOSITORY,
        1,
        "instrukt-test",
        "COLLABORATOR",
        &format!(
            "[instrukt record · {slug} · rev {} on rev {base}]\n{RECORD_MARKER}{next}{CLOSE}\n",
            base + 1
        ),
    );
}

/// The latest trusted record envelope on issue #1.
fn latest_envelope(github: &FakeGitHub) -> Value {
    let line = github
        .comments(REPOSITORY, 1)
        .into_iter()
        .rev()
        .filter(|c| c["author_association"] == "COLLABORATOR")
        .find_map(|c| {
            c["body"]
                .as_str()?
                .lines()
                .find(|l| l.starts_with(RECORD_MARKER))
                .map(str::to_owned)
        })
        .unwrap();
    serde_json::from_str(&line[RECORD_MARKER.len()..line.len() - CLOSE.len()]).unwrap()
}

/// The first line of every comment on issue #1: the revision chain.
fn chain(github: &FakeGitHub) -> String {
    github
        .comments(REPOSITORY, 1)
        .iter()
        .filter_map(|c| c["body"].as_str()?.lines().next().map(str::to_owned))
        .collect::<Vec<_>>()
        .join("\n")
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

/// The board state the fake holds for `<owner>/<number>`: its fields with
/// their options in order, then each item, named by the issue it shows
/// (`#<number>`, by the node ids in `issues`), with its field values.
fn board_state(project: &Project, owner: &str, number: u64, issues: &[(u64, &str)]) -> String {
    let fields = project
        .github
        .project_fields(owner, number)
        .into_iter()
        .map(|(name, options)| format!("field {name}: [{}]", options.join(", ")));
    let mut items: Vec<String> = project
        .github
        .project_items(owner, number)
        .into_iter()
        .map(|(node, values)| {
            let (issue, _) = issues.iter().find(|(_, n)| *n == node).unwrap();
            let values: Vec<String> = values
                .into_iter()
                .map(|(field, option)| format!("{field} = {option}"))
                .collect();
            format!("item #{issue}: {}", values.join(", "))
        })
        .collect();
    items.sort();
    fields.chain(items).collect::<Vec<_>>().join("\n")
}

/// `instrukt todo show` for each of `slugs`, as the developer reads it.
fn shown(project: &Project, slugs: &[&str]) -> String {
    slugs
        .iter()
        .map(|slug| redact(&project.ok(["show", slug])))
        .collect::<Vec<_>>()
        .join("---\n")
}

/// The field name every item on board `<owner>/<number>` carries values
/// under.
fn valued_fields(project: &Project, owner: &str, number: u64) -> BTreeSet<String> {
    project
        .github
        .project_items(owner, number)
        .into_iter()
        .flat_map(|(_, values)| values.into_iter().map(|(field, _)| field))
        .collect()
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn reflected_todos_appear_on_the_chosen_board_and_follow_a_change_of_board() {
    let project = Project::new();
    let github = &project.github;
    // The developer's board, with columns and fields of its own: the
    // built-in `Status`, a non-select field, and a `Record status` already
    // holding one of instrukt's options and one of theirs.
    github.add_project("acme", 7).add_project("acme", 8);
    github.add_project_field("acme", 7, "Status", Some(&["Todo", "Done"]));
    github.add_project_field("acme", 7, "Estimate", None);
    github.add_project_field("acme", 7, "Record status", Some(&["active", "blocked"]));
    let first_before = github.project_fields("acme", 7);
    assert!(github.project_fields("acme", 8).is_empty());
    // The owned field, identified by the board state alone: the last one
    // the developer's board had.
    let (owned, owned_before) = first_before.last().cloned().unwrap();
    choose_board(&project, "acme/7");

    let slugs = ["nested-lists", "parser-depth", "renderer"];
    project.create(slugs[0], "feature", INTENT);
    project.create(slugs[1], "change", "Track list depth in the parser.");
    project.create(slugs[2], "change", "Render lists at their depth.");
    project.ok(["freeze", slugs[1]]);
    project.ok(["update", slugs[2], "--hold", "--reason", "waiting"]);
    let nodes: Vec<String> = (1..=3).map(|n| issue_node(&project, n)).collect();
    let issues: Vec<(u64, &str)> = (1..=3).zip(nodes.iter().map(String::as_str)).collect();
    let every_todo: BTreeSet<String> = nodes.iter().cloned().collect();

    assert_eq!(on_board(&project, "acme", 7), every_todo);
    assert_eq!(
        valued_fields(&project, "acme", 7),
        BTreeSet::from([owned.clone()])
    );
    // Reused, not recreated: every field keeps its place; every field but
    // the owned one is exactly as it was; the owned one keeps its options
    // first.
    let first_after = github.project_fields("acme", 7);
    assert_eq!(first_after.len(), first_before.len());
    assert_eq!(
        first_after[..first_before.len() - 1],
        first_before[..first_before.len() - 1]
    );
    let (name, options) = first_after.last().unwrap();
    assert_eq!(name, &owned);
    assert_eq!(&options[..owned_before.len()], owned_before.as_slice());
    let first_shown = shown(&project, &slugs);
    let first_board = board_state(&project, "acme", 7, &issues);

    // The developer chooses another board; the next reflection puts the
    // work there with its current status.
    choose_board(&project, "acme/8");
    project.ok(["update", slugs[0], "--hold", "--reason", "waiting"]);
    project.ok(["update", slugs[2], "--release"]);
    project.ok(["refresh", slugs[1]]);

    assert_eq!(on_board(&project, "acme", 8), every_todo);
    // Created there, under the same name, as the only field.
    let second_after = github.project_fields("acme", 8);
    assert_eq!(second_after.len(), 1);
    assert_eq!(second_after[0].0, owned);
    assert_eq!(valued_fields(&project, "acme", 8), BTreeSet::from([owned]));
    // The first board is left as the work last put it.
    assert_eq!(github.project_fields("acme", 7), first_after);

    insta::assert_snapshot!(
        "shown-before-and-after-the-switch",
        format!("{first_shown}===\n{}", shown(&project, &slugs))
    );
    insta::assert_snapshot!(
        "boards-before-and-after-the-switch",
        format!(
            "{first_board}\n===\n{}",
            board_state(&project, "acme", 8, &issues)
        )
    );
}
