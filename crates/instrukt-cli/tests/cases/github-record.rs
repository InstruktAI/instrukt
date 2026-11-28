//! `project/instrukt/spec/feature/work/github-record`: the local
//! record keeps accepted work through GitHub outages, a delivered todo's
//! evidence can be followed on GitHub, the issue's conversation belongs to
//! people, people's controls on the issue arrive once, and todos need the
//! project's code on GitHub.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use crate::fixtures::flavor;
use crate::fixtures::runtime;
use crate::fixtures::work as support;
use crate::fixtures::workflow;
use common::TestEnv;
use common::github::CannedResponse;
use std::collections::BTreeSet;
use std::path::Path;

use instrukt_core::config::{GLOBAL_SCHEMA_VERSION, PROJECT_SCHEMA_VERSION};
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
    assert!(
        project.github.requests().is_empty(),
        "todo create must not contact GitHub"
    );
    project.tick();
    let reflected_before = project.github.artifacts(REPOSITORY, 1);

    project.github.stop();
    let recorded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    project.tick();
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
fn progress_whose_reflection_github_refuses_stays_local_and_reports_githubs_reason() {
    let project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    assert!(
        project.github.requests().is_empty(),
        "todo create must not contact GitHub"
    );
    project.tick();
    project.github.respond_once(
        "POST",
        "/issues/1/artifacts",
        CannedResponse {
            status: 422,
            headers: vec![],
            body: serde_json::json!({
                "message": "Validation Failed",
                "errors": [{"resource": "IssueArtifact", "field": "body", "code": "too_long"}],
                "documentation_url": "https://docs.github.com/rest/issues/artifacts"
            }),
        },
    );

    let requests_before = project.github.requests().len();
    let recorded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    assert_eq!(
        project.github.requests().len(),
        requests_before,
        "artifacts write must not contact GitHub"
    );
    project.tick();
    let read = project.ok(["artifacts", "read", "nested-lists", "prd"]);
    let attempts = project
        .github
        .requests_to("POST", "/issues/1/artifacts")
        .len();
    // Another change reflects the todo again; the refused content is not
    // sent again, and the rest of its reflection goes on.
    project.ok(["update", "nested-lists", "--hold", "--reason", "waiting"]);
    project.tick();
    assert_eq!(
        project
            .github
            .requests_to("POST", "/issues/1/artifacts")
            .len(),
        attempts,
        "the refused content was sent again"
    );
    assert!(labels(&project.github.issue(REPOSITORY, 1).unwrap()).contains("on-hold"));
    let shown = project.ok(["show", "nested-lists"]);
    // Writing the refused content again records nothing: nothing changed.
    let rewritten = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    // Once its grounding moves, the same content is a new version; it repeats
    // the refused one, so it is not sent either and the refusal still holds.
    let moved = |project: &Project| {
        let args = ["commit", "--allow-empty", "--quiet", "-m", "moved"];
        project.env.git(project.repo.path(), args);
    };
    moved(&project);
    let regrounded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    project.tick();
    assert_eq!(
        project
            .github
            .requests_to("POST", "/issues/1/artifacts")
            .len(),
        attempts,
        "the refused content was sent again as a later version"
    );
    let reshown = project.ok(["show", "nested-lists"]);

    // The door is the artifact's next version: once GitHub accepts it, the
    // refused version no longer holds the todo's reflection back.
    const NEXT: &str = "Lists keep their depth, and numbering restarts per level.";
    project.ok(["artifacts", "write", "nested-lists", "prd", NEXT]);
    project.tick();
    let next = project.ok(["show", "nested-lists"]);
    // The same holds for an artifact already on GitHub.
    let prd = project
        .github
        .artifacts(REPOSITORY, 1)
        .into_iter()
        .find(|a| a["artifact"]["name"] == "prd")
        .unwrap()["artifact"]["number"]
        .as_u64()
        .unwrap();
    project.github.respond_once(
        "PATCH",
        &format!("/issues/1/artifacts/{prd}"),
        CannedResponse {
            status: 422,
            headers: vec![],
            body: serde_json::json!({
                "message": "Validation Failed",
                "errors": [{"resource": "IssueArtifact", "field": "body", "code": "too_long"}],
            }),
        },
    );
    const REFUSED_AGAIN: &str = "Lists keep their depth in every renderer.";
    project.ok(["artifacts", "write", "nested-lists", "prd", REFUSED_AGAIN]);
    project.tick();
    let refused_again = project.ok(["show", "nested-lists"]);
    const LATER: &str = "Lists keep their depth through parse, render and export.";
    project.ok(["artifacts", "write", "nested-lists", "prd", LATER]);
    project.tick();
    // Accepted content written again on a moved grounding is not sent again.
    let patches = project
        .github
        .requests_to("PATCH", &format!("/issues/1/artifacts/{prd}"));
    moved(&project);
    project.ok(["artifacts", "write", "nested-lists", "prd", LATER]);
    project.tick();
    assert_eq!(
        project
            .github
            .requests_to("PATCH", &format!("/issues/1/artifacts/{prd}"))
            .len(),
        patches.len(),
        "unchanged content was sent to GitHub again"
    );
    let settled = project.ok(["show", "nested-lists"]);
    // Neither refused version was sent again.
    let sent = |content: &str| {
        project
            .github
            .requests()
            .iter()
            .filter(|r| r.path.starts_with("/repos/acme/demo/issues/1/artifacts"))
            .filter(|r| {
                serde_json::from_str::<Value>(&r.body)
                    .is_ok_and(|body| body["body"].as_str() == Some(content))
            })
            .count()
    };
    assert_eq!((sent(PRD), sent(REFUSED_AGAIN)), (1, 1));

    insta::assert_snapshot!(
        "refused-by-github",
        redact(&format!(
            "{recorded}---\n{read}---\n{shown}---\n{rewritten}---\n{regrounded}---\n{reshown}---\n{next}---\n{refused_again}---\n{settled}"
        ))
    );
}

