//! Graph generation per pushed revision of main
//! (`docs/project/spec/feature/blueprint/graph-generation.md`): a push hands
//! the revision to a detached generation through instrukt's pre-push
//! section, answers follow the mapped revision and name it, and a failed
//! generation leaves the last complete map in place.
#![allow(
    clippy::unwrap_used,
    reason = "test setup helpers: a failed setup step fails the test"
)]

mod common;
#[path = "support/mapped.rs"]
mod mapped;

use instrukt_blueprint::GenerationState;
use instrukt_core::fsx::LockGuard;
use mapped::{
    Gate, answer, engine, last_generation, machine, machine_without_graphify, map, project, push,
    revision, sha, store, wait_generated, wait_until,
};
use serde_json::Value;

fn first_map() -> Value {
    map(
        &[("a::run", "src/a.rs"), ("b::call", "src/b.rs")],
        &[("b::call", "a::run", "calls")],
    )
}

fn second_map() -> Value {
    map(
        &[
            ("a::run", "src/a.rs"),
            ("b::call", "src/b.rs"),
            ("c::main", "src/c.rs"),
        ],
        &[
            ("b::call", "a::run", "calls"),
            ("c::main", "a::run", "uses"),
        ],
    )
}

// spec: project/instrukt/spec/feature/blueprint/graph-generation UC-BG1
#[test]
fn a_push_completes_before_the_map_and_answers_then_reflect_the_pushed_revision() {
    let env = machine();
    let gate = Gate::new(&env);
    let repo = project(&env, Some(&gate.engine));
    let one = revision(&repo, "one", Some(&first_map()));

    // The engine is held at its gate: the push returns regardless.
    push(&env, &repo);
    gate.wait_started("one");
    let names = [(one.as_str(), "one")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "status"], &names), @r"
    no blueprint graph of this project yet
    last generation: <one> is running
    ");

    gate.open();
    assert_eq!(wait_generated(&env, &one).state, GenerationState::Mapped);
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "affected", "src/a.rs"], &names), @r"
    blueprint graph of <one>
    dependents of 1 changed file:
      src/b.rs b::call
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-generation UC-BG1
#[test]
fn revisions_pushed_while_one_is_mapped_wait_and_only_the_newest_is_mapped_next() {
    let env = machine();
    let gate = Gate::new(&env);
    let repo = project(&env, Some(&gate.engine));
    let one = revision(&repo, "one", Some(&first_map()));
    push(&env, &repo);
    gate.wait_started("one");

    let two = revision(&repo, "two", Some(&first_map()));
    push(&env, &repo);
    let three = revision(&repo, "three", Some(&second_map()));
    push(&env, &repo);
    gate.open();

    // The newest is mapped next, once the running mapping ends: the gated
    // engine fails any run that starts while another is still running.
    assert_eq!(wait_generated(&env, &three).state, GenerationState::Mapped);
    assert!(
        !gate.started("two"),
        "the superseded revision was not mapped"
    );
    let store = store(&env);
    assert!(store.has_graph(&sha(&one)).unwrap());
    assert!(!store.has_graph(&sha(&two)).unwrap());
    let names = [(one.as_str(), "one"), (three.as_str(), "three")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "affected", "src/a.rs"], &names), @r"
    blueprint graph of <three>
    dependents of 1 changed file:
      src/b.rs b::call
      src/c.rs c::main
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-generation UC-BG1
#[test]
fn a_failed_generation_leaves_answers_on_the_last_complete_map_and_is_named() {
    let env = machine();
    let engine = engine(&env);
    let repo = project(&env, Some(&engine));
    let one = revision(&repo, "one", Some(&first_map()));
    push(&env, &repo);
    assert_eq!(wait_generated(&env, &one).state, GenerationState::Mapped);

    let two = revision(&repo, "two", None);
    push(&env, &repo);
    assert!(wait_generated(&env, &two).is_failure());

    let names = [(one.as_str(), "one"), (two.as_str(), "two")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "affected", "src/a.rs"], &names), @r"
    blueprint graph of <one>
    dependents of 1 changed file:
      src/b.rs b::call
    the newer generation of <two> failed: the graph engine exited with status 3: this revision carries no map; this answer is from <one>
    ");
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "status"], &names), @r"
    blueprint graph of <one>
    last generation: <two> failed: the graph engine exited with status 3: this revision carries no map
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-generation UC-BG1
#[test]
fn without_the_shipped_engine_installed_the_status_says_what_to_install() {
    let env = machine_without_graphify();
    let repo = project(&env, None);
    let one = revision(&repo, "one", Some(&first_map()));
    push(&env, &repo);
    wait_until("the generation to end", || {
        last_generation(&env).is_some_and(|last| last.is_failure())
    });

    let names = [(one.as_str(), "one")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "status"], &names), @r"
    no blueprint graph of this project yet
    last generation: <one> failed: the graph engine `graphify` is not installed; run `instrukt install` to install the release-pinned Graphify into the instrukt home
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-generation UC-BG1
#[test]
fn a_mapping_that_finished_but_is_not_yet_recorded_reads_as_running_then_mapped() {
    let env = machine();
    let gate = Gate::new(&env);
    gate.open();
    let repo = project(&env, Some(&gate.engine));
    let one = revision(&repo, "one", Some(&first_map()));
    // The store's publication lock, held here, stops the worker after its
    // engine has finished and before it records the outcome.
    let publication = LockGuard::acquire(
        &store(&env).dir().join(".lock"),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    push(&env, &repo);
    gate.wait_finished("one");

    let names = [(one.as_str(), "one")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "status"], &names), @r"
    no blueprint graph of this project yet
    last generation: <one> is running
    ");

    // From here the worker records the outcome and exits; every reading
    // across that is the running generation or the mapped one, never an
    // interrupted one.
    drop(publication);
    let mut seen = Vec::new();
    wait_until("the generation to be recorded", || {
        let state = last_generation(&env).unwrap().state;
        let done = state != GenerationState::Running;
        seen.push(state);
        done
    });
    assert_eq!(seen.last(), Some(&GenerationState::Mapped));
    assert!(
        seen.iter()
            .all(|state| matches!(state, GenerationState::Running | GenerationState::Mapped)),
        "{seen:?}"
    );
}

