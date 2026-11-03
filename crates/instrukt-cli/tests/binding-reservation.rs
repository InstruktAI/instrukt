//! `local/projects/instrukt/spec/feature/specs/binding-reservation`:
//! `instrukt specs reserve --slug <slug> --binding <spec_id>:<UC-id>…` —
//! a todo's planned bindings against the use cases other todos claim.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;

use common::{Repo, TestEnv};

const SPEC: &str = "local/projects/demo/spec/feature/things/thing";
const SPEC_PATH: &str = "docs/local/spec/feature/things/thing.md";

/// A feature spec with `extra` frontmatter lines whose `### Use cases`
/// section holds `use_cases`.
fn spec(extra: &str, use_cases: &str) -> String {
    format!(
        "---\ndescription: A thing.\n{extra}---\n\n# Thing — Spec\n\n## What it is\n\nA thing.\n\n\
         ## Canonical fields\n\n### Use cases\n\n{use_cases}"
    )
}

/// A use case: its identifying heading and one Gherkin block.
fn use_case(id: &str) -> String {
    format!(
        "#### {id}: a goal\n\n```gherkin\nGiven a thing\nWhen it is used\nThen it works\n```\n\n"
    )
}

/// `use_case(id)` claimed for `slug` by a `planned` block.
fn planned(slug: &str, id: &str) -> String {
    format!(
        "<!-- planned:{slug} -->\n{}<!-- /planned:{slug} -->\n\n",
        use_case(id)
    )
}

/// `use_case(id)` restated unchanged for `slug` by a `planned-change`
/// block.
fn restated(slug: &str, id: &str) -> String {
    format!(
        "<!-- planned-change:{slug} -->\n{0}<!-- change:{slug} -->\n{0}<!-- /planned-change:{slug} -->\n\n",
        use_case(id)
    )
}

fn project(env: &TestEnv, spec_text: &str) -> Repo {
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", "schema_version: 1\nproject: demo\n")
        .write(SPEC_PATH, spec_text);
    repo
}

/// `instrukt specs reserve --slug <slug> --binding <bindings>`: its exit
/// status and stdout.
fn reserve(env: &TestEnv, repo: &Repo, slug: &str, use_cases: &[&str]) -> String {
    let bindings: Vec<String> = use_cases.iter().map(|uc| format!("{SPEC}:{uc}")).collect();
    let output = env
        .instrukt(repo.path())
        .args(["specs", "reserve", "--slug", slug, "--binding"])
        .args(&bindings)
        .output()
        .unwrap();
    format!(
        "exit: {}\n{}",
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap()
    )
}

// spec: local/projects/instrukt/spec/feature/specs/binding-reservation UC-SR1
#[test]
fn planned_use_cases_free_or_held_by_the_requesting_todo_pass() {
    let env = TestEnv::new();
    let body = format!(
        "{}{}{}",
        use_case("UC-TH1-alt"),
        planned("build-thing", "UC-TH2"),
        restated("build-thing", "UC-TH3")
    );
    let repo = project(&env, &spec("", &body));
    insta::assert_snapshot!(reserve(&env, &repo, "build-thing", &["UC-TH1-alt", "UC-TH2", "UC-TH3"]), @r"
    exit: 0
    clear: local/projects/demo/spec/feature/things/thing UC-TH1-alt
    clear: local/projects/demo/spec/feature/things/thing UC-TH2
    clear: local/projects/demo/spec/feature/things/thing UC-TH3
    ");
}

// spec: local/projects/instrukt/spec/feature/specs/binding-reservation UC-SR1
#[test]
fn a_use_case_another_todo_restated_fails_naming_it_and_its_holder() {
    let env = TestEnv::new();
    let body = format!("{}{}", use_case("UC-TH1"), restated("other-work", "UC-TH2"));
    let repo = project(&env, &spec("", &body));
    insta::assert_snapshot!(reserve(&env, &repo, "build-thing", &["UC-TH1", "UC-TH2"]), @r"
    exit: 3
    clear: local/projects/demo/spec/feature/things/thing UC-TH1
    reserved: local/projects/demo/spec/feature/things/thing UC-TH2 is held by other-work, requested by build-thing
    ");
}

// spec: local/projects/instrukt/spec/feature/specs/binding-reservation UC-SR1
#[test]
fn a_spec_pending_whole_for_another_todo_holds_every_use_case_in_it() {
    let env = TestEnv::new();
    let body = format!("{}{}", use_case("UC-TH1"), use_case("UC-TH2"));
    let repo = project(&env, &spec("delivered_by: [other-work]\n", &body));
    insta::assert_snapshot!(reserve(&env, &repo, "build-thing", &["UC-TH2"]), @r"
    exit: 3
    reserved: local/projects/demo/spec/feature/things/thing UC-TH2 is held by other-work, requested by build-thing
    ");
}

// spec: local/projects/instrukt/spec/feature/specs/binding-reservation UC-SR1
#[test]
fn a_binding_naming_no_use_case_is_refused_as_unknown_before_any_claim_is_checked() {
    let env = TestEnv::new();
    let body = format!("{}{}", use_case("UC-TH1"), planned("other-work", "UC-TH2"));
    let repo = project(&env, &spec("", &body));
    insta::assert_snapshot!(reserve(&env, &repo, "build-thing", &["UC-TH2", "UC-TH9"]), @r"
    exit: 4
    unknown: local/projects/demo/spec/feature/things/thing UC-TH9 names no use case
    ");
}

// spec: local/projects/instrukt/spec/feature/specs/binding-reservation UC-SR1
#[test]
fn a_marker_around_only_an_inheriting_scenario_does_not_reserve_its_use_case() {
    let env = TestEnv::new();
    let body = "#### UC-TH1: a goal\n\n```gherkin\n<!-- planned:other-work -->\n\
                Scenario: the goal\n  Given a thing\n  When it is used\n  Then it works\n\
                <!-- /planned:other-work -->\n```\n\n";
    let repo = project(&env, &spec("", body));
    insta::assert_snapshot!(reserve(&env, &repo, "build-thing", &["UC-TH1"]), @r"
    exit: 0
    clear: local/projects/demo/spec/feature/things/thing UC-TH1
    ");
}
