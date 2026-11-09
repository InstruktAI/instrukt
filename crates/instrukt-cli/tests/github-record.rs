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
#[path = "support/workflow.rs"]
mod workflow;

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
    std::fs::remove_file(project.env.instrukt_home().join("secrets.env")).unwrap();
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
// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
// spec: project/instrukt/spec/feature/repository/attributed-commit UC-RC1
#[test]
fn a_delivered_todo_shows_its_intent_artifacts_decisions_and_commit_on_github() {
    let crew = workflow::Crew::new();
    let project = &crew.project;
    project.create("nested-lists", "feature", INTENT);
    project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "implementation-plan",
        "1. Track depth in the parser.\n2. Render it.",
    ]);
    let workspace = crew.workspace("nested-lists");
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "review-findings",
        "No findings.",
    ]);
    crew.write(
        "nested-lists",
        "prd-review-findings",
        "The product requirement is approved.",
    );
    crew.stamp(
        "nested-lists",
        &["--verdict", "approve", "--phase", "prd_review"],
    );
    crew.stamp("nested-lists", &["--prd-approved", "--by", "human"]);
    crew.write(
        "nested-lists",
        "plan-review-findings",
        "The implementation plan is approved.",
    );
    crew.stamp(
        "nested-lists",
        &["--verdict", "approve", "--phase", "plan_review"],
    );
    crew.next("work", "nested-lists");
    common::agent_profiles(
        &project.env,
        "roles: {work-lead: {github_app: author}, build-reviewer: {github_app: reviewer}}",
    );
    std::fs::write(workspace.join("parser.rs"), "fn parse() {}\n").unwrap();
    project.env.git(&workspace, ["add", "parser.rs"]);
    project
        .env
        .instrukt(&workspace)
        .env("T3CODE_THREAD_ID", "lead-1")
        .args(["repo", "commit", "--message", "Track list depth"])
        .assert()
        .success();
    insta::assert_snapshot!(
        "bot-role-commit-author",
        project
            .env
            .git(&workspace, ["log", "-1", "--format=%an <%ae>"])
    );
    let built = project.env.git(&workspace, ["rev-parse", "HEAD"]);
    project
        .github
        .set_branch(REPOSITORY, "nested-lists/work", &built);

    crew.todo_in(&workspace, ["stamp", "nested-lists", "--build-started"]);
    let worker = crew.worker(
        "nested-lists",
        "build-reviewer",
        Some("crew/procedure/review-build"),
    );
    project
        .env
        .instrukt(Path::new(worker["worktree"].as_str().unwrap()))
        .env(
            "T3CODE_THREAD_ID",
            worker["session"]
                .as_str()
                .unwrap()
                .strip_prefix("t3:")
                .unwrap(),
        )
        .args([
            "todo",
            "stamp",
            "nested-lists",
            "--verdict",
            "approve",
            "--phase",
            "build_review",
        ])
        .assert()
        .success();

    let remote = project.github.issues(REPOSITORY);
    let pull = remote
        .iter()
        .find(|i| i.get("pull_request").is_some())
        .unwrap()["number"]
        .as_u64()
        .unwrap();
    let reviews = project.github.reviews(REPOSITORY, pull);
    insta::assert_json_snapshot!(
        "native-pull-review-identities",
        serde_json::json!({"author":remote.iter().find(|i|i.get("pull_request").is_some()).unwrap()["user"],"reviewer":reviews.last().unwrap()["user"],"state":reviews.last().unwrap()["state"]})
    );
    // Reconstruction must recognize the native verdict already on GitHub.
    let other = project.other_computer();
    other.ok(["update", "nested-lists"]);
    assert_eq!(project.github.reviews(REPOSITORY, pull), reviews);
    project.ok(["deliver", "nested-lists", "--commit", &built]);

    insta::assert_snapshot!("delivery-on-github", redact(&github_view(project)));
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
    std::fs::rename(
        project.env.instrukt_home().join("secrets.env"),
        project.env.instrukt_home().join("secrets.saved"),
    )
    .unwrap();
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

    std::fs::rename(
        project.env.instrukt_home().join("secrets.saved"),
        project.env.instrukt_home().join("secrets.env"),
    )
    .unwrap();
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
    // The machine has a runtime for todo dispatch: only where the code is
    // kept stands in the way.
    let rt = runtime::FakeRuntime::new(&env, "rt", false);
    runtime::global_layer(&env, "todos: false", &[&rt]);
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

    assert_eq!(project.rt.started_count(), 1, "{:?}", project.rt.calls());
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
        BTreeSet::from([owned.clone(), "Needs you".into()])
    );
    // Existing user fields keep their order and contents. Record status
    // retains its options before the required additions; Needs you is appended.
    let first_after = github.project_fields("acme", 7);
    assert_eq!(first_after.len(), first_before.len() + 1);
    assert_eq!(
        first_after[..first_before.len() - 1],
        first_before[..first_before.len() - 1]
    );
    let (name, options) = first_after.iter().find(|(name, _)| name == &owned).unwrap();
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
    // Both owned fields are created on the newly selected board.
    let second_after = github.project_fields("acme", 8);
    assert_eq!(second_after.len(), 2);
    assert_eq!(second_after[0].0, owned);
    assert_eq!(
        valued_fields(&project, "acme", 8),
        BTreeSet::from([owned, "Needs you".into()])
    );
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

