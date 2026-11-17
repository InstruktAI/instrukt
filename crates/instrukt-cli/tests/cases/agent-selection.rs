#![allow(clippy::unwrap_used, reason = "test setup and assertions")]

use crate::common;
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
    let selected = agents.select(Some("builder"), None, None, None).unwrap();
    assert_eq!(selected[0].profile.as_str(), "work");
    assert_eq!(selected[0].instance.as_str(), "codex-work");
    assert_eq!(selected[0].model, "work-strong");
    assert_eq!(
        agents.select(Some("unknown"), None, None, None).unwrap()[0].model,
        "new-standard"
    );
    assert_eq!(
        agents
            .select(
                Some("builder"),
                Some(&instrukt_core::ids::ProfileName::parse("personal").unwrap()),
                Some(Tier::Light),
                None
            )
            .unwrap()[0]
            .model,
        "new-light"
    );
    // A session in a company home that names `profiles` walks that list in
    // place of its role's order; the tier is still the role's, and a task
    // override still comes first.
    let name = |name: &str| instrukt_core::ids::ProfileName::parse(name).unwrap();
    let company = [name("personal")];
    assert_eq!(
        agents
            .order(Some("builder"), None, None, Some(&company))
            .unwrap(),
        [(name("personal"), Tier::Strong)]
    );
    assert_eq!(
        agents
            .order(Some("builder"), Some(&name("work")), None, Some(&company))
            .unwrap(),
        [
            (name("work"), Tier::Strong),
            (name("personal"), Tier::Strong)
        ]
    );
    // The machine's agents.yaml declares every profile a company names, and
    // a company that names `profiles` names at least one.
    let env = TestEnv::new();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    env.run(env.home(), ["config", "init", "--company", "acme"])
        .success();
    for (case, profiles) in [("undeclared", "[undeclared-profile]"), ("empty", "[]")] {
        let revision = env
            .run(
                env.home(),
                ["config", "get", "--company", "acme", "--revision"],
            )
            .success();
        let revision = String::from_utf8_lossy(&revision.get_output().stdout)
            .trim()
            .to_owned();
        let refused = env
            .run(
                env.home(),
                [
                    "config",
                    "patch",
                    "--company",
                    "acme",
                    "--revision",
                    &revision,
                    "--yaml",
                    &format!("profiles: {profiles}"),
                ],
            )
            .failure();
        insta::assert_snapshot!(
            format!("company-profiles-{case}"),
            String::from_utf8_lossy(&refused.get_output().stderr).replace(
                env.path("").to_str().unwrap().trim_end_matches('/'),
                "[TMP]"
            )
        );
    }
    // With profiles named, a catalogue that cannot be read leaves them
    // unchecked: that is the company's finding.
    crate::fixtures::conversing::patch_company(&env, "acme", "profiles: [claude]");
    std::fs::write(env.instrukt_home().join("agents.yaml"), "default: [").unwrap();
    let unchecked = env
        .run(env.home(), ["config", "validate", "--company", "acme"])
        .failure();
    insta::assert_snapshot!(
        "company-profiles-unreadable-catalogue",
        String::from_utf8_lossy(&unchecked.get_output().stdout).replace(
            env.path("").to_str().unwrap().trim_end_matches('/'),
            "[TMP]"
        )
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
fn a_version_one_agents_file_is_inspected_in_memory_and_converted_on_the_next_write() {
    let env = TestEnv::new();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    let file = env.instrukt_home().join("agents.yaml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let old = CONFIG
        .replace("schema_version: 2", "schema_version: 1")
        .replace("effort: high", "reasoningEffort: high");
    std::fs::write(&file, &old).unwrap();
    let output = env.run(env.home(), ["config", "show", "agents"]).success();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), old);
    let revision = env
        .run(env.home(), ["config", "get", "--global", "--revision"])
        .success();
    let revision = String::from_utf8_lossy(&revision.get_output().stdout)
        .trim()
        .to_owned();
    env.run(
        env.home(),
        [
            "config",
            "patch",
            "--global",
            "--revision",
            &revision,
            "--yaml",
            "ticks: off",
        ],
    )
    .success();
    let current: Agents = Agents::parse(&std::fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(
        current.select(Some("builder"), None, None, None).unwrap()[0].effort,
        Some(instrukt_core::config::Effort::High)
    );
    insta::assert_json_snapshot!(
        "neutral-effort-migration",
        serde_json::from_slice::<serde_json::Value>(&output.get_output().stdout).unwrap()
    );
}