/// A todo of `crew` prepared and approved, its build started on its work
/// branch, with its workspace.
fn worked_todo(crew: &workflow::Crew, slug: &str) -> std::path::PathBuf {
    let project = &crew.project;
    project.create(slug, "feature", INTENT);
    project.ok(["artifacts", "write", slug, "prd", PRD]);
    project.ok([
        "artifacts",
        "write",
        slug,
        "implementation-plan",
        "1. Track depth in the parser.\n2. Render it.",
    ]);
    let workspace = crew.workspace(slug);
    crew.write(
        slug,
        "prd-review-findings",
        "The product requirement is approved.",
    );
    crew.stamp(slug, &["--verdict", "approve", "--phase", "prd_review"]);
    crew.stamp(slug, &["--prd-approved", "--by", "human"]);
    crew.write(
        slug,
        "plan-review-findings",
        "The implementation plan is approved.",
    );
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);
    crew.next("work", slug);
    crew.todo_in(&workspace, ["stamp", slug, "--build-started"]);
    workspace
}

/// Commits `file` with `content` on the workspace's branch; its head.
fn land(crew: &workflow::Crew, workspace: &Path, file: &str, content: &str) -> String {
    let env = &crew.project.env;
    std::fs::write(workspace.join(file), content).unwrap();
    env.git(workspace, ["add", file]);
    env.git(workspace, ["commit", "--quiet", "-m", file]);
    env.git(workspace, ["rev-parse", "HEAD"])
}

