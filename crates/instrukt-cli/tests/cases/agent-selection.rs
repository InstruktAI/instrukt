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
    use crate::common::redis::RedisServer;
    use crate::fixtures::{documented, event_transport::Computer, runtime::FakeT3};
    use instrukt_core::config::SHIPPED_AGENTS;
    let computer = Computer::new();
    let runtime = FakeT3::create(&computer.env);
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    std::fs::write(
        computer.env.instrukt_home().join("agents.yaml"),
        "schema_version: 3\nroles:\n  planner: {order: [antigravity]}\n",
    )
    .unwrap();
    // Work for each role: a procedure of that role answering its own fact.
    for (name, role, fact) in [
        ("own-choice", "planner", "example.overridden"),
        ("shipped-choice", "plan-reviewer", "example.shipped"),
    ] {
        computer.project.write(
            &format!("docs/domain/reactions/procedure/{name}.md"),
            &documented::snippet(
                "procedure",
                name,
                "Answer a fact.",
                &format!("role: {role}\nsubscription_home: project\nsubscriptions: [{fact}]\n"),
                &[],
                "Inspect the current fact.\n",
            ),
        );
    }
    computer.project.commit("docs: publish work for two roles");
    documented::sync(&computer.env, &computer.project);
    computer.tick();
    let shipped = Agents::parse(SHIPPED_AGENTS).unwrap();
    let selected = |role: &str, first: Option<&str>| {
        let (profile, tier) = shipped.order(Some(role), None, None, None).unwrap()[0].clone();
        let profile = first.map_or(profile, |name| {
            instrukt_core::ids::ProfileName::parse(name).unwrap()
        });
        let model = shipped.model_chain(&profile, tier).unwrap().remove(0);
        (model.instance.to_string(), model.model)
    };
    // One fact at a time: the thread it starts belongs to the role answering
    // that fact, recognised by arriving after the threads already running.
    let answer = |fact: &str, running: &[String]| {
        let event = serde_json::json!({"id":fact,"type":fact,"source":computer.computer(),"subject":"todo:sample","home":"project:fork","home_owner":"unresolved","time":"2026-11-05T00:00:00Z","data":{}});
        redis::cmd("XADD")
            .arg(instrukt_runner::transport::EVENT_STREAM)
            .arg("*")
            .arg("event")
            .arg(event.to_string())
            .query::<String>(&mut redis.connection())
            .unwrap();
        computer.tick();
        let started: Vec<String> = runtime
            .threads()
            .into_iter()
            .filter(|id| !running.contains(id))
            .collect();
        assert_eq!(started.len(), 1);
        started[0].clone()
    };
    let own = answer("example.overridden", &[]);
    assert_eq!(
        (runtime.provider(&own).to_string(), runtime.model(&own)),
        selected("planner", Some("antigravity"))
    );
    let kept = answer("example.shipped", std::slice::from_ref(&own));
    assert_eq!(
        (runtime.provider(&kept).to_string(), runtime.model(&kept)),
        selected("plan-reviewer", None)
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
    // The entries the file names, as the effective selection shows them;
    // what the shipped defaults add beside them is theirs, not this
    // conversion's.
    let shown: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    let named: serde_yaml_ng::Value = serde_yaml_ng::from_str(&old).unwrap();
    let mut own = serde_json::Map::new();
    for (section, entries) in named.as_mapping().unwrap() {
        let section = section.as_str().unwrap();
        own.insert(
            section.to_owned(),
            match entries.as_mapping().filter(|_| section != "default") {
                Some(entries) => entries
                    .keys()
                    .map(|key| {
                        let key = key.as_str().unwrap();
                        (key.to_owned(), shown[section][key].clone())
                    })
                    .collect(),
                None => shown[section].clone(),
            },
        );
    }
    insta::assert_json_snapshot!("neutral-effort-migration", own);
}
