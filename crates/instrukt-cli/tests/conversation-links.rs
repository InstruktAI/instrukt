#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]
mod common;
use instrukt_runner::links;

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL2
#[test]
fn either_member_closes_the_whole_link_and_retains_progress() {
    let env = common::TestEnv::new();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let _store = history::T3Store::new(&env);
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "open", "child"])
        .assert()
        .success();
    let id = links::status(&home).unwrap()[0].id.clone();
    assert_eq!(links::status(&home).unwrap().len(), 1);
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "child")
        .args(["discuss", "close"])
        .assert()
        .success();
    let statuses = links::status(&home).unwrap();
    assert_eq!(statuses[0].id, id);
    assert!(!statuses[0].active);
    let output = env
        .run(env.home(), ["events", "status", "--json"])
        .success();
    let public: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(public["links"][0]["active"], false);
}
#[allow(
    dead_code,
    reason = "shared native database fixture has other history scenarios"
)]
#[path = "support/history.rs"]
mod history;
#[path = "support/runtime.rs"]
mod runtime;
use std::time::{Duration, Instant};
#[path = "support/work.rs"]
mod support;
#[path = "support/workflow.rs"]
mod workflow;

struct Running(std::process::Child);
impl Drop for Running {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            self.0.kill().unwrap();
        }
        self.0.wait().unwrap();
    }
}
fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "native relay did not make the expected progress"
        );
        std::thread::yield_now();
    }
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL2
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS1
#[test]
fn the_real_runner_relays_native_completed_turns_and_keeps_restart_positions() {
    let env = common::TestEnv::new();
    let store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "peer-a", "opening a");
    fake.seed("child", env.home(), "peer-b", "opening b");
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let adapter = instrukt_runner::runtime::adapter(instrukt_core::config::Runtime::T3);
    let source = adapter
        .conversation_source(&home, &instrukt_core::config::Env::default())
        .unwrap();
    let a: instrukt_core::ids::SessionRef = "t3:parent".parse().unwrap();
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", a.id.as_str())
        .args(["discuss", "open", "child"])
        .assert()
        .success();
    let id = links::status(&home).unwrap()[0].id.clone();
    let run = || {
        Running(
            env.instrukt_process(env.home())
                .arg("run")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    store.native_records();
    let native_db = instrukt_core::state_db::rusqlite::Connection::open(&store.file).unwrap();
    native_db
        .execute(
            "DELETE FROM orchestration_v2_projection_turn_items WHERE type='user_message'",
            [],
        )
        .unwrap();
    let at = "2026-11-08T10:00:00.000Z";
    let ongoing = serde_json::json!({"id":"ongoing","threadId":"child","ordinal":99,"status":"running","userMessageId":"native-user-message","requestedAt":at,"completedAt":null});
    native_db.execute("INSERT INTO orchestration_v2_projection_runs VALUES ('ongoing','child','running',?1,NULL,?2)",instrukt_core::state_db::rusqlite::params![at,ongoing.to_string()]).unwrap();
    let partial = serde_json::json!({"id":"streaming","threadId":"child","runId":"ongoing","role":"assistant","text":"Unfinished output stays here.","streaming":true,"createdAt":at,"updatedAt":at});
    native_db.execute("INSERT INTO orchestration_v2_projection_messages VALUES ('streaming','child','assistant',1,?1,?1,?2)",instrukt_core::state_db::rusqlite::params![at,partial.to_string()]).unwrap();
    let runner = run();
    wait_for(|| fake.message_count("parent") == 2);
    let delivered = fake
        .message("parent", 1)
        .replace(source.identity(), "t3:[SOURCE]");
    insta::assert_snapshot!("finished-turn-relay", delivered);
    assert_eq!(fake.message_count("child"), 1);
    // A native change with unchanged occurrence time still reaches the independent reader.
    let db = instrukt_core::state_db::rusqlite::Connection::open(&store.file).unwrap();
    db.execute("UPDATE orchestration_v2_projection_runs SET payload_json=json_set(payload_json,'$.id','second-run','$.ordinal',2),run_id='second-run' WHERE run_id='native-run'",[]).unwrap();
    db.execute("UPDATE orchestration_v2_projection_messages SET payload_json=json_set(payload_json,'$.runId','second-run') WHERE json_extract(payload_json,'$.runId')='native-run'",[]).unwrap();
    db.execute("UPDATE orchestration_v2_projection_turn_items SET payload_json=json_set(payload_json,'$.runId','second-run') WHERE json_extract(payload_json,'$.runId')='native-run'",[]).unwrap();
    wait_for(|| fake.message_count("parent") == 3);
    db.execute("UPDATE orchestration_v2_projection_runs SET payload_json=json_set(payload_json,'$.id','empty-run','$.ordinal',3),run_id='empty-run' WHERE run_id='second-run'",[]).unwrap();
    db.execute("UPDATE orchestration_v2_projection_messages SET payload_json=json_set(payload_json,'$.runId','empty-run','$.text',CASE role WHEN 'user' THEN '[Instrukt Assignment]' || char(10) || 'internal instructions' || char(10) || '[/Instrukt Assignment]' ELSE 'NOOP' END) WHERE json_extract(payload_json,'$.runId')='second-run'",[]).unwrap();
    db.execute("UPDATE orchestration_v2_projection_turn_items SET payload_json=json_set(payload_json,'$.runId','empty-run') WHERE json_extract(payload_json,'$.runId')='second-run'",[]).unwrap();
    wait_for(|| {
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .any(|d| d.delivered_turn.as_deref() == Some("empty-run"))
    });
    assert_eq!(fake.message_count("parent"), 3);
    assert!(
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .any(|d| matches!(d.receipts.get("empty-run"), Some(links::Disposition::Empty)))
    );
    drop(runner);
    store.turn(
        "child",
        "fresh-after-restart",
        4,
        "A fresh clarification.",
        "A fresh reply.",
    );
    let runner = run();
    wait_for(|| {
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .any(|d| d.delivered_turn.as_deref() == Some("fresh-after-restart"))
    });
    assert_eq!(fake.message_count("parent"), 4);
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", a.id.as_str())
        .args(["discuss", "close"])
        .assert()
        .success();
    assert!(!links::status(&home).unwrap()[0].active);
    assert_eq!(links::status(&home).unwrap()[0].id, id);
    drop(runner);
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn an_interrupted_send_recovers_native_acceptance_without_duplicate_input() {
    let env = common::TestEnv::new();
    let store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "peer-a", "opening a");
    fake.seed("child", env.home(), "peer-b", "opening b");
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let adapter = instrukt_runner::runtime::adapter(instrukt_core::config::Runtime::T3);
    let source = adapter
        .conversation_source(&home, &instrukt_core::config::Env::default())
        .unwrap();
    let a: instrukt_core::ids::SessionRef = "t3:parent".parse().unwrap();
    let b = "t3:child".parse().unwrap();
    links::open(
        &home,
        source.as_ref(),
        instrukt_todos::work::recorded_assignment,
        &links::Opening { a: &a, b: &b },
    )
    .unwrap();
    let run = || {
        Running(
            env.instrukt_process(env.home())
                .arg("run")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    fake.flag("die-after-send", true);
    let mut runner = run();
    store.native_records();
    wait_for(|| runner.0.try_wait().unwrap().is_some());
    assert_eq!(fake.message_count("parent"), 2);
    assert!(
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .any(|d| d.uncertain)
    );
    drop(runner);
    // Upgrade the retained native handover from the preceding owned schema.
    let previous =
        instrukt_core::state_db::rusqlite::Connection::open(home.state_db_file()).unwrap();
    previous.execute_batch("UPDATE conversation_relay_positions SET pending_json=(SELECT json_group_array(json(json_remove(value,'$.origin'))) FROM json_each(conversation_relay_positions.pending_json)); PRAGMA user_version=11;").unwrap();
    drop(previous);
    let runner = run();
    wait_for(|| {
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .any(|d| d.delivered_turn.as_deref() == Some("native-run"))
    });
    assert_eq!(fake.message_count("parent"), 2);
    assert!(
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .all(|d| !d.uncertain)
    );
    drop(runner);
}

struct AdmissionFixture {
    crew: workflow::Crew,
    _store: history::T3Store,
    home: instrukt_core::home::Home,
    a: instrukt_core::ids::SessionRef,
    b: instrukt_core::ids::SessionRef,
}
fn admission_fixture() -> AdmissionFixture {
    let crew = workflow::Crew::new();
    crew.project
        .create("linked-work", "change", "Review the parser.");
    let worker = crew.worker(
        "linked-work",
        "plan-reviewer",
        Some("crew/procedure/review-plan"),
    );
    let env = &crew.project.env;
    let store = history::T3Store::new(env);
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let a: instrukt_core::ids::SessionRef = "t3:lead-1".parse().unwrap();
    let b: instrukt_core::ids::SessionRef = worker["session"].as_str().unwrap().parse().unwrap();
    store.thread(a.id.as_str(), None, None, "2026-11-08T10:00:00Z");
    store.thread(
        b.id.as_str(),
        Some(a.id.as_str()),
        None,
        "2026-11-08T10:00:00Z",
    );
    AdmissionFixture {
        crew,
        _store: store,
        home,
        a,
        b,
    }
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn a_focused_recorded_assignment_reports_through_its_own_output() {
    let f = admission_fixture();
    let denied = f
        .crew
        .project
        .env
        .instrukt(f.crew.project.env.home())
        .env("T3CODE_THREAD_ID", f.a.id.as_str())
        .args(["discuss", "open", f.b.id.as_str()])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "focused-link-refusal",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn a_workers_recorded_dispatcher_relationship_survives_a_later_claim_change() {
    let f = admission_fixture();
    let agents = f.home.agents_file();
    let mut configured: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&agents).unwrap()).unwrap();
    configured["roles"]["plan-reviewer"]["class"] = "agent".into();
    std::fs::write(agents, serde_yaml_ng::to_string(&configured).unwrap()).unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(f.home.state_db_file()).unwrap();
    db.execute("UPDATE todos SET claim_session='t3:later-claimant' WHERE id=(SELECT todo_id FROM session_bindings WHERE session=?1)",[f.b.to_string()]).unwrap();
    let denied = f
        .crew
        .project
        .env
        .instrukt(f.crew.project.env.home())
        .env("T3CODE_THREAD_ID", f.a.id.as_str())
        .args(["discuss", "open", f.b.id.as_str()])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "dispatcher-link-refusal",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP1
#[test]
fn a_migrated_nonfocused_worker_keeps_scoped_commands_but_cannot_link_to_its_dispatcher() {
    let f = admission_fixture();
    let mut agents: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(f.home.agents_file()).unwrap()).unwrap();
    agents["roles"]["plan-reviewer"]["class"] = "agent".into();
    std::fs::write(
        f.home.agents_file(),
        serde_yaml_ng::to_string(&agents).unwrap(),
    )
    .unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(f.home.state_db_file()).unwrap();
    let native: String = db.query_row("SELECT json_extract(evidence,'$.native_assignment') FROM session_bindings WHERE session=?1", [f.b.to_string()], |row| row.get(0)).unwrap();
    let native: serde_json::Value = serde_json::from_str(&native).unwrap();
    let dispatcher: instrukt_core::ids::SessionRef =
        native["dispatcher"].as_str().unwrap().parse().unwrap();
    assert_eq!(dispatcher, f.a);
    let workspace = std::path::Path::new(native["worktree"].as_str().unwrap());
    let old = include_str!("fixtures/assignments/c11-native-assignment.json")
        .replace("[KEY]", native["semantic_key"].as_str().unwrap())
        .replace("[WORKTREE]", workspace.to_str().unwrap())
        .replace("[BRANCH]", native["branch"].as_str().unwrap())
        .replace("[BASE]", native["base"].as_str().unwrap());
    let mut old: serde_json::Value = serde_json::from_str(&old).unwrap();
    old["args"] = native["args"].clone();
    db.execute("UPDATE session_bindings SET evidence=json_set(evidence,'$.native_assignment',json(?1)) WHERE session=?2", instrukt_core::state_db::rusqlite::params![old.to_string(), f.b.to_string()]).unwrap();
    db.pragma_update(None, "user_version", 12).unwrap();
    drop(db);
    let env = &f.crew.project.env;
    let config_path = env.instrukt_home().join("instrukt.yaml");
    let mut config: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    config["github"]["api_url"] = f.crew.project.github.url().into();
    std::fs::write(config_path, serde_yaml_ng::to_string(&config).unwrap()).unwrap();
    env.instrukt(workspace)
        .env("T3CODE_THREAD_ID", f.b.id.as_str())
        .args(["docs", "index"])
        .assert()
        .success();
    env.instrukt(workspace)
        .env("T3CODE_THREAD_ID", f.b.id.as_str())
        .args([
            "todo",
            "artifacts",
            "write",
            "linked-work",
            "plan-review-findings",
            "The retained worker can still report.",
        ])
        .assert()
        .success();
    for (caller, target) in [(&f.b, &dispatcher), (&dispatcher, &f.b)] {
        let denied = env
            .instrukt(workspace)
            .env("T3CODE_THREAD_ID", caller.id.as_str())
            .args(["discuss", "open", target.id.as_str()])
            .assert()
            .failure();
        insta::assert_snapshot!(
            "unknown-dispatcher-link-refusal",
            String::from_utf8_lossy(&denied.get_output().stderr)
        );
    }
    assert!(links::status(&f.home).unwrap().is_empty());
    env.instrukt(workspace)
        .env("T3CODE_THREAD_ID", f.b.id.as_str())
        .args(["docs", "index"])
        .assert()
        .success();
    let db = instrukt_core::state_db::rusqlite::Connection::open(f.home.state_db_file()).unwrap();
    let current: String = db.query_row("SELECT json_extract(evidence,'$.native_assignment') FROM session_bindings WHERE session=?1", [f.b.to_string()], |row| row.get(0)).unwrap();
    let current: serde_json::Value = serde_json::from_str(&current).unwrap();
    assert_eq!(current.get("dispatcher"), Some(&serde_json::Value::Null));
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn a_third_peer_is_refused_without_changing_the_active_conversation() {
    let f = admission_fixture();
    let env = &f.crew.project.env;
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", f.a.id.as_str())
        .args(["discuss", "open", "parent"])
        .assert()
        .success();
    let denied = env
        .instrukt(env.home())
        .env("T3CODE_THREAD_ID", f.a.id.as_str())
        .args(["discuss", "open", "third"])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "third-peer-refusal",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
    assert_eq!(links::status(&f.home).unwrap().len(), 1);
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn opening_an_existing_pair_reuses_its_link_and_positions() {
    let f = admission_fixture();
    let env = &f.crew.project.env;
    let open = || {
        env.instrukt(env.home())
            .env("T3CODE_THREAD_ID", f.a.id.as_str())
            .args(["discuss", "open", "parent"])
            .assert()
            .success()
    };
    open();
    let before = links::status(&f.home).unwrap().remove(0);
    open();
    let after = links::status(&f.home).unwrap().remove(0);
    assert_eq!(before.id, after.id);
    assert_eq!(before.directions[0].observed, after.directions[0].observed);
    assert_eq!(links::status(&f.home).unwrap().len(), 1);
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn an_agent_opens_the_requested_discussion_and_reuses_its_one_opening_contribution() {
    let env = common::TestEnv::new();
    let _store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "peer-a", "opening a");
    fake.seed("child", env.home(), "peer-b", "opening b");
    fake.change("parent", "providerInstanceId", serde_json::json!("codex"));
    fake.change("parent", "model", serde_json::json!("native-model"));
    let open = || {
        env.instrukt(env.home())
            .env("T3CODE_THREAD_ID", "parent")
            .env("INSTRUKT_NOW", "2030-01-01T12:00:00Z")
            .args([
                "discuss",
                "open",
                "child",
                "--message",
                "Discuss the parser with me.",
            ])
            .assert()
            .success()
    };
    open();
    open();
    assert_eq!(fake.message_count("child"), 2);
    assert_eq!(fake.message_count("parent"), 1);
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let source = instrukt_runner::runtime::adapter(instrukt_core::config::Runtime::T3)
        .conversation_source(&home, &instrukt_core::config::Env::default())
        .unwrap();
    insta::assert_snapshot!(
        "cli-opening-contribution",
        fake.message("child", 1)
            .replace(source.identity(), "t3:[SOURCE]")
    );
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL2
#[test]
fn either_agent_sends_a_final_contribution_then_closes_the_discussion_for_everyone() {
    let env = common::TestEnv::new();
    let _store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "peer-a", "opening a");
    fake.seed("child", env.home(), "peer-b", "opening b");
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "open", "t3:child"])
        .assert()
        .success();
    let close = || {
        env.instrukt(env.home())
            .env("T3CODE_THREAD_ID", "child")
            .env("INSTRUKT_NOW", "2030-01-01T12:05:00Z")
            .args([
                "discuss",
                "close",
                "--message",
                "We have reached a conclusion.",
            ])
            .assert()
            .success()
    };
    close();
    close();
    assert_eq!(fake.message_count("parent"), 2);
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    assert!(!links::status(&home).unwrap()[0].active);
    let source = instrukt_runner::runtime::adapter(instrukt_core::config::Runtime::T3)
        .conversation_source(&home, &instrukt_core::config::Env::default())
        .unwrap();
    insta::assert_snapshot!(
        "cli-final-contribution",
        fake.message("parent", 1)
            .replace(source.identity(), "t3:[SOURCE]")
    );
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn opening_requires_a_native_caller_before_creating_a_link() {
    let env = common::TestEnv::new();
    let _store = history::T3Store::new(&env);
    let denied = env
        .instrukt(env.home())
        .args(["discuss", "open", "child"])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "cli-native-caller-required",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    assert!(links::status(&home).unwrap().is_empty());
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn retrying_an_interrupted_open_observes_its_original_contribution_without_resending() {
    let env = common::TestEnv::new();
    let _store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "peer-a", "opening a");
    fake.seed("child", env.home(), "peer-b", "opening b");
    fake.flag("die-after-send", true);
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args([
            "discuss",
            "open",
            "child",
            "--message",
            "Retain this contribution.",
        ])
        .assert()
        .failure();
    assert_eq!(fake.message_count("child"), 2);
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    assert!(
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .any(|d| d.uncertain)
    );
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args([
            "discuss",
            "open",
            "child",
            "--message",
            "Retain this contribution.",
        ])
        .assert()
        .success();
    assert_eq!(fake.message_count("child"), 2);
    assert!(
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .all(|d| !d.uncertain)
    );
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL2
#[test]
fn startup_recovers_an_accepted_final_contribution_and_completes_its_closure() {
    let env = common::TestEnv::new();
    let _store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "peer-a", "opening a");
    fake.seed("child", env.home(), "peer-b", "opening b");
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "open", "child"])
        .assert()
        .success();
    fake.flag("die-after-send", true);
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "child")
        .args(["discuss", "close", "--message", "Final result."])
        .assert()
        .failure();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    assert!(links::status(&home).unwrap()[0].active);
    assert_eq!(fake.message_count("parent"), 2);
    let runner = Running(
        env.instrukt_process(env.home())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for(|| !links::status(&home).unwrap()[0].active);
    assert_eq!(fake.message_count("parent"), 2);
    assert!(
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .all(|d| !d.uncertain)
    );
    drop(runner);
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL2
#[test]
fn changing_the_runtime_source_does_not_reidentify_a_links_members() {
    let env = common::TestEnv::new();
    let _store = history::T3Store::new(&env);
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "open", "child"])
        .assert()
        .success();
    let denied = env
        .instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .env("T3CODE_HOME", env.path("another-t3"))
        .args(["discuss", "close"])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "cli-source-membership-preserved",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    assert!(links::status(&home).unwrap()[0].active);
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS1
#[test]
fn native_completed_turns_suppress_inherited_echoes_but_preserve_other_human_spans() {
    let env = common::TestEnv::new();
    let store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "peer-a", "opening a");
    fake.seed("child", env.home(), "peer-b", "opening b");
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "open", "child"])
        .assert()
        .success();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let source = instrukt_runner::runtime::adapter(instrukt_core::config::Runtime::T3)
        .conversation_source(&home, &instrukt_core::config::Env::default())
        .unwrap();
    let inherited = instrukt_runner::envelopes::peer(
        &format!("{}:parent", source.identity()),
        "agent:original",
        "unknown",
        "Do not echo this contribution.",
    );
    let old = instrukt_runner::envelopes::peer(
        "t3:earlier:peer",
        "human:earlier",
        "unknown",
        "Keep this original human attribution.",
    );
    let input = format!(
        "New leading human clarification.\n{inherited}\n{old}\nNew trailing human clarification."
    );
    let runner = Running(
        env.instrukt_process(env.home())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    store.turn("child", "inherited-turn", 1, &input, "A new agent reply.");
    wait_for(|| {
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .any(|d| d.delivered_turn.as_deref() == Some("inherited-turn"))
    });
    assert_eq!(fake.message_count("parent"), 2);
    assert_eq!(fake.message_count("child"), 1);
    insta::assert_snapshot!(
        "real-relay-no-echo",
        fake.message("parent", 1)
            .replace(source.identity(), "t3:[SOURCE]")
    );
    drop(runner);
}