/// The commit branch `branch` names in the bare repository `origin`.
fn remote_head(crew: &workflow::Crew, branch: &str) -> String {
    crew.project.env.git(
        crew.root(),
        [
            "--git-dir",
            crew.origin.to_str().unwrap(),
            "rev-parse",
            &format!("refs/heads/{branch}"),
        ],
    )
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR16
#[test]
fn commits_landed_on_a_todos_branch_reach_its_pull_request_on_github() {
    let mut crew = workflow::Crew::new();
    crew.project
        .github
        .serve_branches_from(REPOSITORY, &crew.origin.clone());
    // The runner ticks for the published project.
    crate::fixtures::documented::sync(&crew.project.env, &crew.project.repo);
    let workspace = worked_todo(&crew, "nested-lists");
    let tick = |crew: &workflow::Crew| crew.project.env.run(crew.root(), ["tick"]).success();

    // A builder lands a commit; the next tick publishes it and the pull
    // request opens on it.
    let built = land(&crew, &workspace, "parser.rs", "fn parse() {}\n");
    tick(&crew);
    assert_eq!(remote_head(&crew, "nested-lists/work"), built);
    let pulls: Vec<Value> = crew
        .project
        .github
        .issues(REPOSITORY)
        .into_iter()
        .filter(|i| i.get("pull_request").is_some())
        .collect();
    insta::assert_json_snapshot!(
        "pull-request-opened-on-the-published-branch",
        pulls
            .iter()
            .map(|pull| serde_json::json!({"title": pull["title"], "state": pull["state"]}))
            .collect::<Vec<_>>()
    );

    // While GitHub cannot be reached the work goes on; the commit reaches
    // GitHub once it can.
    let away = crew.origin.with_extension("away");
    std::fs::rename(&crew.origin, &away).unwrap();
    crew.project.github.stop();
    let fixed = land(&crew, &workspace, "render.rs", "fn render() {}\n");
    crew.project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "log",
        "Rendering joined.",
    ]);
    tick(&crew);
    std::fs::rename(&away, &crew.origin).unwrap();
    assert_eq!(remote_head(&crew, "nested-lists/work"), built);
    crew.project.github.resume();
    tick(&crew);
    assert_eq!(remote_head(&crew, "nested-lists/work"), fixed);

    // The branch on GitHub moves where the todo's branch cannot follow it:
    // it is left as it is, and the developer is told why.
    let elsewhere = crew.project.env.git(
        crew.root(),
        [
            "commit-tree",
            "-p",
            &fixed,
            "-m",
            "pushed elsewhere",
            &format!("{fixed}^{{tree}}"),
        ],
    );
    crew.project.env.git(
        crew.root(),
        [
            "push",
            "--quiet",
            "origin",
            &format!("{elsewhere}:refs/heads/nested-lists/work"),
        ],
    );
    land(&crew, &workspace, "lists.rs", "fn lists() {}\n");
    tick(&crew);
    assert_eq!(remote_head(&crew, "nested-lists/work"), elsewhere);
    insta::assert_snapshot!(
        "unpublishable-branch",
        redact(&crew.project.ok(["show", "nested-lists"]))
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR14
#[test]
fn a_build_review_on_the_todos_pull_request_states_its_verdict_and_findings_and_links_the_findings_artifact()
 {
    let crew = workflow::Crew::new();
    crew.project
        .github
        .serve_branches_from(REPOSITORY, &crew.origin.clone());
    crate::fixtures::documented::sync(&crew.project.env, &crew.project.repo);
    common::agent_profiles(
        &crew.project.env,
        "roles: {todo-lead: {github_app: author}, build-reviewer: {github_app: reviewer}}",
    );
    let workspace = worked_todo(&crew, "nested-lists");
    let built = land(&crew, &workspace, "parser.rs", "fn parse() {}\n");
    crew.project.env.run(crew.root(), ["tick"]).success();

    crew.project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "review-findings",
        "1. The parser keeps the depth of nested lists.\n2. Rendering is untouched.",
    ]);
    let worker = crew.worker(
        "nested-lists",
        "build-reviewer",
        Some("crew/procedure/review-build"),
    );
    crew.project
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
            "changes_requested",
            "--phase",
            "build_review",
        ])
        .assert()
        .success();
    crew.project.env.run(crew.root(), ["tick"]).success();

    let pull = crew
        .project
        .github
        .issues(REPOSITORY)
        .into_iter()
        .find(|i| i.get("pull_request").is_some())
        .unwrap()["number"]
        .as_u64()
        .unwrap();
    let reviews: Vec<Value> = crew
        .project
        .github
        .reviews(REPOSITORY, pull)
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "reviewer": r["user"]["login"],
                "state": r["state"],
                "commit": if r["commit_id"] == built.as_str() { "the built commit" } else { "another commit" },
                "body": r["body"].as_str().unwrap().replace(&built, "<the built commit>"),
            })
        })
        .collect();
    insta::assert_json_snapshot!("readable-build-review", reviews);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR13
