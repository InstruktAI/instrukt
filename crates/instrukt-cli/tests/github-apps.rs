//! App credentials and bot recovery through the real CLI and local GitHub API.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
mod common;
#[path = "support/work.rs"]
mod support;
use support::{Project, REPOSITORY};

// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn records_use_the_project_app_and_never_a_process_token() {
    let project = Project::new();
    project.create("app-record", "bug", "Verify bot attribution.");
    let config = project
        .env
        .instrukt(project.repo.path())
        .env("TEST_APP_KEY", "must-not-expand")
        .args(["config", "get", "github.apps", "--json"])
        .assert()
        .success();
    let public: serde_json::Value = serde_json::from_slice(&config.get_output().stdout).unwrap();
    assert_eq!(public["project"]["private_key"], "${TEST_APP_KEY}");
    assert_eq!(
        project.github.issues(REPOSITORY)[0]["user"]["login"],
        "instruktai[bot]"
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

// spec: project/instrukt/spec/feature/work/github-apps UC-GA1
#[test]
fn failed_app_identity_scope_or_expiry_keeps_accepted_work_pending() {
    use common::github::CannedResponse;
    use serde_json::json;
    let mut reports = Vec::new();
    for (name, method, endpoint, body) in [
        (
            "identity",
            "GET",
            "/app",
            json!({"id":99,"slug":"wrong-app"}),
        ),
        (
            "actor",
            "POST",
            "/graphql",
            json!({"data":{"viewer":{"id":"wrong","databaseId":99,"login":"wrong[bot]"}}}),
        ),
        (
            "permissions",
            "POST",
            "/app/installations/1/access_tokens",
            json!({"token":"must-not-leak","expires_at":"2099-01-01T00:00:00Z","permissions":{}}),
        ),
        (
            "expiry",
            "POST",
            "/app/installations/1/access_tokens",
            json!({"token":"must-not-leak","expires_at":"2000-01-01T00:00:00Z","permissions":{"issues":"write","contents":"read","pull_requests":"read"}}),
        ),
    ] {
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
            CannedResponse {
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
        reports.push(format!("{name}: {}", support::redact(&written)));
    }
    insta::assert_snapshot!("credential-failures", reports.join("\n"));
}
