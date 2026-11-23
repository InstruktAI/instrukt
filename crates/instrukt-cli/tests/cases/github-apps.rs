//! App credentials and bot recovery through the real CLI and local GitHub API.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
use crate::common;
use crate::fixtures::work as support;
use support::{Project, REPOSITORY};

// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn records_use_the_project_app_and_never_a_process_token() {
    let project = Project::new();
    project.create("app-record", "bug", "Verify bot attribution.");
    assert_eq!(project.github.issues(REPOSITORY).len(), 1);
    let config = project
        .env
        .instrukt(project.repo.path())
        .env("TEST_APP_KEY", "must-not-expand")
        .args(["config", "get", "github.apps", "--json"])
        .assert()
        .success();
    let public: serde_json::Value = serde_json::from_slice(&config.get_output().stdout).unwrap();
    insta::assert_json_snapshot!("public-app-configuration", public);
    insta::assert_json_snapshot!(
        "project-bot-issue-author",
        project.github.issues(REPOSITORY)[0]["user"]
    );
}

// spec: project/instrukt/spec/feature/work/github-apps UC-GA2
#[test]
fn configured_bots_with_none_association_restore_but_similarly_named_actors_do_not() {
    let project = Project::new();
    project.create("app-record", "bug", "Verify bot recovery.");
    let comments = project.github.comments(REPOSITORY, 1);
    let body = comments.last().unwrap()["body"].as_str().unwrap();
    let marker = "<!-- instrukt:work-record:v2:";
    let line = body.lines().find(|line| line.starts_with(marker)).unwrap();
    let mut envelope: serde_json::Value =
        serde_json::from_str(&line[marker.len()..line.len() - 4]).unwrap();
    envelope["base"] = envelope["record"]["revision"].clone();
    envelope["record"]["revision"] = 2.into();
    envelope["record"]["schema"] = 1.into();
    envelope["record"]["hold"] = serde_json::json!({"source":"manual","reason":"recovered from configured bot","by":{"person":{"name":"Test Developer"}},"at":"2026-11-03T10:00:00Z"});
    // NONE carries no repository association: stable configured actor facts admit it.
    project.github.seed_comment(
        REPOSITORY,
        1,
        "instruktai[bot]",
        "NONE",
        &format!("{marker}{envelope} -->"),
    );
    let mut forged = envelope.clone();
    forged["base"] = 2.into();
    forged["record"]["revision"] = 3.into();
    forged["record"]["schema"] = 999.into();
    project.github.seed_comment(
        REPOSITORY,
        1,
        "instruktai-impostor[bot]",
        "NONE",
        &format!("{marker}{forged} -->"),
    );
    let other = project.other_computer();
    let shown = other.ok(["show", "app-record"]);
    insta::assert_snapshot!("bot-recovered", support::redact(&shown));
}

fn rejected_credential(method: &str, endpoint: &str, body: serde_json::Value) -> String {
    let project = Project::new();
    project.create(
        "pending-app",
        "bug",
        "Keep accepted work through credential failure.",
    );
    let comments = project.github.comments(REPOSITORY, 1);
    let artifacts = project.github.artifacts(REPOSITORY, 1);
    project.github.respond_once(
        method,
        endpoint,
        common::github::CannedResponse {
            status: 200,
            headers: Vec::new(),
            body,
        },
    );
    let written = project.ok([
        "artifacts",
        "write",
        "pending-app",
        "log",
        "accepted locally",
    ]);
    assert_eq!(project.github.comments(REPOSITORY, 1), comments);
    assert_eq!(project.github.artifacts(REPOSITORY, 1), artifacts);
    support::redact(&written)
}
// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn an_app_identity_mismatch_keeps_accepted_work_pending() {
    insta::assert_snapshot!(
        "rejected-app-identity",
        rejected_credential(
            "GET",
            "/app",
            serde_json::json!({"id":99,"slug":"wrong-app"})
        )
    );
}
// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn an_installation_actor_mismatch_keeps_accepted_work_pending() {
    insta::assert_snapshot!(
        "rejected-installation-actor",
        rejected_credential(
            "POST",
            "/graphql",
            serde_json::json!({"data":{"viewer":{"id":"wrong","databaseId":99,"login":"wrong[bot]"}}})
        )
    );
}
// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn missing_permissions_keep_accepted_work_pending() {
    insta::assert_snapshot!(
        "rejected-missing-permissions",
        rejected_credential(
            "POST",
            "/app/installations/1/access_tokens",
            serde_json::json!({"token":"must-not-leak","expires_at":"2099-01-01T00:00:00Z","permissions":{}})
        )
    );
}
// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn an_expired_credential_keeps_accepted_work_pending() {
    insta::assert_snapshot!(
        "rejected-expired-credential",
        rejected_credential(
            "POST",
            "/app/installations/1/access_tokens",
            serde_json::json!({"token":"must-not-leak","expires_at":"2000-01-01T00:00:00Z","permissions":{"issues":"write","contents":"read","pull_requests":"read"}})
        )
    );
}

// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
// spec: project/instrukt/spec/feature/repository/attributed-commit UC-RC1
#[test]
fn a_configured_bot_role_commits_in_a_local_repository_without_installation_access() {
    let env = common::TestEnv::new();
    let github = common::github::FakeGitHub::start();
    github.configure(&env);
    let config = env.instrukt_home().join("instrukt.yaml");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(config, format!("{text}runner: {{runtime: t3}}\n")).unwrap();
    common::agent_profiles(
        &env,
        "roles: {local-author: {class: agent, can: [all], github_app: author}}",
    );
    let repo = env.git_repo("local-only");
    repo.write("README.md", "Local repository.\n");
    repo.commit("initial local state");
    repo.git(["switch", "-c", "local-work"]);
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let mut db = instrukt_core::state_db::StateDb::open(&home).unwrap();
    db.write::<_,instrukt_core::state_db::StateDbError>(|tx|{
        tx.execute("INSERT INTO procedure_responsibilities(responsibility,procedure,exact_home,generation,semantic_key,checkout,declaration,request,session) VALUES ('local','sdlc/procedure/local','platform',1,'local',?1,?2,'{}','t3:local-author')",instrukt_core::state_db::rusqlite::params![repo.path().to_string_lossy(),r#"{"role":"local-author"}"#]).unwrap(); Ok(())
    }).unwrap();
    drop(db);
    // Local attribution has no repository-installation credential dependency.
    std::fs::remove_file(env.instrukt_home().join("secrets.env")).unwrap();
    repo.write("change.md", "Local work.\n");
    env.instrukt(repo.path())
        .env("T3CODE_THREAD_ID", "local-author")
        .args([
            "repo",
            "commit",
            "--message",
            "record local work",
            "--only",
            "change.md",
        ])
        .assert()
        .success();
    insta::assert_snapshot!(
        "local-bot-commit",
        repo.git(["log", "-1", "--format=%an <%ae>"])
    );
    assert!(github.requests().is_empty());
}

// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn one_multi_item_reflection_reuses_authenticated_actor_facts_and_operation_credentials() {
    let project = Project::new();
    // Several items are ordinary work, not seven independent authentications.
    for item in 0..8 {
        project.create(&format!("record-{item}"), "bug", "Reflect shared work.");
    }
    project.github.clear_requests();
    project.ok(["update", "record-0"]);
    assert_eq!(
        project
            .github
            .requests_to("POST", "/app/installations/1/access_tokens")
            .len(),
        2
    );
    assert_eq!(project.github.requests_to("GET", "/app").len(), 1);
    assert_eq!(
        project
            .github
            .requests()
            .iter()
            .filter(|r| r.path.starts_with("/apps/"))
            .count(),
        0
    );
}

/// Records `slug`'s work as a retired `work-orchestrator` driver did before
/// the unified orchestrator replaced it: the binding keeps the retired role
/// and names the branch whose pull request is owed.
fn retired_orchestrator_branch(project: &Project, slug: &str) {
    use instrukt_core::state_db::rusqlite::{Connection, params};
    let branch = format!("{slug}/work");
    let head = project.repo.git(["rev-parse", "HEAD"]);
    project.github.set_branch(REPOSITORY, &branch, head.trim());
    let db = Connection::open(project.env.instrukt_home().join("state/instrukt.db")).unwrap();
    let id: String = db
        .query_row("SELECT id FROM todos WHERE slug=?1", [slug], |row| {
            row.get(0)
        })
        .unwrap();
    let key = format!("instrukt:todo:demo/{id}:work-orchestrator:1");
    let evidence = serde_json::json!({
        "role": "work-orchestrator", "session": null, "worktree": null, "runtime": "t3",
        "native_placement": null, "assignment": "driver", "occurrence": null, "attempt": 1,
        "semantic_key": key, "native_assignment": null, "lane": "work", "start_from": null,
        "no_start_reason": null, "branch": branch, "base_head": null,
        "completion_sequence": null, "completed_head": null, "failure_reason": null,
        "failure_head": null, "failure_workspace": null, "failed_at": null, "released_at": null
    });
    db.execute(
        "UPDATE todos SET branch=?2 WHERE id=?1",
        params![id, branch],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_bindings(todo_id,role,attempt,runtime,semantic_key,branch,evidence) \
         VALUES (?1,'work-orchestrator',1,'t3',?2,?3,?4)",
        params![id, key, branch, evidence.to_string()],
    )
    .unwrap();
}

/// The developer's todos in the order one reflection pass reaches them.
fn reflection_order(project: &Project) -> Vec<String> {
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        project.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let mut rows = db.prepare("SELECT slug FROM todos ORDER BY id").unwrap();
    rows.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The number of the issue reflecting `slug`.
fn issue_of(project: &Project, slug: &str) -> u64 {
    project
        .github
        .issues(REPOSITORY)
        .into_iter()
        .find(|issue| issue["title"].as_str().is_some_and(|t| t.contains(slug)))
        .unwrap()["number"]
        .as_u64()
        .unwrap()
}

fn pull_requests(project: &Project) -> Vec<serde_json::Value> {
    project
        .github
        .issues(REPOSITORY)
        .into_iter()
        .filter(|issue| issue.get("pull_request").is_some())
        .collect()
}

// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn a_retired_orchestrator_pull_request_is_authored_by_the_unified_orchestrator_app() {
    let project = Project::new();
    common::agent_profiles(
        &project.env,
        "roles: {todo-orchestrator: {github_app: author}}",
    );
    project.create(
        "legacy-pull",
        "bug",
        "Reflect work bound before the split retired.",
    );
    retired_orchestrator_branch(&project, "legacy-pull");
    project.ok(["update", "legacy-pull"]);
    let pulls = pull_requests(&project);
    assert_eq!(pulls.len(), 1);
    insta::assert_json_snapshot!("retired-orchestrator-pull-author", pulls[0]["user"]);
}

// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn one_todos_app_setup_failure_leaves_the_later_todos_reflecting() {
    let project = Project::new();
    common::agent_profiles(
        &project.env,
        "roles: {todo-orchestrator: {github_app: author}}",
    );
    project.create("todo-one", "bug", "Reflect beside another todo's App.");
    project.create("todo-two", "bug", "Reflect beside another todo's App.");
    // The todo the pass reaches first carries the failing App.
    let order = reflection_order(&project);
    let (failing, later) = (&order[0], &order[1]);
    retired_orchestrator_branch(&project, failing);
    let issue = issue_of(&project, later);
    let artifacts = project.github.artifacts(REPOSITORY, issue);
    project.github.respond_once(
        "POST",
        "/app/installations/2/access_tokens",
        common::github::CannedResponse {
            status: 201,
            headers: Vec::new(),
            body: serde_json::json!({"token":"must-not-leak","expires_at":"2099-01-01T00:00:00Z","permissions":{}}),
        },
    );
    project.ok([
        "artifacts",
        "write",
        later,
        "log",
        "reflected beside the failing App",
    ]);
    assert_ne!(project.github.artifacts(REPOSITORY, issue), artifacts);
    assert!(pull_requests(&project).is_empty());
    let shown = project
        .ok(["show", failing])
        .replace(failing.as_str(), "<failing>")
        .replace(
            &format!("issue: #{}", issue_of(&project, failing)),
            "issue: <failing>",
        );
    insta::assert_snapshot!(
        "app-setup-failure-stays-with-its-todo",
        support::redact(&shown)
    );
}