// spec: project/instrukt/spec/feature/blueprint/graph-generation UC-BG1
#[test]
fn an_interrupted_mapping_is_reported_and_answers_stay_on_the_last_complete_map() {
    let env = machine();
    let gate = Gate::new(&env);
    gate.open();
    let repo = project(&env, Some(&gate.engine));
    let one = revision(&repo, "one", Some(&first_map()));
    push(&env, &repo);
    assert_eq!(wait_generated(&env, &one).state, GenerationState::Mapped);

    gate.close();
    let two = revision(&repo, "two", Some(&second_map()));
    push(&env, &repo);
    gate.wait_started("two");
    gate.kill_worker("two");
    gate.abort();
    assert_eq!(
        wait_generated(&env, &two).state,
        GenerationState::Interrupted
    );

    let names = [(one.as_str(), "one"), (two.as_str(), "two")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "status"], &names), @r"
    blueprint graph of <one>
    last generation: <two> was interrupted
    ");
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "affected", "src/a.rs"], &names), @r"
    blueprint graph of <one>
    dependents of 1 changed file:
      src/b.rs b::call
    the newer generation of <two> was interrupted; this answer is from <one>
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-generation UC-BG1
#[test]
fn an_engine_left_running_by_an_interrupted_worker_is_never_joined_by_a_second() {
    let env = machine();
    let gate = Gate::new(&env);
    let repo = project(&env, Some(&gate.engine));
    let one = revision(&repo, "one", Some(&first_map()));
    push(&env, &repo);
    gate.wait_started("one");
    gate.kill_worker("one");

    // The next revision waits for the engine its interrupted worker left:
    // the gated engine fails any run started while another is running.
    let two = revision(&repo, "two", Some(&second_map()));
    push(&env, &repo);
    let names = [(one.as_str(), "one"), (two.as_str(), "two")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "status"], &names), @r"
    no blueprint graph of this project yet
    last generation: <one> is running without its worker; its engine is ended once past its time limit and a newer revision waits
    ");

    gate.open();
    assert_eq!(wait_generated(&env, &two).state, GenerationState::Mapped);
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "affected", "src/a.rs"], &names), @r"
    blueprint graph of <two>
    dependents of 1 changed file:
      src/b.rs b::call
      src/c.rs c::main
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-generation UC-BG1
#[test]
fn an_engine_left_hanging_past_its_time_limit_is_ended_for_the_next_revision() {
    let mut env = machine();
    env.set_env("INSTRUKT_NOW", "2000-01-01T00:00:00Z");
    let gate = Gate::new(&env);
    let repo = project(&env, Some(&gate.engine));
    let one = revision(&repo, "one", Some(&first_map()));
    push(&env, &repo);
    gate.wait_started("one");
    gate.kill_worker("one");
    let names_one = [(one.as_str(), "one")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "status"], &names_one), @r"
    no blueprint graph of this project yet
    last generation: <one> is running without its worker; its engine is ended once past its time limit and a newer revision waits
    ");

    // An hour later by the machine's clock the left engine is past its
    // limit: the next revision's worker ends it and maps its own.
    env.set_env("INSTRUKT_NOW", "2000-01-01T01:00:00Z");
    let two = revision(&repo, "two", Some(&second_map()));
    push(&env, &repo);
    gate.wait_started("two");
    let names = [(one.as_str(), "one"), (two.as_str(), "two")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "status"], &names), @r"
    no blueprint graph of this project yet
    last generation: <two> is running
    ");

    gate.open();
    assert_eq!(wait_generated(&env, &two).state, GenerationState::Mapped);
    assert!(
        !gate.finished("one"),
        "the left engine was ended, not finished"
    );
}

// spec: project/instrukt/spec/feature/workflow/integration-gate UC-IG4
// spec: project/instrukt/spec/feature/blueprint/graph-generation UC-BG1
#[test]
fn a_runner_fast_forward_hands_the_fetched_main_revision_to_blueprint() {
    let env = machine();
    let command = engine(&env);
    let repo = project(&env, Some(&command));
    let first = revision(&repo, "baseline", Some(&first_map()));
    repo.git(["push", "--quiet", "-u", "origin", "main"]);
    wait_generated(&env, &first);
    // The local runner considers the published project, even without events.
    let global = env.instrukt_home().join("instrukt.yaml");
    let mut text = std::fs::read_to_string(&global).unwrap();
    text.push_str("modules: {todos: true}\n");
    std::fs::write(global, text).unwrap();
    env.run(repo.path(), ["sync"]).success();
    if !repo.git(["status", "--porcelain"]).is_empty() {
        repo.commit("integrator: include generated baseline files");
        repo.git(["push", "--quiet", "origin", "main"]);
    }
    let other = env.clone_repo(&env.path("origin.git"), "integrator");
    let delivered = revision(&other, "integrated", Some(&second_map()));
    other.git(["push", "--quiet", "origin", "main"]);
    env.run(env.home(), ["tick"]).success();
    assert_eq!(repo.head(), delivered);
    assert_eq!(
        wait_generated(&env, &delivered).state,
        GenerationState::Mapped
    );
}
