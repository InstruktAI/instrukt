//! `local/projects/instrukt/spec/feature/runner/anomaly-notification`: a
//! task whose execution failed waits in no conversation, so the next
//! wake-up reports it once through the runtime's notification facility,
//! by reference to the session holding the failure, and holds the task.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/flavor.rs"]
mod flavor;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod work;

use flavor::Tracked;
use jiff::Timestamp;

// spec: local/projects/instrukt/spec/feature/runner/anomaly-notification UC-AN1
#[test]
fn a_failed_task_is_reported_once_by_reference_to_the_session_holding_it() {
    // Anomaly reporting is left at its default: the runtime's own
    // notification, which the fake runtime records.
    let project = Tracked::new(true);
    let now = Timestamp::from_second(Timestamp::now().as_second()).unwrap();
    project.authorized_task("weekly-status", &now.to_string(), &[]);
    project.tick();
    assert_eq!(project.rt.calls().len(), 1, "{:?}", project.rt.calls());

    // The task's session records the failure and nobody is asked anything.
    project.todo([
        "stamp",
        "weekly-status",
        "--task-outcome",
        "error",
        "--reason",
        "the status channel refused the post",
    ]);
    assert!(project.rt.notifications().is_empty());

    project.tick();
    project.tick();

    insta::assert_snapshot!(project.rt.notifications().join("\n"), @"rt:s1");
    insta::assert_snapshot!(
        "failed-task-held",
        work::redact(&project.todo(["show", "weekly-status"]))
    );
    assert_eq!(project.rt.calls().len(), 1, "a held task is not run again");
}
