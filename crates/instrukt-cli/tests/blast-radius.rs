//! `instrukt blueprint affected` — blast radius from the latest graph
//! (`docs/local/spec/feature/blueprint/blast-radius.md`).
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

    fn located(mut self, id: &str, file: &str, label: &str, location: &str) -> Self {
        self.nodes.push(json!({
            "id": id, "source_file": file, "label": label, "source_location": location
        }));
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

/// A project `demo` with the blueprint module switched as given.
fn project(env: &TestEnv, blueprint: bool) -> Repo {
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        &format!("schema_version: 1\nproject: demo\nmodules:\n  blueprint: {blueprint}\n"),
    );
    repo.write("src/a.rs", "");
    repo.commit("feat: start");
    repo
}

fn store(env: &TestEnv) -> GraphStore {
    GraphStore::new(
        &Home::at_user_home(env.home().to_path_buf()),
        &ProjectSlug::parse("demo").unwrap(),
    )
}

fn seed(env: &TestEnv, sha: &str, graph: Value) {
    let graph = Graph::from_value(graph).unwrap();
    store(env)
        .publish(&CommitSha::parse(sha).unwrap(), &graph)
        .unwrap();
}

fn stdout(assert: assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

fn mapped() -> Value {
    GraphBuilder::default()
        .located("a::run", "src/a.rs", "run", "L1-9")
        .node("a_tests", "src/a.rs")
        .located("b::helper", "src/b.rs", "helper", "L3")
        .located("b::early", "src/b.rs", "early", "L1")
        .node("c::main", "src/c.rs")
        .node("d::unrelated", "src/d.rs")
        .node("src/e.rs", "src/e.rs")
        .link("b::helper", "a::run", "calls")
        .link("b::early", "a_tests", "references")
        .link("c::main", "b::helper", "imports")
        .link("a::run", "d::unrelated", "calls")
        .link("src/e.rs", "a::run", "contains")
        .build()
}

// spec: local/projects/instrukt/spec/feature/blueprint/blast-radius UC-BA1
#[test]
fn an_agent_learns_what_depends_on_the_files_it_changes() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    seed(&env, SHA, mapped());
    let absolute = repo.path().join("src/a.rs");
    let sub = repo.path().join("src");
    std::fs::create_dir_all(&sub).unwrap();

    // Relative arguments are relative to the repository root, wherever the
    // caller stands; an absolute path inside the root is made relative.
    let out = stdout(
        env.run(
            &sub,
            [
                "blueprint".as_ref(),
                "affected".as_ref(),
                absolute.as_os_str(),
                "src/new.rs".as_ref(),
            ],
        )
        .success(),
    );
    insta::assert_snapshot!(out, @r"
    blueprint graph of 0123456789abcdef0123456789abcdef01234567
    dependents of 2 changed files:
      src/b.rs:1 early
      src/b.rs:3 helper
      src/c.rs c::main
    files the graph does not know (read them by hand):
      src/new.rs
    ");
}

// spec: local/projects/instrukt/spec/feature/blueprint/blast-radius UC-BA1
#[test]
fn files_with_no_dependents_say_so_and_name_the_revision() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    seed(&env, SHA, mapped());
    let out = stdout(
        env.run(
            repo.path(),
            ["blueprint", "affected", "src/c.rs", "./src/e.rs"],
        )
        .success(),
    );
    insta::assert_snapshot!(out, @r"
    blueprint graph of 0123456789abcdef0123456789abcdef01234567
    no dependents found for 2 changed files
    ");
}

// spec: local/projects/instrukt/spec/feature/blueprint/blast-radius UC-BA1
#[test]
fn the_latest_graph_answers_after_a_newer_one_is_published() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    seed(&env, SHA, mapped());
    let newer = "fedcba9876543210fedcba9876543210fedcba98";
    seed(
        &env,
        newer,
        GraphBuilder::default()
            .node("a", "src/a.rs")
            .node("z", "src/z.rs")
            .link("z", "a", "uses")
            .build(),
    );
    let out = stdout(
        env.run(repo.path(), ["blueprint", "affected", "src/a.rs"])
            .success(),
    );
    insta::assert_snapshot!(out, @r"
    blueprint graph of fedcba9876543210fedcba9876543210fedcba98
    dependents of 1 changed file:
      src/z.rs z
    ");
}

// spec: local/projects/instrukt/spec/feature/blueprint/blast-radius UC-BA1
#[test]
fn with_no_mapped_revision_the_agent_is_told_to_explore_by_hand() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    let out = stdout(
        env.run(repo.path(), ["blueprint", "affected", "src/a.rs"])
            .success(),
    );
    insta::assert_snapshot!(out, @"blueprint unavailable: no graph of this project to answer from; explore by hand");
}

// spec: local/projects/instrukt/spec/feature/blueprint/blast-radius UC-BA1
#[test]
fn an_unreadable_graph_is_answered_as_no_graph() {
    let env = TestEnv::new();
    let repo = project(&env, true);
    seed(&env, SHA, mapped());
    let graph_file = store(&env).dir().join(SHA).join("graph.json");
    std::fs::write(
        &graph_file,
        r#"{"nodes": [{"id": "x", "source_file": "/abs.rs"}], "links": []}"#,
    )
    .unwrap();
    let out = stdout(
        env.run(repo.path(), ["blueprint", "affected", "src/a.rs"])
            .success(),
    );
    assert_eq!(
        out,
        "blueprint unavailable: no graph of this project to answer from; explore by hand\n"
    );
}

// spec: local/projects/instrukt/spec/feature/blueprint/blast-radius UC-BA1
#[test]
fn with_the_module_off_the_command_refuses_and_names_the_switch() {
    let env = TestEnv::new();
    let repo = project(&env, false);
    seed(&env, SHA, mapped());
    let assert = env
        .run(repo.path(), ["blueprint", "affected", "src/a.rs"])
        .failure();
    let output = assert.get_output();
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("blueprint module is off"), "{stderr}");
    assert!(stderr.contains("modules.blueprint"), "{stderr}");
}
