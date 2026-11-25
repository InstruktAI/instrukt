//! `instrukt docs validate`
//! (`docs/project/spec/feature/docs/snippet-validation.md`).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers: a failed setup step fails the test"
)]

use crate::common;
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
const MACHINE: &str =
    "schema_version: 1\nrunner: {runtime: t3}\nagents: {order: [claude], tier: standard}\n";

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
",
            "",
        ),
    )
    .write(
        "docs/domain/acme/procedure/listen.md",
        &snippet(
            "procedure",
            "role: listener\nsubscription_home: project\nsubscriptions: ['acme.events.*']\n",
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
                "activation_trigger: Now.\nsubscription_home: project\nsubscriptions: ['a.*']\nparameters: [{name: x}]\n",
                "",
            ),
        )
        .write(
            "docs/domain/acme/procedure/ghostly.md",
            &snippet(
                "procedure",
                "role: haunter\nactivation_trigger: At night.\n\
                 dispatch:\n  agent: [claude, ghost]\n  tier: strong\n",
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
        .write("docs/domain/acme/workflow/workflow.yaml", "roles: not-a-list\n")
        // Third-party
        .write(
            "docs/third-party/tool/broken.md",
            "---\nvisibility: public\ntype: spec\ntrue: unexpected\n---\n\n# Broken\n\n## Sources\n\n- somewhere on the web\n",
        )
        // Directives
        .write(
            "docs/domain/acme/concept/runs.md",
            &snippet("concept", "", "\n<!-- @exec: touch ran -->\n"),
        )
        .write(
            "docs/domain/acme/principle/session.md",
            &snippet("principle", "", "\n<!-- @config: runtime.session_id -->\n"),
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
        .write("instrukt.yaml", "schema_version: 1\nproject: flows\n")
        .write(
            "docs/domain/crew/workflow/workflow.yaml",
            "orchestrator: lead\nrequires: []\nroles: [lead, reviewer, checker]\n",
        )
        .write(
            "docs/domain/crew/procedure/lead.md",
            &snippet(
                "procedure",
                "role: lead\nactivation_trigger: When work is led.\n",
                "",
            ),
        )
        .write(
            "docs/domain/crew/procedure/review.md",
            &snippet(
                "procedure",
                "role: reviewer\nactivation_trigger: When work is reviewed.\n",
                "",
            ),
        );
    flows.commit("publish the crew flavor");
    env.run(flows.path(), ["sync"]).success();
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn with_todos_on_a_procedure_role_the_selected_flavor_does_not_declare_is_reported() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: demo\nmodules: {todos: true}\nworkflow: {domain: crew}\n",
    )
    .write(
        "docs/domain/crew/workflow/workflow.yaml",
        "orchestrator: lead\nrequires: []\nroles: [lead, reviewer, checker]\n",
    )
    .write(
        "docs/domain/crew/procedure/lead.md",
        &snippet(
            "procedure",
            "role: lead\nactivation_trigger: When work is led.\n",
            "",
        ),
    )
    .write(
        "docs/domain/crew/procedure/review.md",
        &snippet(
            "procedure",
            "role: reviewer\nactivation_trigger: When work is reviewed.\n",
            "",
        ),
    )
    .write(
        "docs/general/procedure/company-assistance.md",
        &snippet("procedure", "role: company-agent\n", ""),
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
         agents:\n  claude: [general/policy/top]\n  cursor: [general/policy/top]\n  1invalid: [general/policy/top]\n\
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

// spec: project/instrukt/spec/feature/docs/position-index UC-PI3
#[test]
fn an_operator_validating_a_companys_repository_is_told_nothing_about_its_index() {
    // Bytes that are no text: validation that opened the file before deciding
    // what it is would fail to read it.
    const NOT_TEXT: &[u8] = &[0xff, 0xfe, 0x00, 0x9f];
    let env = TestEnv::new();
    let clone = env.git_repo("acme-company");
    std::fs::create_dir_all(clone.path().join("docs")).unwrap();
    std::fs::write(clone.path().join("docs/index.yaml"), NOT_TEXT).unwrap();
    let company = validate(&env, &clone, &[]);
    assert!(company.success, "{}{}", company.out, company.err);
    insta::assert_snapshot!("company_index_left_alone", company.out);

    let repo = project(&env);
    valid_tree(&repo);
    std::fs::write(repo.path().join("docs/index.yaml"), NOT_TEXT).unwrap();
    let project = validate(&env, &repo, &[]);
    assert!(!project.success, "{}", project.err);
    insta::assert_snapshot!("project_index_at_no_position", project.out);
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

/// A leaf requiring an overview and the operations domain. The domain's
/// expanded snippet itself requires another snippet, exercising closure.
fn required_domain_tree(repo: &Repo, overview_reads: &str) {
    valid_tree(repo);
    repo.write(
        "docs/domain/ops/index.yaml",
        "description: Operations.\nexpanded: [ops/concept/operations]\n",
    )
    .write(
        "docs/domain/ops/concept/operations.md",
        &snippet("concept", "", "").replace(
            "# Sample — Concept\n",
            "# Sample — Concept\n\n## Required reads\n\n- acme/principle/care\n",
        ),
    )
    .write(
        "docs/project/concept/overview.md",
        &snippet("concept", "", "").replace(
            "# Sample — Concept\n",
            &format!("# Sample — Concept\n\n## Required reads\n\n{overview_reads}"),
        ),
    )
    .write(
        "docs/project/concept/leaf.md",
        &snippet("concept", "", "").replace(
            "# Sample — Concept\n",
            "# Sample — Concept\n\n## Required reads\n\n- project/demo/concept/overview\n- ops\n",
        ),
    )
    .write(
        "docs/project/index.yaml",
        "listed: [project/demo/adr/0049-choice, project/demo/design/retired, project/demo/concept/leaf]\n",
    );
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV2
#[test]
fn a_required_domain_the_overview_already_brings_is_reported() {
    let env = TestEnv::new();
    let repo = project(&env);
    required_domain_tree(&repo, "- ops\n");
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}{}", run.out, run.err);
    insta::assert_snapshot!("required_domain_implied", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV2
#[test]
fn a_required_domain_implied_through_its_expanded_snippets_is_reported() {
    let env = TestEnv::new();
    let repo = project(&env);
    required_domain_tree(&repo, "- ops/concept/operations\n");
    let run = validate(&env, &repo, &[]);
    assert!(!run.success, "{}{}", run.out, run.err);
    insta::assert_snapshot!("required_domain_expansion_implied", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV2
#[test]
fn a_required_domain_only_partly_brought_by_the_overview_is_not_redundant() {
    let env = TestEnv::new();
    let repo = project(&env);
    required_domain_tree(&repo, "- ops/concept/operations\n");
    repo.write(
        "docs/domain/ops/index.yaml",
        "description: Operations.\nexpanded: [ops/concept/operations, ops/concept/independent]\n",
    )
    .write(
        "docs/domain/ops/concept/independent.md",
        &snippet("concept", "", ""),
    );
    let run = validate(&env, &repo, &[]);
    assert!(run.success, "{}{}", run.out, run.err);
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

/// Replace the rules snippet with a linked required read and soft references.
fn linked_rules(repo: &Repo, read: &str, see_also: &str) {
    repo.write(
        "docs/domain/acme/policy/rules.md",
        &snippet("policy", "", &format!("\n## See Also\n\n{see_also}\n")).replace(
            "# Sample — Policy\n",
            &format!("# Sample — Policy\n\n## Required reads\n\n- {read}\n"),
        ),
    );
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV5
#[test]
fn same_repository_links_and_plain_references_validate_together() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    linked_rules(
        &repo,
        "[acme/principle/care](../principle/care.md)",
        "- [acme/procedure/listen](../procedure/listen.md)\n- acme/principle/care",
    );
    let run = validate(&env, &repo, &[]);
    assert!(run.success, "{}{}", run.out, run.err);
    insta::assert_snapshot!("linked_references_valid", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV5
#[test]
fn a_link_to_a_moved_snippet_reports_the_stale_path() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    linked_rules(
        &repo,
        "acme/principle/care",
        "- [acme/principle/care](../principle/old-care.md)",
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success);
    insta::assert_snapshot!("linked_reference_moved", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV5
#[test]
fn a_link_to_an_existing_file_of_another_id_is_reported() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    linked_rules(
        &repo,
        "[acme/principle/care](../procedure/listen.md)",
        "- acme/principle/care",
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success);
    insta::assert_snapshot!("linked_reference_wrong_file", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV5
#[test]
fn reference_link_text_must_be_exactly_one_snippet_id() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    linked_rules(
        &repo,
        "[acme/principle/care and more](../principle/care.md)",
        "- [Care](../principle/care.md)\n- [acme](../principle/care.md)",
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success);
    insta::assert_snapshot!("linked_reference_text", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV5
#[test]
fn a_link_cannot_stand_for_a_reference_to_another_repository() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    let other = env.instrukt_home().join("docs/general/principle");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("elsewhere.md"), "published elsewhere\n").unwrap();
    linked_rules(
        &repo,
        "acme/principle/care",
        "- [general/principle/elsewhere](../principle/care.md)\n- general/principle/elsewhere",
    );
    let run = validate(&env, &repo, &[]);
    assert!(!run.success);
    insta::assert_snapshot!("linked_reference_foreign", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV5
#[test]
fn reference_links_must_stay_inside_the_working_tree() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    let outside = env.path("outside.md");
    std::fs::write(&outside, snippet("principle", "", "")).unwrap();
    std::os::unix::fs::symlink(
        &outside,
        repo.path().join("docs/domain/acme/principle/escape.md"),
    )
    .unwrap();
    linked_rules(
        &repo,
        "acme/principle/care",
        "- [acme/principle/care](../../../../../outside.md)\n\
         - [acme/principle/care](../principle/escape.md)",
    );
    let run = validate(&env, &repo, &["docs/domain/acme/policy/rules.md"]);
    assert!(!run.success);
    insta::assert_snapshot!("linked_reference_outside", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV2
#[test]
fn linked_required_reads_take_part_in_redundancy_checks_by_id() {
    let env = TestEnv::new();
    let repo = project(&env);
    valid_tree(&repo);
    repo.write(
        "docs/domain/acme/concept/overlap.md",
        &snippet("concept", "", "").replace(
            "# Sample — Concept\n",
            "# Sample — Concept\n\n## Required reads\n\n\
             - [acme/principle/care](../principle/care.md)\n\
             - [acme/policy/rules](../policy/rules.md)\n",
        ),
    );
    let run = validate(&env, &repo, &["docs/domain/acme/concept/overlap.md"]);
    assert!(!run.success);
    insta::assert_snapshot!("linked_required_read_implied", run.out);
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV6
// spec: project/instrukt/spec/feature/configuration/roles-and-capabilities UC-RCAP2
#[test]
fn independent_subscriptions_use_their_own_roles_and_publish_the_implicit_handler() {
    let env = TestEnv::new();
    publish_crew_flavor(&env);
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml","schema_version: 1\nproject: demo\nmodules: {todos: true, events: true}\nworkflow: {domain: crew}\n");
    for (name, role) in [
        ("explicit", "role: custom-handler\n"),
        ("implicit", ""),
        ("second-default", ""),
    ] {
        repo.write(
            &format!("docs/project/procedure/{name}.md"),
            &snippet(
                "procedure",
                &format!("{role}subscriptions: ['runner.*']\nsubscription_home: project\n"),
                "",
            ),
        );
    }
    let run = validate(&env, &repo, &[]);
    assert!(run.success, "{}{}", run.out, run.err);
    repo.commit("publish independent handlers");
    env.run(repo.path(), ["sync"]).success();
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let index = instrukt_core::index::GlobalIndex::load(&home)
        .unwrap()
        .unwrap();
    let implicit = index
        .snippets
        .iter()
        .find(|entry| entry.id.as_str() == "project/demo/procedure/implicit")
        .unwrap();
    assert_eq!(implicit.role.as_deref(), Some("event-handler"));
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV7
#[test]
fn pending_docs_validate_without_a_todo_store_and_without_writing_home_state() {
    let empty_env = TestEnv::new();
    let repo = empty_env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: {}\nproject: demo\nmodules: {{todos: true}}\nworkflow: {{domain: crew}}\ngithub: {{repository: acme/demo}}\n",
            instrukt_core::config::PROJECT_SCHEMA_VERSION
        ),
    )
    .write(
        "docs/domain/crew/workflow/workflow.yaml",
        "orchestrator: lead\nrequires: []\nroles: [lead, reviewer, checker]\n",
    )
    .write(
        "docs/domain/crew/procedure/lead.md",
        &snippet("procedure", "role: lead\nactivation_trigger: When work is led.\n", ""),
    )
    .write(
        "docs/domain/crew/procedure/review.md",
        &snippet(
            "procedure",
            "role: reviewer\nactivation_trigger: When work is reviewed.\n",
            "",
        ),
    )
    .write(
        "docs/domain/crew/index.yaml",
        "description: The crew workflow.\nlisted: [crew/procedure/lead, crew/procedure/review]\n",
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
        "docs/project/index.yaml",
        "listed: [project/demo/procedure/check, project/demo/spec/live]\n",
    )
    .write(
        "docs/project/spec/live.md",
        &snippet(
            "spec",
            "",
            "\n<!-- planned:plan-work -->\nAdded by plan-work.\n<!-- /planned:plan-work -->\n\n\
             <!-- planned-change:plan-work -->\nCurrent.\n<!-- change:plan-work -->\nReplaced.\n\
             <!-- /planned-change:plan-work -->\n",
        ),
    )
    .write(
        "docs/project/spec/fresh.md",
        &snippet("spec", "delivered_by: [plan-work]\n", ""),
    );

    let db_path = empty_env.instrukt_home().join("state/instrukt.db");
    let lock_path = empty_env.instrukt_home().join("state/writer.lock");

    // Clause 1: --no-todo-lookup validates pending markers in an isolated empty home
    // without creating state files, while still enforcing slug form, marker structure
    // and workflow roles.
    let clean = validate(&empty_env, &repo, &["--no-todo-lookup"]);
    assert!(clean.success, "{}{}", clean.out, clean.err);
    assert!(clean.out.is_empty(), "{}", clean.out);
    assert!(!db_path.exists());
    assert!(!lock_path.exists());

    repo.write(
        "docs/project/procedure/check.md",
        &snippet(
            "procedure",
            "role: haunter\nactivation_trigger: When checking.\n",
            "",
        ),
    )
    .write(
        "docs/project/concept/bad.md",
        &snippet(
            "concept",
            "",
            "\n<!-- planned:Bad_Slug -->\nBad slug and unbalanced marker.\n",
        ),
    );
    let structural = validate(&empty_env, &repo, &["--no-todo-lookup"]);
    assert!(!structural.success, "{}", structural.err);
    insta::assert_snapshot!("no_todo_lookup_structural_findings", structural.out);
    assert!(!db_path.exists());
    assert!(!lock_path.exists());

    // Restore valid docs before Clause 2.
    std::fs::remove_file(repo.path().join("docs/project/concept/bad.md")).unwrap();
    repo.write(
        "docs/project/procedure/check.md",
        &snippet(
            "procedure",
            "role: checker\nactivation_trigger: When checking.\n",
            "",
        ),
    );

    // Clause 2: default docs validate (without --no-todo-lookup) against the empty home
    // reports the unavailable todo lookup and leaves neither state DB nor writer lock.
    let unavailable = validate(&empty_env, &repo, &[]);
    assert!(!unavailable.success, "{}", unavailable.out);
    assert!(unavailable.out.is_empty(), "{}", unavailable.out);
    insta::assert_snapshot!("todo_lookup_unavailable_empty_home", unavailable.err);
    assert!(!db_path.exists());
    assert!(!lock_path.exists());

    // Clause 3: default docs validate, run from an agent session, against a home whose
    // state database records the todo in an older release's schema succeeds while
    // writer.lock is held, and leaves the database file, its schema version and its
    // absent sidecars exactly as found: no migration, no purge, no lock.
    let mut work = crate::fixtures::work::Project::new();
    publish_crew_flavor(&work.env);
    work.repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: {}\nproject: demo\nboard: acme/1\nmodules:\n  todos: true\nworkflow:\n  domain: crew\ngithub:\n  repository: {}\n",
            instrukt_core::config::PROJECT_SCHEMA_VERSION,
            crate::fixtures::work::REPOSITORY
        ),
    );
    let state = work.env.instrukt_home().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let work_db = state.join("instrukt.db");
    let sidecars =
        ["-wal", "-shm", "-journal"].map(|side| state.join(format!("instrukt.db{side}")));
    for file in std::iter::once(&work_db).chain(&sidecars) {
        if file.exists() {
            std::fs::remove_file(file).unwrap();
        }
    }
    let stored_version = || -> u32 {
        instrukt_core::state_db::rusqlite::Connection::open(&work_db)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    };
    {
        let db = instrukt_core::state_db::rusqlite::Connection::open(&work_db).unwrap();
        db.execute_batch(include_str!("../fixtures/state-v35.sql"))
            .unwrap();
        db.execute_batch(
            "INSERT INTO todos(id,project,slug,kind,status,created_at,created_by,revision,catch_up,sequence,removed_at) VALUES
             ('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','demo','plan-work','change','active','2026-11-20T00:00:00Z','{\"person\":\"Test Developer\"}',1,'once',1,NULL),
             ('bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','demo','expired-work','change','removed','2026-11-20T00:00:00Z','{\"person\":\"Test Developer\"}',2,'once',2,'2020-01-01T00:00:00Z');",
        )
        .unwrap();
    }
    assert!(stored_version() < instrukt_core::state_db::schema_version());
    let version = stored_version();
    let bytes = std::fs::read(&work_db).unwrap();
    // The command runs as an agent session, so admission resolves its caller.
    work.env.set_env("T3CODE_THREAD_ID", "validating-session");

    work.repo
        .write(
            "docs/project/index.yaml",
            "listed: [project/demo/spec/live]\n",
        )
        .write(
            "docs/project/spec/live.md",
            &snippet(
                "spec",
                "",
                "\n<!-- planned:plan-work -->\nAdded by plan-work.\n<!-- /planned:plan-work -->\n\n\
                 <!-- planned-change:plan-work -->\nCurrent.\n<!-- change:plan-work -->\nReplaced.\n\
                 <!-- /planned-change:plan-work -->\n",
            ),
        )
        .write(
            "docs/project/spec/fresh.md",
            &snippet("spec", "delivered_by: [plan-work]\n", ""),
        );

    let lock = instrukt_core::fsx::LockGuard::acquire(
        &work.env.instrukt_home().join("state/writer.lock"),
        std::time::Duration::from_secs(2),
    )
    .unwrap();
    let backed = validate(&work.env, &work.repo, &[]);
    assert!(backed.success, "{}{}", backed.out, backed.err);
    assert!(backed.out.is_empty(), "{}", backed.out);
    drop(lock);

    assert_eq!(stored_version(), version);
    assert!(
        std::fs::read(&work_db).unwrap() == bytes,
        "validation changed the state database file"
    );
    for sidecar in &sidecars {
        assert!(!sidecar.exists(), "{}", sidecar.display());
    }
}
