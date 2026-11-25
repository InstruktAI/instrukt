//! `project/instrukt/spec/feature/configuration/configuration-editing`:
//! `instrukt config init|get|patch` — revision checks, whole-document
//! validation and atomic writes — and `instrukt config schema`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use std::path::Path;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::config::CURRENT_SCHEMA_VERSION;
use serde_yaml_ng::Value;

/// Emitted text with the test's temporary directory shown as `[TMP]`.
fn stderr(env: &TestEnv, assert: &Assert) -> String {
    let base = env.path("").display().to_string();
    String::from_utf8_lossy(&assert.get_output().stderr)
        .replace(base.trim_end_matches('/'), "[TMP]")
}

fn revision(env: &TestEnv, cwd: &Path) -> String {
    let out = env.run(cwd, ["config", "get", "--revision"]).success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let revision = text.trim().to_owned();
    assert!(!revision.is_empty(), "a revision is printed");
    revision
}

fn layer(repo: &Repo) -> Value {
    serde_yaml_ng::from_str(&repo.read("instrukt.yaml")).unwrap()
}

fn initialized(env: &TestEnv, name: &str) -> Repo {
    let repo = env.git_repo(name);
    env.run(repo.path(), ["config", "init"]).success();
    repo
}

fn patch(env: &TestEnv, repo: &Repo, revision: &str, yaml: &str) -> Assert {
    env.run(
        repo.path(),
        ["config", "patch", "--revision", revision, "--yaml", yaml],
    )
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn a_patch_from_the_current_revision_changes_only_what_it_names() {
    let env = TestEnv::new();
    let repo = initialized(&env, "demo");
    let before = revision(&env, repo.path());
    assert_eq!(
        revision(&env, repo.path()),
        before,
        "stable while unchanged"
    );
    let untouched = layer(&repo);

    let patched = patch(
        &env,
        &repo,
        &before,
        "board: acme/7\ngithub:\n  repository: '{project}/{runtime.session_id}'\n",
    )
    .success();
    assert!(
        patched.get_output().stdout.is_empty(),
        "a patch prints nothing"
    );

    let after = revision(&env, repo.path());
    assert_ne!(after, before);
    let changed = layer(&repo);
    assert_eq!(changed["project"], untouched["project"]);
    assert_eq!(changed["schema_version"], untouched["schema_version"]);
    assert_eq!(changed["board"], Value::String("acme/7".into()));
    let stored = Value::String("{project}/{runtime.session_id}".into());
    assert_eq!(
        changed["github"]["repository"], stored,
        "references are stored unexpanded"
    );

    // A null removes a key; the patch may come from a file or standard input.
    let file = env.path("patch.yaml");
    std::fs::write(&file, "board: ~\n").unwrap();
    env.run(
        repo.path(),
        [
            "config",
            "patch",
            "--revision",
            &after,
            "--from-file",
            file.to_str().unwrap(),
        ],
    )
    .success();
    assert!(layer(&repo).get("board").is_none());
    let third = revision(&env, repo.path());
    env.instrukt(repo.path())
        .args(["config", "patch", "--revision", &third])
        .write_stdin("approvals:\n  prd: peer\n")
        .assert()
        .success();
    let approvals: Value = serde_yaml_ng::from_str("{prd: peer}").unwrap();
    assert_eq!(layer(&repo)["approvals"], approvals);
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn a_patch_from_a_stale_revision_is_refused_and_writes_nothing() {
    let env = TestEnv::new();
    let repo = initialized(&env, "demo");
    let read = revision(&env, repo.path());
    patch(&env, &repo, &read, "board: acme/7").success();
    let content = repo.read("instrukt.yaml");

    let refused = patch(&env, &repo, &read, "board: false").failure();
    insta::assert_snapshot!("stale-revision", stderr(&env, &refused));
    assert_eq!(repo.read("instrukt.yaml"), content);
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn of_two_patches_from_one_revision_exactly_one_lands() {
    let env = TestEnv::new();
    let repo = initialized(&env, "demo");
    let read = revision(&env, repo.path());
    let outcomes: Vec<bool> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|i| {
                let (env, repo, read) = (&env, &repo, &read);
                scope.spawn(move || {
                    env.instrukt(repo.path())
                        .args([
                            "config",
                            "patch",
                            "--revision",
                            read,
                            "--yaml",
                            &format!("approvals:\n  {}: peer\n", ["prd", "task"][i]),
                        ])
                        .output()
                        .expect("instrukt runs")
                        .status
                        .success()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(outcomes.iter().filter(|ok| **ok).count(), 1, "{outcomes:?}");
    let approvals = layer(&repo)["approvals"].as_mapping().map(|m| m.len());
    assert_eq!(approvals, Some(1), "only the winning patch is stored");
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn a_patch_leaving_the_configuration_invalid_is_refused_naming_what_is_wrong() {
    let env = TestEnv::new();
    let repo = initialized(&env, "demo");
    let read = revision(&env, repo.path());
    let content = repo.read("instrukt.yaml");

    let refused = patch(
        &env,
        &repo,
        &read,
        "runtimes: []\nboard: maybe\nactions:\n  echo: {command: echo, takes_files: not-a-bool}\n",
    )
    .failure();
    insta::assert_snapshot!("invalid-patch", stderr(&env, &refused));
    let not_a_mapping = patch(&env, &repo, &read, "- a list").failure();
    insta::assert_snapshot!(stderr(&env, &not_a_mapping), @"instrukt: the patch is not a YAML mapping");
    assert_eq!(repo.read("instrukt.yaml"), content);
    assert_eq!(revision(&env, repo.path()), read);
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE2
#[test]
fn a_machine_without_configuration_is_initialized_without_a_project_identity() {
    let env = TestEnv::new();
    let outside = env.path("");
    let before = env
        .run(
            &outside,
            [
                "config",
                "patch",
                "--global",
                "--revision",
                "r",
                "--yaml",
                "ticks: 'off'",
            ],
        )
        .failure();
    insta::assert_snapshot!("global-patch-before-init", stderr(&env, &before));

    env.run(&outside, ["config", "init", "--global"]).success();
    let global: Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(env.instrukt_home().join("instrukt.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        global["schema_version"],
        Value::from(CURRENT_SCHEMA_VERSION)
    );
    assert!(
        global.get("project").is_none(),
        "a machine needs no project identity"
    );
    env.run(&outside, ["config", "validate", "--global"])
        .success();
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE2
#[test]
fn a_project_without_configuration_is_named_after_its_folder() {
    let env = TestEnv::new();
    let repo = env.git_repo("My Project_v2");
    let before = env
        .run(
            repo.path(),
            [
                "config",
                "patch",
                "--revision",
                "r",
                "--yaml",
                "board: acme/7",
            ],
        )
        .failure();
    insta::assert_snapshot!("project-patch-before-init", stderr(&env, &before));

    env.run(repo.path(), ["config", "init"]).success();
    assert_eq!(
        repo.read("instrukt.yaml"),
        format!("schema_version: {CURRENT_SCHEMA_VERSION}\nproject: my-project-v2\n")
    );
    env.run(repo.path(), ["config", "validate"]).success();
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE2
#[test]
fn a_linked_worktree_is_named_after_the_primary_worktree() {
    let env = TestEnv::new();
    let repo = env.git_repo("primary-checkout");
    repo.write("README.md", "x\n");
    repo.commit("feat: start");
    let linked = env.path("linked-elsewhere");
    repo.git([
        "worktree",
        "add",
        "--quiet",
        "-b",
        "side",
        linked.to_str().unwrap(),
    ]);
    env.run(&linked, ["config", "init"]).success();
    assert_eq!(
        std::fs::read_to_string(linked.join("instrukt.yaml")).unwrap(),
        format!("schema_version: {CURRENT_SCHEMA_VERSION}\nproject: primary-checkout\n")
    );
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE2
#[test]
fn an_empty_project_identity_is_refused() {
    let env = TestEnv::new();
    let repo = env.git_repo("___");
    let refused = env.run(repo.path(), ["config", "init"]).failure();
    insta::assert_snapshot!("empty-slug", stderr(&env, &refused));
    assert!(!repo.path().join("instrukt.yaml").exists());
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE2
#[test]
fn existing_configuration_at_the_current_schema_is_kept() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    let layer = format!(
        "# my notes\nschema_version: {CURRENT_SCHEMA_VERSION}\nproject: chosen\nboard: acme/7\n"
    );
    repo.write("instrukt.yaml", &layer);
    env.run(repo.path(), ["config", "init"]).success();
    assert_eq!(repo.read("instrukt.yaml"), layer);
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE2
#[test]
fn existing_configuration_of_a_newer_schema_is_refused() {
    let env = TestEnv::new();
    let repo = env.git_repo("newer");
    let layer = format!(
        "schema_version: {}\nproject: newer\n",
        CURRENT_SCHEMA_VERSION + 1
    );
    repo.write("instrukt.yaml", &layer);
    let refused = env.run(repo.path(), ["config", "init"]).failure();
    insta::assert_snapshot!("newer-schema", stderr(&env, &refused));
    assert_eq!(repo.read("instrukt.yaml"), layer);
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn a_patched_on_or_off_setting_is_written_as_a_string_every_yaml_reader_keeps() {
    let env = TestEnv::new();
    let outside = env.path("");
    env.run(&outside, ["config", "init", "--global"]).success();
    let out = env
        .run(&outside, ["config", "get", "--global", "--revision"])
        .success();
    let read = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    env.run(
        &outside,
        [
            "config",
            "patch",
            "--global",
            "--revision",
            read.trim(),
            "--yaml",
            "ticks: 'off'",
        ],
    )
    .success();
    let written = std::fs::read_to_string(env.instrukt_home().join("instrukt.yaml")).unwrap();
    insta::assert_snapshot!("patched-global-layer", written);
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE5
#[test]
fn switching_on_event_responses_asks_for_the_event_transport_first() {
    let env = TestEnv::new();
    let outside = env.path("");
    env.run(&outside, ["config", "init", "--global"]).success();
    let repo = initialized(&env, "demo");
    let read = revision(&env, repo.path());
    let content = repo.read("instrukt.yaml");

    let refused = patch(&env, &repo, &read, "modules: {events: true}").failure();
    insta::assert_snapshot!("events-without-transport", stderr(&env, &refused));
    assert_eq!(repo.read("instrukt.yaml"), content);

    let out = env
        .run(&outside, ["config", "get", "--global", "--revision"])
        .success();
    let global = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    env.run(
        &outside,
        [
            "config",
            "patch",
            "--global",
            "--revision",
            global.trim(),
            "--yaml",
            "redis: {cluster: {url: 'redis://127.0.0.1:6379'}}",
        ],
    )
    .success();
    patch(&env, &repo, &read, "modules: {events: true}").success();
    assert_eq!(layer(&repo)["modules"]["events"], Value::from(true));
    // With event responses on, the transport cannot be taken from under them.
    let global_patch = |yaml: &str| {
        let out = env
            .run(&outside, ["config", "get", "--global", "--revision"])
            .success();
        let revision = String::from_utf8(out.get_output().stdout.clone()).unwrap();
        env.run(
            &outside,
            [
                "config",
                "patch",
                "--global",
                "--revision",
                revision.trim(),
                "--yaml",
                yaml,
            ],
        )
    };
    global_patch("modules: {events: true}").success();
    let machine = env.instrukt_home().join("instrukt.yaml");
    let before = std::fs::read_to_string(&machine).unwrap();
    let refused = global_patch("redis: null").failure();
    insta::assert_snapshot!("transport-removed-under-events", stderr(&env, &refused));
    assert_eq!(std::fs::read_to_string(&machine).unwrap(), before);
}

fn stdout(assert: &Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

/// The Slack app manifest `project/instrukt/spec/feature/company/slack-connector`
/// gives in full (`### The Slack app`).
const SPECIFIED_SLACK_MANIFEST: &str = r#"display_information:
  name: agent
  description: The company's agent. Tag it in a channel or message it directly.
features:
  app_home:
    home_tab_enabled: false
    messages_tab_enabled: true
    messages_tab_read_only_enabled: false
  bot_user:
    display_name: agent
    always_online: true
oauth_config:
  scopes:
    bot:
      - app_mentions:read
      - chat:write
      - channels:history
      - channels:read
      - groups:history
      - groups:read
      - im:history
      - im:read
      - users:read
      - users:read.email
settings:
  event_subscriptions:
    bot_events:
      - app_mention
      - message.im
  interactivity:
    is_enabled: false
  org_deploy_enabled: false
  socket_mode_enabled: true
  token_rotation_enabled: false
"#;

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE4
#[test]
fn an_operators_agent_learns_from_the_program_what_a_company_home_can_declare() {
    let env = TestEnv::new();
    let here = env.path("");
    let schema = |path: &[&str]| {
        let mut args = vec!["config", "schema", "--company", "acme"];
        args.extend_from_slice(path);
        env.run(&here, args)
    };

    // The help of the configuration command names the verb.
    insta::assert_snapshot!(
        "config-help",
        stdout(&env.run(&here, ["config", "-h"]).success())
    );

    // Every key of a company home, its connector kinds' setup and its
    // services' fields, with no company home on the computer.
    let whole = stdout(&schema(&[]).success());
    insta::assert_snapshot!("company-home-schema", whole);
    assert!(
        !env.instrukt_home().join("companies/acme").exists(),
        "asking reads no home and creates none"
    );

    // A path shows that key and what lies under it.
    insta::assert_snapshot!(
        "company-connectors-schema",
        stdout(&schema(&["connectors"]).success())
    );
    insta::assert_snapshot!(
        "company-slack-connector-schema",
        stdout(&schema(&["connectors.slack"]).success())
    );

    // The JSON Schema carries the same setup, the specification's manifest in
    // full, and the token fields a Slack entry requires.
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&schema(&["--json", "connectors.slack"]).success())).unwrap();
    let setup = json["description"].as_str().unwrap();
    let (_, manifest) = setup.split_once("\nManifest:\n").unwrap();
    assert_eq!(
        serde_yaml_ng::from_str::<Value>(manifest).unwrap(),
        serde_yaml_ng::from_str::<Value>(SPECIFIED_SLACK_MANIFEST).unwrap()
    );
    assert_eq!(
        json["required"],
        serde_json::json!(["app_token", "bot_token"])
    );
    let whole_json: serde_json::Value =
        serde_json::from_str(&stdout(&schema(&["--json"]).success())).unwrap();
    assert_eq!(
        whole_json["properties"]["connectors"]["properties"]["slack"],
        json
    );

    // What it shows is what validation admits: a Slack entry with exactly the
    // fields shown is accepted, and one the entry cannot take is refused.
    env.run(&here, ["config", "init", "--company", "acme"])
        .success();
    let company_patch = |yaml: &str| {
        let revision = stdout(
            &env.run(&here, ["config", "get", "--company", "acme", "--revision"])
                .success(),
        );
        env.run(
            &here,
            [
                "config",
                "patch",
                "--company",
                "acme",
                "--revision",
                revision.trim(),
                "--yaml",
                yaml,
            ],
        )
    };
    // The token fields' values are written as the schema says they are.
    for name in ["SLACK_APP_TOKEN", "SLACK_BOT_TOKEN"] {
        env.instrukt(&here)
            .args(["secrets", "set", name, "--company", "acme"])
            .write_stdin("token")
            .assert()
            .success();
    }
    company_patch(
        "connectors: {slack: {app_token: '${SLACK_APP_TOKEN}', bot_token: '${SLACK_BOT_TOKEN}', context_messages: 10}}",
    )
    .success();
    let refused =
        company_patch("connectors: {slack: {signing_secret: '${SLACK_SIGNING}'}}").failure();
    insta::assert_snapshot!(
        "company-undeclarable-connector-field",
        stderr(&env, &refused)
    );

    // A key a company home cannot declare names the keys it can declare there.
    let unknown = schema(&["board"]).failure();
    insta::assert_snapshot!("company-schema-undeclarable-key", stderr(&env, &unknown));
}
