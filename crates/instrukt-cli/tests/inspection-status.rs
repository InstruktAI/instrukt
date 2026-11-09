#![allow(clippy::unwrap_used, reason = "integration test setup")]
mod common;
#[path = "support/documented.rs"]
mod documented;
use common::TestEnv;

fn status(env: &TestEnv, dir: &std::path::Path) -> serde_json::Value {
    let result = env.run(dir, ["status", "--json"]).success();
    serde_json::from_slice(&result.get_output().stdout).unwrap()
}
// spec: project/instrukt/spec/feature/publication/instrukt-status UC-IS2
#[test]
fn fresh_status_is_complete_and_writes_nothing() {
    let env = TestEnv::new();
    let repo = env.git_repo("fresh");
    let report = status(&env, repo.path());
    assert_eq!(report["machine"]["configured"], false);
    assert_eq!(report["runtime"]["selected"], serde_json::Value::Null);
    assert_eq!(report["repository"]["adoption"], "not_adopted");
    for module in ["todos", "events", "blueprint"] {
        assert_eq!(report["modules"][module]["enabled"], false);
    }
    assert_eq!(report["instrukt"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(!env.instrukt_home().exists());
    assert!(!env.home().join(".local").exists());
    let outside = status(&env, env.home());
    assert!(outside["repository"].is_null());
}
// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI1
#[test]
fn inspection_converts_old_config_in_memory_without_creating_locks_or_agents() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    let file = env.instrukt_home().join("instrukt.yaml");
    let original =
        "schema_version: 1\nrunner: {runtime: t3}\nagents: {order: [codex], tier: strong}\n";
    std::fs::write(&file, original).unwrap();
    let result = env
        .run(env.home(), ["install", "--check", "--json"])
        .success();
    let report: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert!(!report["notices"].as_array().unwrap().is_empty());
    let report = status(&env, env.home());
    assert_eq!(report["machine"]["schema_version"], 1);
    env.run(env.home(), ["config", "get", "--global", "--json"])
        .success();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    assert!(!env.instrukt_home().join("locks").exists());
    assert!(!env.instrukt_home().join("agents.yaml").exists());
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    let current: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(
        current["schema_version"].as_u64(),
        Some(instrukt_core::config::CURRENT_SCHEMA_VERSION)
    );
}
// spec: project/instrukt/spec/feature/publication/instrukt-status UC-IS3
#[test]
fn status_uses_compiled_capabilities_and_recognizes_native_caller_without_runtime_io() {
    let mut env = TestEnv::new();
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 3\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    env.set_env("T3CODE_THREAD_ID", "caller");
    let report = status(&env, env.home());
    assert_eq!(report["runtime"]["selected"], "t3");
    assert_eq!(report["runtime"]["caller"], "t3:caller");
    assert_eq!(
        report["modules"]["features"]["standing_agents"]["available"],
        true
    );
    assert_eq!(
        report["modules"]["features"]["anomaly_notification"]["available"],
        false
    );
    assert_eq!(
        report["modules"]["features"]["anomaly_notification"]["missing"],
        serde_json::json!(["notification"])
    );
    assert!(!env.instrukt_home().join("agents.yaml").exists());
}
// spec: project/instrukt/spec/feature/publication/instrukt-status UC-IS4
#[test]
fn status_reports_claim_owner_and_keeps_sections_when_configuration_is_broken() {
    let env = TestEnv::new();
    let owner = documented::project(
        &env,
        "owner",
        &[(
            "docs/domain/ideas/concept/idea.md",
            documented::snippet("concept", "Idea", "An idea.", "", &[], "A fact."),
        )],
    );
    documented::sync(&env, &owner);
    let contender = documented::project(
        &env,
        "contender",
        &[(
            "docs/domain/ideas/concept/other.md",
            documented::snippet("concept", "Other", "Another idea.", "", &[], "A fact."),
        )],
    );
    let report = status(&env, contender.path());
    assert_eq!(report["repository"]["positions"][0]["owner"], "owner");
    assert_eq!(report["repository"]["positions"][0]["conflict"], true);
    std::fs::write(env.instrukt_home().join("instrukt.yaml"), "broken: [").unwrap();
    let result = env.run(contender.path(), ["status", "--json"]).failure();
    let report: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert!(!report["machine"]["errors"].as_array().unwrap().is_empty());
    assert_eq!(report["instrukt"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["published"][0]["owner"], "owner");
    assert!(report["modules"]["events"]["enabled"].is_null());
}
// spec: project/instrukt/spec/feature/publication/instrukt-status UC-IS1
#[test]
fn status_reports_adoption_publication_modules_and_other_projects_without_changes() {
    let env = TestEnv::new();
    let first = documented::project(
        &env,
        "first",
        &[(
            "docs/domain/ideas/concept/idea.md",
            documented::snippet("concept", "Idea", "An idea.", "", &[], "A fact."),
        )],
    );
    documented::sync(&env, &first);
    let second = documented::project(
        &env,
        "second",
        &[(
            "docs/domain/reviews/concept/review.md",
            documented::snippet("concept", "Review", "A review.", "", &[], "A fact."),
        )],
    );
    documented::sync(&env, &second);
    env.run(first.path(), ["hooks", "install"]).success();
    first.write("instrukt.yaml","schema_version: 3\nproject: first\nmodules: {todos: true}\ngithub: {repository: example/project}\n");
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 3\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    let report = status(&env, first.path());
    assert_eq!(report["repository"]["adoption"], "adopted");
    assert_eq!(report["repository"]["published_commit"], first.head());
    assert_eq!(report["repository"]["main_ahead"], false);
    assert_eq!(report["modules"]["todos"]["enabled"], true);
    assert_eq!(report["modules"]["todos"]["layer"], "project");
    assert_eq!(report["projects"][0]["project"], "second");
    assert_eq!(report["published"].as_array().unwrap().len(), 2);
    first.write("new", "new");
    first.commit("feat: another change");
    assert_eq!(status(&env, first.path())["repository"]["main_ahead"], true);
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE2
// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF2
#[test]
fn revision_from_old_inspection_is_accepted_by_the_next_atomic_writing_command() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    let file = env.instrukt_home().join("instrukt.yaml");
    let original = "schema_version: 2\nticks: off\n";
    std::fs::write(&file, original).unwrap();
    let revision = env
        .run(env.home(), ["config", "get", "--global", "--revision"])
        .success();
    let revision = String::from_utf8_lossy(&revision.get_output().stdout)
        .trim()
        .to_owned();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    env.run(
        env.home(),
        [
            "config",
            "patch",
            "--global",
            "--revision",
            &revision,
            "--yaml",
            "ticks: on",
        ],
    )
    .success();
    let value: serde_yaml_ng::Value =
        serde_yaml_ng::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(
        value["schema_version"].as_u64(),
        Some(instrukt_core::config::CURRENT_SCHEMA_VERSION)
    );
    assert_eq!(value["ticks"].as_str(), Some("on"));
}