fn native_input(env: &common::TestEnv, id: &str, text: &str) {
    use std::io::Write as _;
    let mut process = env
        .command(&env.path("runtime-bin/t3"), env.home())
        .args(["thread", "send", "--", id, "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
    assert!(process.wait().unwrap().success());
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL2
#[test]
fn both_members_continue_locally_after_close_while_the_runner_processes_fresh_turns() {
    let env = common::TestEnv::new();
    let store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    for id in ["parent", "child", "witness-a", "witness-b"] {
        store.thread(id, None, None, "2026-11-08T10:00:00.000Z");
        fake.seed(id, env.home(), id, "Local opening.");
    }
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "open", "child"])
        .assert()
        .success();
    let runner = Running(
        env.instrukt_process(env.home())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    store.turn("child", "before-close", 1, "Shared input.", "Shared reply.");
    wait_for(|| fake.message_count("parent") == 2);
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "close"])
        .assert()
        .success();
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "witness-a")
        .args(["discuss", "open", "witness-b"])
        .assert()
        .success();
    let parent_before = fake.message_count("parent");
    let child_before = fake.message_count("child");
    native_input(
        &env,
        "parent",
        "Continue independently in the first thread.",
    );
    fake.finish("parent", "idle", "First local reply.");
    native_input(
        &env,
        "child",
        "Continue independently in the second thread.",
    );
    fake.finish("child", "idle", "Second local reply.");
    store.turn(
        "parent",
        "parent-after-close",
        2,
        "Continue independently in the first thread.",
        "First local reply.",
    );
    store.turn(
        "child",
        "child-after-close",
        2,
        "Continue independently in the second thread.",
        "Second local reply.",
    );
    store.turn(
        "witness-a",
        "fresh-barrier",
        1,
        "Process the new source pass.",
        "Fresh progress.",
    );
    wait_for(|| {
        links::status(&home)
            .unwrap()
            .iter()
            .flat_map(|l| &l.directions)
            .any(|d| d.delivered_turn.as_deref() == Some("fresh-barrier"))
    });
    assert_eq!(fake.message_count("parent"), parent_before + 2);
    assert_eq!(fake.message_count("child"), child_before + 2);
    insta::assert_snapshot!(
        "closed-first-conversation-local-reply",
        fake.message("parent", parent_before + 1)
    );
    insta::assert_snapshot!(
        "closed-second-conversation-local-reply",
        fake.message("child", child_before + 1)
    );
    drop(runner);
}