#[test]
fn progress_agents_record_adds_nothing_to_the_conversation_people_hold_on_the_issue() {
    use sha2::{Digest, Sha256};
    let project = Project::new();
    project.repo.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: demo\nmodules: {todos: true}\ngithub: {repository: acme/demo, approvers: [maintainer]}\n",
    );
    project.create("nested-lists", "feature", INTENT);
    project.tick();
    // The issue already carries a record comment an earlier instrukt wrote,
    // and the developer and a collaborator discuss the todo there.
    project.github.seed_comment(
        REPOSITORY,
        1,
        "instrukt-test",
        "COLLABORATOR",
        &format!("[instrukt record · nested-lists · rev 1 on rev 0]\n{RECORD_MARKER}{{}}{CLOSE}\n"),
    );
    project.github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        "Should tables keep their depth too?",
    );
    project.github.seed_comment(
        REPOSITORY,
        1,
        "contributor",
        "CONTRIBUTOR",
        "Only lists, I think.",
    );

    // Agents keep recording progress.
    project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "log",
        "Started the parser.",
    ]);
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "log",
        "Joined the cases.",
    ]);
    project.tick();
    let prefix = &hex::encode(Sha256::digest(PRD.as_bytes()))[..12];
    project.github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        &format!("/instrukt approve prd {prefix}"),
    );
    project.tick();
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "log",
        "Approved; building.",
    ]);
    project.tick();

    let conversation: Vec<String> = project
        .github
        .comments(REPOSITORY, 1)
        .iter()
        .map(|c| {
            format!(
                "{}: {}",
                c["user"]["login"].as_str().unwrap(),
                c["body"].as_str().unwrap()
            )
        })
        .collect();
    insta::assert_snapshot!(
        "conversation-holds-only-peoples-exchange",
        conversation.join("\n---\n").replace(prefix, "<prefix>")
    );
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR1
#[test]
fn progress_recorded_without_credentials_stays_local_and_says_why_it_is_pending() {
    let mut project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    project.tick();
    std::fs::remove_file(project.env.instrukt_home().join("secrets.env")).unwrap();
    let requests_before = project.github.requests().len();
    let gh_config = project.env.path("gh-config");
    std::fs::create_dir_all(&gh_config).unwrap();
    project
        .env
        .remove_env("GITHUB_TOKEN")
        .set_env("GH_CONFIG_DIR", gh_config.to_str().unwrap());

    let recorded = project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    project.tick();
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
fn reflection_held_back_by_rate_limiting_or_another_reflection_completes_later() {
    let project = Project::new();
    project.create("nested-lists", "feature", INTENT);
    project.tick();
    // GitHub limits the next two requests; its window has already reset.
    project.github.rate_limit_next(2, 0);

    project.ok(["artifacts", "write", "nested-lists", "prd", PRD]);
    project.tick();
    let limited = project.ok(["show", "nested-lists"]);
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth, and numbering restarts per level.",
    ]);
    project.tick();
    let still_limited = project.ok(["show", "nested-lists"]);
    project.ok(["artifacts", "write", "nested-lists", "log", "Started."]);
    project.tick();
    let accepted = project.ok(["show", "nested-lists"]);
    let reflected = retained_artifacts(&project);

    // Another process (the runner tick) is reflecting: its reflection stops at
    // its issue update. A concurrent todo command records its progress and
    // returns without waiting for that reflection or contacting GitHub.
    let todo = |args: &[&str]| {
        let mut command = project.env.instrukt_process(project.repo.path());
        command
            .arg("todo")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command
    };
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "log",
        "Joined the cases.",
    ]);
    let fields = project.github.pause_once("PATCH", "/issues/1");
    let first = project
        .tick_command()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    assert!(
        fields.wait_until_received(std::time::Duration::from_secs(30)),
        "the runner tick's reflection never reached GitHub"
    );
    let second = exited_within(
        todo(&[
            "artifacts",
            "write",
            "nested-lists",
            "log",
            "Rendered the depth.",
        ])
        .spawn()
        .unwrap(),
        std::time::Duration::from_secs(20),
    )
    .expect("the second command waited for the reflection in progress");
    assert!(second.status.success(), "{second:?}");
    let while_reflecting = String::from_utf8(second.stdout).unwrap();
    drop(fields);
    let first = first.wait_with_output().unwrap();
    assert!(first.status.success(), "{first:?}");
    // The next runner tick reflects what the second command left pending.
    project.tick();

    insta::assert_snapshot!(
        "rate-limited-then-reflected",
        redact(&format!(
            "{limited}---\n{still_limited}---\n{accepted}---\n{reflected}---\n{while_reflecting}---\n{}",
            retained_artifacts(&project)
        ))
    );
}

