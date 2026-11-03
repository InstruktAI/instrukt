//! `local/projects/instrukt/spec/feature/runner/work-dispatch`: a wake-up
//! dispatches eligible work into the procedure declaring its lane's
//! orchestrator role when the project chose automatic pickup, and leaves
//! it for a deliberate start otherwise; with both the todos and events
//! modules off, the runner's commands refuse and name both switches.
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

use common::TestEnv;
use flavor::Tracked;
use runtime::{FakeRuntime, global_layer};

// spec: local/projects/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn with_automatic_pickup_eligible_work_is_dispatched_to_an_agent() {
    let project = Tracked::new(true);
    project.todo([
        "create",
        "tidy-imports",
        "Sort the imports of every module.",
        "--kind",
        "change",
        "--approved",
    ]);

    project.tick();

    assert_eq!(project.rt.calls().len(), 1, "{:?}", project.rt.calls());
    insta::assert_snapshot!("dispatched-opening", project.rt.opening("s1"));
    insta::assert_snapshot!(
        "dispatched-record",
        work::redact(&project.todo(["show", "tidy-imports"]))
    );
}

fn create_change(project: &Tracked) {
    project.todo([
        "create",
        "tidy-imports",
        "Sort the imports of every module.",
        "--kind",
        "change",
        "--approved",
    ]);
}

// spec: local/projects/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn a_project_choosing_automatic_pickup_in_its_own_layer_is_dispatched_by_the_machines_wake_up() {
    // The machine's global layer leaves work tracking off; only the
    // project's own layer switches it on.
    let project = Tracked::with_global_modules(true, "todos: false");
    create_change(&project);

    project.tick();

    assert_eq!(project.rt.calls().len(), 1, "{:?}", project.rt.calls());
}

// spec: local/projects/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn work_claimed_while_github_is_unreachable_is_dispatched_once_github_answers() {
    let mut project = Tracked::new(true);
    create_change(&project);

    project.github.stop();
    project.tick();
    assert!(project.rt.calls().is_empty(), "{:?}", project.rt.calls());

    project.github.resume();
    project.tick();
    assert_eq!(project.rt.calls().len(), 1, "{:?}", project.rt.calls());
    insta::assert_snapshot!(
        "dispatched-after-pending-claim",
        work::redact(&project.todo(["show", "tidy-imports"]))
    );
}

// spec: local/projects/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn with_deliberate_pickup_eligible_work_awaits_the_developers_start() {
    let project = Tracked::new(false);
    project.todo([
        "create",
        "tidy-imports",
        "Sort the imports of every module.",
        "--kind",
        "change",
        "--approved",
    ]);

    project.tick();

    assert!(project.rt.calls().is_empty(), "{:?}", project.rt.calls());
}

// spec: local/projects/instrukt/spec/feature/runner/work-dispatch UC-RD2
#[test]
fn with_work_tracking_and_event_responses_off_the_runner_refuses_and_names_both() {
    let env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", false);
    global_layer(&env, "todos: false, events: false", &[&rt]);
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", "schema_version: 1\nproject: demo\n");
    repo.commit("the developer's project");
    for args in [
        vec!["tick"],
        vec!["run"],
        vec!["sessions", "send", "rt:s1", "hello"],
    ] {
        let refused = env.run(repo.path(), &args).failure();
        insta::assert_snapshot!(
            format!("refused-{}", args[0]),
            String::from_utf8_lossy(&refused.get_output().stderr)
        );
    }
    assert!(rt.calls().is_empty(), "{:?}", rt.calls());
}
