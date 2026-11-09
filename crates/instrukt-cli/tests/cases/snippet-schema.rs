//! Snippet shapes: the root rules, a position's `schema.yaml` and
//! `instrukt docs schema <folder>`
//! (`docs/project/spec/feature/docs/snippet-schema.md`).

#![allow(
    clippy::unwrap_used,
    reason = "test helpers: a failed setup step fails the test"
)]

use crate::common;
use common::{Repo, TestEnv};

/// A spec carrying the root rules' required sections and `extra` H2s.
fn spec(extra: &[&str]) -> String {
    let mut text =
        "---\ndescription: A spec.\n---\n\n# Sample — Spec\n\n## What it is\n\nText.\n\n## Canonical fields\n\nText.\n"
            .to_owned();
    for section in extra {
        text.push_str(&format!("\n## {section}\n\nText.\n"));
    }
    text
}

/// A repository declaring project `demo`, its feature specs required to
/// carry `Use cases`.
fn project(env: &TestEnv) -> Repo {
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", "schema_version: 5\nproject: demo\n")
        .write(
            "docs/project/schema.yaml",
            "spec/feature:\n  required: [Use cases]\n  optional: [Examples]\n",
        );
    repo
}

struct Run {
    out: String,
    err: String,
    success: bool,
}

fn instrukt(env: &TestEnv, repo: &Repo, args: &[&str]) -> Run {
    let output = env.instrukt(repo.path()).args(args).output().unwrap();
    Run {
        out: String::from_utf8(output.stdout).unwrap(),
        err: String::from_utf8(output.stderr).unwrap(),
        success: output.status.success(),
    }
}

// spec: project/instrukt/spec/feature/docs/snippet-schema UC-SH1
#[test]
fn a_folders_schema_shows_each_section_with_what_imposes_it_and_a_skeleton() {
    let env = TestEnv::new();
    let repo = project(&env);
    // The folder does not exist yet.
    let run = instrukt(
        &env,
        &repo,
        &["docs", "schema", "docs/project/spec/feature/billing"],
    );
    assert!(run.success, "{}", run.err);
    insta::assert_snapshot!("feature_folder", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-schema UC-SH1
#[test]
fn a_position_root_shows_the_schema_of_every_type() {
    let env = TestEnv::new();
    let repo = project(&env);
    let run = instrukt(&env, &repo, &["docs", "schema", "docs/project"]);
    assert!(run.success, "{}", run.err);
    insta::assert_snapshot!("position_root", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-schema UC-SH1
#[test]
fn a_folder_outside_every_position_is_refused_naming_the_positions() {
    let env = TestEnv::new();
    let repo = project(&env);
    let run = instrukt(&env, &repo, &["docs", "schema", "docs/global/spec"]);
    assert!(!run.success);
    insta::assert_snapshot!("outside_positions", run.err);
}

// spec: project/instrukt/spec/feature/docs/snippet-schema UC-SH1
#[test]
fn a_schema_with_findings_is_reported_instead_of_a_shape() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        "docs/project/schema.yaml",
        "spec/feature:\n  optional: [What it is]\nfeature:\n  required: [X]\n",
    );
    let run = instrukt(
        &env,
        &repo,
        &["docs", "schema", "docs/project/spec/feature"],
    );
    assert!(!run.success);
    insta::assert_snapshot!("schema_findings", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-schema UC-SH2
#[test]
fn a_snippet_under_a_folder_lacking_the_section_its_schema_adds_is_reported() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write("docs/project/spec/feature/billing.md", &spec(&[]))
        .write("docs/project/spec/feature/payments/refunds.md", &spec(&["Use cases", "Examples"]))
        // Outside `spec/feature`: neither held to `Use cases` nor allowed `Examples`.
        .write("docs/project/spec/domain/ledger.md", &spec(&["Examples"]))
        .write(
            "docs/project/index.yaml",
            "listed: [project/demo/spec/feature/billing, project/demo/spec/feature/payments/refunds, project/demo/spec/domain/ledger]\n",
        );
    let run = instrukt(&env, &repo, &["docs", "validate"]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("added_section_missing", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-schema UC-SH2
#[test]
fn a_schema_making_optional_a_section_a_shorter_path_requires_is_reported() {
    let env = TestEnv::new();
    let repo = project(&env);
    repo.write(
        "docs/project/schema.yaml",
        "spec/feature:\n  required: [Use cases]\nspec/feature/legacy:\n  optional: [Use cases, Canonical fields]\n",
    );
    let run = instrukt(&env, &repo, &["docs", "validate"]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("section_dropped", run.out);
}

/// Publishes domain `acme` on the computer of `env`, its specs required to
/// carry `Audit trail`.
fn publish_strict_domain(env: &TestEnv) {
    let acme = env.git_repo("acme");
    acme.write("instrukt.yaml", "schema_version: 1\nproject: acme\n")
        .write(
            "docs/domain/acme/schema.yaml",
            "spec:\n  required: [Audit trail]\n",
        )
        .write(
            "docs/domain/acme/index.yaml",
            "description: Acme work.\nlisted: [acme/spec/audit]\n",
        )
        .write("docs/domain/acme/spec/audit.md", &spec(&["Audit trail"]));
    acme.commit("publish acme");
    env.run(acme.path(), ["sync"]).success();
}

/// The `demo` project with one feature spec lacking `Use cases`.
fn demo_with_one_finding(env: &TestEnv) -> Repo {
    let repo = project(env);
    repo.write("docs/project/spec/feature/billing.md", &spec(&[]))
        .write(
            "docs/project/index.yaml",
            "listed: [project/demo/spec/feature/billing]\n",
        );
    repo
}

// spec: project/instrukt/spec/feature/docs/snippet-schema UC-SH3
#[test]
fn a_projects_snippets_are_judged_the_same_whatever_the_computer_publishes() {
    let mine = TestEnv::new();
    publish_strict_domain(&mine);
    let on_mine = instrukt(&mine, &demo_with_one_finding(&mine), &["docs", "validate"]);
    let teammates = TestEnv::new();
    let on_theirs = instrukt(
        &teammates,
        &demo_with_one_finding(&teammates),
        &["docs", "validate"],
    );
    assert!(!on_mine.success, "{}", on_mine.err);
    assert_eq!(on_mine.out, on_theirs.out);
    insta::assert_snapshot!("same_on_every_computer", on_mine.out);
}