/// Waits for `child` to exit within `bound`; `None` when it is still
/// running then (it is left to finish on its own).
fn exited_within(
    child: std::process::Child,
    bound: std::time::Duration,
) -> Option<std::process::Output> {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || done.send(child.wait_with_output().unwrap()));
    finished.recv_timeout(bound).ok()
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
    project.tick();

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
    // A later reflection recognizes the native verdict already on GitHub.
    project.tick();
    assert_eq!(project.github.reviews(REPOSITORY, pull), reviews);
    project.ok(["deliver", "nested-lists", "--commit", &built]);
    project.tick();

    insta::assert_snapshot!("delivery-on-github", redact(&github_view(project)));
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
        format!("schema_version: {GLOBAL_SCHEMA_VERSION}\nmodules:\n  todos: true\n"),
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
        &format!("schema_version: {PROJECT_SCHEMA_VERSION}\nproject: demo\n"),
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

/// What a collaborator sees on GitHub: each issue and pull request with
/// its state, body, conversation, artifacts and reviews.
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
    project.tick();
    project.create(slugs[1], "change", "Track list depth in the parser.");
    project.tick();
    project.create(slugs[2], "change", "Render lists at their depth.");
    project.tick();
    let requests_before_freeze = project.github.requests().len();
    project.ok(["freeze", slugs[1]]);
    project.ok(["update", slugs[2], "--hold", "--reason", "waiting"]);
    assert_eq!(
        project.github.requests().len(),
        requests_before_freeze,
        "todo freeze and update must not contact GitHub"
    );
    project.tick();
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
    project.tick();

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

