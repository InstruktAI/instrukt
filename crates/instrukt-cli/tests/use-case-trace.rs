//! `instrukt specs trace` — use cases joined with the structural graph
//! through the tests that bind them
//! (`docs/project/spec/feature/blueprint/use-case-trace.md`).
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
const SPEC_ID: &str = "project/demo/spec/feature/things/thing";
const SPEC_PATH: &str = "docs/project/spec/feature/things/thing.md";
const TESTS: &str = "crates/thing/tests/thing.rs";
const GADGET_ID: &str = "project/demo/spec/feature/gadgets/gadget";
const GADGET_PATH: &str = "docs/project/spec/feature/gadgets/gadget.md";

fn use_case(id: &str, title: &str) -> String {
    format!(
        "#### {id}: {title}\n\n```gherkin\nGiven a thing\nWhen it is used\nThen it works\n```\n\n"
    )
}

/// A project `demo` with the blueprint module on, a feature spec holding
/// UC-TH1 and UC-TH2, another in its own folder holding UC-GA1, and tests
/// binding them: `first_works` binds UC-TH1, `second_works` UC-TH2,
/// `both_work` both and `gadget_works` UC-GA1.
fn project(env: &TestEnv) -> Repo {
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: demo\nmodules:\n  blueprint: true\n",
    )
    .write(
        SPEC_PATH,
        &format!(
            "---\ndescription: A thing.\n---\n\n# Thing — Spec\n\n## What it is\n\nA thing.\n\n\
             ## Canonical fields\n\n### Use cases\n\n{}{}",
            use_case("UC-TH1", "first"),
            use_case("UC-TH2", "second")
        ),
    )
    .write(
        GADGET_PATH,
        &format!(
            "---\ndescription: A gadget.\n---\n\n# Gadget — Spec\n\n## What it is\n\nA gadget.\n\n\
             ## Canonical fields\n\n### Use cases\n\n{}",
            use_case("UC-GA1", "gadget")
        ),
    )
    .write(
        TESTS,
        &format!(
            "// spec: {SPEC_ID} UC-TH1\n#[test]\nfn first_works() {{}}\n\n\
             // spec: {SPEC_ID} UC-TH2\n#[test]\nfn second_works() {{}}\n\n\
             // spec: {SPEC_ID} UC-TH1\n// spec: {SPEC_ID} UC-TH2\n#[test]\nfn both_work() {{}}\n\n\
             // spec: {GADGET_ID} UC-GA1\n#[test]\nfn gadget_works() {{}}\n"
        ),
    );
    repo.commit("feat: a thing");
    repo
}

fn node(id: &str, label: &str, file: &str, location: &str) -> Value {
    json!({"id": id, "label": label, "source_file": file, "source_location": location})
}

fn link(source: &str, target: &str, relation: &str) -> Value {
    json!({"source": source, "target": target, "relation": relation})
}

/// The mapped structure: the tests' own nodes, a helper in the test file,
/// and the code they reach.
fn seed(env: &TestEnv) {
    let graph = json!({
        "nodes": [
            node("tests_first", "first_works()", TESTS, "L3"),
            node("tests_second", "second_works()", TESTS, "L7"),
            node("tests_both", "both_work()", TESTS, "L12"),
            node("tests_fixture", "fixture()", TESTS, "L15"),
            node("tests_gadget", "gadget_works()", TESTS, "L18"),
            node("thing_make", "make()", "src/thing.rs", "L1"),
            node("thing_check", "check()", "src/thing.rs", "L9"),
            node("store_open", "open()", "src/store.rs", "L4"),
            node("cli_main", "main()", "src/main.rs", "L1")
        ],
        "links": [
            link("tests_first", "tests_fixture", "calls"),
            link("tests_fixture", "thing_make", "calls"),
            link("thing_make", "store_open", "imports"),
            link("tests_second", "thing_check", "calls"),
            link("tests_both", "thing_check", "calls"),
            link("tests_gadget", "thing_check", "calls"),
            link("cli_main", "thing_make", "calls"),
            link("src/thing.rs", "thing_make", "contains")
        ]
    });
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

// spec: project/instrukt/spec/feature/blueprint/use-case-trace UC-ST1
#[test]
fn tracing_a_use_case_shows_its_tests_and_the_code_they_reach() {
    let env = TestEnv::new();
    let repo = project(&env);
    seed(&env);
    let out = stdout(
        env.run(repo.path(), ["specs", "trace", SPEC_ID, "UC-TH1"])
            .success(),
    );
    insta::assert_snapshot!(out, @r"
    trace of project/demo/spec/feature/things/thing UC-TH1 from the blueprint graph of 0123456789abcdef0123456789abcdef01234567
    tests binding it:
      crates/thing/tests/thing.rs::first_works
      crates/thing/tests/thing.rs::both_work
    code they reach:
      src/store.rs:4 open()
      src/thing.rs:1 make()
      src/thing.rs:9 check()
    ");
}

// spec: project/instrukt/spec/feature/blueprint/use-case-trace UC-ST2
#[test]
fn tracing_files_shows_the_use_cases_at_stake_by_spec_folder_and_the_files_without_any() {
    let env = TestEnv::new();
    let repo = project(&env);
    seed(&env);
    let absolute = repo.path().join("src/thing.rs");
    let out = stdout(
        env.run(
            repo.path(),
            [
                "specs",
                "trace",
                "--file",
                "src/thing.rs",
                "--file",
                "src/main.rs",
                "--file",
                "src/missing.rs",
                "--file",
                absolute.to_str().unwrap(),
            ],
        )
        .success(),
    );
    insta::assert_snapshot!(out, @r"
    trace of 3 files from the blueprint graph of 0123456789abcdef0123456789abcdef01234567
    use cases at stake, by spec folder:
      docs/project/spec/feature/gadgets/
        project/demo/spec/feature/gadgets/gadget UC-GA1
          crates/thing/tests/thing.rs::gadget_works
      docs/project/spec/feature/things/
        project/demo/spec/feature/things/thing UC-TH1
          crates/thing/tests/thing.rs::first_works
          crates/thing/tests/thing.rs::both_work
        project/demo/spec/feature/things/thing UC-TH2
          crates/thing/tests/thing.rs::second_works
          crates/thing/tests/thing.rs::both_work
    files no use-case-binding test reaches (the trace offers no use-case evidence for them):
      src/main.rs
      src/missing.rs (the graph does not know it)
    ");
}
