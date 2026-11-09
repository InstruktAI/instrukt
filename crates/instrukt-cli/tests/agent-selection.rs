#![allow(clippy::unwrap_used, reason = "test setup and assertions")]
mod common;

use common::TestEnv;
use instrukt_core::config::{Agents, Tier};

const CONFIG: &str = r#"
schema_version: 2
default: {class: agent, can: [], order: [personal, work], tier: standard}
roles:
  builder: {class: focused, can: [read], github_app: author, order: [work, personal], tier: strong}
bundles: {read: [{action: docs-read, scope: any}]}
profiles:
  personal:
    instance: codex-personal
    kind: codex
    light: [{model: new-light, effort: high}]
    standard: [{model: new-standard, effort: high}]
    strong: [{model: new-strong, effort: high}]
    premium: [{model: new-premium, effort: high}, {model: new-strong, effort: high}]
  work:
    instance: codex-work
    kind: codex
    light: [{model: work-light, effort: high}]
    standard: [{model: work-standard, effort: high}]
    strong: [{model: work-strong, effort: high}]
    premium: [{model: work-premium, effort: high}, {model: work-strong, effort: high}]
"#;

// spec: project/instrukt/spec/feature/configuration/agent-selection UC-AS1
#[test]
fn role_mapping_falls_back_and_task_inputs_override_independently() {
    let agents = Agents::parse(CONFIG).unwrap();
    let selected = agents.select(Some("builder"), None, None).unwrap();
    assert_eq!(selected[0].profile.as_str(), "work");
    assert_eq!(selected[0].instance.as_str(), "codex-work");
    assert_eq!(selected[0].model, "work-strong");
    assert_eq!(
        agents.select(Some("unknown"), None, None).unwrap()[0].model,
        "new-standard"
    );
    assert_eq!(
        agents
            .select(
                Some("builder"),
                Some(&instrukt_core::ids::ProfileName::parse("personal").unwrap()),
                Some(Tier::Light)
            )
            .unwrap()[0]
            .model,
        "new-light"
    );
}

// spec: project/instrukt/spec/feature/configuration/agent-selection UC-AS3
#[test]
fn open_models_accept_native_options_but_reject_malformed_shapes() {
    assert!(Agents::parse(CONFIG).is_ok());
    assert!(Agents::parse(&CONFIG.replace("effort: high", "effort: invalid")).is_err());
    assert!(Agents::parse(&CONFIG.replace("effort: high", "reasoningEffort: high")).is_err());
    assert!(Agents::parse(&CONFIG.replace("kind: codex", "kind: unknown")).is_err());
    let agents = Agents::parse(CONFIG).unwrap();
    assert_eq!(agents.undeclared_roles(&[]), vec!["builder"]);
}

// spec: project/instrukt/spec/feature/configuration/agent-selection UC-AS4
#[test]
fn defaults_upgrade_replaces_only_the_deployed_unedited_version() {
    use instrukt_core::config::SHIPPED_AGENTS;
    use sha2::{Digest, Sha256};
    let env = TestEnv::new();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    let file = env.instrukt_home().join("agents.yaml");
    let record = env.instrukt_home().join("state/agents-deployment.json");
    let prior = SHIPPED_AGENTS.replace("gpt-6.1-sol", "prior-strong-model");
    let digest = hex::encode(Sha256::digest(prior.as_bytes()));
    let deployed = serde_json::json!({"schema_version":1,"digest":digest});
    std::fs::write(&file, &prior).unwrap();
    std::fs::write(&record, deployed.to_string()).unwrap();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), SHIPPED_AGENTS);
    let edited = format!("{prior}# user customization\n");
    std::fs::write(&file, &edited).unwrap();
    std::fs::write(record, deployed.to_string()).unwrap();
    let result = env
        .run(env.home(), ["config", "init", "--global"])
        .success();
    insta::assert_snapshot!(
        "edited-defaults-upgrade",
        String::from_utf8_lossy(&result.get_output().stdout).replace(
            env.path("").to_str().unwrap().trim_end_matches('/'),
            "[TMP]"
        )
    );
    assert_eq!(std::fs::read_to_string(file).unwrap(), edited);
}

// spec: project/instrukt/spec/feature/configuration/agent-selection UC-AS3
#[test]
fn a_version_one_agents_file_migrates_edited_choices_to_neutral_effort() {
    let env = TestEnv::new();
    let file = env.instrukt_home().join("agents.yaml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let old = CONFIG
        .replace("schema_version: 2", "schema_version: 1")
        .replace("effort: high", "reasoningEffort: high");
    std::fs::write(&file, &old).unwrap();
    let output = env.run(env.home(), ["config", "show", "agents"]).success();
    let current: Agents = Agents::parse(&std::fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(
        current.select(Some("builder"), None, None).unwrap()[0].effort,
        Some(instrukt_core::config::Effort::High)
    );
    insta::assert_json_snapshot!(
        "neutral-effort-migration",
        serde_json::from_slice::<serde_json::Value>(&output.get_output().stdout).unwrap()
    );
}
