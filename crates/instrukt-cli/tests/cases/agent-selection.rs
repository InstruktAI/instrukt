#![allow(clippy::unwrap_used, reason = "test setup and assertions")]

use crate::common;
use common::TestEnv;
use instrukt_core::config::{Agents, Tier};

const CONFIG: &str = r#"
schema_version: 3
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
    // The file names `builder`; every other role it configures through the
    // shipped defaults has an installed procedure here.
    let agents = Agents::parse(CONFIG).unwrap();
    let installed: Vec<&str> = agents
        .roles
        .keys()
        .map(String::as_str)
        .filter(|role| *role != "builder")
        .collect();
    assert_eq!(agents.undeclared_roles(&installed), vec!["builder"]);
}

// spec: project/instrukt/spec/feature/configuration/agent-selection UC-AS4
#[test]
fn a_file_overriding_one_roles_order_leaves_every_other_role_on_its_shipped_selection() {
    use instrukt_core::config::SHIPPED_AGENTS;
    let mut crew = crate::fixtures::workflow::Crew::new();
    std::fs::write(
        crew.project.env.instrukt_home().join("agents.yaml"),
        "schema_version: 3\nroles:\n  planner: {order: [antigravity]}\n",
    )
    .unwrap();
    // Each start is its own todo led from its own session: an outstanding
    // role attempt holds its todo's workspace.
    let mut start = |lead: &str, slug: &str, role: &str, procedure: &str| {
        crew.project.env.set_env("T3CODE_THREAD_ID", lead);
        crew.project
            .create(slug, "change", "Keep one choice, receive the rest.");
        crew.worker(slug, role, Some(procedure))
    };
    let chosen = start("lead-1", "own-choice", "planner", "crew/procedure/plan");
    let untouched = start(
        "lead-2",
        "shipped-choice",
        "plan-reviewer",
        "crew/procedure/review-plan",
    );
    let shipped = Agents::parse(SHIPPED_AGENTS).unwrap();
    let profile = |selection: &serde_json::Value| {
        (
            selection["selection"]["profile"]
                .as_str()
                .unwrap()
                .to_owned(),
            selection["selection"]["model"].as_str().unwrap().to_owned(),
        )
    };
    let planner_tier = shipped.order(Some("planner"), None, None, None).unwrap()[0].1;
    let antigravity = instrukt_core::ids::ProfileName::parse("antigravity").unwrap();
    let own = &shipped.model_chain(&antigravity, planner_tier).unwrap()[0];
    assert_eq!(
        profile(&chosen),
        (own.profile.to_string(), own.model.clone())
    );
    let default = &shipped
        .select(Some("plan-reviewer"), None, None, None)
        .unwrap()[0];
    assert_eq!(
        profile(&untouched),
        (default.profile.to_string(), default.model.clone())
    );
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
        .replace("schema_version: 3", "schema_version: 1")
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
