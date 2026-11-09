#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]
mod common;
use instrukt_runner::{
    envelopes::{self, Span},
    links,
};

// spec: project/instrukt/spec/feature/runner/conversation-links UC-CL1
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS1
#[test]
fn recorded_human_spans_keep_attribution_and_never_echo_to_their_source() {
    let text = "clarification\n[Peer (t3:scope:b · agent:codex/model · unknown)]\npeer reply\n[/Peer (t3:scope:b · agent:codex/model · unknown)]\ntrailing words";
    let output = links::human_contributions("t3:scope:a", "t3:scope:b", text, None);
    assert_eq!(output.len(), 2);
    assert!(output.iter().all(
        |s| matches!(&envelopes::segment(s)[0],Span::Identity{source,..} if *source=="t3:scope:a")
    ));
}

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
    assert_eq!(public[0]["active"], false);
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
    let runner = run();
    // Observe startup catch-up through the public status rather than waiting a guessed delay.
    wait_for(|| {
        links::status(&home).unwrap()[0]
            .directions
            .iter()
            .any(|d| d.delivered_turn.as_deref() == Some("empty-run"))
    });
    assert_eq!(fake.message_count("parent"), 3);
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
