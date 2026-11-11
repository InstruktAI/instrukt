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

/// A test machine whose `PATH` holds the built `instrukt`, as an installed
/// one would: the hook sections call it by name.
fn machine() -> TestEnv {
    let mut env = TestEnv::new();
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap();
    let path = format!("{}:{}", bin.display(), env.injected("PATH").unwrap());
    env.set_env("PATH", &path);
    env
}

fn schema_version(text: &str) -> Option<u64> {
    serde_yaml_ng::from_str::<serde_yaml_ng::Value>(text).unwrap()["schema_version"].as_u64()
}

// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
#[test]
fn an_upgrade_migrates_the_machine_layer_and_the_next_commit_carries_the_project_layer() {
    let env = machine();
    let repo = env.git_repo("migrating");
    let project = "# yaml-language-server: $schema=.instrukt/schemas/instrukt.schema.json\nschema_version: 2\nproject: migrating\nactions: {test: make test}\napprovals: {prd: peer}\n";
    repo.write("instrukt.yaml", project)
        .write("docs/project/index.yaml", "expanded: []\n");
    repo.commit("adopt");
    env.run(repo.path(), ["sync"]).success();
    let global = env.instrukt_home().join("instrukt.yaml");
    let old_global = "schema_version: 1\nticks: off\n";
    std::fs::write(&global, old_global).unwrap();
    let current = Some(instrukt_core::config::CURRENT_SCHEMA_VERSION);

    // Sync on an unchanged main: the machine layer is written current, the
    // committed project layer stays as committed.
    env.run(repo.path(), ["sync"]).success();
    let machine = std::fs::read_to_string(&global).unwrap();
    assert_eq!(schema_version(&machine), current);
    let kept: serde_yaml_ng::Value = serde_yaml_ng::from_str(&machine).unwrap();
    insta::assert_snapshot!(kept["ticks"].as_str().unwrap(), @"off");
    assert_eq!(repo.read("instrukt.yaml"), project);

    // An ordinary read inspects the converted project layer, writes nothing
    // and reports the pending migration.
    let read = env
        .run(repo.path(), ["config", "get", "approvals.prd"])
        .success();
    insta::assert_snapshot!(String::from_utf8_lossy(&read.get_output().stdout), @"peer");
    let notice = String::from_utf8_lossy(&read.get_output().stderr)
        .replace(&repo.path().display().to_string(), "[REPO]");
    insta::assert_snapshot!(notice, @"instrukt: [REPO]/instrukt.yaml: schema_version 2 inspected as 5; file migrates on the next writing command");
    assert_eq!(repo.read("instrukt.yaml"), project);

    // A tick persists the machine migration before its refusal.
    std::fs::write(&global, old_global).unwrap();
    env.run(repo.path(), ["tick"]).failure();
    assert_eq!(
        schema_version(&std::fs::read_to_string(&global).unwrap()),
        current
    );
    assert_eq!(repo.read("instrukt.yaml"), project);

    // A conversion that fails keeps its source.
    let current_global = std::fs::read_to_string(&global).unwrap();
    let unconvertible = "schema_version: 1\nagents: {order: [missing], tier: strong}\n";
    std::fs::write(&global, unconvertible).unwrap();
    env.run(repo.path(), ["config", "get", "ticks", "--global"])
        .failure();
    assert_eq!(std::fs::read_to_string(&global).unwrap(), unconvertible);
    std::fs::write(&global, current_global).unwrap();

    // The developer's next commit carries the project layer at the current
    // schema and leaves the checkout clean.
    env.run(repo.path(), ["hooks", "install"]).success();
    repo.write("notes.md", "a note\n");
    repo.commit("note");
    let committed = repo.git(["show", "HEAD:instrukt.yaml"]);
    assert_eq!(schema_version(&committed), current);
    let committed: serde_yaml_ng::Value = serde_yaml_ng::from_str(&committed).unwrap();
    assert!(committed.get("actions").is_none());
    insta::assert_snapshot!(committed["approvals"]["prd"].as_str().unwrap(), @"peer");
    assert_eq!(
        repo.git(["status", "--porcelain", "--", "instrukt.yaml"]),
        ""
    );
    let record = env
        .instrukt_home()
        .join("state/hooks/migrating/post-commit.yaml");
    common::wait_until(
        "the post-commit sync",
        std::time::Duration::from_secs(60),
        || {
            std::fs::read_to_string(&record).is_ok_and(|text| {
                let record: instrukt_core::repository::HookRunRecord =
                    serde_yaml_ng::from_str(&text).unwrap();
                record.sync != Some(instrukt_core::repository::SyncOutcome::Running)
            })
        },
    );
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
