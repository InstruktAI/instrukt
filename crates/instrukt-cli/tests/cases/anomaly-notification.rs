//! `project/instrukt/spec/feature/runner/anomaly-notification`: a
//! task whose execution failed, and waits in no conversation, is reported
//! once through the runtime's notification facility by reference to the
//! session holding it. A conversation still waiting for the developer
//! keeps the failure in its thread; a run that ended without recording any
//! outcome is asked once to finish, then reported.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::fixtures::flavor;
use flavor::Tracked;

/// A project with an authorized one-off task due at 09:00, dispatched by
/// the wake-up at its time into session `s1`. Anomaly reporting is left
/// at its default: the runtime's own notification, which the fake runtime
/// records.
fn dispatched_task() -> Tracked {
    let mut project = Tracked::new(true);
    project.authorized_task("weekly-status", "2030-01-06T09:00:00Z", &[]);
    project.tick_at("2030-01-06T09:00:20Z");
    assert_eq!(project.rt.started_count(), 1, "{:?}", project.rt.calls());
    project
}

fn check_configured_notification() {
    let mut project = dispatched_task();
    project.todo_in_session(
        "s1",
        [
            "stamp",
            "weekly-status",
            "--task-outcome",
            "error",
            "--reason",
            "the status channel refused the post",
        ],
    );

    // While the conversation waits for the developer, the failure is in
    // its thread: no notification.
    project.rt.set_status("s1", "needs-input");
    project.tick_at("2030-01-06T09:01:20Z");
    assert!(project.rt.notifications().is_empty());

    // The conversation ended: nobody waits on it.
    project.rt.set_status("s1", "ended failed");
    project.tick_at("2030-01-06T09:02:20Z");
    project.tick_at("2030-01-06T09:03:20Z");

    assert_eq!(
        project.rt.notifications(),
        [format!("t3:{}", project.rt.native_id("s1"))]
    );
}

// spec: project/instrukt/spec/feature/runner/anomaly-notification UC-AN1
#[test]
fn a_stopped_execution_is_visible_without_a_notification_command() {
    let mut project = dispatched_task();
    let path = project.env.instrukt_home().join("instrukt.yaml");
    let mut settings: serde_yaml_ng::Value =
        serde_yaml_ng::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    settings["runner"]
        .as_mapping_mut()
        .unwrap()
        .remove("notify");
    std::fs::write(path, serde_yaml_ng::to_string(&settings).unwrap()).unwrap();
    let status = |p: &Tracked| {
        let out = p
            .env
            .run(p.repo.path(), ["events", "status", "--json"])
            .success();
        serde_json::from_slice::<serde_json::Value>(&out.get_output().stdout).unwrap()
    };
    project.rt.set_status("s1", "ended ok");
    // The pump accepted the recovery before the execution observer recorded it.
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        project.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.execute(
        "INSERT INTO runtime_inputs(runtime,session,input_id,intent,content,state,kind,arrived_at,accepted_at,waiting,group_id,selected_content)
        SELECT 't3',?1,'recovery:' || session || ':' || workflow_state,'asap','interrupted recovery','accepted','recovery',
        '2030-01-06T09:01:00Z','2030-01-06T09:01:00Z',1,'recovery:' || session || ':' || workflow_state,'interrupted recovery'
        FROM executions WHERE session=?2",
        [project.rt.native_id("s1"),format!("t3:{}",project.rt.native_id("s1"))],
    ).unwrap();
    project.tick_at("2030-01-06T09:04:20Z");
    let report = status(&project);
    assert_eq!(report["notification_configured"], false);
    assert_eq!(
        report["stopped_executions"][0]["session"],
        format!("t3:{}", project.rt.native_id("s1"))
    );
    assert_eq!(report["stopped_executions"][0]["todo"], "weekly-status");
    assert!(project.rt.notifications().is_empty());
    project.todo_in_session(
        "s1",
        ["stamp", "weekly-status", "--task-outcome", "success"],
    );
    project.tick_at("2030-01-06T09:05:20Z");
    assert_eq!(
        status(&project)["stopped_executions"],
        serde_json::json!([])
    );
    check_configured_notification();
    check_backup_staleness_notification();
}

