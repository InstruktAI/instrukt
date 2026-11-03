//! Graph generation per pushed revision of main
//! (`docs/local/spec/feature/blueprint/graph-generation.md`): a push hands
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

// spec: local/projects/instrukt/spec/feature/blueprint/graph-generation UC-BG1
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

// spec: local/projects/instrukt/spec/feature/blueprint/graph-generation UC-BG1
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

// spec: local/projects/instrukt/spec/feature/blueprint/graph-generation UC-BG1
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

// spec: local/projects/instrukt/spec/feature/blueprint/graph-generation UC-BG1
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
    last generation: <one> failed: the graph engine `graphify` is not installed; install Graphify (the `graphifyy` package), e.g. `uv tool install graphifyy` or `pipx install graphifyy`
    ");
}
