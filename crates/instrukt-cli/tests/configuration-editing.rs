//! `project/instrukt/spec/feature/configuration/configuration-editing`:
//! `instrukt config init|get|patch` — revision checks, whole-document
//! validation and atomic writes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;

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
