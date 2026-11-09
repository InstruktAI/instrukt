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
