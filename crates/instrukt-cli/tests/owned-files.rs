#![allow(clippy::unwrap_used, reason = "test setup and assertions")]
mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/event_transport.rs"]
mod transport_support;

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
fn ordinary_reads_migrate_old_configuration_and_preserve_supported_values() {
    let env = TestEnv::new();
    let home = Home::at_user_home(env.home().into());
    std::fs::create_dir_all(home.root()).unwrap();
    std::fs::write(home.global_config(), "schema_version: 1\nticks: off\n").unwrap();
    env.run(env.home(), ["config", "get", "ticks", "--global"])
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
fn commands_use_an_explicit_isolated_home_and_the_normal_machine_secret_file() {
    let mut env = TestEnv::new();
    env.instrukt(env.home())
        .args(["secrets", "set", "SHARED"])
        .write_stdin("protected")
        .assert()
        .success();
    let one = env.path("worktree-one/.instrukt/home");
    let two = env.path("worktree-two/.instrukt/home");
    env.set_env("INSTRUKT_HOME", one.to_str().unwrap());
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    env.set_env("INSTRUKT_HOME", two.to_str().unwrap());
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    assert!(one.join("instrukt.yaml").exists());
    assert!(two.join("instrukt.yaml").exists());
    assert!(!env.instrukt_home().join("instrukt.yaml").exists());
    env.instrukt(env.home())
        .args(["secrets", "set", "OTHER"])
        .write_stdin("also-protected")
        .assert()
        .success();
    assert!(env.instrukt_home().join("secrets.env").exists());
    assert!(!one.join("secrets.env").exists());
    assert!(!two.join("secrets.env").exists());
}
