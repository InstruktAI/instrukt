//! `instrukt docs validate`
//! (`docs/local/spec/feature/docs/snippet-validation.md`).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers: a failed setup step fails the test"
)]

mod common;

use common::{Repo, TestEnv};

/// A snippet of `taxonomy` with every required section, `extra`
/// frontmatter lines and `body` appended to its last section.
fn snippet(taxonomy: &str, extra: &str, body: &str) -> String {
    let (title, sections): (&str, &[&str]) = match taxonomy {
        "principle" => (
            "Principle",
            &["Principle", "Rationale", "Implications", "Tensions"],
        ),
        "concept" => ("Concept", &["What", "Why"]),
        "policy" => (
            "Policy",
            &["Rules", "Rationale", "Scope", "Enforcement", "Exceptions"],
        ),
        "procedure" => (
            "Procedure",
            &["Goal", "Preconditions", "Steps", "Outputs", "Recovery"],
        ),
        "design" => (
            "Design",
            &[
                "Purpose",
                "Inputs/Outputs",
                "Invariants",
                "Primary flows",
                "Failure modes",
            ],
        ),
        "spec" => ("Spec", &["What it is", "Canonical fields"]),
        other => panic!("no builder for {other}"),
    };
    let mut text = format!("---\ndescription: A {taxonomy}.\n{extra}---\n\n# Sample — {title}\n");
    for section in sections {
        text.push_str(&format!("\n## {section}\n\nText.\n"));
    }
    text.push_str(body);
    text
}

/// The machine's runtime mapping: one agent CLI, `claude`, that dispatch
/// defaults may name.
const MACHINE: &str = r#"schema_version: 1
runtimes:
  - name: claude-cli
    agent: claude
    session_id_env: CLAUDE_SESSION
    start: claude
    send: claude send
    alive: claude alive
    conversation: claude history
"#;

/// A repository declaring project `demo` that publishes the `acme` domain,
/// on a machine whose runtime mapping can start `claude`.
fn project(env: &TestEnv) -> Repo {
    let config = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(config, MACHINE).unwrap();
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", "schema_version: 1\nproject: demo\n");
    repo
}

/// A documentation tree that keeps the whole contract: both halves,
/// required reads of every valid form, a baseline, a fragment, a
/// third-party doc and every pending carrier.
fn valid_tree(repo: &Repo) {
    repo.write("docs/global/acme/principle/care.md", &snippet("principle", "", ""))
        .write(
            "docs/global/acme/policy/rules.md",
            &snippet(
                "policy",
                "visibility: public\neval: true\n",
                "\n## See also\n\n- acme/principle/care\n",
            )
            .replace(
                "# Sample — Policy\n",
                "# Sample — Policy\n\n## Required reads\n\n- @~/.instrukt/docs/acme/principle/care.md\n",
            ),
        )
        .write(
            "docs/global/acme/procedure/work/overview.md",
            &snippet(
                "procedure",
                "role: worker\nactivation_trigger: When work arrives.\n\
                 parameters:\n  - name: slug\n    required: true\n\
                 dispatch:\n  agent: [claude]\n  thinking_mode: med\n",
                "\n<!-- @include: docs/snippets/shared.md -->\n",
            ),
        )
        .write(
            "docs/global/acme/procedure/listen.md",
            &snippet(
                "procedure",
                "role: listener\nsubscriptions: ['acme.events.*']\nevent_delivery: batched\n",
                "",
            ),
        )
        .write(
            "docs/global/acme/baseline.md",
            "@~/.instrukt/docs/acme/policy/rules.md\n@~/.instrukt/docs/acme/procedure/listen.md\n",
        )
        .write("docs/global/snippets/shared.md", "Shared text.\n")
        .write(
            "docs/local/spec/feature/thing.md",
            &snippet(
                "spec",
                "delivered_by: [build-thing]\n",
                "\n<!-- @exec: instrukt config get project -->\n",
            )
            .replace(
                "# Sample — Spec\n",
                "# Sample — Spec\n\n## Required reads\n\n- @docs/local/design/shape.md\n- @~/.instrukt/docs/acme/bundle.md\n",
            ),
        )
        .write(
            "docs/local/design/shape.md",
            &snippet(
                "design",
                "",
                "\n<!-- planned:build-thing -->\nAdded.\n<!-- /planned:build-thing -->\n\n\
                 <!-- planned-change:rework-shape -->\nOld.\n<!-- change:rework-shape -->\nNew.\n\
                 <!-- /planned-change:rework-shape -->\n",
            ),
        )
        .write("docs/local/design/shape.next-shape.md", &snippet("design", "", ""))
        .write("docs/local/design/retired.md", &snippet("design", "", ""))
        .write("docs/local/design/retired.drop-it.md", "")
        .write("docs/local/baseline.md", "@docs/local/design/shape.md\n")
        .write(
            "docs/third-party/tool/usage.md",
            "---\ndescription: How to use the tool.\n---\n\n# Tool usage\n\n## Sources\n\n\
             - https://tool.example/docs\n- [Guide](https://tool.example/guide)\n- /tool/docs\n",
        )
        .write("docs/global/acme/_drafts/notes.md", "no frontmatter at all\n")
        .write("docs/global/acme/workflow/templates/step.md", "A template, not a snippet.\n");
}

