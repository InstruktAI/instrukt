//! `instrukt blueprint path` — how one part reaches another in the latest
//! graph (`docs/project/spec/feature/blueprint/graph-navigation.md`).
//!
//! Generation is not part of these tests: each seeds the project's store in
//! the temporary home through the store's own publication.
#![allow(
    clippy::unwrap_used,
    reason = "test setup helpers: a failed setup step fails the test"
)]

mod common;

use common::{Repo, TestEnv};
use instrukt_blueprint::{Graph, GraphStore};
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use serde_json::{Value, json};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// A graph document built node by node and link by link.
#[derive(Default)]
struct GraphBuilder {
    nodes: Vec<Value>,
    links: Vec<Value>,
}

impl GraphBuilder {
    fn node(mut self, id: &str, file: &str) -> Self {
        self.nodes.push(json!({"id": id, "source_file": file}));
        self
    }

    fn link(mut self, source: &str, target: &str, relation: &str) -> Self {
        self.links
            .push(json!({"source": source, "target": target, "relation": relation}));
        self
    }

    fn build(self) -> Value {
        json!({"nodes": self.nodes, "links": self.links})
    }
}

fn project(env: &TestEnv, blueprint: bool) -> Repo {
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        &format!("schema_version: 5\nproject: demo\nmodules:\n  blueprint: {blueprint}\n"),
    );
    repo.commit("feat: start");
    repo
}

fn seed(env: &TestEnv, graph: Value) {
    let store = GraphStore::new(
        &Home::at_user_home(env.home().to_path_buf()),
        &ProjectSlug::parse("demo").unwrap(),
    );
    let published = store
        .publish(
            &CommitSha::parse(SHA).unwrap(),
            &Graph::from_value(graph).unwrap(),
        )
        .unwrap();
    assert!(published.cleanup_failures.is_empty());
}

