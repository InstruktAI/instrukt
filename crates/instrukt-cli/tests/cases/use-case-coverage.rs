//! `project/instrukt/spec/feature/specs/use-case-coverage`:
//! `instrukt specs coverage [--check] [--update-baseline]` — the use-case
//! inventory against the tests' bindings, and the ratchet over orphans and
//! anonymous scenarios.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::common;
use common::{Repo, TestEnv};

const SPEC_ID: &str = "project/demo/spec/feature/things/thing";
const SPEC_PATH: &str = "docs/project/spec/feature/things/thing.md";

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

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
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
    orphan: crates/thing/tests/thing.rs::realizes_1 cites project/demo/spec/feature/things/thing UC-TH9
    orphan: crates/thing/tests/thing.rs::realizes_2 cites docs/x.md UC-TH2
    anonymous: project/demo/spec/feature/things/thing ## Behaviour
    unbound: project/demo/spec/feature/things/thing UC-TH2
    unbound: project/demo/spec/feature/things/thing UC-TH3
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
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
    unbound: project/demo/spec/feature/things/thing UC-TH3
    multi-bound: project/demo/spec/feature/things/thing UC-TH1 (2 tests)
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
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
    unbound: project/demo/spec/feature/things/thing UC-TH1
    unbound: project/demo/spec/feature/things/thing UC-TH3
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC2
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
    orphan: tests/thing.rs::realizes_1 cites project/demo/spec/feature/things/thing UC-TH7
    unbound: project/demo/spec/feature/things/thing UC-TH2
    unbound: project/demo/spec/feature/things/thing UC-TH3
    check: orphans 1 above baseline 0
    check: failed
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC2
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
    unbound: project/demo/spec/feature/things/thing UC-TH2
    unbound: project/demo/spec/feature/things/thing UC-TH3
    unbound: project/demo/spec/feature/things/thing UC-TH4
    check: passed
    ");
}

/// `#### <id>` whose Gherkin block holds one unnamed `Scenario:` per title.
fn scenarios_under(id: &str, titles: &[&str]) -> String {
    let scenarios: String = titles
        .iter()
        .map(|t| format!("Scenario: {t}\n  Given a thing\n  When it is used\n  Then it works\n"))
        .collect();
    format!("#### {id}: a goal\n\n```gherkin\n{scenarios}```\n\n")
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[test]
fn prose_beginning_with_when_is_not_a_scenario() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        SPEC_PATH,
        &format!(
            "{}## Notes\n\nWhen the thing runs, nothing here is Gherkin.\n\n\
             Scenario: told in prose\n",
            repo.read(SPEC_PATH)
        ),
    );
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 3
    bindable use cases: 3
    anonymous scenarios: 0
    bound: 0
    unbound: 3
    multi-bound: 0
    orphans: 0
    unbound: project/demo/spec/feature/things/thing UC-TH1
    unbound: project/demo/spec/feature/things/thing UC-TH2
    unbound: project/demo/spec/feature/things/thing UC-TH3
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[test]
fn a_single_unnamed_scenario_takes_its_headings_id_and_is_bound_under_it() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        SPEC_PATH,
        &spec(&format!(
            "{}{}",
            use_case("UC-TH1", "first"),
            scenarios_under("UC-TH2", &["the second goal"])
        )),
    )
    .write("tests/thing.rs", &rust_tests(&[(SPEC_ID, "UC-TH2")]));
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 2
    bindable use cases: 2
    anonymous scenarios: 0
    bound: 1
    unbound: 1
    multi-bound: 0
    orphans: 0
    unbound: project/demo/spec/feature/things/thing UC-TH1
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[test]
fn unnamed_scenarios_that_are_not_their_headings_only_one_are_anonymous() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        SPEC_PATH,
        &spec(&scenarios_under(
            "UC-TH2",
            &["the second goal", "another goal"],
        )),
    );
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 3
    bindable use cases: 1
    anonymous scenarios: 2
    bound: 0
    unbound: 1
    multi-bound: 0
    orphans: 0
    anonymous: project/demo/spec/feature/things/thing Scenario: the second goal
    anonymous: project/demo/spec/feature/things/thing Scenario: another goal
    unbound: project/demo/spec/feature/things/thing UC-TH2
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[test]
fn an_unnamed_scenario_beside_a_named_one_is_anonymous() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        SPEC_PATH,
        &spec(&scenarios_under("UC-TH2", &["UC-TH3 named", "unnamed"])),
    );
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 3
    bindable use cases: 2
    anonymous scenarios: 1
    bound: 0
    unbound: 2
    multi-bound: 0
    orphans: 0
    anonymous: project/demo/spec/feature/things/thing Scenario: unnamed
    unbound: project/demo/spec/feature/things/thing UC-TH2
    unbound: project/demo/spec/feature/things/thing UC-TH3
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[test]
fn a_citation_no_test_function_follows_is_an_orphan_not_coverage() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        "tests/thing.rs",
        &format!("// spec: {SPEC_ID} UC-TH1\nstruct NotATest;\n"),
    );
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 3
    bindable use cases: 3
    anonymous scenarios: 0
    bound: 0
    unbound: 3
    multi-bound: 0
    orphans: 1
    orphan: tests/thing.rs:1 cites project/demo/spec/feature/things/thing UC-TH1 (no test function follows it)
    unbound: project/demo/spec/feature/things/thing UC-TH1
    unbound: project/demo/spec/feature/things/thing UC-TH2
    unbound: project/demo/spec/feature/things/thing UC-TH3
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[cfg(unix)]
#[test]
fn tracked_symlinks_resolving_outside_the_repository_are_not_its_tests() {
    use std::os::unix::fs::symlink;

    let env = TestEnv::new();
    let repo = project(&env);
    let outside = env.path("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("thing.rs"), rust_tests(&[(SPEC_ID, "UC-TH1")])).unwrap();
    std::fs::create_dir_all(repo.path().join("tests")).unwrap();
    symlink(
        outside.join("thing.rs"),
        repo.path().join("tests/linked.rs"),
    )
    .unwrap();
    symlink(&outside, repo.path().join("tests/linked_dir")).unwrap();
    repo.commit("track links that leave the repository");
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 3
    bindable use cases: 3
    anonymous scenarios: 0
    bound: 0
    unbound: 3
    multi-bound: 0
    orphans: 0
    unbound: project/demo/spec/feature/things/thing UC-TH1
    unbound: project/demo/spec/feature/things/thing UC-TH2
    unbound: project/demo/spec/feature/things/thing UC-TH3
    ");
}

// spec: project/instrukt/spec/feature/specs/use-case-coverage UC-SC1
#[test]
fn a_use_case_id_with_letters_and_hyphens_after_its_number_is_bindable() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        SPEC_PATH,
        &spec(&format!(
            "{}{}",
            use_case("UC-TH1-alt", "an alternative"),
            use_case("UC-TH2b", "a variant")
        )),
    )
    .write(
        "tests/thing.rs",
        &rust_tests(&[(SPEC_ID, "UC-TH1-alt"), (SPEC_ID, "UC-TH2b")]),
    );
    insta::assert_snapshot!(coverage(&env, &repo, &[]), @r"
    exit: 0
    spec files with scenarios: 1
    scenarios: 2
    bindable use cases: 2
    anonymous scenarios: 0
    bound: 2
    unbound: 0
    multi-bound: 0
    orphans: 0
    ");
}
