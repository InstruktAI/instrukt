//! A developer's own engine (`docs/project/spec/feature/blueprint/graph-engine.md`):
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

// spec: project/instrukt/spec/feature/blueprint/graph-engine UC-BE1
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

// spec: project/instrukt/spec/feature/blueprint/graph-engine UC-BE1
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

// spec: project/instrukt/spec/feature/blueprint/graph-engine UC-BE2
#[test]
fn shipped_generation_and_questions_use_home_binary_and_ignore_path_graphify() {
    let mut env = machine();
    let path_tool = script(&env, "graphify", "touch \"$HOME/path-tool-ran\"; exit 9\n");
    let home_tool = env.instrukt_home().join("tools/bin/graphify");
    std::fs::create_dir_all(home_tool.parent().unwrap()).unwrap();
    let tool = script(
        &env,
        "installed-graphify",
        r#"case "$1" in
update) mkdir -p "$2/graphify-out"; cat "$2/map.json" > "$2/graphify-out/graph.json";;
query|explain) printf '%s\n' "$1"; cat "$4";;
*) exit 8;;
esac
"#,
    );
    std::fs::copy(tool, &home_tool).unwrap();
    let old_path = env.injected("PATH").unwrap().to_owned();
    env.set_env(
        "PATH",
        &format!("{}:{old_path}", path_tool.parent().unwrap().display()),
    );
    // Home accepts a relative override; engine cwd changes must not redirect it.
    env.set_env("INSTRUKT_HOME", "../home/.instrukt");
    let repo = project(&env, None);
    let one = revision(&repo, "one", Some(&mapped()));
    push(&env, &repo);
    assert_eq!(wait_generated(&env, &one).state, GenerationState::Mapped);
    for question in ["query", "explain"] {
        let output = answer(&env, &repo, &["blueprint", question, "question"], &[]);
        assert!(!output.is_empty());
    }
    assert!(!env.home().join("path-tool-ran").exists());
    std::fs::remove_file(home_tool).unwrap();
    let two = revision(&repo, "two", Some(&mapped()));
    push(&env, &repo);
    assert!(wait_generated(&env, &two).is_failure());
    assert!(!env.home().join("path-tool-ran").exists());
}

/// Dedicated CI contract test: ordinary checks never enable this feature, so
/// only this job needs network access and a real Graphify installation.
// spec: project/instrukt/spec/feature/blueprint/graph-engine UC-BE2
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI12
#[cfg(feature = "graphify-contract")]
#[test]
fn real_pinned_graphify_contract() {
    let mut env = machine();
    env.set_env("UV_CACHE_DIR", env.path("uv-cache").to_str().unwrap());
    let repo = project(&env, None);
    repo.write(
        "instrukt.yaml",
        "schema_version: 4\nproject: demo\nmodules: {blueprint: true, history: false}\n",
    );
    repo.write(
        "sample.py",
        "def callee():\n    return 1\n\ndef caller():\n    return callee()\n",
    );
    let one = repo.commit("feat: real code-only contract fixture");
    env.run(repo.path(), ["sync"]).success();
    env.run(repo.path(), ["install", "--module", "blueprint"])
        .success();
    let inspection = common::LiveGuard::over(vec![], vec![env.home().into()]);
    env.run(repo.path(), ["install", "--check", "--module", "blueprint"])
        .success();
    assert!(
        inspection.changed().is_empty(),
        "{:?}",
        inspection.changed()
    );
    push(&env, &repo);
    assert_eq!(wait_generated(&env, &one).state, GenerationState::Mapped);
    let latest = mapped::store(&env).latest().unwrap().unwrap();
    assert_eq!(latest.sha, mapped::sha(&one));
    assert!(!latest.graph.nodes().is_empty());
    assert!(!latest.graph.links().is_empty());
}
