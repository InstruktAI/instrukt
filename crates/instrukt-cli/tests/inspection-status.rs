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
    assert_eq!(report["modules"]["history"]["enabled"], true);
    assert_eq!(report["runtime"]["selected"], serde_json::Value::Null);
    insta::assert_json_snapshot!(report["repository"]["adoption"], @r#"
"not_adopted"
"#);
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
        "schema_version: 4\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    env.set_env("T3CODE_THREAD_ID", "caller");
    let report = status(&env, env.home());
    insta::assert_json_snapshot!(report["runtime"]["selected"], @r#"
"t3"
"#);
    insta::assert_json_snapshot!(report["runtime"]["caller"], @r#"
"t3:caller"
"#);
    assert_eq!(
        report["modules"]["features"]["standing_agents"]["available"],
        true
    );
    assert_eq!(
        report["modules"]["features"]["anomaly_notification"]["available"],
        false
    );
    insta::assert_json_snapshot!(report["modules"]["features"]["anomaly_notification"]["missing"], @r#"
[
  "notification"
]
"#);
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
    insta::assert_json_snapshot!(report["repository"]["positions"][0]["owner"], @r#"
"owner"
"#);
    assert_eq!(report["repository"]["positions"][0]["conflict"], true);
    std::fs::write(env.instrukt_home().join("instrukt.yaml"), "broken: [").unwrap();
    let result = env.run(contender.path(), ["status", "--json"]).failure();
    let report: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert!(!report["machine"]["errors"].as_array().unwrap().is_empty());
    assert_eq!(report["instrukt"]["version"], env!("CARGO_PKG_VERSION"));
    insta::assert_json_snapshot!(report["published"][0]["owner"], @r#"
"owner"
"#);
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
    first.write("instrukt.yaml","schema_version: 4\nproject: first\nmodules: {todos: true}\ngithub: {repository: example/project}\n");
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 4\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    let report = status(&env, first.path());
    insta::assert_json_snapshot!(report["repository"]["adoption"], @r#"
"adopted"
"#);
    assert_eq!(report["repository"]["published_commit"], first.head());
    assert_eq!(report["repository"]["main_ahead"], false);
    assert_eq!(report["modules"]["todos"]["enabled"], true);
    insta::assert_json_snapshot!(report["modules"]["todos"]["layer"], @r#"
"project"
"#);
    insta::assert_json_snapshot!(report["projects"][0]["project"], @r#"
"second"
"#);
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
    insta::assert_json_snapshot!(value["ticks"].as_str(), @r#"
"on"
"#);
}

fn filesystem(
    root: &std::path::Path,
) -> std::collections::BTreeMap<std::path::PathBuf, (u32, Option<std::time::SystemTime>, Vec<u8>)> {
    use std::os::unix::fs::PermissionsExt;
    if !root.exists() {
        return Default::default();
    }
    walkdir::WalkDir::new(root)
        .into_iter()
        .map(|entry| {
            let entry = entry.unwrap();
            let metadata = std::fs::symlink_metadata(entry.path()).unwrap();
            let bytes = if metadata.is_file() {
                std::fs::read(entry.path()).unwrap()
            } else if metadata.file_type().is_symlink() {
                std::fs::read_link(entry.path())
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec()
            } else {
                vec![]
            };
            (
                entry.path().strip_prefix(root).unwrap().to_path_buf(),
                (
                    metadata.permissions().mode(),
                    metadata.modified().ok(),
                    bytes,
                ),
            )
        })
        .collect()
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI1
#[test]
fn strict_inspection_creates_no_files_in_home_or_log_destinations() {
    let mut env = TestEnv::new();
    env.set_env("XDG_STATE_HOME", env.path("xdg-state").to_str().unwrap());
    let before = filesystem(&env.path(""));
    env.run(env.home(), ["install", "--check", "--json"])
        .success();
    assert_eq!(before, filesystem(&env.path("")));
    status(&env, env.home());
    assert_eq!(before, filesystem(&env.path("")));
}
// spec: project/instrukt/spec/feature/publication/instrukt-status UC-IS1
#[test]
fn admission_reads_existing_database_and_live_wal_without_creating_or_changing_files() {
    use instrukt_core::{
        home::Home,
        state_db::{StateDb, rusqlite::Connection},
    };
    let mut env = TestEnv::new();
    let home = Home::at_user_home(env.home().into());
    drop(StateDb::open(&home).unwrap());
    std::fs::write(
        home.global_config(),
        "schema_version: 4\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    env.set_env("T3CODE_THREAD_ID", "caller");
    let before = filesystem(env.home());
    status(&env, env.home());
    assert_eq!(before, filesystem(env.home()));
    let writer = Connection::open(home.state_db_file()).unwrap();
    writer
        .execute_batch("PRAGMA journal_mode=WAL;PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    writer.execute("INSERT INTO runtime_bindings(runtime,semantic_key,kind,subject,session,checkout,accepted) VALUES('t3','inspection-live-key','standing','subject','caller',?1,1)",[env.home().to_str().unwrap()]).unwrap();
    let before = filesystem(env.home());
    let result = env.run(env.home(), ["status", "--json"]).failure();
    let report: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    let errors = serde_json::to_string_pretty(&report["runtime"]["errors"])
        .unwrap()
        .replace(env.path("").to_str().unwrap(), "[TMP]");
    insta::assert_snapshot!("live_wal_admission_errors", errors);
    assert_eq!(before, filesystem(env.home()));
}
// spec: project/instrukt/spec/feature/publication/instrukt-status UC-IS1
#[test]
fn foreign_checkout_edits_cannot_change_machine_status_or_its_exit_status() {
    let env = TestEnv::new();
    let foreign = documented::project(&env, "foreign", &[]);
    documented::sync(&env, &foreign);
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 4\nmodules: {todos: true}\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    let first = status(&env, env.home());
    foreign.write("instrukt.yaml", "broken: [");
    let second = status(&env, env.home());
    assert_eq!(first["machine"], second["machine"]);
}

// spec: project/instrukt/spec/feature/publication/instrukt-status UC-IS2
#[test]
fn status_renders_the_configuration_owners_default_value() {
    let env = TestEnv::new();
    let configured = env
        .run(env.home(), ["config", "get", "modules.history", "--json"])
        .success();
    let expected: serde_json::Value =
        serde_json::from_slice(&configured.get_output().stdout).unwrap();
    let report = status(&env, env.home());
    assert_eq!(report["modules"]["history"]["enabled"], expected);
}
