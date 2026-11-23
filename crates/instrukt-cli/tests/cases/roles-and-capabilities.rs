#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]
use crate::common;
use crate::fixtures::documented;
use crate::fixtures::handler;
use crate::fixtures::work;
use crate::fixtures::workflow;
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
    let native = include_str!("../fixtures/assignments/c11-native-assignment.json")
        .replace("[KEY]", worker["semantic_key"].as_str().unwrap())
        .replace("[WORKTREE]", worker["worktree"].as_str().unwrap())
        .replace("[BRANCH]", worker["branch"].as_str().unwrap())
        .replace("[BASE]", worker["base"].as_str().unwrap());
    let native: serde_json::Value = serde_json::from_str(&native).unwrap();
    let home = instrukt_core::home::Home::at_user_home(crew.project.env.home().to_path_buf());
    let db = instrukt_core::state_db::rusqlite::Connection::open(home.state_db_file()).unwrap();
    db.execute("UPDATE session_bindings SET evidence=json_set(evidence,'$.native_assignment',json(?1)) WHERE session=?2",instrukt_core::state_db::rusqlite::params![native.to_string(),worker["session"].as_str().unwrap()]).unwrap();
    db.execute_batch("ALTER TABLE poll_deferrals DROP COLUMN cause;")
        .unwrap();
    db.execute_batch("CREATE TABLE runtime_starts (runtime TEXT NOT NULL,semantic_key TEXT NOT NULL,project TEXT,kind TEXT NOT NULL,subject TEXT NOT NULL,checkout TEXT NOT NULL,brief TEXT NOT NULL,request TEXT NOT NULL,begun_at TEXT NOT NULL,reported INTEGER NOT NULL,resolved_at TEXT,resolved_by TEXT,resolution TEXT,state TEXT NOT NULL,PRIMARY KEY(runtime,semantic_key)); ALTER TABLE runtime_bindings DROP COLUMN generation; ALTER TABLE runtime_bindings DROP COLUMN start_key; ALTER TABLE runtime_bindings DROP COLUMN replaced_session; ALTER TABLE runtime_bindings DROP COLUMN legacy_modes; ALTER TABLE connected_conversations DROP COLUMN display_title; DROP TABLE runtime_generations;").unwrap();
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
            "schema_version: 4
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
        .env(
            "T3CODE_THREAD_ID",
            crate::fixtures::t3::FakeT3::create(&h.computer.env).native_id("s1"),
        )
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
            "schema_version: 4\nproject: other\nmodules: {events: true}\n".into(),
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
        .env(
            "T3CODE_THREAD_ID",
            crate::fixtures::t3::FakeT3::create(&h.computer.env).native_id("s1"),
        )
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
        serde_json::from_str::<serde_json::Value>(&work::redact(&String::from_utf8_lossy(
            &shown.get_output().stdout,
        )))
        .unwrap()
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

