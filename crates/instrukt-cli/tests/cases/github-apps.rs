//! App credentials and bot recovery through the real CLI and local GitHub API.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
use crate::common;
use crate::fixtures::work as support;
use support::{Project, REPOSITORY};

// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn issue_maintenance_uses_the_project_app_and_never_a_process_token() {
    let project = Project::new();
    project.create("app-record", "bug", "Verify bot attribution.");
    assert!(
        project.github.requests().is_empty(),
        "bugs report must not contact GitHub"
    );
    project.tick();
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

fn rejected_credential(method: &str, endpoint: &str, body: serde_json::Value) -> String {
    let project = Project::new();
    project.create(
        "pending-app",
        "bug",
        "Keep accepted work through credential failure.",
    );
    project.tick();
    let comments = project.github.comments(REPOSITORY, 1);
    let artifacts = project.github.artifacts(REPOSITORY, 1);
    let requests_before = project.github.requests().len();
    let written = project.ok([
        "artifacts",
        "write",
        "pending-app",
        "log",
        "accepted locally",
    ]);
    assert_eq!(
        project.github.requests().len(),
        requests_before,
        "artifacts write must not contact GitHub"
    );
    project.github.respond_once(
        method,
        endpoint,
        common::github::CannedResponse {
            status: 200,
            headers: Vec::new(),
            body,
        },
    );
    project.tick();
    let requests_after_tick = project.github.requests().len();
    let shown = project.ok(["show", "pending-app"]);
    assert_eq!(
        project.github.requests().len(),
        requests_after_tick,
        "todo show must not contact GitHub"
    );
    assert_eq!(project.github.comments(REPOSITORY, 1), comments);
    assert_eq!(project.github.artifacts(REPOSITORY, 1), artifacts);
    support::redact(&format!("{written}---\n{shown}"))
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
    repo.git(["add", "change.md"]);
    env.instrukt(repo.path())
        .env("T3CODE_THREAD_ID", "local-author")
        .args(["repo", "commit", "--message", "record local work"])
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
    assert!(
        project.github.requests().is_empty(),
        "bugs report must not contact GitHub"
    );
    project.tick();
    project.github.clear_requests();
    project.ok(["update", "record-0"]);
    assert!(
        project.github.requests().is_empty(),
        "todo update must not contact GitHub"
    );
    project.tick();
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

// spec: project/instrukt/spec/feature/work/github-apps UC-GA3
#[test]
fn an_automatic_review_verdict_reaches_the_pull_request_and_its_exact_commit() {
    use crate::fixtures::reviewed::{REPOSITORY, Reviewed};
    let project = Reviewed::opted_in();
    let head = project.push("feature", "one.txt");
    let pull = project.open("feature", false);
    project.tick();
    project
        .record_at(
            &project.rt.native_id("s1"),
            &head,
            "needs_work",
            Some("2026-11-24T12:00:00Z"),
        )
        .success();

    project.tick();
    let later = project.push("feature", "two.txt");
    project.tick();

    let reviewer = |actor: &serde_json::Value| actor["login"] == "instruktai-reviewer[bot]";
    let published = serde_json::json!({
        "pull_request_reviews": project
            .github
            .reviews(REPOSITORY, pull)
            .iter()
            .filter(|review| reviewer(&review["user"]))
            .map(|review| serde_json::json!({
                "state": review["state"],
                "commit": review["commit_id"],
                "body": review["body"],
            }))
            .collect::<Vec<_>>(),
        "reviewed_commit_statuses": project
            .github
            .statuses(REPOSITORY, &head)
            .iter()
            .filter(|status| reviewer(&status["creator"]))
            .map(|status| serde_json::json!({
                "context": status["context"],
                "state": status["state"],
                "description": status["description"],
                "target_url": status["target_url"],
            }))
            .collect::<Vec<_>>(),
        "later_head_statuses": project
            .github
            .statuses(REPOSITORY, &later)
            .iter()
            .filter(|status| reviewer(&status["creator"]))
            .count(),
    });
    insta::assert_snapshot!(
        "automatic-review-verdict-on-github",
        serde_json::to_string_pretty(&published)
            .unwrap()
            .replace(&head, "<reviewed-head>")
    );
}
