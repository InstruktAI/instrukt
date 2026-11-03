//! A developer's own engine (`docs/local/spec/feature/blueprint/graph-engine.md`):
//! a command configured in the machine's global layer receives a checkout of
//! the pushed revision and writes contract JSON; agents' answers come from its
//! output, and output that breaks the contract is never served.
#![allow(
    clippy::unwrap_used,
    reason = "test setup helpers: a failed setup step fails the test"
)]

mod common;
#[path = "support/mapped.rs"]
mod mapped;

use instrukt_blueprint::GenerationState;
use mapped::{answer, machine, map, project, push, revision, script, wait_generated};
use serde_json::{Value, json};

fn mapped() -> Value {
    map(
        &[
            ("cli::main", "src/main.rs"),
            ("cli::run", "src/main.rs"),
            ("db::open", "src/db.rs"),
        ],
        &[
            ("cli::main", "cli::run", "calls"),
            ("cli::run", "db::open", "imports"),
        ],
    )
}

// spec: local/projects/instrukt/spec/feature/blueprint/graph-engine UC-BE1
#[test]
fn answers_come_from_the_projects_own_engine_through_the_same_questions() {
    let env = machine();
    // The developer's tool: maps the checkout it is given, here by reading
    // the description the revision carries.
    let engine = script(&env, "own-engine", "cat \"$1/map.json\"\n");
    let repo = project(&env, Some(&engine));
    let one = revision(&repo, "one", Some(&mapped()));
    push(&env, &repo);
    assert_eq!(wait_generated(&env, &one).state, GenerationState::Mapped);

    let names = [(one.as_str(), "one")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "path", "cli::main", "src/db.rs"], &names), @r"
    blueprint graph of <one>
    path from cli::main to src/db.rs:
      cli::main -[calls]-> cli::run
      cli::run -[imports]-> db::open
    ");
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "affected", "src/db.rs"], &names), @r"
    blueprint graph of <one>
    dependents of 1 changed file:
      src/main.rs cli::main
      src/main.rs cli::run
    ");
}

// spec: local/projects/instrukt/spec/feature/blueprint/graph-engine UC-BE1
#[test]
fn output_breaking_the_contract_is_never_served_and_the_previous_map_stays() {
    let env = machine();
    let engine = script(&env, "own-engine", "cat \"$1/map.json\"\n");
    let repo = project(&env, Some(&engine));
    let one = revision(&repo, "one", Some(&mapped()));
    push(&env, &repo);
    assert_eq!(wait_generated(&env, &one).state, GenerationState::Mapped);

    let escaping = json!({
        "nodes": [{"id": "cli::main", "source_file": "../outside.rs"}],
        "links": []
    });
    let two = revision(&repo, "two", Some(&escaping));
    push(&env, &repo);
    assert!(wait_generated(&env, &two).is_failure());

    let names = [(one.as_str(), "one"), (two.as_str(), "two")];
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "path", "cli::main", "db::open"], &names), @r"
    blueprint graph of <one>
    path from cli::main to db::open:
      cli::main -[calls]-> cli::run
      cli::run -[imports]-> db::open
    ");
    insta::assert_snapshot!(answer(&env, &repo, &["blueprint", "status"], &names), @r"
    blueprint graph of <one>
    last generation: <two> failed: the graph engine's output was refused: node 0: `source_file` `../outside.rs` contains a `..` segment
    ");
}
