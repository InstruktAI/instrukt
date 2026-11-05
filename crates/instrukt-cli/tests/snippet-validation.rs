//! `instrukt docs validate`
//! (`docs/project/spec/feature/docs/snippet-validation.md`).

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

/// A documentation tree that keeps the whole contract: a domain and the
/// project, required reads of every valid form (an id, a bare domain), a
/// domain index, a baseline, a third-party doc and every pending carrier.
/// Every delivered snippet is named by an index or a required read, so the
/// run carries no notice.
fn valid_tree(repo: &Repo) {
    repo.write(
        "docs/project/index.yaml",
        "listed: [project/demo/adr/0049-choice, project/demo/design/retired]
",
    );
    repo.write(
        "docs/domain/acme/principle/care.md",
        &snippet("principle", "", ""),
    )
    .write(
        "docs/domain/acme/policy/rules.md",
        &snippet(
            "policy",
            "visibility: public\neval: true\n",
            "\n## See also\n\n- acme/principle/care\n",
        )
        .replace(
            "# Sample — Policy\n",
            "# Sample — Policy\n\n## Required reads\n\n- acme/principle/care\n",
        ),
    )
    .write(
        "docs/domain/acme/procedure/work/overview.md",
        &snippet(
            "procedure",
            "role: worker\nactivation_trigger: When work arrives.\n\
                 parameters:\n  - name: slug\n    required: true\n\
                 dispatch:\n  agent: [claude]\n  thinking_mode: med\n",
            "",
        ),
    )
    .write(
        "docs/domain/acme/procedure/listen.md",
        &snippet(
            "procedure",
            "role: listener\nsubscriptions: ['acme.events.*']\nevent_delivery: batched\n",
            "",
        ),
    )
    .write(
        "docs/domain/acme/baseline.md",
        "@~/.instrukt/docs/acme/policy/rules.md\n@~/.instrukt/docs/acme/procedure/listen.md\n",
    )
    .write(
        "docs/domain/acme/index.yaml",
        "description: The acme way of working.\nexpanded: [acme/policy/rules]\n\
             listed: [acme/procedure/listen, acme/procedure/work/overview]\n",
    )
    .write(
        "docs/project/spec/feature/thing.md",
        &snippet(
            "spec",
            "delivered_by: [build-thing]\n",
            // Read in a session, never compiled: a runtime value is fine.
            "\n<!-- @config: runtime.session_id -->\n",
        )
        .replace(
            "# Sample — Spec\n",
            "# Sample — Spec\n\n## Required reads\n\n- project/demo/design/shape\n- acme\n",
        ),
    )
    .write(
        "docs/project/design/shape.md",
        &snippet(
            "design",
            "",
            "\n<!-- planned:build-thing -->\nAdded.\n<!-- /planned:build-thing -->\n\n\
                 <!-- planned-change:rework-shape -->\nOld.\n<!-- change:rework-shape -->\nNew.\n\
                 <!-- /planned-change:rework-shape -->\n",
        ),
    )
    .write(
        "docs/project/design/shape.next-shape.md",
        &snippet("design", "", ""),
    )
    .write("docs/project/design/retired.md", &snippet("design", "", ""))
    .write("docs/project/design/retired.drop-it.md", "")
    .write(
        "docs/project/baseline.md",
        "@docs/project/design/shape.md\n",
    )
    .write(
        "docs/third-party/tool/usage.md",
        "---\ndescription: How to use the tool.\n---\n\n# Tool usage\n\n## Sources\n\n\
             - https://tool.example/docs\n- [Guide](https://tool.example/guide)\n- /tool/docs\n\
             - <https://tool.example/reference>\n",
    )
    .write(
        "docs/project/adr/0049-choice.md",
        "---\ndescription: A choice.\ndate: 2026-01-02\nnumber: 0049\n---\n\n# Choice — ADR\n\n\
             ## Context\n\n## Decision\n\n## Consequences\n",
    )
    .write(
        "docs/domain/acme/_drafts/notes.md",
        "no frontmatter at all\n",
    );
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

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn documentation_that_keeps_the_contract_validates_successfully() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    let run = validate(&env, &repo, &[]);
    assert!(run.success, "{}", run.err);
    insta::assert_snapshot!("clean_run", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn a_snippet_without_a_description_is_reported_not_left_out() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/acme/concept/bare.md",
        "---\nvisibility: public\n---\n\n# Bare — Concept\n\n## What\n\nx\n\n## Why\n\ny\n",
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("description_missing", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn a_runtime_value_in_a_snippet_an_index_renders_in_full_is_reported() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/acme/principle/session.md",
        &snippet("principle", "", "\n<!-- @config: runtime.session_id -->\n"),
    )
    .write(
        "docs/project/concept/way.md",
        &snippet("concept", "", "").replace(
            "# Sample — Concept\n",
            "# Sample — Concept\n\n## Required reads\n\n- acme/principle/session\n",
        ),
    )
    .write(
        "docs/project/index.yaml",
        "expanded: [project/demo/concept/way]\n",
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("runtime_value_rendered_through_an_index", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn an_include_directive_is_reported_naming_what_it_includes() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/acme/concept/shared.md",
        &snippet(
            "concept",
            "",
            "\n<!-- @include: docs/snippets/step.md -->\n",
        ),
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("include_directive", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn every_broken_snippet_is_reported_in_one_run_with_its_file_and_what_is_wrong() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    // Identity
    repo.write("docs/domain/acme/notes/idea.md", &snippet("concept", "", ""))
        .write("docs/domain/loose.md", &snippet("concept", "", ""))
        .write(
            "docs/domain/acme/policy/authored.md",
            &snippet("policy", "id: acme/policy/authored\ntype: policy\n", ""),
        )
        // Frontmatter
        .write(
            "docs/domain/acme/concept/fields.md",
            &snippet(
                "concept",
                "visibility: secret\neval: often\nrole: helper\n",
                "",
            ),
        )
        .write(
            "docs/domain/acme/procedure/both.md",
            &snippet(
                "procedure",
                "activation_trigger: Now.\nsubscriptions: ['a.*']\nparameters: [{name: x}]\n",
                "",
            ),
        )
        .write(
            "docs/domain/acme/procedure/ghostly.md",
            &snippet(
                "procedure",
                "role: haunter\nactivation_trigger: At night.\n\
                 dispatch:\n  agent: [claude, ghost]\n  thinking_mode: slow\n",
                "",
            ),
        )
        .write(
            "docs/project/adr/0001-choice.md",
            "---\ndescription: A choice.\n---\n\n# Choice — ADR\n\n## Context\n\n## Decision\n\n## Consequences\n",
        )
        .write(
            "docs/project/adr/0002-pending.md",
            "---\ndescription: Pending.\ndate: 2026-01-02\nnumber: 2\ndelivered_by: [later]\n---\n\n\
             # Pending — ADR\n\n## Context\n\n<!-- planned:later -->\nx\n<!-- /planned:later -->\n\n\
             ## Decision\n\n## Consequences\n",
        )
        .write("docs/project/adr/0002-pending.later.md", "")
        // Structure
        .write(
            "docs/domain/acme/principle/shape.md",
            "---\ndescription: Misshapen.\n---\n\nIntro first.\n\n# Shape — Concept\n\n## Principle\n\n\
             ## See also\n\n## Rationale\n\n## Implications\n\n## Extra\n",
        )
        // References
        .write(
            "docs/domain/acme/policy/refs.md",
            &snippet(
                "policy",
                "",
                "\nSee @~/.instrukt/docs/acme/principle/care.md inline.\n",
            )
            .replace(
                "# Sample — Policy\n",
                "# Sample — Policy\n\n## Rules\n\n## Required reads\n\n- acme/principle/missing\n\
                 - project/demo/design/shape\n",
            ),
        )
        // Pending
        .write(
            "docs/project/spec/pending.md",
            &snippet(
                "spec",
                "delivered_by: []\n",
                "\nInline <!-- planned:x --> marker.\n\n<!-- planned:Bad_Slug -->\nx\n<!-- /planned:Bad_Slug -->\n\n\
                 <!-- planned:a -->\n<!-- planned:b -->\n<!-- /planned:b -->\n<!-- /planned:a -->\n\n\
                 <!-- planned:open -->\nnever closed\n",
            ),
        )
        .write("docs/project/spec/pending.Not_A_Slug.md", &snippet("spec", "", ""))
        // Baselines
        .write(
            "docs/domain/acme/baseline-progressive.md",
            "@~/.instrukt/docs/acme/policy/rules.md\n@~/.instrukt/docs/acme/principle/shape.md\nnot a reference\n",
        )
        .write(
            "docs/project/baseline.md",
            "@~/.instrukt/docs/acme/bundle.md\n@docs/project/design/shape.md\n\
             @~/.instrukt/docs/acme/principle/care.md\n",
        )
        // Workflow flavor
        .write("docs/domain/acme/workflow/actions.yaml", "roles: not-a-list\n")
        // Third-party
        .write(
            "docs/third-party/tool/broken.md",
            "---\nvisibility: public\ntype: spec\n---\n\n# Broken\n\n## Sources\n\n- somewhere on the web\n",
        )
        // Directives
        .write(
            "docs/domain/acme/concept/runs.md",
            &snippet("concept", "", "\n<!-- @exec: touch ran -->\n"),
        )
        .write(
            "docs/domain/acme/principle/session.md",
            &snippet("principle", "", "\n<!-- @config: runtime.session_id -->\n"),
        )
        .write(
            "AGENTS.master.md",
            "## Required reads\n\n@~/.instrukt/docs/acme/principle/session.md\n\n## Purpose\n\n\
             <!-- @config: runtime.conversation -->\n\n<!-- @exec: touch ran -->\n",
        );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!(run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn findings_print_as_a_json_list_with_file_code_and_value() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/acme/concept/fields.md",
        &snippet("concept", "visibility: secret\n", ""),
    );
    let run = validate(&env, &repo, &["--json"]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("json_findings", run.out);
    let clean = env.git_repo("clean");
    // The valid tree names its project snippets as project `demo`'s.
    clean.write("instrukt.yaml", "schema_version: 1\nproject: demo\n");
    valid_tree(&clean);
    let run = validate(&env, &clean, &["--json"]);
    assert!(run.success, "{}", run.err);
    insta::assert_snapshot!("json_clean", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn named_paths_limit_the_run_to_those_files_and_their_cross_snippet_checks() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/acme/concept/one.md",
        &snippet("concept", "visibility: x\n", ""),
    )
    .write(
        "docs/domain/acme/concept/two.md",
        &snippet("concept", "visibility: y\n", ""),
    )
    .write(
        "docs/domain/acme/procedure/rival.md",
        &snippet("procedure", "role: worker\nactivation_trigger: Also.\n", ""),
    );
    let one = validate(&env, &repo, &["docs/domain/acme/concept/one.md"]);
    assert!(!one.success);
    insta::assert_snapshot!("scoped_one_file", one.out);
    // The rival shares a role with the work overview: naming either file
    // reports the collision on both.
    let rival = validate(&env, &repo, &["docs/domain/acme/procedure/rival.md"]);
    assert!(!rival.success);
    insta::assert_snapshot!("scoped_role_clash", rival.out);
    let untouched = validate(&env, &repo, &["docs/domain/acme/policy/rules.md"]);
    assert!(untouched.success);
    insta::assert_snapshot!("scoped_clean_file", untouched.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn an_entry_procedure_requiring_its_own_sub_procedure_is_reported() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/acme/procedure/work/step.md",
        &snippet("procedure", "", ""),
    )
    .write(
        "docs/domain/acme/procedure/work/overview.md",
        &snippet(
            "procedure",
            "role: worker\nactivation_trigger: When work arrives.\n",
            "",
        )
        .replace(
            "# Sample — Procedure\n",
            "# Sample — Procedure\n\n## Required reads\n\n- acme/procedure/work/step\n",
        ),
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("subprocedure_required", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn references_to_domains_published_elsewhere_resolve_through_the_home() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    let home_docs = env.instrukt_home().join("docs/general/principle");
    std::fs::create_dir_all(&home_docs).unwrap();
    std::fs::write(home_docs.join("stewardship.md"), "published elsewhere\n").unwrap();
    repo.write(
        "docs/domain/acme/concept/borrowing.md",
        &snippet("concept", "", "").replace(
            "# Sample — Concept\n",
            "# Sample — Concept\n\n## Required reads\n\n- general/principle/stewardship\n\
             - general/principle/absent\n- general\n- nowhere\n",
        ),
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("home_references", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn local_snippets_in_a_repository_without_a_project_have_no_identity() {
    let env = TestEnv::new();
    let repo = env.git_repo("anonymous");
    repo.write("docs/project/spec/x.md", &snippet("spec", "", ""));
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("undeclared_project", run.out);
}

/// A project `flows` that publishes domain `crew`, whose workflow flavor
/// declares the roles `lead`, `reviewer` and `checker`.
fn publish_crew_flavor(env: &TestEnv) {
    let flows = env.git_repo("flows");
    flows
        .write(
            "instrukt.yaml",
            "schema_version: 1\nproject: flows\n",
        )
        .write(
            "docs/domain/crew/workflow/actions.yaml",
            "roles: [lead, reviewer, checker]\n\
             review:\n  command: instrukt docs get crew/procedure/review\n  template: templates/review.md\n",
        )
        .write("docs/domain/crew/workflow/templates/review.md", "Review {slug}.\n")
        .write(
            "docs/domain/crew/workflow/work.yaml",
            "flow: work\norchestrator: lead\ninitial: start\nstates: [start, reviewing]\n\
             transitions:\n- from: start\n  to: reviewing\n  when: {exists: prd.digest}\n  action: review\n",
        )
        .write(
            "docs/domain/crew/procedure/lead.md",
            &snippet("procedure", "role: lead\nactivation_trigger: When work is led.\n", ""),
        )
        .write(
            "docs/domain/crew/procedure/review.md",
            &snippet("procedure", "role: reviewer\nactivation_trigger: When work is reviewed.\n", ""),
        );
    flows.commit("publish the crew flavor");
    env.run(flows.path(), ["sync"]).success();
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn with_todos_on_a_procedure_role_the_selected_flavor_does_not_declare_is_reported() {
    let env = TestEnv::new();
    publish_crew_flavor(&env);
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: demo\nmodules: {todos: true}\nworkflow: {domain: crew}\n",
    )
    .write(
        "docs/project/procedure/check.md",
        &snippet(
            "procedure",
            "role: checker\nactivation_trigger: When checking.\n",
            "",
        ),
    )
    .write(
        "docs/project/procedure/haunt.md",
        &snippet(
            "procedure",
            "role: haunter\nactivation_trigger: At night.\n",
            "",
        ),
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("undeclared_flavor_role", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV4
#[test]
fn a_snippet_naming_another_by_its_path_is_reported_with_the_path() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/acme/concept/paths.md",
        &snippet(
            "concept",
            "",
            "\nRead docs/domain/acme/principle/care.md first, or `docs/domain/acme/principle/care.md` \
             in code, which is fine.\n\n## See also\n\n- acme/principle/gone\n- acme/principle/care\n",
        )
        .replace(
            "# Sample — Concept\n",
            "# Sample — Concept\n\n## Required reads\n\n- @~/.instrukt/docs/acme/principle/care.md\n\
             - acme/principle/nothing\n- two words\n",
        ),
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("path_references", run.out);
}

// spec: project/instrukt/spec/feature/docs/position-index UC-PI1
#[test]
fn a_publisher_learns_everything_their_indexes_cannot_say_in_one_run() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/general/principle/root.md",
        &snippet("principle", "", ""),
    )
    .write(
        "docs/general/policy/top.md",
        &snippet("policy", "", "").replace(
            "# Sample — Policy\n",
            "# Sample — Policy\n\n## Required reads\n\n- general/principle/root\n",
        ),
    )
    .write(
        "docs/general/spec/coming.md",
        &snippet("spec", "delivered_by: [build-coming]\n", ""),
    )
    .write(
        "docs/general/index.yaml",
        "expanded: [general/policy/top, general/spec/coming, nowhere/policy/x]\n\
         listed: [general/principle/root, not an id]\n\
         agents:\n  claude: [general/policy/top]\n  cursor: [general/policy/top]\n\
         bundles: [x]\n",
    )
    .write(
        "docs/domain/acme/index.yaml",
        "agents: {}\nexpanded: [general/policy/top]\nlisted: [acme/procedure/listen]\n",
    )
    .write(
        "docs/project/index.yaml",
        "description: The project.\nexpanded: project/demo/design/shape\n",
    )
    .write("docs/domain/acme/policy/index.yaml", "listed: []\n");
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("index_findings", run.out);
}

// spec: project/instrukt/spec/feature/docs/position-index UC-PI2
#[test]
fn a_publisher_is_told_a_domain_has_no_index_and_the_run_does_not_fail() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/beta/concept/thing.md",
        &snippet("concept", "", ""),
    );
    let run = validate(&env, &repo, &[]);
    assert!(run.success, "{}{}", run.out, run.err);
    insta::assert_snapshot!("domain_without_index", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV2
#[test]
fn a_required_read_another_required_read_already_brings_is_reported() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    // `rules` already requires `care`; `overlap` requires both, and also
    // `listen`, which nothing else brings.
    repo.write(
        "docs/domain/acme/concept/overlap.md",
        &snippet("concept", "", "").replace(
            "# Sample — Concept\n",
            "# Sample — Concept\n\n## Required reads\n\n- acme/principle/care\n\
             - acme/policy/rules\n- acme/procedure/listen\n",
        ),
    )
    .write(
        "docs/domain/acme/index.yaml",
        "description: The acme way of working.\nexpanded: [acme/policy/rules]\n\
         listed: [acme/procedure/listen, acme/procedure/work/overview, acme/concept/overlap]\n",
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}", run.err);
    insta::assert_snapshot!("required_read_implied", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV3
#[test]
fn a_snippet_nothing_names_is_a_notice_that_does_not_fail_the_run() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/project/concept/orphan.md",
        &snippet("concept", "", ""),
    );
    let run = validate(&env, &repo, &[]);
    assert!(run.success, "{}", run.err);
    insta::assert_snapshot!("unreferenced_snippet", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV3
#[test]
fn a_snippet_a_published_snippet_requires_is_not_unreferenced() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/acme/concept/borrowed.md",
        &snippet("concept", "", ""),
    );
    let crew = env.instrukt_home().join("docs/crew/concept");
    std::fs::create_dir_all(&crew).unwrap();
    std::fs::write(
        crew.join("uses.md"),
        snippet("concept", "", "").replace(
            "# Sample — Concept\n",
            "# Sample — Concept\n\n## Required reads\n\n- acme/concept/borrowed\n",
        ),
    )
    .unwrap();
    let run = validate(&env, &repo, &[]);
    assert!(run.success, "{}", run.err);
    insta::assert_snapshot!("referenced_from_the_computer", run.out);
}