/// A company's agent: a standing session of the `company-agent` role.
fn company_agent() -> TestEnv {
    let env = TestEnv::new();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let mut db = instrukt_core::state_db::StateDb::open(&home).unwrap();
    db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
        tx.execute("INSERT INTO procedure_responsibilities(responsibility,procedure,exact_home,generation,semantic_key,checkout,declaration,request,session) VALUES ('assistance','general/procedure/company-assistance','platform',1,'assistance',?1,?2,'{}','t3:company')", instrukt_core::state_db::rusqlite::params![env.home().to_string_lossy(),r#"{"role":"company-agent"}"#]).unwrap();
        Ok(())
    }).unwrap();
    drop(db);
    env
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP3
#[test]
fn a_company_agent_may_report_defects_and_change_no_other_work() {
    let env = company_agent();
    let help = |args: &[&str]| {
        let output = env
            .instrukt(env.home())
            .env("T3CODE_THREAD_ID", "company")
            .args(args)
            .assert()
            .success();
        String::from_utf8_lossy(&output.get_output().stdout).into_owned()
    };
    // Its help names what it may do: reading the record and reporting a
    // defect into any project, and no other work-record write.
    insta::assert_snapshot!("company-agent-help", help(&["--help"]));

    // An agents.yaml edited before `report` existed keeps working: its
    // company agent holds what that file gives it, and no `bug-report`.
    let file = env.instrukt_home().join("agents.yaml");
    let edited = std::fs::read_to_string(&file)
        .unwrap()
        .replace("can: [read, tools, report]", "can: [read, tools]")
        .replace("  report:\n    - {action: bug-report, scope: any}\n", "")
        .replace("    - {action: bug-report, scope: own}\n", "")
        .replace("    - {action: bug-report, scope: any}\n", "");
    std::fs::write(&file, edited).unwrap();
    let without = help(&["--help"]);
    insta::assert_snapshot!(
        without.lines().skip_while(|line| !line.starts_with("Role:")).collect::<Vec<_>>().join("\n"),
        @r"
    Role: company-agent · class: standing. Scope comes from assignment assistance. Object claims, phase
    and writer constraints apply.
    Usual capabilities: role: company-agent; class: standing; grants: docs-read:any, todo-read:any,
    artifact-read:any, agents-read:any, config-read:any, repository-read:any, code-check:any,
    eval-read:any, events-read:any, installation-read:any, mcp-access:own, company-reply:own
    "
    );
}

/// The state database of `env`'s instrukt home, opened directly.
fn state_db(env: &TestEnv) -> instrukt_core::state_db::rusqlite::Connection {
    instrukt_core::state_db::rusqlite::Connection::open(
        env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap()
}

/// A runner-picked-up `parser` orchestrator rewound to its recorded launch
/// window: the runner holds the start of `key` for the returned native thread,
/// while work still holds the reserved attempt without its session. Returns
/// the native thread id, the semantic key and the thread's worktree.
fn orchestrator_in_its_launch_window(
    p: &crate::fixtures::flavor::Tracked,
) -> (String, String, String) {
    p.todo([
        "create",
        "parser",
        "Improve the parser.",
        "--kind",
        "change",
        "--approved",
    ]);
    p.tick();
    let fake = crate::fixtures::runtime::FakeT3::create(&p.env);
    let (native, key) = (fake.native_id("s1"), fake.key("s1"));
    let db = state_db(&p.env);
    let worktree: String = db
        .query_row(
            "SELECT worktree FROM runtime_bindings WHERE semantic_key=?1",
            [&key],
            |row| row.get(0),
        )
        .unwrap();
    db.execute("UPDATE session_bindings SET session=NULL,worktree=NULL,evidence=json_set(evidence,'$.session',json('null'),'$.worktree',json('null')) WHERE semantic_key=?1",[&key]).unwrap();
    db.execute(
        "UPDATE todos SET claim_session=NULL WHERE slug='parser'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE runtime_bindings SET accepted=0,worktree=NULL WHERE semantic_key=?1",
        [&key],
    )
    .unwrap();
    (native, key, worktree)
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn a_started_orchestrator_is_admitted_on_its_first_turn_before_work_records_its_session() {
    let p = crate::fixtures::flavor::Tracked::new(true);
    let (native, key, worktree) = orchestrator_in_its_launch_window(&p);
    let admitted = || {
        for args in [&["todo", "show", "parser"][..], &["docs", "index"][..]] {
            p.env
                .instrukt(std::path::Path::new(&worktree))
                .env("T3CODE_THREAD_ID", &native)
                .args(args)
                .assert()
                .success();
        }
    };
    // Before the runtime accepted the start.
    admitted();
    // After the runtime accepted it, before work records the session.
    state_db(&p.env)
        .execute(
            "UPDATE runtime_bindings SET accepted=1,worktree=?2 WHERE semantic_key=?1",
            [&key, &worktree],
        )
        .unwrap();
    admitted();
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn a_start_whose_attempt_is_bound_to_another_session_remains_a_resolution_failure() {
    let p = crate::fixtures::flavor::Tracked::new(true);
    let (native, key, worktree) = orchestrator_in_its_launch_window(&p);
    state_db(&p.env).execute("UPDATE session_bindings SET session='t3:unrelated',evidence=json_set(evidence,'$.session','t3:unrelated') WHERE semantic_key=?1",[&key]).unwrap();
    let refused = p
        .env
        .instrukt(std::path::Path::new(&worktree))
        .env("T3CODE_THREAD_ID", &native)
        .args(["todo", "show", "parser"])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "unrelated-binding-start-refused",
        String::from_utf8_lossy(&refused.get_output().stderr)
            .replace(
                p.env.path("").to_str().unwrap().trim_end_matches('/'),
                "[TMP]"
            )
            .replace(&native, "[NATIVE]")
    );
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn a_successor_is_admitted_before_work_moves_the_attempt_to_it() {
    let mut p = crate::fixtures::flavor::Tracked::new(true);
    p.authorized_task("weekly-status", "2030-01-06T09:00:00Z", &[]);
    p.tick_at("2030-01-06T09:00:20Z");
    let fake = crate::fixtures::runtime::FakeT3::create(&p.env);
    let (old, key) = (format!("t3:{}", fake.native_id("s1")), fake.key("s1"));
    fake.delete("s1");
    p.tick_at("2030-01-06T09:01:20Z");
    let successor = fake.native_id("s2");
    // Rewind work to the successor's launch window: the attempt and claim
    // still name the deleted thread the runner's start replaces.
    let db = state_db(&p.env);
    let replaced: String = db
        .query_row(
            "SELECT replaced_session FROM runtime_bindings WHERE semantic_key=?1",
            [&key],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(replaced, old);
    db.execute("UPDATE session_bindings SET session=?2,evidence=json_set(evidence,'$.session',?2) WHERE semantic_key=?1",[&key,&old]).unwrap();
    db.execute(
        "UPDATE todos SET claim_session=?1 WHERE slug='weekly-status'",
        [&old],
    )
    .unwrap();
    db.execute(
        "UPDATE runtime_bindings SET accepted=0 WHERE semantic_key=?1",
        [&key],
    )
    .unwrap();
    p.env
        .instrukt(p.repo.path())
        .env("T3CODE_THREAD_ID", &successor)
        .args(["todo", "show", "weekly-status"])
        .assert()
        .success();
}

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn a_started_standing_agent_carries_its_focused_role_before_its_responsibility_records_it() {
    let env = focused_reviewer();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let db = instrukt_core::state_db::rusqlite::Connection::open(home.state_db_file()).unwrap();
    db.execute(
        "UPDATE procedure_responsibilities SET session=NULL WHERE responsibility='handler'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO runtime_bindings(runtime,semantic_key,kind,subject,session,checkout,accepted,generation) VALUES('t3','handler','standing','handler','t3:focused',?1,0,1)",[env.home().to_str().unwrap()]).unwrap();
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

// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP3
#[test]
fn a_started_conversation_agent_carries_its_company_role_before_its_first_message_binds_it() {
    let env = TestEnv::new();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    drop(instrukt_core::state_db::StateDb::open(&home).unwrap());
    let db = instrukt_core::state_db::rusqlite::Connection::open(home.state_db_file()).unwrap();
    db.execute_batch("INSERT INTO connected_conversations(company,connector,conversation,semantic_key,source_identity) VALUES ('acme','slack','C1/1700.1','instrukt:conversation:acme:slack:C1%2F1700.1','src')").unwrap();
    db.execute("INSERT INTO runtime_bindings(runtime,semantic_key,kind,subject,session,checkout,accepted,generation) VALUES('t3','instrukt:conversation:acme:slack:C1%2F1700.1','conversation','instrukt:conversation:acme:slack:C1%2F1700.1','t3:company',?1,0,1)",[env.home().to_str().unwrap()]).unwrap();
    let output = env
        .instrukt(env.home())
        .env("T3CODE_THREAD_ID", "company")
        .args(["--help"])
        .assert()
        .success();
    let help = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    insta::assert_snapshot!(
        help.lines().skip_while(|line| !line.starts_with("Role:")).collect::<Vec<_>>().join("\n"),
        @r"
    Role: company-agent · class: standing. Scope comes from assignment
    instrukt:conversation:acme:slack:C1%2F1700.1. Object claims, phase and writer constraints apply.
    Usual capabilities: role: company-agent; class: standing; grants: docs-read:any, todo-read:any,
    bug-report:any, artifact-read:any, agents-read:any, config-read:any, repository-read:any,
    code-check:any, eval-read:any, events-read:any, installation-read:any, mcp-access:own,
    company-reply:own
    "
    );
}