fn diagnostic_reference(text: &str) -> String {
    let mut result = text.to_owned();
    if let Some(start) = result.find("[diagnostic:") {
        let hash = start + "[diagnostic:".len();
        if result
            .get(hash..hash + 64)
            .is_some_and(|value| value.bytes().all(|b| b.is_ascii_hexdigit()))
            && result.as_bytes().get(hash + 64) == Some(&b']')
        {
            result.replace_range(hash..hash + 64, "REF");
        }
    }
    result
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL2
#[test]
fn native_send_failure_keeps_a_safe_actionable_cause_and_pending_handover() {
    let env = common::TestEnv::new();
    let _store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "parent", "Local opening.");
    fake.seed("child", env.home(), "child", "Local opening.");
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "open", "child"])
        .assert()
        .success();
    fake.flag("send-unreachable", true);
    let failure = env
        .instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "close", "--message", "CONFIDENTIAL_PEER_BODY"])
        .assert()
        .failure();
    insta::assert_snapshot!(diagnostic_reference(&String::from_utf8_lossy(&failure.get_output().stderr)),@"instrukt: conversation link: native send failed: t3 send: native server unreachable (ECONNREFUSED) [diagnostic:REF]; handover remains uncertain");
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let status = links::status(&home).unwrap().remove(0);
    assert!(status.active);
    assert!(status.directions.iter().any(|d| d.uncertain));
    insta::assert_snapshot!(
        "management-failure-correlation",
        discussion_log(&env, "discuss", &status.id)
    );
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn native_observation_failure_keeps_its_transport_cause_without_resending() {
    let env = common::TestEnv::new();
    let _store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "parent", "Local opening.");
    fake.seed("child", env.home(), "child", "Local opening.");
    fake.flag("die-after-send", true);
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args([
            "discuss",
            "open",
            "child",
            "--message",
            "Opening contribution.",
        ])
        .assert()
        .failure();
    fake.server_down();
    let failure = env
        .instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "open", "child"])
        .assert()
        .failure();
    insta::assert_snapshot!(diagnostic_reference(&String::from_utf8_lossy(&failure.get_output().stderr)),@"instrukt: conversation link: native acceptance observation failed: t3 show: native transport failure (ECONNREFUSED) [diagnostic:REF]; handover remains uncertain");
    assert_eq!(fake.message_count("child"), 2);
}