struct Run {
    out: String,
    err: String,
    success: bool,
}

fn validate(env: &TestEnv, repo: &Repo, args: &[&str]) -> Run {
    let output = env
        .instrukt(repo.path())
        .args(["docs", "validate"])
        .args(args)
        .output()
        .unwrap();
    Run {
        out: String::from_utf8(output.stdout).unwrap(),
        err: String::from_utf8(output.stderr).unwrap(),
        success: output.status.success(),
    }
}

// spec: local/projects/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn documentation_that_keeps_the_contract_validates_successfully() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    let run = validate(&env, &repo, &[]);
    assert!(run.success, "{}", run.err);
    insta::assert_snapshot!("clean_run", run.out);
}

// spec: local/projects/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn a_snippet_without_a_description_is_reported_not_left_out() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/global/acme/concept/bare.md",
        "---\nvisibility: public\n---\n\n# Bare — Concept\n\n## What\n\nx\n\n## Why\n\ny\n",
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("description_missing", run.out);
}

// spec: local/projects/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn every_broken_snippet_is_reported_in_one_run_with_its_file_and_what_is_wrong() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    // Identity
    repo.write("docs/global/acme/notes/idea.md", &snippet("concept", "", ""))
        .write("docs/global/loose.md", &snippet("concept", "", ""))
        .write(
            "docs/global/acme/policy/authored.md",
            &snippet("policy", "id: acme/policy/authored\ntype: policy\n", ""),
        )
        // Frontmatter
        .write(
            "docs/global/acme/concept/fields.md",
            &snippet(
                "concept",
                "visibility: secret\neval: often\nrole: helper\n",
                "",
            ),
        )
        .write(
            "docs/global/acme/procedure/both.md",
            &snippet(
                "procedure",
                "activation_trigger: Now.\nsubscriptions: ['a.*']\nparameters: [{name: x}]\n",
                "",
            ),
        )
        .write(
            "docs/global/acme/procedure/ghostly.md",
            &snippet(
                "procedure",
                "role: haunter\nactivation_trigger: At night.\n\
                 dispatch:\n  agent: [claude, ghost]\n  thinking_mode: slow\n",
                "",
            ),
        )
        .write(
            "docs/local/adr/0001-choice.md",
            "---\ndescription: A choice.\n---\n\n# Choice — ADR\n\n## Context\n\n## Decision\n\n## Consequences\n",
        )
        .write(
            "docs/local/adr/0002-pending.md",
            "---\ndescription: Pending.\ndate: 2026-01-02\nnumber: 2\ndelivered_by: [later]\n---\n\n\
             # Pending — ADR\n\n## Context\n\n<!-- planned:later -->\nx\n<!-- /planned:later -->\n\n\
             ## Decision\n\n## Consequences\n",
        )
        .write("docs/local/adr/0002-pending.later.md", "")
        // Structure
        .write(
            "docs/global/acme/principle/shape.md",
            "---\ndescription: Misshapen.\n---\n\nIntro first.\n\n# Shape — Concept\n\n## Principle\n\n\
             ## See also\n\n## Rationale\n\n## Implications\n\n## Extra\n",
        )
        // References
        .write(
            "docs/global/acme/policy/refs.md",
            &snippet(
                "policy",
                "",
                "\nSee @~/.instrukt/docs/acme/principle/care.md inline.\n",
            )
            .replace(
                "# Sample — Policy\n",
                "# Sample — Policy\n\n## Rules\n\n## Required reads\n\n- @docs/global/acme/principle/care.md\n\
                 - @~/.instrukt/docs/acme/principle/missing.md\n- @docs/local/design/shape.md\n",
            ),
        )
        // Pending
        .write(
            "docs/local/spec/pending.md",
            &snippet(
                "spec",
                "delivered_by: []\n",
                "\nInline <!-- planned:x --> marker.\n\n<!-- planned:Bad_Slug -->\nx\n<!-- /planned:Bad_Slug -->\n\n\
                 <!-- planned:a -->\n<!-- planned:b -->\n<!-- /planned:b -->\n<!-- /planned:a -->\n\n\
                 <!-- planned:open -->\nnever closed\n",
            ),
        )
        .write("docs/local/spec/pending.Not_A_Slug.md", &snippet("spec", "", ""))
        // Baselines
        .write(
            "docs/global/acme/baseline-progressive.md",
            "@~/.instrukt/docs/acme/policy/rules.md\n@~/.instrukt/docs/acme/principle/shape.md\nnot a reference\n",
        )
        .write(
            "docs/local/baseline.md",
            "@~/.instrukt/docs/acme/bundle.md\n@docs/local/design/shape.md\n\
             @~/.instrukt/docs/acme/principle/care.md\n",
        )
        // Third-party
        .write(
            "docs/third-party/tool/broken.md",
            "---\nvisibility: public\ntype: spec\n---\n\n# Broken\n\n## Sources\n\n- somewhere on the web\n",
        );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!(run.out);
}

