#![allow(clippy::unwrap_used, reason = "test setup and assertions")]

use crate::common;
use crate::fixtures::event_transport as transport_support;
use common::TestEnv;
use instrukt_core::home::Home;
use std::os::unix::fs::PermissionsExt;

// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF1
#[test]
fn protected_secrets_are_shared_by_commands_and_noninteractive_reads() {
    let mut computer = transport_support::Computer::new();
    let redis = common::redis::RedisServer::new(&computer.env);
    let mut admin = redis.connection();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("instrukt")
        .arg("on")
        .arg(">machine-password")
        .arg("~*")
        .arg("&*")
        .arg("+@all")
        .query::<()>(&mut admin)
        .unwrap();
    computer.config(&redis, None);
    let home = Home::at_user_home(computer.env.home().into());
    let mut config: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(home.global_config()).unwrap()).unwrap();
    config["redis"]["cluster"]["username"] = "${REDIS_USER}".into();
    config["redis"]["cluster"]["password"] = "${REDIS_PASSWORD}".into();
    std::fs::write(
        home.global_config(),
        serde_yaml_ng::to_string(&config).unwrap(),
    )
    .unwrap();
    for (name, value) in [
        ("REDIS_USER", "instrukt"),
        ("REDIS_PASSWORD", "machine-password"),
    ] {
        computer
            .env
            .instrukt(computer.env.home())
            .args(["secrets", "set", name])
            .write_stdin(value)
            .assert()
            .success()
            .stdout("");
    }
    computer
        .env
        .set_env("REDIS_PASSWORD", "environment-must-not-win");
    computer
        .project
        .write(".env", "REDIS_PASSWORD=project-must-not-win\n");
    computer.tick();
    computer.change();
    computer.tick();
    assert_eq!(
        redis::cmd("XLEN")
            .arg(instrukt_runner::transport::EVENT_STREAM)
            .query::<u64>(&mut admin)
            .unwrap(),
        1
    );
    computer.env.remove_env("REDIS_PASSWORD");
    computer.change();
    computer.tick();
    assert_eq!(
        redis::cmd("XLEN")
            .arg(instrukt_runner::transport::EVENT_STREAM)
            .query::<u64>(&mut admin)
            .unwrap(),
        2
    );
    assert_eq!(
        std::fs::metadata(home.secrets_file())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    std::fs::set_permissions(home.secrets_file(), std::fs::Permissions::from_mode(0o644)).unwrap();
    computer.env.run(computer.env.home(), ["tick"]).failure();
}

// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
#[test]
fn ordinary_reads_convert_in_memory_and_writing_commands_preserve_supported_values() {
    let env = TestEnv::new();
    let home = Home::at_user_home(env.home().into());
    std::fs::create_dir_all(home.root()).unwrap();
    std::fs::write(home.global_config(), "schema_version: 1\nticks: off\n").unwrap();
    env.run(env.home(), ["config", "get", "ticks", "--global"])
        .success();
    assert_eq!(
        std::fs::read_to_string(home.global_config()).unwrap(),
        "schema_version: 1\nticks: off\n"
    );
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    let stored: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(home.global_config()).unwrap()).unwrap();
    assert_eq!(
        stored["schema_version"].as_u64(),
        Some(instrukt_core::config::CURRENT_SCHEMA_VERSION)
    );
    assert_eq!(stored["ticks"].as_str(), Some("off"));
}

// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
#[test]
fn sync_migrates_both_layers_even_when_its_published_commit_is_unchanged() {
    let env = TestEnv::new();
    let repo = env.git_repo("migrating");
    let old = "schema_version: 1\nproject: migrating\n";
    repo.write("instrukt.yaml", old)
        .write("docs/project/index.yaml", "expanded: []\n");
    repo.commit("adopt");
    env.run(repo.path(), ["sync"]).success();
    repo.write("instrukt.yaml", old);
    let global = env.instrukt_home().join("instrukt.yaml");
    std::fs::write(&global, "schema_version: 1\nticks: off\n").unwrap();
    env.run(repo.path(), ["sync"]).success();
    let project: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&repo.read("instrukt.yaml")).unwrap();
    let global: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(global).unwrap()).unwrap();
    assert_eq!(
        project["schema_version"].as_u64(),
        Some(instrukt_core::config::CURRENT_SCHEMA_VERSION)
    );
    assert_eq!(
        global["schema_version"].as_u64(),
        Some(instrukt_core::config::CURRENT_SCHEMA_VERSION)
    );
}

// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
#[test]
fn failed_selection_conversion_preserves_the_old_configuration() {
    let env = TestEnv::new();
    let file = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let original = "schema_version: 1\nagents: {order: [missing], tier: strong}\n";
    std::fs::write(&file, original).unwrap();
    env.run(env.home(), ["config", "get", "ticks", "--global"])
        .failure();
    assert_eq!(std::fs::read_to_string(file).unwrap(), original);
    assert!(!env.instrukt_home().join("agents.yaml").exists());
}

// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF3
#[test]
fn the_development_script_seeds_each_worktrees_own_home_without_changing_the_machine() {
    use assert_cmd::assert::OutputAssertExt;
    let mut env = TestEnv::new();
    let repo = env.git_repo("development");
    repo.write("instrukt.yaml","schema_version: 2\nproject: development\n")
        .write("docs/general/index.yaml","expanded: []\n")
        .write("docs/general/concept/sample.md","---\ndescription: Development source.\n---\n# Sample — Concept\n\n## What\n\nFirst source.\n\n## Why\n\nIsolation.\n")
        .write("scripts/dev-instrukt",include_str!("../../../../scripts/dev-instrukt"))
        .write("scripts/with-build-lock.sh",include_str!("../../../../scripts/with-build-lock.sh"));
    repo.write(".gitignore", "/.instrukt-home/\n");
    for name in ["scripts/dev-instrukt", "scripts/with-build-lock.sh"] {
        std::fs::set_permissions(
            repo.path().join(name),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    repo.commit("first development source");
    let two = env.path("development-two");
    repo.git([
        "worktree",
        "add",
        "--quiet",
        "-b",
        "second",
        two.to_str().unwrap(),
    ]);
    env.git(&two, ["config", "user.name", "Test Developer"]);
    env.git(&two, ["config", "user.email", "developer@example.test"]);
    std::fs::write(two.join("docs/general/concept/sample.md"),"---\ndescription: Development source.\n---\n# Sample — Concept\n\n## What\n\nSecond source.\n\n## Why\n\nIsolation.\n").unwrap();
    env.git(&two, ["add", "docs/general/concept/sample.md"]);
    env.git(
        &two,
        ["commit", "--quiet", "-m", "second development source"],
    );
    let cargo = env.path("runtime-bin/cargo");
    std::fs::write(&cargo,"#!/usr/bin/env python3\nimport os,pathlib,shutil,sys\nif sys.argv[1]=='build':\n p=pathlib.Path(os.environ['CARGO_TARGET_DIR'])/'debug/instrukt'; p.parent.mkdir(parents=True,exist_ok=True); shutil.copy2(os.environ['TEST_INSTRUKT_BINARY'],p)\n").unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    env.set_env("TEST_INSTRUKT_BINARY", env!("CARGO_BIN_EXE_instrukt"));
    env.set_env(
        "CARGO_TARGET_DIR",
        env.path("build-target").to_str().unwrap(),
    );
    let normal = env.instrukt_home();
    std::fs::create_dir_all(&normal).unwrap();
    std::fs::write(
        normal.join("instrukt.yaml"),
        "schema_version: 2\nticks: off\n",
    )
    .unwrap();
    env.instrukt(env.home())
        .args(["secrets", "set", "SHARED"])
        .write_stdin("machine-only")
        .assert()
        .success();
    let machine_config = std::fs::read(normal.join("instrukt.yaml")).unwrap();
    for root in [repo.path(), two.as_path()] {
        env.command(&root.join("scripts/dev-instrukt"), root)
            .args(["config", "get", "ticks", "--global"])
            .output()
            .unwrap()
            .assert()
            .success();
        assert!(root.join(".instrukt-home/state/index.yaml").exists());
        assert!(root.join(".instrukt-home/bin/instrukt").exists());
        assert!(!root.join(".instrukt-home/secrets.env").exists());
    }
    assert_ne!(
        std::fs::read(
            repo.path()
                .join(".instrukt-home/docs/general/concept/sample.md")
        )
        .unwrap(),
        std::fs::read(two.join(".instrukt-home/docs/general/concept/sample.md")).unwrap()
    );
    let private_a = repo.path().join(".instrukt-home");
    let private_b = two.join(".instrukt-home");
    let a_before = std::fs::read(private_a.join("docs/general/concept/sample.md")).unwrap();
    let b_before = std::fs::read(private_b.join("docs/general/concept/sample.md")).unwrap();
    let b_index = std::fs::read(private_b.join("state/index.yaml")).unwrap();
    repo.write("docs/general/concept/sample.md","---\ndescription: Updated development source.\n---\n# Sample — Concept\n\n## What\n\nUpdated source.\n\n## Why\n\nIsolation.\n");
    repo.commit("update the first worktree source after seeding");
    env.command(&repo.path().join("scripts/dev-instrukt"), repo.path())
        .args(["sync"])
        .output()
        .unwrap()
        .assert()
        .success();
    let a_after = std::fs::read(private_a.join("docs/general/concept/sample.md")).unwrap();
    assert_ne!(a_after, a_before);
    assert_eq!(
        a_after,
        std::fs::read(repo.path().join("docs/general/concept/sample.md")).unwrap()
    );
    assert_eq!(
        std::fs::read(private_b.join("docs/general/concept/sample.md")).unwrap(),
        b_before
    );
    assert_eq!(
        std::fs::read(private_b.join("state/index.yaml")).unwrap(),
        b_index
    );
    assert_eq!(
        std::fs::read(normal.join("instrukt.yaml")).unwrap(),
        machine_config
    );
    assert!(!normal.join("state/index.yaml").exists());
    let revision = env
        .command(&repo.path().join("scripts/dev-instrukt"), repo.path())
        .args(["config", "get", "--revision", "--global"])
        .output()
        .unwrap();
    assert!(revision.status.success());
    let revision = String::from_utf8(revision.stdout).unwrap();
    env.command(&repo.path().join("scripts/dev-instrukt"), repo.path())
        .args([
            "config",
            "patch",
            "--global",
            "--revision",
            revision.trim(),
            "--yaml",
            "ticks: off",
        ])
        .output()
        .unwrap()
        .assert()
        .success();
    let edited = std::fs::read(private_a.join("instrukt.yaml")).unwrap();
    env.command(&repo.path().join("scripts/dev-instrukt"), repo.path())
        .args(["config", "get", "ticks", "--global"])
        .output()
        .unwrap()
        .assert()
        .success();
    assert_eq!(
        std::fs::read(private_a.join("instrukt.yaml")).unwrap(),
        edited
    );
    let second = std::fs::read(two.join(".instrukt-home/instrukt.yaml")).unwrap();
    env.command(&repo.path().join("scripts/dev-instrukt"), repo.path())
        .args(["secrets", "generate", "GENERATED"])
        .output()
        .unwrap()
        .assert()
        .success();
    assert_eq!(
        std::fs::read(two.join(".instrukt-home/instrukt.yaml")).unwrap(),
        second
    );
    assert_eq!(
        std::fs::read(normal.join("instrukt.yaml")).unwrap(),
        machine_config
    );
    assert!(!normal.join("state/index.yaml").exists());
    assert!(normal.join("secrets.env").exists());
}

// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL3
#[test]
fn retired_project_actions_convert_for_inspection_and_persist_before_an_unchanged_sync() {
    let env = TestEnv::new();
    let repo = env.git_repo("retired-actions");
    let old = "# yaml-language-server: $schema=.instrukt/schemas/instrukt.schema.json\nschema_version: 2\nproject: retired-actions\nactions: {test: make test}\napprovals: {prd: peer}\n";
    repo.write("instrukt.yaml", old);
    repo.commit("old project configuration");
    let read = env
        .run(repo.path(), ["config", "get", "approvals.prd"])
        .success();
    assert_eq!(repo.read("instrukt.yaml"), old);
    insta::assert_snapshot!(String::from_utf8_lossy(&read.get_output().stdout), @"peer");
    env.run(repo.path(), ["sync"]).success();
    repo.write("instrukt.yaml", old);
    env.run(repo.path(), ["sync"]).success();
    let stored: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&repo.read("instrukt.yaml")).unwrap();
    assert!(stored.get("actions").is_none());
    let after_sync = env
        .run(repo.path(), ["config", "get", "approvals.prd"])
        .success();
    insta::assert_snapshot!(String::from_utf8_lossy(&after_sync.get_output().stdout), @"peer");
}

fn history_migration_setup(global: &str, project: &str) -> (TestEnv, common::Repo) {
    let env = TestEnv::new();
    let repo = env.git_repo("history-migration");
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    std::fs::write(env.instrukt_home().join("instrukt.yaml"), global).unwrap();
    repo.write("instrukt.yaml", project);
    (env, repo)
}
fn effective_history(env: &TestEnv, repo: &common::Repo) -> serde_json::Value {
    let result = env
        .run(repo.path(), ["config", "get", "modules.history", "--json"])
        .success();
    serde_json::from_slice(&result.get_output().stdout).unwrap()
}
// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL1
#[test]
fn history_migration_preserves_inherited_global_false_without_a_project_override() {
    let (env, repo) = history_migration_setup(
        "schema_version: 3\nmodules: {history: false}\n",
        "schema_version: 3\nproject: history-migration\n",
    );
    assert_eq!(effective_history(&env, &repo), false);
    env.run(repo.path(), ["config", "init"]).success();
    assert_eq!(effective_history(&env, &repo), false);
    let migrated: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&repo.read("instrukt.yaml")).unwrap();
    assert_eq!(
        migrated["schema_version"].as_u64(),
        Some(instrukt_core::config::CURRENT_SCHEMA_VERSION)
    );
    assert!(migrated.get("modules").is_none());
}
// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL1
#[test]
fn history_migration_preserves_an_explicit_project_override() {
    let (env, repo) = history_migration_setup(
        "schema_version: 3\nmodules: {history: false}\n",
        "schema_version: 3\nproject: history-migration\nmodules: {history: true}\n",
    );
    env.run(repo.path(), ["config", "init"]).success();
    assert_eq!(effective_history(&env, &repo), true);
}
// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL1
#[test]
fn history_migration_keeps_the_default_when_neither_layer_sets_a_value() {
    let (env, repo) = history_migration_setup(
        "schema_version: 3\n",
        "schema_version: 3\nproject: history-migration\n",
    );
    env.run(repo.path(), ["config", "init"]).success();
    assert_eq!(effective_history(&env, &repo), true);
}
