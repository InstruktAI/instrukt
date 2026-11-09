#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]
mod common;
use common::TestEnv;
use instrukt_core::config::{Agents, SHIPPED_AGENTS};

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn handler_defaults_and_unknown_roles_preserve_their_recorded_name() {
    let agents = Agents::parse(SHIPPED_AGENTS).unwrap();
    let handler = agents.capabilities("event-handler").unwrap();
    assert_eq!(
        handler.class,
        instrukt_core::capabilities::RoleClass::Standing
    );
    let unknown = agents.capabilities("custom-handler").unwrap();
    assert_eq!(unknown.role, "custom-handler");
    assert_eq!(unknown.class, instrukt_core::capabilities::RoleClass::Agent);
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_unknown_role_class_is_rejected() {
    assert!(Agents::parse(&SHIPPED_AGENTS.replacen("class: agent", "class: admin", 1)).is_err());
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn a_missing_capability_bundle_is_rejected() {
    assert!(Agents::parse(&SHIPPED_AGENTS.replacen("can: [all]", "can: [missing]", 1)).is_err());
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn focused_help_and_execution_use_the_recorded_binding() {
    let env = TestEnv::new();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 1\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let mut db = instrukt_core::state_db::StateDb::open(&home).unwrap();
    db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
        tx.execute("INSERT INTO procedure_responsibilities(responsibility,procedure,exact_home,generation,semantic_key,checkout,declaration,request,session) VALUES ('handler','sdlc/procedure/handler','platform',1,'handler',?1,?2,'{}','t3:focused')", instrukt_core::state_db::rusqlite::params![env.home().to_string_lossy(),r#"{"role":"build-reviewer"}"#]).unwrap();
        Ok(())
    }).unwrap();
    drop(db);
    let output = env
        .instrukt(env.home())
        .env("T3CODE_THREAD_ID", "focused")
        .args(["--help"])
        .assert()
        .success();
    insta::assert_snapshot!(
        "focused-role-help",
        String::from_utf8_lossy(&output.get_output().stdout)
    );
    let output = env
        .instrukt(env.home())
        .env("T3CODE_THREAD_ID", "focused")
        .args(["config", "init", "--global"])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "focused-role-refusal",
        String::from_utf8_lossy(&output.get_output().stderr)
    );
}

#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod support;
#[path = "support/workflow.rs"]
mod workflow;

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn a_bound_reviewer_can_write_its_own_evidence_but_cannot_choose_another_assignment() {
    let crew = workflow::Crew::new();
    crew.project
        .create("parser", "change", "Improve parser depth.");
    let assignment = crew.worker(
        "parser",
        "plan-reviewer",
        Some("crew/procedure/review-plan"),
    );
    let session = assignment["session"]
        .as_str()
        .unwrap()
        .strip_prefix("t3:")
        .unwrap();
    let workspace = std::path::Path::new(assignment["worktree"].as_str().unwrap());
    crew.project
        .env
        .instrukt(workspace)
        .env("T3CODE_THREAD_ID", session)
        .args([
            "todo",
            "artifacts",
            "write",
            "parser",
            "plan-review-findings",
            "Review completed.",
        ])
        .assert()
        .success();
    // Supplying a target role never widens the caller's recorded reviewer grant.
    let refused = crew
        .project
        .env
        .instrukt(workspace)
        .env("T3CODE_THREAD_ID", session)
        .args([
            "todo", "attempt", "parser", "--role", "builder", "--brief", "-",
        ])
        .write_stdin("Build it.")
        .assert()
        .failure();
    insta::assert_snapshot!(
        "target-role-does-not-widen-caller",
        String::from_utf8_lossy(&refused.get_output().stderr)
    );
    crew.project.create("other", "change", "Separate work.");
    let refused = crew
        .project
        .env
        .instrukt(workspace)
        .env("T3CODE_THREAD_ID", session)
        .args([
            "todo",
            "artifacts",
            "write",
            "other",
            "plan-review-findings",
            "Wrong target.",
        ])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "other-todo-refused",
        String::from_utf8_lossy(&refused.get_output().stderr)
    );
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn edited_pre_capability_roles_resolve_shipped_fields_without_rewriting_the_file() {
    use instrukt_core::capabilities::{Action, RoleClass, Scope};
    let env = TestEnv::new();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
    edited["default"].as_mapping_mut().unwrap().remove("class");
    edited["default"].as_mapping_mut().unwrap().remove("can");
    edited["roles"]["builder"]
        .as_mapping_mut()
        .unwrap()
        .remove("class");
    edited["roles"]["builder"]
        .as_mapping_mut()
        .unwrap()
        .remove("can");
    edited["roles"]["builder"]["order"] = serde_yaml_ng::from_str("[codex]").unwrap();
    edited["roles"]["builder"]["tier"] = "light".into();
    edited.as_mapping_mut().unwrap().remove("bundles");
    let text = serde_yaml_ng::to_string(&edited).unwrap();
    let file = env.instrukt_home().join("agents.yaml");
    std::fs::write(&file, &text).unwrap();
    env.run(env.home(), ["config", "show", "agents"]).success();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    let resolved = Agents::parse(&text).unwrap();
    let builder = resolved.capabilities("builder").unwrap();
    assert_eq!(builder.class, RoleClass::Focused);
    assert_eq!(builder.scope(Action::DocsWrite), Some(Scope::Own));
    assert_eq!(
        resolved.capabilities("default").unwrap().class,
        RoleClass::Agent
    );
    let selected = resolved.select(Some("builder"), None, None).unwrap();
    assert_eq!(selected[0].profile.as_str(), "codex");
    assert_eq!(selected[0].model, "gpt-6-luna");
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_unknown_configured_role_inherits_the_effective_default_capabilities() {
    use instrukt_core::capabilities::{Action, RoleClass, Scope};
    let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
    edited["default"]["class"] = "focused".into();
    edited["default"]["can"] =
        serde_yaml_ng::from_str("[{action: docs-read, scope: own}]").unwrap();
    edited["roles"]["custom"] = serde_yaml_ng::from_str("{order: [codex], tier: light}").unwrap();
    let resolved = Agents::parse(&serde_yaml_ng::to_string(&edited).unwrap()).unwrap();
    let role = resolved.capabilities("custom").unwrap();
    assert_eq!(role.class, RoleClass::Focused);
    assert_eq!(role.scope(Action::DocsRead), Some(Scope::Own));
    assert_eq!(role.scope(Action::CodeCommit), None);
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_explicit_role_class_wins_while_its_missing_grants_use_the_shipped_role() {
    use instrukt_core::capabilities::{Action, RoleClass, Scope};
    let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
    edited["roles"]["builder"]["class"] = "agent".into();
    edited["roles"]["builder"]
        .as_mapping_mut()
        .unwrap()
        .remove("can");
    let role = Agents::parse(&serde_yaml_ng::to_string(&edited).unwrap())
        .unwrap()
        .capabilities("builder")
        .unwrap();
    assert_eq!(role.class, RoleClass::Agent);
    assert_eq!(role.scope(Action::DocsWrite), Some(Scope::Own));
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_explicit_empty_grant_list_wins_while_the_missing_class_uses_the_shipped_role() {
    use instrukt_core::capabilities::RoleClass;
    let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
    edited["roles"]["builder"]
        .as_mapping_mut()
        .unwrap()
        .remove("class");
    edited["roles"]["builder"]["can"] = serde_yaml_ng::from_str("[]").unwrap();
    let role = Agents::parse(&serde_yaml_ng::to_string(&edited).unwrap())
        .unwrap()
        .capabilities("builder")
        .unwrap();
    assert_eq!(role.class, RoleClass::Focused);
    assert!(role.grants.is_empty());
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn explicit_bundle_edits_win_when_a_role_inherits_its_grant_names() {
    use instrukt_core::capabilities::Action;
    let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
    edited["roles"]["builder"]
        .as_mapping_mut()
        .unwrap()
        .remove("can");
    edited["bundles"]["worker"] = serde_yaml_ng::from_str("[]").unwrap();
    let role = Agents::parse(&serde_yaml_ng::to_string(&edited).unwrap())
        .unwrap()
        .capabilities("builder")
        .unwrap();
    assert_eq!(role.scope(Action::DocsWrite), None);
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_edited_legacy_agents_file_keeps_its_model_choices_while_capabilities_inherit() {
    use instrukt_core::capabilities::RoleClass;
    let env = TestEnv::new();
    let mut edited: serde_yaml_ng::Value = serde_yaml_ng::from_str(SHIPPED_AGENTS).unwrap();
    edited["schema_version"] = 1.into();
    edited["default"].as_mapping_mut().unwrap().remove("class");
    edited["default"].as_mapping_mut().unwrap().remove("can");
    edited["roles"]["builder"]
        .as_mapping_mut()
        .unwrap()
        .remove("class");
    edited["roles"]["builder"]
        .as_mapping_mut()
        .unwrap()
        .remove("can");
    edited["roles"]["builder"]["order"] = serde_yaml_ng::from_str("[codex]").unwrap();
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    let file = env.instrukt_home().join("agents.yaml");
    std::fs::write(&file, serde_yaml_ng::to_string(&edited).unwrap()).unwrap();
    env.run(env.home(), ["config", "show", "agents"]).success();
    let text = std::fs::read_to_string(file).unwrap();
    let resolved = Agents::parse(&text).unwrap();
    assert_eq!(
        resolved.capabilities("builder").unwrap().class,
        RoleClass::Focused
    );
    assert_eq!(
        resolved.select(Some("builder"), None, None).unwrap()[0]
            .profile
            .as_str(),
        "codex"
    );
    let retained: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
    assert_eq!(retained["schema_version"].as_u64(), Some(2));
    assert!(retained["roles"]["builder"].get("class").is_none());
}
