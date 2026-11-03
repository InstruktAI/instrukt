//! `local/projects/instrukt/spec/feature/configuration/configuration-editing`:
//! `instrukt config init|get|patch` — revision checks, whole-document
//! validation and atomic writes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;

use std::path::Path;

use common::{Repo, TestEnv};
use predicates::str::contains;

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

fn revision(env: &TestEnv, cwd: &Path, global: bool) -> String {
    let mut args = vec!["config", "get", "--revision"];
    if global {
        args.push("--global");
    }
    stdout(&env.run(cwd, &args).success()).trim().to_owned()
}

fn initialized(env: &TestEnv, name: &str) -> Repo {
    let repo = env.git_repo(name);
    env.run(repo.path(), ["config", "init"]).success();
    repo
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn a_patch_from_the_current_revision_changes_only_what_it_names() {
    let env = TestEnv::new();
    let repo = initialized(&env, "demo");
    let before = revision(&env, repo.path(), false);
    assert_eq!(
        revision(&env, repo.path(), false),
        before,
        "stable while unchanged"
    );

    env.run(
        repo.path(),
        [
            "config",
            "patch",
            "--revision",
            &before,
            "--yaml",
            "board: true\nactions:\n  test: make test {runtime.session_id}\n",
        ],
    )
    .success();

    let after = revision(&env, repo.path(), false);
    assert_ne!(after, before);
    let layer: serde_yaml_ng::Value = serde_yaml_ng::from_str(&repo.read("instrukt.yaml")).unwrap();
    assert_eq!(layer["project"], "demo");
    assert_eq!(layer["schema_version"], 1);
    assert_eq!(layer["board"], true);
    assert_eq!(
        layer["actions"]["test"], "make test {runtime.session_id}",
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
    let third = revision(&env, repo.path(), false);
    env.instrukt(repo.path())
        .args(["config", "patch", "--revision", &third])
        .write_stdin("approvals:\n  prd: peer\n")
        .assert()
        .success();
    env.run(repo.path(), ["config", "get", "approvals.prd"])
        .success()
        .stdout("peer\n");
    env.run(repo.path(), ["config", "get", "board"])
        .success()
        .stdout("false\n");
    assert!(!repo.read("instrukt.yaml").contains("board"));
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn a_patch_from_a_stale_revision_is_refused_and_writes_nothing() {
    let env = TestEnv::new();
    let repo = initialized(&env, "demo");
    let read = revision(&env, repo.path(), false);
    env.run(
        repo.path(),
        [
            "config",
            "patch",
            "--revision",
            &read,
            "--yaml",
            "board: true",
        ],
    )
    .success();
    let current = revision(&env, repo.path(), false);
    let content = repo.read("instrukt.yaml");

    env.run(
        repo.path(),
        [
            "config",
            "patch",
            "--revision",
            &read,
            "--yaml",
            "board: false",
        ],
    )
    .failure()
    .stderr(contains(format!("revision {read}")))
    .stderr(contains(format!("current revision {current}")))
    .stderr(contains("read it again"));
    assert_eq!(repo.read("instrukt.yaml"), content);
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn of_two_patches_from_one_revision_exactly_one_lands() {
    let env = TestEnv::new();
    let repo = initialized(&env, "demo");
    let read = revision(&env, repo.path(), false);
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
                            &format!("actions:\n  writer{i}: echo {i}\n"),
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
    let content = repo.read("instrukt.yaml");
    assert_eq!(content.matches("writer").count(), 1, "{content}");
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn a_patch_leaving_the_document_invalid_is_refused_naming_the_finding() {
    let env = TestEnv::new();
    let repo = initialized(&env, "demo");
    let read = revision(&env, repo.path(), false);
    let content = repo.read("instrukt.yaml");

    env.run(
        repo.path(),
        [
            "config",
            "patch",
            "--revision",
            &read,
            "--yaml",
            "runtimes: []\nboard: maybe\n",
        ],
    )
    .failure()
    .stderr(contains("nothing was written"))
    .stderr(contains("runtimes: not permitted in the project layer"))
    .stderr(contains("board: expected true or false"));
    env.run(
        repo.path(),
        ["config", "patch", "--revision", &read, "--yaml", "- a list"],
    )
    .failure()
    .stderr(contains("not a YAML mapping"));
    assert_eq!(repo.read("instrukt.yaml"), content);
    assert_eq!(revision(&env, repo.path(), false), read);
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn a_patch_needs_an_existing_layer_and_a_project() {
    let env = TestEnv::new();
    let repo = env.git_repo("bare");
    env.run(
        repo.path(),
        [
            "config",
            "patch",
            "--revision",
            "r",
            "--yaml",
            "board: true",
        ],
    )
    .failure()
    .stderr(contains("instrukt config init"));
    env.run(
        env.path("").as_path(),
        [
            "config",
            "patch",
            "--revision",
            "r",
            "--yaml",
            "board: true",
        ],
    )
    .failure()
    .stderr(contains("not inside a git repository"));
    assert!(!repo.path().join("instrukt.yaml").exists());
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE2
#[test]
fn initialization_creates_a_valid_machine_and_project_configuration() {
    let env = TestEnv::new();
    let outside = env.path("");
    env.run(&outside, ["config", "init", "--global"]).success();
    let global = std::fs::read_to_string(env.instrukt_home().join("instrukt.yaml")).unwrap();
    assert_eq!(
        global, "schema_version: 1\n",
        "a machine needs no project identity"
    );
    env.run(&outside, ["config", "validate", "--global"])
        .success();

    let repo = env.git_repo("My Project_v2");
    env.run(repo.path(), ["config", "init"]).success();
    assert_eq!(
        repo.read("instrukt.yaml"),
        "schema_version: 1\nproject: my-project-v2\n"
    );
    env.run(repo.path(), ["config", "validate"]).success();
    env.run(repo.path(), ["config", "get", "workflow.domain"])
        .success()
        .stdout("sdlc\n");
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE2
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
        "schema_version: 1\nproject: primary-checkout\n"
    );
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE2
#[test]
fn an_empty_project_identity_is_refused() {
    let env = TestEnv::new();
    let repo = env.git_repo("___");
    env.run(repo.path(), ["config", "init"])
        .failure()
        .stderr(contains("instrukt init --slug <slug>"));
    assert!(!repo.path().join("instrukt.yaml").exists());
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE2
#[test]
fn existing_configuration_is_kept_and_a_newer_schema_is_refused() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    let layer = "# my notes\nschema_version: 1\nproject: chosen\nboard: true\n";
    repo.write("instrukt.yaml", layer);
    env.run(repo.path(), ["config", "init"]).success();
    assert_eq!(repo.read("instrukt.yaml"), layer);

    let newer = env.git_repo("newer");
    newer.write("instrukt.yaml", "schema_version: 99\nproject: newer\n");
    env.run(newer.path(), ["config", "init"])
        .failure()
        .stderr(contains("upgrade instrukt"));
    assert_eq!(
        newer.read("instrukt.yaml"),
        "schema_version: 99\nproject: newer\n"
    );
}

// spec: local/projects/instrukt/spec/feature/configuration/configuration-editing UC-CE1
#[test]
fn get_prints_scalars_bare_structures_as_yaml_or_json_and_names_a_missing_segment() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: demo\ngithub:\n  approvers: [ann, bob]\n",
    );
    env.run(repo.path(), ["config", "get", "project"])
        .success()
        .stdout("demo\n");
    env.run(repo.path(), ["config", "get", "github.approvers"])
        .success()
        .stdout("- ann\n- bob\n");
    let json = stdout(&env.run(repo.path(), ["config", "get", "--json"]).success());
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["github"]["approvers"][1], "bob");
    assert_eq!(parsed["github"]["api_url"], "https://api.github.com");
    env.run(repo.path(), ["config", "get", "github.owner.name"])
        .failure()
        .stderr(contains("no `owner` in `github`"));
}
