#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]
mod common;
use common::TestEnv;

fn focused_reviewer() -> TestEnv {
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
    env
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn focused_help_and_execution_use_the_recorded_binding() {
    let env = focused_reviewer();
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

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn a_retained_baseline_worker_keeps_command_access_with_explicitly_unknown_dispatcher() {
    let crew = workflow::Crew::new();
    crew.project
        .create("legacy", "change", "Retain the existing worker.");
    let worker = crew.worker(
        "legacy",
        "plan-reviewer",
        Some("crew/procedure/review-plan"),
    );
    let native = include_str!("fixtures/assignments/c11-native-assignment.json")
        .replace("[KEY]", worker["semantic_key"].as_str().unwrap())
        .replace("[WORKTREE]", worker["worktree"].as_str().unwrap())
        .replace("[BRANCH]", worker["branch"].as_str().unwrap())
        .replace("[BASE]", worker["base"].as_str().unwrap());
    let native: serde_json::Value = serde_json::from_str(&native).unwrap();
    let home = instrukt_core::home::Home::at_user_home(crew.project.env.home().to_path_buf());
    let db = instrukt_core::state_db::rusqlite::Connection::open(home.state_db_file()).unwrap();
    db.execute("UPDATE session_bindings SET evidence=json_set(evidence,'$.native_assignment',json(?1)) WHERE session=?2",instrukt_core::state_db::rusqlite::params![native.to_string(),worker["session"].as_str().unwrap()]).unwrap();
    db.pragma_update(None, "user_version", 12).unwrap();
    drop(db);
    let caller = worker["session"]
        .as_str()
        .unwrap()
        .strip_prefix("t3:")
        .unwrap();
    let workspace = std::path::Path::new(worker["worktree"].as_str().unwrap());
    crew.project
        .env
        .instrukt(workspace)
        .env("T3CODE_THREAD_ID", caller)
        .args(["docs", "index"])
        .assert()
        .success();
    crew.project
        .env
        .instrukt(workspace)
        .env("T3CODE_THREAD_ID", caller)
        .args([
            "todo",
            "artifacts",
            "write",
            "legacy",
            "plan-review-findings",
            "Baseline worker can still report.",
        ])
        .assert()
        .success();
    let denied = crew
        .project
        .env
        .instrukt(workspace)
        .env("T3CODE_THREAD_ID", caller)
        .args(["config", "init", "--global"])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "legacy-worker-still-focused",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
    let db = instrukt_core::state_db::rusqlite::Connection::open(home.state_db_file()).unwrap();
    let current:String=db.query_row("SELECT json_extract(evidence,'$.native_assignment') FROM session_bindings WHERE session=?1",[worker["session"].as_str().unwrap()],|row|row.get(0)).unwrap();
    let mut current: serde_json::Value = serde_json::from_str(&current).unwrap();
    current["worktree"] = "[WORKTREE]".into();
    current["branch"] = "[BRANCH]".into();
    current["base"] = "[BASE]".into();
    current["semantic_key"] = "[KEY]".into();
    current["title"] = "[KEY]".into();
    insta::assert_json_snapshot!("baseline-assignment-current-evidence", current);
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn directly_addressed_denied_help_uses_the_reporting_path_without_exposing_forms() {
    let env = focused_reviewer();
    let output = env
        .instrukt(env.home())
        .env("T3CODE_THREAD_ID", "focused")
        .args(["config", "init", "--help"])
        .assert()
        .success();
    insta::assert_snapshot!(String::from_utf8_lossy(&output.get_output().stdout), @"role `build-reviewer` (focused) cannot perform config-write on config init with its ungranted scope; return the required action to the dispatcher in your own output or assigned artifact");
}

#[path = "support/documented.rs"]
mod documented;
#[path = "support/handler.rs"]
mod handler;
#[path = "support/event_transport.rs"]
mod transport;

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_omitted_role_activates_as_event_handler_with_todos_off() {
    let h = handler::Handler::activate(None, |_| {});
    insta::assert_snapshot!("activated-event-handler", h.help());
    h.call(&["events", "list"]).assert().success();
    h.call(&["config", "init", "--global"]).assert().success();
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn a_newly_activated_focused_handler_admits_reads_and_refuses_configuration_writes() {
    let h = handler::Handler::activate(Some("focused-handler"), |agents| {
        agents["roles"]["focused-handler"] =
            serde_yaml_ng::from_str("{class: focused, can: [read]}").unwrap();
    });
    h.call(&["events", "list"]).assert().success();
    let denied = h.call(&["config", "init", "--global"]).assert().failure();
    insta::assert_snapshot!(
        "activated-focused-handler-refusal",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
    insta::assert_snapshot!("activated-focused-handler", h.help());
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn standing_bind_own_never_reserves_another_project_home() {
    let h = handler::Handler::activate(Some("focused-handler"), |agents| {
        agents["roles"]["focused-handler"] = serde_yaml_ng::from_str(
            "{class: focused, can: [read, {action: standing-bind, scope: own}]}",
        )
        .unwrap();
    });
    let other = documented::project(
        &h.computer.env,
        "other",
        &[(
            "instrukt.yaml",
            "schema_version: 3
project: other
modules: {events: true}
"
            .into(),
        )],
    );
    documented::sync(&h.computer.env, &other);
    let before: i64 = h
        .computer
        .db()
        .query_row(
            "SELECT count(*) FROM manual_standing_reservations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let denied = h
        .computer
        .env
        .instrukt(other.path())
        .env("T3CODE_THREAD_ID", "s1")
        .args(["events", "bind", "reactions/procedure/respond", "--reserve"])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "foreign-standing-reservation",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
    let after: i64 = h
        .computer
        .db()
        .query_row(
            "SELECT count(*) FROM manual_standing_reservations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
}

fn foreign_reservation(h: &handler::Handler) -> (common::Repo, serde_json::Value) {
    let env = &h.computer.env;
    let other = documented::project(
        env,
        "other",
        &[(
            "instrukt.yaml",
            "schema_version: 3\nproject: other\nmodules: {events: true}\n".into(),
        )],
    );
    documented::sync(env, &other);
    let file = env.instrukt_home().join("agents.yaml");
    let original = std::fs::read(&file).unwrap();
    let mut configured: serde_yaml_ng::Value = serde_yaml_ng::from_slice(&original).unwrap();
    configured["roles"]["focused-handler"]["class"] = "agent".into();
    std::fs::write(&file, serde_yaml_ng::to_string(&configured).unwrap()).unwrap();
    let reserved = env
        .instrukt(other.path())
        .env("T3CODE_THREAD_ID", "s1")
        .args(["events", "bind", "reactions/procedure/respond", "--reserve"])
        .assert()
        .success();
    let value = serde_json::from_slice(&reserved.get_output().stdout).unwrap();
    std::fs::write(file, original).unwrap();
    (other, value)
}
fn bind_handler() -> handler::Handler {
    handler::Handler::activate(Some("focused-handler"), |agents| {
        agents["roles"]["focused-handler"] = serde_yaml_ng::from_str(
            "{class: focused, can: [read, {action: standing-bind, scope: own}]}",
        )
        .unwrap();
    })
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn standing_bind_own_does_not_bind_a_foreign_home_reservation_from_its_own_cwd() {
    let h = bind_handler();
    let (other, reserved) = foreign_reservation(&h);
    let evidence = serde_json::json!({"session":"t3:foreign-native","title":reserved["key"],"assignment":reserved["brief"]});
    h.runtime.seed(
        "foreign-native",
        other.path(),
        reserved["key"].as_str().unwrap(),
        reserved["brief"].as_str().unwrap(),
    );
    let denied = h
        .call(&[
            "events",
            "bind",
            "reactions/procedure/respond",
            "--reservation",
            reserved["reservation"].as_str().unwrap(),
            "--session",
            "t3:foreign-native",
            "--evidence",
            "-",
        ])
        .write_stdin(evidence.to_string())
        .assert()
        .failure();
    insta::assert_snapshot!(
        "foreign-standing-binding",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
    let bound: Option<String> = h
        .computer
        .db()
        .query_row(
            "SELECT session FROM manual_standing_reservations WHERE token=?1",
            [reserved["reservation"].as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(bound.is_none());
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn standing_bind_own_does_not_resolve_a_foreign_home_start_from_its_own_cwd() {
    let h = bind_handler();
    let (_other, reserved) = foreign_reservation(&h);
    let denied = h
        .call(&[
            "events",
            "bind",
            "reactions/procedure/respond",
            "--reservation",
            reserved["reservation"].as_str().unwrap(),
            "--not-started",
            "--reason",
            "The native start did not occur.",
        ])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "foreign-standing-no-start",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
    let retry: bool = h
        .computer
        .db()
        .query_row(
            "SELECT retry_allowed FROM manual_standing_reservations WHERE token=?1",
            [reserved["reservation"].as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!retry);
}

fn read_but_not_configure(h: &handler::Handler, name: &str) {
    h.call(&["events", "list"]).assert().success();
    let denied = h.call(&["config", "init", "--global"]).assert().failure();
    insta::assert_snapshot!(name, String::from_utf8_lossy(&denied.get_output().stderr));
}
fn inherited_builder(agents: &mut serde_yaml_ng::Value) {
    agents["roles"]["builder"]
        .as_mapping_mut()
        .unwrap()
        .remove("class");
    agents["roles"]["builder"]
        .as_mapping_mut()
        .unwrap()
        .remove("can");
    agents["roles"]["builder"]["order"] = serde_yaml_ng::from_str("[codex]").unwrap();
    agents["roles"]["builder"]["tier"] = "light".into();
}
fn native_selection(h: &handler::Handler, name: &str) {
    let shown = h
        .computer
        .env
        .run(h.computer.project.path(), ["sessions", "list", "--json"])
        .success();
    insta::assert_json_snapshot!(
        name,
        serde_json::from_slice::<serde_json::Value>(&shown.get_output().stdout).unwrap()
    );
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_activated_known_role_inherits_scope_without_rewriting_edited_model_choices() {
    let h = handler::Handler::activate(Some("builder"), |agents| {
        inherited_builder(agents);
        agents.as_mapping_mut().unwrap().remove("bundles");
    });
    read_but_not_configure(&h, "inherited-known-handler-scope");
    native_selection(&h, "inherited-known-handler-selection");
    assert_eq!(
        std::fs::read(h.computer.env.instrukt_home().join("agents.yaml")).unwrap(),
        h.agents_before
    );
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_activated_unknown_role_uses_the_configured_effective_default_scope() {
    let h = handler::Handler::activate(Some("custom-handler"), |agents| {
        agents["default"]["class"] = "focused".into();
        agents["default"]["can"] = serde_yaml_ng::from_str("[read]").unwrap();
        agents["roles"]["custom-handler"] =
            serde_yaml_ng::from_str("{order: [codex], tier: light}").unwrap();
    });
    read_but_not_configure(&h, "inherited-unknown-handler-scope");
    insta::assert_snapshot!("inherited-unknown-handler-assignment", h.help());
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_explicit_agent_class_overrides_the_shipped_focused_class_during_admission() {
    let h = handler::Handler::activate(Some("builder"), |agents| {
        agents["roles"]["builder"]["class"] = "agent".into();
        agents["roles"]["builder"]
            .as_mapping_mut()
            .unwrap()
            .remove("can");
    });
    h.call(&["config", "init", "--global"]).assert().success();
    insta::assert_snapshot!("explicit-agent-handler-assignment", h.help());
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn explicit_empty_grants_remain_empty_for_the_recorded_handler() {
    let h = handler::Handler::activate(Some("builder"), |agents| {
        agents["roles"]["builder"]
            .as_mapping_mut()
            .unwrap()
            .remove("class");
        agents["roles"]["builder"]["can"] = serde_yaml_ng::from_str("[]").unwrap();
    });
    let denied = h.call(&["events", "list"]).assert().failure();
    insta::assert_snapshot!(
        "empty-handler-grants",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn an_explicit_bundle_override_controls_the_inherited_handler_write_scope() {
    let h = handler::Handler::activate(Some("builder"), |agents| {
        agents["roles"]["builder"]
            .as_mapping_mut()
            .unwrap()
            .remove("can");
        agents["bundles"]["worker"] = serde_yaml_ng::from_str("[]").unwrap();
    });
    h.call(&["events", "list"]).assert().success();
    let denied = h
        .call(&["docs", "fold", "unused-delivery"])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "inherited-handler-bundle-override",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn a_legacy_edited_file_yields_a_focused_handler_and_its_preserved_native_model_choice() {
    let h = handler::Handler::activate(Some("builder"), |agents| {
        inherited_builder(agents);
        agents["schema_version"] = 1.into();
    });
    read_but_not_configure(&h, "legacy-inherited-handler-scope");
    native_selection(&h, "legacy-inherited-handler-selection");
}