fn check_backup_staleness_notification() {
    let mut project = Tracked::new(true);
    let home = project.env.instrukt_home();
    let recipient = age::x25519::Identity::generate().to_public().to_string();
    let path = home.join("instrukt.yaml");
    let mut settings: serde_yaml_ng::Value =
        serde_yaml_ng::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    settings["schema_version"] =
        serde_yaml_ng::Value::from(instrukt_core::config::GLOBAL_SCHEMA_VERSION);
    settings["backup"] = serde_yaml_ng::from_str(&format!(
        "endpoint: https://s3.example.test\nregion: auto\nbucket: bkt\naccess_key_id: '${{BK_KEY}}'\nsecret_access_key: '${{BK_SECRET}}'\nage_recipient: {recipient}\n"
    ))
    .unwrap();
    std::fs::write(&path, serde_yaml_ng::to_string(&settings).unwrap()).unwrap();
    std::fs::write(home.join("secrets.env"), "BK_KEY=ak\nBK_SECRET=sk\n").unwrap();
    std::fs::create_dir_all(home.join("state")).unwrap();
    let mid = "22223333444455556666777788889999";
    std::fs::write(home.join("state/machine-id"), format!("{mid}\n")).unwrap();
    let state_file = home.join("state/backup-state.json");
    std::fs::write(
        &state_file,
        r#"{"schema_version":1,"first_attempt_at":"2030-01-06T03:00:00Z","last_attempt_at":"2030-01-06T03:00:00Z","last_outcome":"ok","last_success_at":"2030-01-06T03:00:00Z","last_generation":"g1","last_error":null}"#,
    )
    .unwrap();

    project.tick_at("2030-01-07T15:00:00Z");
    assert!(project.rt.notifications().is_empty());

    project.tick_at("2030-01-07T15:00:01Z");
    project.tick_at("2030-01-07T15:05:00Z");
    assert_eq!(project.rt.notifications(), [format!("backup:stale:{mid}")]);

    project.env.set_env("INSTRUKT_NOW", "2030-01-07T15:05:00Z");
    let out = project
        .env
        .run(project.repo.path(), ["events", "status", "--json"])
        .success();
    let report: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    insta::assert_snapshot!(
        serde_json::to_string_pretty(&report["backup"]).unwrap(),
        @r#"
    {
      "standing": "stale",
      "admitted": true,
      "stale": true,
      "last_attempt_at": "2030-01-06T03:00:00Z",
      "last_success_at": "2030-01-06T03:00:00Z",
      "last_generation": "g1",
      "last_error": null,
      "repair_door": "run `instrukt backup run` to publish a fresh generation, and run `instrukt install --check` to verify the daily backup timer"
    }
    "#
    );

    // Failed backup with an admission error on the machine layer still surfaces
    // the backup failure alongside its repair door without exposing secrets.
    std::fs::write(
        &state_file,
        r#"{"schema_version":1,"first_attempt_at":"2030-01-06T03:00:00Z","last_attempt_at":"2030-01-07T15:05:00Z","last_outcome":"failed","last_success_at":"2030-01-06T03:00:00Z","last_generation":"g1","last_error":"upload failed"}"#,
    )
    .unwrap();
    let saved_global = std::fs::read_to_string(&path).unwrap();
    let mut broken = saved_global.clone();
    broken.push_str("unknown_top_level_key: true\n");
    std::fs::write(&path, &broken).unwrap();
    let out_err = project
        .env
        .run(project.repo.path(), ["events", "status", "--json"])
        .success();
    let report_err: serde_json::Value =
        serde_json::from_slice(&out_err.get_output().stdout).unwrap();
    assert!(report_err["admission"]["error"].is_string());
    assert!(
        !serde_json::to_string(&report_err["backup"])
            .unwrap()
            .contains("BK_SECRET")
    );
    assert!(
        !serde_json::to_string(&report_err["backup"])
            .unwrap()
            .contains("sk")
    );
    insta::assert_snapshot!(
        serde_json::to_string_pretty(&report_err["backup"]).unwrap(),
        @r#"
    {
      "standing": "failed",
      "admitted": true,
      "stale": true,
      "last_attempt_at": "2030-01-07T15:05:00Z",
      "last_success_at": "2030-01-06T03:00:00Z",
      "last_generation": "g1",
      "last_error": "upload failed",
      "repair_door": "resolve the error in `last_error` and run `instrukt backup run` to publish a fresh generation; run `instrukt install --check` if scheduled runs are failing"
    }
    "#
    );
    std::fs::write(&path, saved_global).unwrap();

    std::fs::write(
        &state_file,
        r#"{"schema_version":1,"first_attempt_at":"2030-01-06T03:00:00Z","last_attempt_at":"2030-01-07T16:00:00Z","last_outcome":"ok","last_success_at":"2030-01-07T16:00:00Z","last_generation":"g2","last_error":null}"#,
    )
    .unwrap();
    project.tick_at("2030-01-07T16:01:00Z");
    assert!(!home.join("state/backup-staleness-notified.json").exists());
    project.env.set_env("INSTRUKT_NOW", "2030-01-07T16:01:00Z");
    let out_ok = project
        .env
        .run(project.repo.path(), ["events", "status", "--json"])
        .success();
    let report_ok: serde_json::Value = serde_json::from_slice(&out_ok.get_output().stdout).unwrap();
    insta::assert_snapshot!(
        serde_json::to_string_pretty(&report_ok["backup"]).unwrap(),
        @r#"
    {
      "standing": "ok",
      "admitted": true,
      "stale": false,
      "last_attempt_at": "2030-01-07T16:00:00Z",
      "last_success_at": "2030-01-07T16:00:00Z",
      "last_generation": "g2",
      "last_error": null,
      "repair_door": null
    }
    "#
    );
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn a_failed_recovery_send_retries_the_same_thread_without_starting_another() {
    let mut project = dispatched_task();
    project.rt.set_status("s1", "ended ok");
    let failure = project.rt.state_dir().join("send-failure");
    std::fs::write(&failure, "").unwrap();
    project.tick_at("2030-01-06T09:01:20Z");
    assert_eq!(project.rt.sent_count("s1"), 0);
    assert!(project.rt.notifications().is_empty());

    std::fs::remove_file(failure).unwrap();
    project.tick_at("2030-01-06T09:02:20Z");
    assert_eq!(project.rt.sent_count("s1"), 1);
    assert_eq!(project.rt.started_count(), 1);
    assert!(project.rt.notifications().is_empty());
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn a_confirmed_gone_thread_is_replaced_for_an_eligible_execution() {
    let mut project = dispatched_task();
    let fake = crate::fixtures::t3::FakeT3::create(&project.env);
    let old = fake.native_id("s1");
    fake.delete("s1");
    project.tick_at("2030-01-06T09:01:20Z");
    project.tick_at("2030-01-06T09:02:20Z");
    assert_eq!(project.rt.started_count(), 1);
    assert_ne!(fake.native_id("s2"), old);
    assert!(project.rt.notifications().is_empty());
}
