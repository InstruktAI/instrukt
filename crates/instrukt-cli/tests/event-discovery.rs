//! Catalog discovery through the real binary, without event infrastructure.
mod common;

use common::TestEnv;

// spec: project/instrukt/spec/feature/runner/event-discovery UC-ED1
#[test]
fn declared_events_are_discoverable_without_configuration_or_transport() {
    let env = TestEnv::new();
    let run = env.run(&env.path(""), ["events", "list"]).success();
    insta::assert_snapshot!(String::from_utf8_lossy(&run.get_output().stdout));
}

// spec: project/instrukt/spec/feature/runner/event-discovery UC-ED2
#[test]
fn schema_describes_the_complete_producer_contract() {
    let env = TestEnv::new();
    let run = env
        .run(&env.path(""), ["events", "schema", "runner.ci_completed"])
        .success();
    insta::assert_snapshot!(String::from_utf8_lossy(&run.get_output().stdout));
}

// spec: project/instrukt/spec/feature/runner/event-discovery UC-ED3
#[test]
fn the_owner_filter_selects_only_that_vocabulary() {
    let env = TestEnv::new();
    let run = env
        .run(&env.path(""), ["events", "list", "--domain", "workflow"])
        .success();
    insta::assert_snapshot!(String::from_utf8_lossy(&run.get_output().stdout));
}

// spec: project/instrukt/spec/feature/runner/event-discovery UC-ED2
#[test]
fn an_unknown_schema_names_the_requested_type_in_its_refusal() {
    let env = TestEnv::new();
    let run = env
        .run(&env.path(""), ["events", "schema", "external.undeclared"])
        .failure();
    insta::assert_snapshot!(String::from_utf8_lossy(&run.get_output().stderr));
}

// spec: project/instrukt/spec/feature/runner/event-discovery UC-ED3
#[test]
fn an_unknown_owner_has_an_empty_catalog() {
    let env = TestEnv::new();
    let run = env
        .run(&env.path(""), ["events", "list", "--domain", "external"])
        .success();
    assert!(run.get_output().stdout.is_empty());
}

// spec: project/instrukt/spec/feature/runner/event-discovery UC-ED2
#[test]
fn the_dead_letter_schema_describes_an_act_without_a_lifecycle_state() {
    let env = TestEnv::new();
    let run = env
        .run(
            &env.path(""),
            ["events", "schema", "instrukt.system.event.dead_lettered"],
        )
        .success();
    insta::assert_snapshot!(String::from_utf8_lossy(&run.get_output().stdout));
}