// spec: project/instrukt/spec/feature/work/github-record UC-GR7
#[test]
fn an_existing_local_item_uses_the_later_pickup_copy_and_shows_its_age() {
    let mut project = flavor::Tracked::new(false);
    project.env.set_env("INSTRUKT_NOW", "2030-01-06T09:00:00Z");
    project.todo([
        "create",
        "held-away",
        "Keep this record visible locally.",
        "--kind",
        "change",
        "--approved",
    ]);
    seed_revision(&project.github, |record| {
        record["claim"] = serde_json::json!({"machine":OTHER_MACHINE,"at":"2030-01-06T09:00:00Z"});
    });
    project.tick_at("2030-01-06T09:01:00Z");
    project.env.set_env("INSTRUKT_NOW", "2030-01-06T09:06:00Z");
    project.github.clear_requests();
    let listed = project.todo(["list"]);
    assert!(project.github.requests().is_empty());
    insta::assert_snapshot!("local-copy-age", redact(&listed));
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR9
#[test]
fn attributable_hold_labels_control_dispatch_and_output_label_edits_never_change_work() {
    let project = Project::new();
    project.create(
        "held-work",
        "feature",
        "Retain this work through a dispatch pause.",
    );
    let action = |id: u64, event: &str, login: &str| serde_json::json!({"id":id,"event":event,"label":{"name":"hold"},"actor":{"login":login},"created_at":"2026-11-06T10:00:00Z"});
    let respond = |actions: Vec<Value>, login: &str, permission: &str| {
        project.github.respond_once(
            "GET",
            "/issues/1/events",
            CannedResponse {
                status: 200,
                headers: vec![],
                body: actions.into(),
            },
        );
        project.github.respond_once(
            "GET",
            &format!("/collaborators/{login}/permission"),
            CannedResponse {
                status: 200,
                headers: vec![],
                body: serde_json::json!({"permission":permission}),
            },
        );
    };
    project
        .github
        .edit_issue(REPOSITORY, 1, |issue| issue.labels.push("hold".into()));
    respond(
        vec![action(801, "labeled", "maintainer")],
        "maintainer",
        "write",
    );
    let held = project.ok(["show", "held-work"]);
    project.ok([
        "artifacts",
        "write",
        "held-work",
        "log",
        "Progress remains available.",
    ]);
    let before = project.github.artifacts(REPOSITORY, 1);
    project.github.edit_issue(REPOSITORY, 1, |issue| {
        issue.labels.retain(|l| l != "hold" && l != "status:active");
        issue.labels.push("status:delivered".into());
        issue.labels.push("team-owned".into());
    });
    respond(
        vec![
            action(801, "labeled", "maintainer"),
            action(802, "unlabeled", "collaborator"),
        ],
        "collaborator",
        "write",
    );
    let released = project.ok(["show", "held-work"]);
    // A subsequent write reflects the accepted release and restores output labels.
    project.ok(["update", "held-work"]);
    assert_eq!(project.github.artifacts(REPOSITORY, 1), before);
    respond(vec![action(803, "labeled", "outsider")], "outsider", "read");
    let refused = project.ok(["show", "held-work"]);
    insta::assert_snapshot!(
        "github-hold-label-audit",
        redact(&format!("{held}---\n{released}---\n{refused}"))
    );
    insta::assert_json_snapshot!(
        "github-restored-output-labels",
        project.github.issue(REPOSITORY, 1).unwrap()["labels"]
    );
}
// spec: project/instrukt/spec/feature/work/github-record UC-GR8
// spec: project/instrukt/spec/feature/workflow/preparation UC-PR2
#[test]
fn a_github_decision_wakes_only_its_live_claimant_and_replay_creates_no_second_notice() {
    let crew = workflow::Crew::new();
    let project = &crew.project;
    project.create("approval-work", "feature", "Review the desired outcome.");
    crew.next("prepare", "approval-work");
    crew.write("approval-work", "prd", "The desired reviewed outcome.");
    let reviewer = crew.worker(
        "approval-work",
        "prd-reviewer",
        Some("crew/procedure/review-prd"),
    );
    let reviewer = reviewer["session"]
        .as_str()
        .unwrap()
        .strip_prefix("t3:")
        .unwrap();
    for args in [
        vec![
            "artifacts",
            "write",
            "approval-work",
            "prd-review-findings",
            "The outcome is complete.",
        ],
        vec![
            "stamp",
            "approval-work",
            "--verdict",
            "approve",
            "--phase",
            "prd_review",
        ],
    ] {
        project
            .env
            .instrukt(&crew.phase_workspace("approval-work", true))
            .env("T3CODE_THREAD_ID", reviewer)
            .arg("todo")
            .args(args)
            .assert()
            .success();
    }

    crew.runtime.open_session("lead-1");
    crew.runtime.set_status("lead-1", "ended ok");
    let config = project.repo.path().join("instrukt.yaml");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text = text.replace("github:\n", "github:\n  approvers: [maintainer]\n");
    std::fs::write(config, text).unwrap();
    let issue = project.github.issue(REPOSITORY, 1).unwrap();
    let body = issue["body"].as_str().unwrap();
    let prefix = body
        .lines()
        .find(|line| line.starts_with("**Awaiting approval:** prd"))
        .unwrap()
        .split('`')
        .nth(1)
        .unwrap();
    project.github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        &format!("/instrukt approve prd {prefix}"),
    );
    crew.todo(["show", "approval-work"]);
    assert_eq!(crew.runtime.sent_count("lead-1"), 0); // This manually opened thread had no initial assignment message.
    let first = crew.runtime.input_log("lead-1");
    insta::assert_snapshot!("github-decision-attention", redact(&first));
    let started = crew.runtime.started_count();
    insta::assert_snapshot!(
        "preparation-after-github-decision",
        redact(&crew.next("prepare", "approval-work"))
    );
    assert_eq!(crew.runtime.started_count(), started);
    crew.todo(["show", "approval-work"]);
    let second = crew.runtime.input_log("lead-1");
    assert_eq!(first, second);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR8
#[test]
fn a_confirmation_failure_retries_without_reapplying_the_github_decision() {
    let project = Project::new();
    project.repo.write("instrukt.yaml","schema_version: 1\nproject: demo\nmodules: {todos: true}\ngithub: {repository: acme/demo, approvers: [maintainer]}\n");
    project.create(
        "reply-work",
        "feature",
        "Preserve the decision through confirmation failure.",
    );
    project.ok([
        "artifacts",
        "write",
        "reply-work",
        "prd",
        "The current outcome.",
    ]);
    project.github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        "/instrukt request-changes prd Clarify scope.",
    );
    let before = project.github.comments(REPOSITORY, 1).len();
    project.github.respond_once(
        "POST",
        "/issues/1/comments",
        CannedResponse {
            status: 502,
            headers: vec![],
            body: serde_json::json!({"message":"Bad Gateway"}),
        },
    );
    project.ok(["show", "reply-work"]);
    assert_eq!(project.github.comments(REPOSITORY, 1).len(), before);
    let recovered = project.ok(["show", "reply-work"]);
    assert_eq!(project.github.comments(REPOSITORY, 1).len(), before + 1);
    let again = project.ok(["show", "reply-work"]);
    assert_eq!(project.github.comments(REPOSITORY, 1).len(), before + 1);
    assert_eq!(redact(&recovered), redact(&again));
    insta::assert_snapshot!("decision-after-reply-retry", redact(&again));
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR8
#[test]
fn moving_work_to_another_computer_keeps_the_decisions_source_identity() {
    let project = Project::new();
    let config = "schema_version: 1\nproject: demo\nmodules: {todos: true}\ngithub: {repository: acme/demo, approvers: [maintainer]}\n";
    project.repo.write("instrukt.yaml", config);
    project.create(
        "moved-decision",
        "feature",
        "Keep the applied decision when work moves.",
    );
    project.ok([
        "artifacts",
        "write",
        "moved-decision",
        "prd",
        "The current outcome.",
    ]);
    project.github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        "/instrukt request-changes prd Clarify the result.",
    );
    project.ok(["show", "moved-decision"]);
    project.ok([
        "artifacts",
        "write",
        "moved-decision",
        "log",
        "Reflect the applied decision.",
    ]);
    let other = project.other_computer();
    other.repo.write("instrukt.yaml", config);
    other.ok([
        "update",
        "moved-decision",
        "--hold",
        "--reason",
        "Inspect on this computer.",
    ]);
    insta::assert_snapshot!(
        "moved-decision-applied-once",
        redact(&other.ok(["show", "moved-decision"]))
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR8
#[test]
fn a_failed_mixed_handover_does_not_resend_an_approval_to_a_released_claimant() {
    let crew = workflow::Crew::new();
    let project = &crew.project;
    project.create(
        "mixed-attention",
        "feature",
        "Keep attention with its current claimant.",
    );
    crew.next("prepare", "mixed-attention");
    crew.write("mixed-attention", "prd", "The outcome.");
    let child = crew.worker(
        "mixed-attention",
        "prd-reviewer",
        Some("crew/procedure/review-prd"),
    );
    let child = child["session"]
        .as_str()
        .unwrap()
        .strip_prefix("t3:")
        .unwrap();
    crew.runtime.open_session("lead-1");
    crew.runtime.set_status("lead-1", "ended ok");
    project.env.run(crew.root(), ["sync"]).success();
    crew.runtime
        .finish(child, "completed", "The review turn ended.");
    let file = project.repo.path().join("instrukt.yaml");
    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace("github:\n", "github:\n  approvers: [maintainer]\n");
    std::fs::write(file, text).unwrap();
    project.github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        "/instrukt request-changes prd Clarify scope.",
    );
    std::fs::write(crew.runtime.state_dir().join("send-failure"), "").unwrap();
    project.env.run(crew.root(), ["tick"]).success();
    crew.todo(["update", "mixed-attention", "--unclaim"]);
    std::fs::remove_file(crew.runtime.state_dir().join("send-failure")).unwrap();
    crew.todo(["show", "mixed-attention"]);
    assert_eq!(crew.runtime.message_count("lead-1"), 0);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR8
// spec: project/instrukt/spec/feature/workflow/preparation UC-PR2
#[test]
fn request_changes_wakes_the_live_claimant_once_without_a_worker_ending() {
    let crew = workflow::Crew::new();
    let project = &crew.project;
    let slug = "request-direction";
    project.create(slug, "feature", "Keep the requested direction.");
    crew.next("prepare", slug);
    crew.write(slug, "prd", "The current outcome.");
    crew.runtime.open_session("lead-1");
    crew.runtime.set_status("lead-1", "ended ok");
    let source = project
        .repo
        .read("instrukt.yaml")
        .replace("github:\n", "github:\n  approvers: [maintainer]\n");
    project.repo.write("instrukt.yaml", &source);
    project.github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        "/instrukt request-changes prd Clarify the audience.",
    );
    crew.todo(["show", slug]);
    assert_eq!(crew.runtime.message_count("lead-1"), 1);
    insta::assert_snapshot!(
        "github-request-changes-attention",
        redact(&crew.runtime.input_log("lead-1"))
    );
    crew.todo(["show", slug]);
    assert_eq!(crew.runtime.message_count("lead-1"), 1);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR9
#[test]
fn replaying_label_history_on_another_computer_preserves_a_newer_manual_hold() {
    let project = Project::new();
    project.create(
        "retained-hold",
        "change",
        "Keep the operator's later decision.",
    );
    let actions = serde_json::json!([
        {"id":801,"event":"labeled","label":{"name":"hold"},"actor":{"login":"maintainer"},"created_at":"2026-11-06T10:00:00Z"},
        {"id":802,"event":"unlabeled","label":{"name":"hold"},"actor":{"login":"maintainer"},"created_at":"2026-11-06T10:01:00Z"}
    ]);
    let history = || {
        project.github.respond_once(
            "GET",
            "/issues/1/events",
            CannedResponse {
                status: 200,
                headers: vec![],
                body: actions.clone(),
            },
        )
    };
    project.github.respond_once(
        "GET",
        "/collaborators/maintainer/permission",
        CannedResponse {
            status: 200,
            headers: vec![],
            body: serde_json::json!({"permission":"write"}),
        },
    );
    project.github.respond_once(
        "GET",
        "/collaborators/maintainer/permission",
        CannedResponse {
            status: 200,
            headers: vec![],
            body: serde_json::json!({"permission":"write"}),
        },
    );
    history();
    project.ok(["show", "retained-hold"]);
    project.ok([
        "update",
        "retained-hold",
        "--hold",
        "--reason",
        "Await operator decision.",
    ]);
    let other = project.other_computer();
    history();
    other.ok(["update", "retained-hold"]);
    let first = other.ok(["show", "retained-hold"]);
    history();
    let replay = other.ok(["show", "retained-hold"]);
    insta::assert_snapshot!(
        "cross-computer-label-history",
        redact(&format!("{first}---\n{replay}"))
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR8
// spec: project/instrukt/spec/feature/workflow/preparation UC-PR2
#[test]
fn a_decision_imported_by_another_computer_reaches_the_claim_host_once() {
    let crew = workflow::Crew::new();
    let project = &crew.project;
    let slug = "remote-decision";
    project.create(slug, "feature", "Retain the remote decision.");
    crew.next("prepare", slug);
    crew.write(slug, "prd", "The current audience.");
    crew.runtime.open_session("lead-1");
    crew.runtime.set_status("lead-1", "ended ok");
    let other = project.other_computer();
    let source = other
        .repo
        .read("instrukt.yaml")
        .replace("github:\n", "github:\n  approvers: [maintainer]\n");
    other.repo.write("instrukt.yaml", &source);
    let fake = runtime::FakeRuntime::new(&other.env, "other", false);
    project.github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        "/instrukt request-changes prd Explain the intended audience.",
    );
    other.ok(["update", slug]);
    assert_eq!(fake.calls().len(), 0);
    crew.todo(["show", slug]);
    assert_eq!(crew.runtime.message_count("lead-1"), 1);
    crew.todo(["show", slug]);
    assert_eq!(crew.runtime.message_count("lead-1"), 1);
    insta::assert_snapshot!(
        "imported-decision-attention",
        redact(&crew.runtime.input_log("lead-1"))
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR9
#[test]
fn imported_accepted_label_identity_supersedes_local_ignored_bookkeeping_through_reconstruction() {
    let project = Project::new();
    let slug = "reconciled-action";
    project.create(slug, "change", "Preserve the accepted action identity.");
    let other = project.other_computer();
    let action = serde_json::json!({"id":901,"event":"labeled","label":{"name":"hold"},"actor":{"login":"maintainer"},"created_at":"2026-11-06T10:00:00Z"});
    let history = || {
        project.github.respond_once(
            "GET",
            "/issues/1/events",
            CannedResponse {
                status: 200,
                headers: vec![],
                body: serde_json::json!([action.clone()]),
            },
        )
    };
    let permission = |value: &str| {
        project.github.respond_once(
            "GET",
            "/collaborators/maintainer/permission",
            CannedResponse {
                status: 200,
                headers: vec![],
                body: serde_json::json!({"permission":value}),
            },
        )
    };
    // B observed the historical action while its actor could not be admitted.
    history();
    permission("read");
    other.ok(["update", slug]);
    // A subsequently accepts and reflects that same action, then records a later hold.
    history();
    permission("write");
    project.ok(["show", slug]);
    project.ok(["update", slug, "--release"]);
    project.ok([
        "update",
        slug,
        "--hold",
        "--reason",
        "Await the later operator decision.",
    ]);
    history();
    other.ok(["update", slug]);
    other.ok([
        "artifacts",
        "write",
        slug,
        "log",
        "Continue from the imported accepted record.",
    ]);
    // Replace B's local database after all writes reflected, as on machine recovery.
    for name in ["instrukt.db", "instrukt.db-wal", "instrukt.db-shm"] {
        let file = other.env.instrukt_home().join("state").join(name);
        if file.exists() {
            std::fs::remove_file(file).unwrap();
        }
    }
    history();
    permission("write");
    other.ok(["update", slug]);
    insta::assert_snapshot!(
        "accepted-label-identity-reconstructed",
        redact(&other.ok(["show", slug]))
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR10
#[test]
fn newer_remote_schema_refuses_progress_before_any_write() {
    let project = Project::new();
    project.create("schema-change", "bug", INTENT);
    project.create("unrelated-record", "bug", "Continue independent work.");
    seed_revision(&project.github, |record| record["schema"] = 999.into());
    let before = project.github.comments(REPOSITORY, 1);
    let artifacts = project.github.artifacts(REPOSITORY, 1);
    let result = project
        .todo([
            "artifacts",
            "write",
            "schema-change",
            "log",
            "must not append",
        ])
        .failure();
    insta::assert_snapshot!("newer-schema-refusal", told(&project.env, &result));
    assert_eq!(project.github.comments(REPOSITORY, 1), before);
    assert_eq!(project.github.artifacts(REPOSITORY, 1), artifacts);
    project.ok([
        "artifacts",
        "write",
        "unrelated-record",
        "log",
        "independent progress",
    ]);
    assert_eq!(project.github.comments(REPOSITORY, 1), before);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR10
#[test]
fn older_remote_schema_converts_without_losing_retained_evidence() {
    let project = Project::new();
    project.create("schema-change", "bug", INTENT);
    seed_revision(&project.github, |record| record["schema"] = 1.into());
    project.ok([
        "artifacts",
        "write",
        "schema-change",
        "log",
        "continued after conversion",
    ]);
    assert_eq!(latest_envelope(&project.github)["record"]["schema"], 3);
    insta::assert_snapshot!(
        "converted-schema-evidence",
        project.ok(["artifacts", "read", "schema-change", "input"])
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn missing_board_is_set_up_automatically_and_reused() {
    let project = Project::new();
    // This project has no board reference; the existing unrelated board is preserved.
    let file = project.repo.path().join("instrukt.yaml");
    let config = std::fs::read_to_string(&file)
        .unwrap()
        .replace("board: acme/1\n", "");
    std::fs::write(file, config).unwrap();
    project.create("cockpit", "bug", INTENT);
    let board = project
        .env
        .run(project.repo.path(), ["config", "get", "board"])
        .success();
    insta::assert_snapshot!("automatic-board-reference", told(&project.env, &board));
    project
        .github
        .add_project_field("acme", 2, "User field", Some(&["theirs"]));
    project
        .github
        .remove_project_field("acme", 2, "Record status");
    project.github.clear_project_items("acme", 2);
    project.ok(["update", "cockpit"]);
    assert_eq!(project.github.project_items("acme", 2).len(), 1);
    assert_eq!(project.github.project_fields("acme", 2).len(), 3);
    insta::assert_snapshot!(
        "automatic-board-state",
        board_state(
            &project,
            "acme",
            2,
            &[(1, issue_node(&project, 1).as_str())]
        )
    );
}

fn cockpit_views(project: &Project) -> Vec<Value> {
    let fields = project.github.project_field_details("acme", 1);
    let mut views=project.github.project_views("acme",1).into_iter().map(|view| {
        let columns=view["vertical_group_by"].as_array().unwrap().iter().map(|id|fields.iter().find(|f|f["databaseId"]==*id).unwrap()["name"].clone()).collect::<Vec<_>>();
        serde_json::json!({"name":view["name"],"layout":view["layout"],"filter":view["filter"],"columns":columns})
    }).collect::<Vec<_>>();
    views.sort_by_key(|view| view["name"].as_str().unwrap().to_owned());
    views
}

fn marked_needs_you(project: &Project, number: u64) -> bool {
    let node = issue_node(project, number);
    project
        .github
        .project_items("acme", 1)
        .into_iter()
        .find(|(n, _)| *n == node)
        .unwrap()
        .1
        .into_iter()
        .any(|(field, value)| field == "Needs you" && value == "Yes")
}
fn approvals_config(project: &Project, prd: &str, task: &str) {
    let file = project.repo.path().join("instrukt.yaml");
    let mut layer: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    layer["approvals"] = serde_yaml_ng::from_str(&format!("prd: {prd}\ntask: {task}")).unwrap();
    std::fs::write(file, serde_yaml_ng::to_string(&layer).unwrap()).unwrap();
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn cockpit_views_are_created_and_reconciled_without_touching_user_views_or_workflows() {
    let project = Project::new();
    project
        .github
        .add_project_field("acme", 1, "User field", Some(&["theirs"]));
    project
        .github
        .seed_project_view("acme", 1, "My view", "table", "label:mine", &[]);
    project.github.seed_project_workflow(
        "acme",
        1,
        serde_json::json!({"id":"workflow-1","enabled":true,"name":"Auto-close"}),
    );
    let user_view = project.github.project_views("acme", 1)[0].clone();
    let workflows = project.github.project_workflows("acme", 1);
    project.create("cockpit-views", "bug", "Show the cockpit views.");
    assert_eq!(project.github.project_views("acme", 1).len(), 3);
    let views = project.github.project_views("acme", 1);
    insta::assert_json_snapshot!("cockpit-views-created", cockpit_views(&project));
    let field = project
        .github
        .project_field_details("acme", 1)
        .into_iter()
        .find(|f| f["name"] == "Needs you")
        .unwrap();
    let options=field["options"].as_array().unwrap().iter().map(|o|serde_json::json!({"name":o["name"],"color":o["color"],"description":o["description"]})).collect::<Vec<_>>();
    insta::assert_json_snapshot!("needs-you-option", options);
    project.ok(["update", "cockpit-views"]);
    assert_eq!(project.github.project_views("acme", 1), views);
    assert_eq!(project.github.project_views("acme", 1)[0], user_view);
    assert_eq!(project.github.project_workflows("acme", 1), workflows);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn cockpit_drifted_owned_views_and_options_are_repaired_and_missing_ones_recreated() {
    let project = Project::new();
    project
        .github
        .add_project_field("acme", 1, "Status", Some(&["Todo", "Done"]));
    project
        .github
        .add_project_field("acme", 1, "Needs you", Some(&["Yes", "No"]));
    project
        .github
        .seed_project_view("acme", 1, "Board", "table", "is:open", &["Status"]);
    project
        .github
        .seed_project_view("acme", 1, "Needs you", "board", "is:closed", &[]);
    project
        .github
        .seed_project_view("acme", 1, "My view", "table", "label:mine", &[]);
    project.create("repair-cockpit", "bug", "Keep the cockpit configured.");
    let views = project.github.project_views("acme", 1);
    assert_eq!(views.len(), 3);
    assert_eq!(
        views.iter().find(|v| v["name"] == "Board").unwrap()["layout"],
        "board"
    );
    assert_eq!(
        views.iter().find(|v| v["name"] == "Needs you").unwrap()["filter"],
        "needs-you:Yes"
    );
    let expected = cockpit_views(&project);
    insta::assert_json_snapshot!("cockpit-views-repaired", expected);
    let field = project
        .github
        .project_field_details("acme", 1)
        .into_iter()
        .find(|f| f["name"] == "Needs you")
        .unwrap();
    assert_eq!(field["options"].as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!("repaired-needs-you-option",field["options"].as_array().unwrap().iter().map(|o|serde_json::json!({"name":o["name"],"color":o["color"],"description":o["description"]})).collect::<Vec<_>>());
    project.github.drift_project_option("acme", 1, "Needs you");
    project.github.edit_project_view(
        "acme",
        1,
        "Board",
        &serde_json::json!({"layout":"table","filter":"label:drift"}),
    );
    project.github.edit_project_view(
        "acme",
        1,
        "Needs you",
        &serde_json::json!({"layout":"board","filter":"is:closed"}),
    );
    project.ok(["update", "repair-cockpit"]);
    assert_eq!(project.github.project_views("acme", 1), views);
    let restored = project
        .github
        .project_field_details("acme", 1)
        .into_iter()
        .find(|f| f["name"] == "Needs you")
        .unwrap();
    assert_eq!(restored, field);
    project.github.remove_project_field("acme", 1, "Needs you");
    project.github.remove_project_view("acme", 1, "Needs you");
    project.ok(["update", "repair-cockpit"]);
    assert_eq!(cockpit_views(&project), expected);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn cockpit_needs_you_tracks_holds_and_human_approval_configuration_and_clears_resolved_waits() {
    let project = Project::new();
    project.create(
        "person-wait",
        "feature",
        "A person approves the intended outcome.",
    );
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "update",
        "person-wait",
        "--hold",
        "--reason",
        "Ask the owner.",
    ]);
    assert!(marked_needs_you(&project, 1));
    project.ok(["update", "person-wait", "--release"]);
    assert!(!marked_needs_you(&project, 1));
    approvals_config(&project, "peer", "human");
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd",
        "The intended outcome.",
    ]);
    assert!(!marked_needs_you(&project, 1));
    approvals_config(&project, "human", "human");
    project.ok(["update", "person-wait"]);
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd-review-findings",
        "Approved.",
    ]);
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "stamp",
        "person-wait",
        "--verdict",
        "approve",
        "--phase",
        "prd_review",
    ]);
    assert!(marked_needs_you(&project, 1));
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd-review-findings",
        "The review must be assessed again.",
    ]);
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "stamp",
        "person-wait",
        "--verdict",
        "needs_work",
        "--phase",
        "prd_review",
    ]);
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd-review-findings",
        "Approved again.",
    ]);
    project.ok([
        "stamp",
        "person-wait",
        "--verdict",
        "approve",
        "--phase",
        "prd_review",
    ]);
    assert!(marked_needs_you(&project, 1));
    project.ok([
        "update",
        "person-wait",
        "--hold",
        "--reason",
        "Another person still needs to answer.",
    ]);
    project.ok([
        "stamp",
        "person-wait",
        "--prd-approved",
        "--by",
        "human",
        "--session",
        "thread-host:person-1",
    ]);
    assert!(marked_needs_you(&project, 1));
    project.ok(["update", "person-wait", "--release"]);
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd",
        "A revised outcome needs a new approval.",
    ]);
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd",
        "The intended outcome.",
    ]);
    assert!(marked_needs_you(&project, 1));
    approvals_config(&project, "peer", "human");
    project.ok(["update", "person-wait"]);
    assert!(!marked_needs_you(&project, 1));
    project.create(
        "task-person-wait",
        "task",
        "A person approves this task plan.",
    );
    project.ok([
        "artifacts",
        "write",
        "task-person-wait",
        "task-plan",
        "Run the documented task.",
    ]);
    assert!(marked_needs_you(&project, 2));
    approvals_config(&project, "peer", "peer");
    project.ok(["update", "task-person-wait"]);
    assert!(!marked_needs_you(&project, 2));
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn cockpit_setup_failure_keeps_issue_reflection_and_retries_without_duplicate_views() {
    let project = Project::new();
    project.github.respond_once(
        "POST",
        "/orgs/acme/projectsV2/1/views",
        CannedResponse {
            status: 503,
            headers: Vec::new(),
            body: serde_json::json!({"message":"Service unavailable"}),
        },
    );
    let pending = project.create("retry-views", "bug", "Keep the cockpit retryable.");
    assert_eq!(project.github.issues(REPOSITORY).len(), 1);
    assert_eq!(project.github.artifacts(REPOSITORY, 1).len(), 1);
    assert!(project.github.project_views("acme", 1).is_empty());
    insta::assert_snapshot!("cockpit-view-setup-pending", redact(&pending));
    project.ok(["update", "retry-views"]);
    let views = project.github.project_views("acme", 1);
    assert_eq!(views.len(), 2);
    project.ok(["update", "retry-views"]);
    assert_eq!(project.github.project_views("acme", 1), views);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn cockpit_setup_finds_owned_views_after_user_views_and_reconciles_duplicate_owned_names() {
    let project = Project::new();
    project.github.add_project_field(
        "acme",
        1,
        "Record status",
        Some(&["active", "iceboxed", "held", "delivered", "withdrawn"]),
    );
    for number in 0..105 {
        project.github.seed_project_view(
            "acme",
            1,
            &format!("User view {number}"),
            "table",
            "label:mine",
            &[],
        );
    }
    project
        .github
        .seed_project_view("acme", 1, "Board", "board", "", &["Record status"]);
    project
        .github
        .seed_project_view("acme", 1, "Needs you", "table", "needs-you:Yes", &[]);
    let expected = project.github.project_views("acme", 1);
    project
        .github
        .seed_project_view("acme", 1, "Board", "table", "is:closed", &[]);
    project.create(
        "many-views",
        "bug",
        "Retain the existing cockpit and user views.",
    );
    assert_eq!(project.github.project_views("acme", 1), expected);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn cockpit_repair_failure_stays_pending_for_an_already_reflected_item() {
    let project = Project::new();
    project.create("reflected-cockpit", "bug", "Keep cockpit failures visible.");
    project.github.remove_project_view("acme", 1, "Board");
    project.github.respond_once(
        "POST",
        "/orgs/acme/projectsV2/1/views",
        CannedResponse {
            status: 503,
            headers: Vec::new(),
            body: serde_json::json!({"message":"Service unavailable"}),
        },
    );
    insta::assert_snapshot!(
        "reflected-item-cockpit-pending",
        redact(&project.ok(["update", "reflected-cockpit"]))
    );
    let snapshot = project.github.comments(REPOSITORY, 1);
    project.ok(["update", "reflected-cockpit"]);
    assert_eq!(project.github.comments(REPOSITORY, 1), snapshot);
    assert_eq!(project.github.project_views("acme", 1).len(), 2);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn an_empty_projects_cockpit_failure_is_a_project_reflection_failure() {
    let project = flavor::Tracked::new(false);
    project
        .github
        .add_project_field("acme", 1, "User field", Some(&["theirs"]));
    project
        .github
        .seed_project_view("acme", 1, "My view", "table", "label:mine", &[]);
    project.github.seed_project_workflow(
        "acme",
        1,
        serde_json::json!({"id":"workflow-1","enabled":true,"name":"Auto-close"}),
    );
    let user_field = project.github.project_field_details("acme", 1)[0].clone();
    let user_view = project.github.project_views("acme", 1)[0].clone();
    let workflows = project.github.project_workflows("acme", 1);
    let cockpit = || {
        let fields = project.github.project_field_details("acme", 1);
        let mut views=project.github.project_views("acme",1).into_iter().map(|view| {
            let columns=view["vertical_group_by"].as_array().unwrap().iter().map(|id|fields.iter().find(|field|field["databaseId"]==*id).unwrap()["name"].clone()).collect::<Vec<_>>();
            serde_json::json!({"name":view["name"],"layout":view["layout"],"filter":view["filter"],"columns":columns})
        }).collect::<Vec<_>>();
        views.sort_by_key(|view| view["name"].as_str().unwrap().to_owned());
        serde_json::json!({"fields":project.github.project_fields("acme",1),"views":views,"workflows":project.github.project_workflows("acme",1)})
    };
    project.github.respond_once(
        "POST",
        "/orgs/acme/projectsV2/1/views",
        CannedResponse {
            status: 503,
            headers: Vec::new(),
            body: serde_json::json!({"message":"Service unavailable"}),
        },
    );
    project.tick().success();
    assert!(project.github.issues(REPOSITORY).is_empty());
    assert_eq!(
        project.github.project_views("acme", 1),
        vec![user_view.clone()]
    );
    assert_eq!(
        project.github.project_field_details("acme", 1)[0],
        user_field
    );
    assert_eq!(project.github.project_workflows("acme", 1), workflows);
    insta::assert_json_snapshot!("empty-project-cockpit-failed", cockpit());

    project.tick().success();
    assert!(project.github.issues(REPOSITORY).is_empty());
    let views = project.github.project_views("acme", 1);
    assert_eq!(views.len(), 3);
    assert_eq!(
        views
            .iter()
            .find(|view| view["node_id"] == user_view["node_id"])
            .unwrap(),
        &user_view
    );
    assert_eq!(
        project.github.project_field_details("acme", 1)[0],
        user_field
    );
    assert_eq!(project.github.project_workflows("acme", 1), workflows);
    insta::assert_json_snapshot!("empty-project-cockpit-recovered", cockpit());
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR10
#[test]
fn old_remote_native_placement_is_converted_before_record_mutation() {
    let project = Project::new();
    project.create("legacy-driver", "change", INTENT);
    let mut key = String::new();
    seed_revision(&project.github, |record| {
        record["schema"] = 2.into();
        let mut placement: Value = serde_json::from_str(
            &include_str!("fixtures/native-placement-v14.json")
                .replace("__WORKTREE__", project.repo.path().to_str().unwrap())
                .replace("__HEAD__", &project.repo.head()),
        )
        .unwrap();
        key = instrukt_runner::keyed::todo_key(
            "demo",
            record["id"].as_str().unwrap(),
            "prepare-orchestrator",
            1,
        );
        placement["key"] = key.clone().into();
        record["bindings"] = serde_json::json!([{"role":"prepare-orchestrator","session":placement["session"],"worktree":placement["worktree"],"runtime":"t3","native_placement":placement,"assignment":"driver","attempt":1,"semantic_key":key}]);
    });
    project.ok([
        "artifacts",
        "write",
        "legacy-driver",
        "log",
        "Continue after upgrade.",
    ]);
    let current = latest_envelope(&project.github);
    insta::assert_snapshot!(
        "converted-old-native-placement-title",
        redact(
            current["record"]["bindings"][0]["native_placement"]["title"]
                .as_str()
                .unwrap()
        )
    );
}