// spec: local/projects/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn findings_print_as_a_json_list_with_file_code_and_value() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/global/acme/concept/fields.md",
        &snippet("concept", "visibility: secret\n", ""),
    );
    let run = validate(&env, &repo, &["--json"]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("json_findings", run.out);
    let clean = env.git_repo("clean");
    clean.write("instrukt.yaml", "schema_version: 1\nproject: clean\n");
    valid_tree(&clean);
    let run = validate(&env, &clean, &["--json"]);
    assert!(run.success, "{}", run.err);
    insta::assert_snapshot!("json_clean", run.out);
}

// spec: local/projects/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn named_paths_limit_the_run_to_those_files_and_their_cross_snippet_checks() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/global/acme/concept/one.md",
        &snippet("concept", "visibility: x\n", ""),
    )
    .write(
        "docs/global/acme/concept/two.md",
        &snippet("concept", "visibility: y\n", ""),
    )
    .write(
        "docs/global/acme/procedure/rival.md",
        &snippet("procedure", "role: worker\nactivation_trigger: Also.\n", ""),
    );
    let one = validate(&env, &repo, &["docs/global/acme/concept/one.md"]);
    assert!(!one.success);
    insta::assert_snapshot!("scoped_one_file", one.out);
    // The rival shares a role with the work overview: naming either file
    // reports the collision on both.
    let rival = validate(&env, &repo, &["docs/global/acme/procedure/rival.md"]);
    assert!(!rival.success);
    insta::assert_snapshot!("scoped_role_clash", rival.out);
    let untouched = validate(&env, &repo, &["docs/global/acme/policy/rules.md"]);
    assert!(untouched.success);
    insta::assert_snapshot!("scoped_clean_file", untouched.out);
}

// spec: local/projects/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn an_entry_procedure_requiring_its_own_sub_procedure_is_reported() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write("docs/global/acme/procedure/work/step.md", &snippet("procedure", "", ""))
        .write(
            "docs/global/acme/procedure/work/overview.md",
            &snippet("procedure", "role: worker\nactivation_trigger: When work arrives.\n", "")
                .replace(
                    "# Sample — Procedure\n",
                    "# Sample — Procedure\n\n## Required reads\n\n- @~/.instrukt/docs/acme/procedure/work/step.md\n",
                ),
        );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("subprocedure_required", run.out);
}

// spec: local/projects/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn references_to_domains_published_elsewhere_resolve_through_the_home() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    let home_docs = env.instrukt_home().join("docs/general/principle");
    std::fs::create_dir_all(&home_docs).unwrap();
    std::fs::write(home_docs.join("stewardship.md"), "published elsewhere\n").unwrap();
    repo.write(
        "docs/global/acme/concept/borrowing.md",
        &snippet("concept", "", "").replace(
            "# Sample — Concept\n",
            "# Sample — Concept\n\n## Required reads\n\n- @~/.instrukt/docs/general/principle/stewardship.md\n\
             - @~/.instrukt/docs/general/principle/absent.md\n- @~/.instrukt/docs/general/bundle.md\n\
             - @~/.instrukt/docs/nowhere/bundle.md\n",
        ),
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("home_references", run.out);
}

// spec: local/projects/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn local_snippets_in_a_repository_without_a_project_have_no_identity() {
    let env = TestEnv::new();
    let repo = env.git_repo("anonymous");
    repo.write("docs/local/spec/x.md", &snippet("spec", "", ""));
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("undeclared_project", run.out);
}