// spec: project/instrukt/spec/feature/work/github-record UC-GR9
#[test]
fn attributable_hold_labels_control_dispatch_and_output_label_edits_never_change_work() {
    let project = flavor::Tracked::new(false);
    project.todo([
        "create",
        "held-work",
        "Retain this work through a dispatch pause.",
        "--kind",
        "change",
        "--approved",
    ]);
    assert!(
        project.github.requests().is_empty(),
        "todo create must not contact GitHub"
    );
    // First tick with deliberate pickup reflects issue #1 without dispatching.
    project.tick();
    assert_eq!(project.rt.started_count(), 0);
    // Enable automatic pickup so the next tick exercises inbound hold
    // ingestion running before dispatch sees the todo.
    let layer = project.repo.path().join("instrukt.yaml");
    let config = std::fs::read_to_string(&layer)
        .unwrap()
        .replace("autostart: false", "autostart: true");
    std::fs::write(&layer, config).unwrap();
    // People's label actions are the issue's events, read from where the
    // last read stopped; each actor's access is GitHub's answer.
    let label = |event: &str, login: &str, permission: &str| {
        project.github.seed_event(
            REPOSITORY,
            1,
            serde_json::json!({"event":event,"label":{"name":"hold"},"actor":{"login":login},"created_at":"2026-11-06T10:00:00Z"}),
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
    label("labeled", "maintainer", "write");
    project.tick();
    assert_eq!(
        project.rt.started_count(),
        0,
        "a GitHub hold label must prevent dispatch on the same tick"
    );
    let requests_before_show = project.github.requests().len();
    let held = project.todo(["show", "held-work"]);
    assert_eq!(
        project.github.requests().len(),
        requests_before_show,
        "todo show must not contact GitHub"
    );
    project.todo([
        "artifacts",
        "write",
        "held-work",
        "log",
        "Progress remains available.",
    ]);
    project.tick();
    assert_eq!(
        project.rt.started_count(),
        0,
        "held work stays out of dispatch while progress reflects"
    );
    let before = project.github.artifacts(REPOSITORY, 1);
    project.github.edit_issue(REPOSITORY, 1, |issue| {
        issue.labels.retain(|l| l != "hold" && l != "status:active");
        issue.labels.push("status:delivered".into());
        issue.labels.push("team-owned".into());
    });
    label("unlabeled", "collaborator", "write");
    // The tick ingests the release before pickup, dispatches the unheld todo,
    // and restores output labels in the same pass.
    project.tick();
    assert_eq!(
        project.rt.started_count(),
        1,
        "releasing the GitHub hold label must allow dispatch on the same tick"
    );
    let released = project.todo(["show", "held-work"]);
    assert_eq!(project.github.artifacts(REPOSITORY, 1), before);
    project.github.edit_issue(REPOSITORY, 1, |_| {});
    label("labeled", "outsider", "read");
    project.tick();
    assert_eq!(
        project.rt.started_count(),
        1,
        "an unauthorized hold action must not disturb dispatch"
    );
    let refused = project.todo(["show", "held-work"]);
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

    crew.runtime.open_session_in("lead-1", crew.root());
    crew.runtime.set_status("lead-1", "ended ok");
    let config = project.repo.path().join("instrukt.yaml");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text = text.replace("github:\n", "github:\n  approvers: [maintainer]\n");
    std::fs::write(config, text).unwrap();
    project.tick();
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
    project.tick();
    assert_eq!(crew.runtime.sent_count("lead-1"), 0); // This manually opened thread had no initial assignment message.
    let first = crew.runtime.input_log("lead-1");
    insta::assert_snapshot!("github-decision-attention", redact(&first));
    let started = crew.runtime.started_count();
    insta::assert_snapshot!(
        "preparation-after-github-decision",
        redact(&crew.next("prepare", "approval-work"))
    );
    assert_eq!(crew.runtime.started_count(), started);
    project.tick();
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
    project.tick();
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
    project.tick();
    assert_eq!(project.github.comments(REPOSITORY, 1).len(), before);
    project.tick();
    let recovered = project.ok(["show", "reply-work"]);
    assert_eq!(project.github.comments(REPOSITORY, 1).len(), before + 1);
    project.tick();
    let again = project.ok(["show", "reply-work"]);
    assert_eq!(project.github.comments(REPOSITORY, 1).len(), before + 1);
    assert_eq!(redact(&recovered), redact(&again));
    insta::assert_snapshot!("decision-after-reply-retry", redact(&again));
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
    crew.runtime.open_session_in("lead-1", crew.root());
    crew.runtime.set_status("lead-1", "ended ok");
    project.env.run(crew.root(), ["sync"]).success();
    // Reflect the todo to GitHub before the worker finishes so issue #1 exists.
    project.env.run(crew.root(), ["tick"]).success();
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
    project.env.run(crew.root(), ["tick"]).success();
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
    crew.runtime.open_session_in("lead-1", crew.root());
    crew.runtime.set_status("lead-1", "ended ok");
    let source = project
        .repo
        .read("instrukt.yaml")
        .replace("github:\n", "github:\n  approvers: [maintainer]\n");
    project.repo.write("instrukt.yaml", &source);
    project.tick();
    project.github.seed_comment(
        REPOSITORY,
        1,
        "maintainer",
        "COLLABORATOR",
        "/instrukt request-changes prd Clarify the audience.",
    );
    project.tick();
    assert_eq!(crew.runtime.message_count("lead-1"), 1);
    insta::assert_snapshot!(
        "github-request-changes-attention",
        redact(&crew.runtime.input_log("lead-1"))
    );
    project.tick();
    assert_eq!(crew.runtime.message_count("lead-1"), 1);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn missing_board_is_set_up_automatically_and_reused() {
    let project = Project::new();
    // This project has no board reference; the existing unrelated board is preserved.
    without_board(project.repo.path());
    let file = project.repo.path().join("instrukt.yaml");
    let config = std::fs::read_to_string(&file).unwrap();
    project.create("cockpit", "bug", INTENT);
    project.tick();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        config,
        "reflection leaves the committed project layer as it is"
    );
    project
        .github
        .add_project_field("acme", 2, "User field", Some(&["theirs"]));
    project
        .github
        .remove_project_field("acme", 2, "Record status");
    project.github.clear_project_items("acme", 2);
    project.tick();
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

/// Removes the board reference from the project layer at `checkout`.
fn without_board(checkout: &Path) {
    let file = checkout.join("instrukt.yaml");
    let config = std::fs::read_to_string(&file)
        .unwrap()
        .replace("board: acme/1\n", "");
    std::fs::write(&file, config).unwrap();
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR11
#[test]
fn a_public_projects_new_board_is_public_and_a_maintainers_later_choice_is_kept() {
    let project = Project::new();
    project.github.set_repo_public(REPOSITORY);
    without_board(project.repo.path());
    project.create("cockpit", "bug", INTENT);
    project.tick();
    let todo = issue_node(&project, 1);
    assert!(project.github.project_public("acme", 2));
    assert!(on_board(&project, "acme", 2).contains(&todo));

    project.github.set_project_public("acme", 2, false);
    project.tick();
    assert!(
        !project.github.project_public("acme", 2),
        "the recorded board keeps the maintainer's choice"
    );
    let elsewhere = project.other_computer();
    without_board(elsewhere.repo.path());
    elsewhere.ok([
        "create",
        "dashboard",
        INTENT,
        "--kind",
        "change",
        "--approved",
    ]);
    elsewhere.tick();
    let theirs = issue_node(&project, 2);
    assert!(
        on_board(&project, "acme", 2).contains(&theirs),
        "the other computer reflects its own todo onto the board it found"
    );
    assert_eq!(
        project.github.project_numbers("acme"),
        [1, 2],
        "the other computer reuses the board instead of creating one"
    );
    assert!(
        !project.github.project_public("acme", 2),
        "a board found by its title keeps the maintainer's choice"
    );

    choose_board(&project, "acme/1");
    project.tick();
    assert!(
        !project.github.project_public("acme", 1),
        "a named board keeps its visibility"
    );
    assert!(on_board(&project, "acme", 1).contains(&todo));
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR12
#[test]
fn a_new_board_github_will_not_make_public_is_set_up_private_and_holds_the_work() {
    let project = Project::new();
    project.github.set_repo_public(REPOSITORY);
    project.github.set_project_visibility_refused(true);
    without_board(project.repo.path());
    let created = project.ok([
        "create",
        "cockpit",
        INTENT,
        "--kind",
        "change",
        "--approved",
    ]);
    project.tick();
    assert!(!project.github.project_public("acme", 2));
    insta::assert_snapshot!("refused-visibility-created", redact(&created));
    insta::assert_snapshot!(
        "refused-visibility-board-state",
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
    project.tick();
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
    project.tick();
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
    project.tick();
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
    project.tick();
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
    project.tick();
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
    project.tick();
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "update",
        "person-wait",
        "--hold",
        "--reason",
        "Ask the owner.",
    ]);
    project.tick();
    assert!(marked_needs_you(&project, 1));
    project.ok(["update", "person-wait", "--release"]);
    project.tick();
    assert!(!marked_needs_you(&project, 1));
    approvals_config(&project, "peer", "human");
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd",
        "The intended outcome.",
    ]);
    project.tick();
    assert!(!marked_needs_you(&project, 1));
    approvals_config(&project, "human", "human");
    project.tick();
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd-review-findings",
        "Approved.",
    ]);
    project.tick();
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "stamp",
        "person-wait",
        "--verdict",
        "approve",
        "--phase",
        "prd_review",
    ]);
    project.tick();
    assert!(marked_needs_you(&project, 1));
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd-review-findings",
        "The review must be assessed again.",
    ]);
    project.tick();
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "stamp",
        "person-wait",
        "--verdict",
        "needs_work",
        "--phase",
        "prd_review",
    ]);
    project.tick();
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
    project.tick();
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
    project.tick();
    assert!(marked_needs_you(&project, 1));
    project.ok(["update", "person-wait", "--release"]);
    project.tick();
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd",
        "A revised outcome needs a new approval.",
    ]);
    project.tick();
    assert!(!marked_needs_you(&project, 1));
    project.ok([
        "artifacts",
        "write",
        "person-wait",
        "prd",
        "The intended outcome.",
    ]);
    project.tick();
    assert!(marked_needs_you(&project, 1));
    approvals_config(&project, "peer", "human");
    project.tick();
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
    project.tick();
    assert!(marked_needs_you(&project, 2));
    approvals_config(&project, "peer", "peer");
    project.tick();
    assert!(!marked_needs_you(&project, 2));
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn cockpit_setup_failure_keeps_issue_reflection_and_retries_without_duplicate_views() {
    let project = Project::new();
    // An unauthorized (401) board-setup failure stays with the board: its
    // cause remains visibly pending while unrelated issue and artifact
    // reflection still runs in the same tick (`github-apps`, Installation
    // credentials).
    project.github.respond_once(
        "POST",
        "/orgs/acme/projectsV2/1/views",
        CannedResponse {
            status: 401,
            headers: Vec::new(),
            body: serde_json::json!({"message":"Bad credentials"}),
        },
    );
    project.create("retry-views", "bug", "Keep the cockpit retryable.");
    project.tick();
    let pending = project.ok(["show", "retry-views"]);
    assert_eq!(project.github.issues(REPOSITORY).len(), 1);
    assert_eq!(project.github.artifacts(REPOSITORY, 1).len(), 1);
    assert!(project.github.project_views("acme", 1).is_empty());
    insta::assert_snapshot!("cockpit-view-setup-pending", redact(&pending));
    project.tick();
    let views = project.github.project_views("acme", 1);
    assert_eq!(views.len(), 2);
    project.tick();
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
    project.tick();
    assert_eq!(project.github.project_views("acme", 1), expected);
}

// spec: project/instrukt/spec/feature/work/github-record UC-GR5
#[test]
fn cockpit_repair_failure_stays_pending_for_an_already_reflected_item() {
    let project = Project::new();
    project.create("reflected-cockpit", "bug", "Keep cockpit failures visible.");
    project.tick();
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
    project.tick();
    insta::assert_snapshot!(
        "reflected-item-cockpit-pending",
        redact(&project.ok(["show", "reflected-cockpit"]))
    );
    let snapshot = project.github.comments(REPOSITORY, 1);
    project.tick();
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
