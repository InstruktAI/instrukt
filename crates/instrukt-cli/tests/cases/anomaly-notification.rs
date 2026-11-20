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

    insta::assert_snapshot!(project.rt.notifications().join("\n"), @"t3:s1");
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
        SELECT 't3','s1','recovery:' || session || ':' || workflow_state,'asap','interrupted recovery','accepted','recovery',
        '2030-01-06T09:01:00Z','2030-01-06T09:01:00Z',1,'recovery:' || session || ':' || workflow_state,'interrupted recovery'
        FROM executions WHERE session='t3:s1'",
        [],
    ).unwrap();
    project.tick_at("2030-01-06T09:04:20Z");
    let report = status(&project);
    assert_eq!(report["notification_configured"], false);
    assert_eq!(report["stopped_executions"][0]["session"], "t3:s1");
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
fn a_confirmed_gone_thread_is_reported_once_after_its_recovery_send_fails() {
    let mut project = dispatched_task();
    let thread = project.rt.state_dir().join("s1.json");
    std::fs::remove_file(&thread).unwrap();
    project.tick_at("2030-01-06T09:01:20Z");
    project.tick_at("2030-01-06T09:02:20Z");
    insta::assert_snapshot!(project.rt.notifications().join("\n"), @"t3:s1");
    assert!(!thread.exists(), "the missing thread was never replaced");
}