fn discussion_log(env: &common::TestEnv, command: &str, link: &str) -> String {
    let log = std::fs::read_to_string(
        env.home()
            .join(format!(".local/state/instrukt-ai/instrukt/{command}.log")),
    )
    .unwrap();
    log.lines()
        .filter(|line| line.contains("event=conversation_") || line.contains("event=discussion_"))
        .map(|line| {
            let line = line.split_once(' ').unwrap().1.replace(link, "[LINK]");
            let mut line = diagnostic_reference(&line);
            if let Some(start) = line.find("position=") {
                let start = start + "position=".len();
                let end = line[start..]
                    .find(' ')
                    .map_or(line.len(), |end| start + end);
                let position = &line[start..end];
                assert_eq!(position.len(), 64);
                assert!(position.bytes().all(|b| b.is_ascii_hexdigit()));
                line.replace_range(start..end, "[POSITION]");
            }
            if let Some(start) = line.find("duration_ms=") {
                let start = start + "duration_ms=".len();
                let end = line[start..]
                    .find(' ')
                    .map_or(line.len(), |end| start + end);
                line.replace_range(start..end, "[MS]");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL2
#[test]
fn membership_and_automatic_delivery_have_body_free_boundary_summaries() {
    let env = common::TestEnv::new();
    let store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "parent", "Local opening.");
    fake.seed("child", env.home(), "child", "Local opening.");
    for _ in 0..2 {
        env.instrukt(env.home())
            .env("T3CODE_THREAD_ID", "parent")
            .args(["discuss", "open", "child"])
            .assert()
            .success();
    }
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let id = links::status(&home).unwrap()[0].id.clone();
    store.turn(
        "child",
        "logged-turn-1",
        1,
        "PRIVATE HUMAN TEXT",
        "PRIVATE ASSISTANT TEXT",
    );
    store.turn(
        "child",
        "logged-turn-2",
        2,
        "PRIVATE SECOND HUMAN TEXT",
        "PRIVATE SECOND ASSISTANT TEXT",
    );
    let runner = Running(
        env.instrukt_process(env.home())
            .arg("run")
            .env("INSTRUKT_LOG_LEVEL", "debug")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for(|| {
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .any(|d| d.delivered_turn.as_deref() == Some("logged-turn-2"))
    });
    // Close acquires the same link lock, after the successful delivery summary.
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "close"])
        .assert()
        .success();
    insta::assert_snapshot!(
        "membership-operation-log",
        discussion_log(&env, "discuss", &id)
    );
    insta::assert_snapshot!("automatic-delivery-log", discussion_log(&env, "run", &id));
    drop(runner);
}

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
#[test]
fn source_failure_before_a_pending_turn_reports_each_affected_direction_and_position() {
    let env = common::TestEnv::new();
    let store = history::T3Store::new(&env);
    let fake = runtime::FakeT3::create(&env);
    fake.seed("parent", env.home(), "parent", "Local opening.");
    fake.seed("child", env.home(), "child", "Local opening.");
    env.instrukt(env.home())
        .env("T3CODE_THREAD_ID", "parent")
        .args(["discuss", "open", "child"])
        .assert()
        .success();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let id = links::status(&home).unwrap()[0].id.clone();
    let runner = Running(
        env.instrukt_process(env.home())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for(|| {
        std::fs::read_to_string(env.home().join(".local/state/instrukt-ai/instrukt/run.log"))
            .is_ok_and(|log| log.contains("event=runner_started"))
    });
    instrukt_core::state_db::rusqlite::Connection::open(&store.file).unwrap()
        .execute_batch("ALTER TABLE orchestration_v2_projection_messages RENAME COLUMN role TO unsupported_role").unwrap();
    wait_for(|| {
        let link = links::status(&home).unwrap().remove(0);
        link.failure.is_some() || link.directions.iter().all(|d| d.failure.is_some())
    });
    insta::assert_snapshot!(
        "source-failure-correlation",
        discussion_log(&env, "run", &id).replace(env.home().to_str().unwrap(), "[HOME]")
    );
    let status = env
        .run(env.home(), ["events", "status", "--json"])
        .success();
    let public: serde_json::Value = serde_json::from_slice(&status.get_output().stdout).unwrap();
    assert!(
        public["links"][0]["failure"].is_string()
            || public["links"][0]["directions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|d| d["failure"].is_string())
    );
    drop(runner);
}