fn stdout(assert: assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

fn mapped() -> Value {
    GraphBuilder::default()
        .node("cli::main", "src/main.rs")
        .node("cli::run", "src/main.rs")
        .node("api::serve", "src/api.rs")
        .node("db::open", "src/db.rs")
        .node("db::pool", "src/db.rs")
        .link("cli::main", "cli::run", "calls")
        .link("cli::run", "api::serve", "calls")
        .link("api::serve", "db::pool", "uses")
        .link("src/db.rs", "db::open", "contains")
        .link("cli::main", "db::open", "imports_from")
        .build()
}

// spec: project/instrukt/spec/feature/blueprint/graph-navigation UC-BN1
#[test]
fn an_agent_finds_the_chain_from_one_part_to_another() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    seed(&env, mapped());
    let out = stdout(
        env.run(repo.path(), ["blueprint", "path", "cli::run", "db::pool"])
            .success(),
    );
    insta::assert_snapshot!(out, @r"
    blueprint graph of 0123456789abcdef0123456789abcdef01234567
    path from cli::run to db::pool:
      cli::run -[calls]-> api::serve
      api::serve -[uses]-> db::pool
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-navigation UC-BN1
#[test]
fn endpoints_may_be_files() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    seed(&env, mapped());
    let out = stdout(
        env.run(
            repo.path(),
            ["blueprint", "path", "src/main.rs", "src/db.rs"],
        )
        .success(),
    );
    insta::assert_snapshot!(out, @r"
    blueprint graph of 0123456789abcdef0123456789abcdef01234567
    path from src/main.rs to src/db.rs:
      cli::main -[imports_from]-> db::open
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-navigation UC-BN1
#[test]
fn an_unreachable_part_is_answered_as_such() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    seed(&env, mapped());
    let out = stdout(
        env.run(repo.path(), ["blueprint", "path", "db::pool", "cli::main"])
            .success(),
    );
    insta::assert_snapshot!(out, @r"
    blueprint graph of 0123456789abcdef0123456789abcdef01234567
    cli::main is not reachable from db::pool
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-navigation UC-BN1
#[test]
fn an_endpoint_naming_no_node_is_refused_as_unknown() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    seed(&env, mapped());
    let assert = env
        .run(repo.path(), ["blueprint", "path", "cli::main", "nowhere"])
        .failure();
    let output = assert.get_output();
    assert!(output.stdout.is_empty());
    insta::assert_snapshot!(String::from_utf8_lossy(&output.stderr), @"instrukt: in the blueprint graph of 0123456789abcdef0123456789abcdef01234567: unknown endpoint `nowhere`: it names no node id or source file in the graph");
}

// spec: project/instrukt/spec/feature/blueprint/graph-navigation UC-BN1
#[test]
fn with_no_graph_or_the_module_off_path_answers_as_blast_radius_does() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    let out = stdout(
        env.run(repo.path(), ["blueprint", "path", "a", "b"])
            .success(),
    );
    insta::assert_snapshot!(out, @"blueprint unavailable: no graph of this project to answer from; explore by hand");

    let env = TestEnv::new();
    let repo = project(&env, false);
    let assert = env
        .run(repo.path(), ["blueprint", "path", "a", "b"])
        .failure();
    insta::assert_snapshot!(String::from_utf8_lossy(&assert.get_output().stderr), @"instrukt: the blueprint module is off; switch it on with `modules.blueprint: true` in instrukt.yaml");
}

/// Installs a process-line Graphify stand-in in this temporary instrukt home's
/// owned binary directory: it prints the question it was asked, the arguments it
/// received and how many nodes the graph it was given holds.
fn with_graphify(env: &TestEnv) {
    use std::os::unix::fs::PermissionsExt as _;

    let bin = env.instrukt_home().join("tools/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let graphify = bin.join("graphify");
    std::fs::write(
        &graphify,
        "#!/bin/sh\nset -eu\nquestion=\"$1\"; shift\ngraph=\"\"; asked=\"\"\n\
         while [ $# -gt 0 ]; do\n\
         \x20 case \"$1\" in\n\
         \x20   --graph) graph=\"$2\"; shift 2 ;;\n\
         \x20   *) asked=\"$asked [$1]\"; shift ;;\n\
         \x20 esac\n\
         done\n\
         nodes=$(grep -o '\"id\":\"[^\"]*\"' \"$graph\" | wc -l | tr -d ' ')\n\
         echo \"graphify $question$asked over $nodes nodes\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&graphify, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// spec: project/instrukt/spec/feature/blueprint/graph-navigation UC-BN2
#[test]
fn a_free_query_is_forwarded_to_the_shipped_engine_over_the_latest_graph() {
    let env = TestEnv::new();
    with_graphify(&env);
    let repo = project(&env, true);
    seed(&env, mapped());
    let out = stdout(
        env.run(
            repo.path(),
            [
                "blueprint",
                "query",
                "how does the cli reach the db",
                "--dfs",
            ],
        )
        .success(),
    );
    insta::assert_snapshot!(out, @r"
    blueprint graph of 0123456789abcdef0123456789abcdef01234567
    graphify query [how does the cli reach the db] [--dfs] over 5 nodes
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-navigation UC-BN2
#[test]
fn an_explanation_is_forwarded_to_the_shipped_engine_over_the_latest_graph() {
    let env = TestEnv::new();
    with_graphify(&env);
    let repo = project(&env, true);
    seed(&env, mapped());
    let out = stdout(
        env.run(repo.path(), ["blueprint", "explain", "api::serve"])
            .success(),
    );
    insta::assert_snapshot!(out, @r"
    blueprint graph of 0123456789abcdef0123456789abcdef01234567
    graphify explain [api::serve] over 5 nodes
    ");
}

// spec: project/instrukt/spec/feature/blueprint/graph-navigation UC-BN2
#[test]
fn an_engine_that_answers_no_free_questions_says_so() {
    let env = TestEnv::new();
    // The machine's own engine command: it offers only what the contract
    // supports.
    let global = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(global.parent().unwrap()).unwrap();
    std::fs::write(
        &global,
        "schema_version: 1\nblueprint:\n  engine: own-engine\n",
    )
    .unwrap();
    let repo = project(&env, true);
    seed(&env, mapped());
    let query = stdout(
        env.run(
            repo.path(),
            ["blueprint", "query", "how does the cli reach the db"],
        )
        .success(),
    );
    insta::assert_snapshot!(query, @"the configured engine does not offer `query`");
    let explain = stdout(
        env.run(repo.path(), ["blueprint", "explain", "api::serve"])
            .success(),
    );
    insta::assert_snapshot!(explain, @"the configured engine does not offer `explain`");
}
