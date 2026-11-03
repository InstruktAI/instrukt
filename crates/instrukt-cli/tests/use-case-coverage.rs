//! `local/projects/instrukt/spec/feature/specs/use-case-coverage`:
//! `instrukt specs coverage [--check] [--update-baseline]` — the use-case
//! inventory against the tests' bindings, and the ratchet over orphans and
//! anonymous scenarios.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;

use common::{Repo, TestEnv};

const SPEC_ID: &str = "local/projects/demo/spec/feature/things/thing";
const SPEC_PATH: &str = "docs/local/spec/feature/things/thing.md";

/// A feature spec whose `### Use cases` section holds `use_cases`.
fn spec(use_cases: &str) -> String {
    format!(
        "---\ndescription: A thing.\n---\n\n# Thing — Spec\n\n## What it is\n\nA thing.\n\n\
         ## Canonical fields\n\n### Use cases\n\n{use_cases}"
    )
}

/// A use case: its identifying heading and one Gherkin block.
fn use_case(id: &str, title: &str) -> String {
    format!(
        "#### {id}: {title}\n\n```gherkin\nGiven a thing\nWhen it is used\nThen it works\n```\n\n"
    )
}

/// A Rust test file whose tests bind `bindings` (`(spec id, use-case id)`).
fn rust_tests(bindings: &[(&str, &str)]) -> String {
    bindings
        .iter()
        .enumerate()
        .map(|(n, (spec, uc))| format!("// spec: {spec} {uc}\n#[test]\nfn realizes_{n}() {{}}\n\n"))
        .collect()
}

/// A project `demo` with one feature spec holding UC-TH1, UC-TH2, UC-TH3.
fn project(env: &TestEnv) -> Repo {
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", "schema_version: 1\nproject: demo\n")
        .write(
            SPEC_PATH,
            &spec(&format!(
                "{}{}{}",
                use_case("UC-TH1", "first"),
                use_case("UC-TH2", "second"),
                use_case("UC-TH3", "third")
            )),
        );
    repo
}

/// `instrukt specs coverage <args>`: its exit status and stdout.
fn coverage(env: &TestEnv, repo: &Repo, args: &[&str]) -> String {
    let output = env
        .instrukt(repo.path())
        .args(["specs", "coverage"])
        .args(args)
        .output()
        .unwrap();
    format!(
        "exit: {}\n{}",
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap()
    )
}

// spec: local/projects/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[test]
fn the_report_shows_bound_and_unbound_use_cases_orphans_and_anonymous_scenarios() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        SPEC_PATH,
        &format!(
            "{}## Behaviour\n\n```gherkin\nWhen it runs\nThen nothing is named\n```\n",
            repo.read(SPEC_PATH)
        ),
    )
    .write(
        "crates/thing/tests/thing.rs",
        &rust_tests(&[
            (SPEC_ID, "UC-TH1"),
            (SPEC_ID, "UC-TH9"),
            ("docs/x.md", "UC-TH2"),
        ]),
    );
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 4
    bindable use cases: 3
    anonymous scenarios: 1
    bound: 1
    unbound: 2
    multi-bound: 0
    orphans: 2
    orphan: crates/thing/tests/thing.rs::realizes_1 cites local/projects/demo/spec/feature/things/thing UC-TH9
    orphan: crates/thing/tests/thing.rs::realizes_2 cites docs/x.md UC-TH2
    anonymous: local/projects/demo/spec/feature/things/thing ## Behaviour
    unbound: local/projects/demo/spec/feature/things/thing UC-TH2
    unbound: local/projects/demo/spec/feature/things/thing UC-TH3
    ");
}

// spec: local/projects/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[test]
fn python_decorators_bind_like_rust_comments_and_extra_tests_are_counted() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        "tests/test_thing.py",
        &format!(
            "import pytest\n\nSPEC = \"{SPEC_ID}\"\n\n@pytest.mark.spec(SPEC, \"UC-TH2\")\n\
             def test_second():\n    pass\n\n@pytest.mark.spec(\"{SPEC_ID}\", \"UC-TH1\")\n\
             def test_first_again():\n    pass\n"
        ),
    )
    .write("src/lib_test.rs", &rust_tests(&[(SPEC_ID, "UC-TH1")]));
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 3
    bindable use cases: 3
    anonymous scenarios: 0
    bound: 2
    unbound: 1
    multi-bound: 1
    orphans: 0
    unbound: local/projects/demo/spec/feature/things/thing UC-TH3
    multi-bound: local/projects/demo/spec/feature/things/thing UC-TH1 (2 tests)
    ");
}

// spec: local/projects/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[test]
fn a_use_case_heading_left_without_gherkin_is_retired_not_unbound() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        SPEC_PATH,
        &spec(&format!(
            "{}#### UC-TH2: retired\n\nNo longer specified.\n\n{}",
            use_case("UC-TH1", "first"),
            use_case("UC-TH3", "third")
        )),
    );
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 2
    bindable use cases: 2
    anonymous scenarios: 0
    bound: 0
    unbound: 2
    multi-bound: 0
    orphans: 0
    unbound: local/projects/demo/spec/feature/things/thing UC-TH1
    unbound: local/projects/demo/spec/feature/things/thing UC-TH3
    ");
}

// spec: local/projects/instrukt/spec/feature/specs/use-case-coverage UC-SC2
#[test]
fn a_test_citing_a_missing_use_case_fails_the_check_naming_the_count_that_grew() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write("tests/thing.rs", &rust_tests(&[(SPEC_ID, "UC-TH1")]));
    env.run(repo.path(), ["specs", "coverage", "--update-baseline"])
        .success();
    repo.commit("record the coverage baseline");
    repo.write(
        "tests/thing.rs",
        &rust_tests(&[(SPEC_ID, "UC-TH1"), (SPEC_ID, "UC-TH7")]),
    );
    insta::assert_snapshot!(coverage(&env, &repo, &["--check"]), @r"
    exit: 1
    spec files with scenarios: 1
    scenarios: 3
    bindable use cases: 3
    anonymous scenarios: 0
    bound: 1
    unbound: 2
    multi-bound: 0
    orphans: 1
    orphan: tests/thing.rs::realizes_1 cites local/projects/demo/spec/feature/things/thing UC-TH7
    unbound: local/projects/demo/spec/feature/things/thing UC-TH2
    unbound: local/projects/demo/spec/feature/things/thing UC-TH3
    check: orphans 1 above baseline 0
    check: failed
    ");
}

// spec: local/projects/instrukt/spec/feature/specs/use-case-coverage UC-SC2
#[test]
fn a_change_that_only_leaves_use_cases_unbound_passes_the_check() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write("tests/thing.rs", &rust_tests(&[(SPEC_ID, "UC-TH1")]));
    env.run(repo.path(), ["specs", "coverage", "--update-baseline"])
        .success();
    repo.commit("record the coverage baseline");
    repo.write(
        SPEC_PATH,
        &format!("{}{}", repo.read(SPEC_PATH), use_case("UC-TH4", "fourth")),
    );
    insta::assert_snapshot!(coverage(&env, &repo, &["--check"]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 4
    bindable use cases: 4
    anonymous scenarios: 0
    bound: 1
    unbound: 3
    multi-bound: 0
    orphans: 0
    unbound: local/projects/demo/spec/feature/things/thing UC-TH2
    unbound: local/projects/demo/spec/feature/things/thing UC-TH3
    unbound: local/projects/demo/spec/feature/things/thing UC-TH4
    check: passed
    ");
}
