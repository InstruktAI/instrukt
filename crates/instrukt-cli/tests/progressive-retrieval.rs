//! `instrukt docs index|get`
//! (`docs/project/spec/feature/docs/progressive-retrieval.md`).
//!
//! Every home is published by the real `instrukt sync` from temporary
//! project repositories (`support/documented.rs`).

#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/documented.rs"]
mod documented;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use documented::{project, snippet, sync};

/// Command output with the test's temporary directory shown as `[TMP]`.
fn text(env: &TestEnv, bytes: &[u8]) -> String {
    let base = env.home().parent().unwrap().display().to_string();
    common::mask_published_commits(&String::from_utf8_lossy(bytes).replace(&base, "[TMP]"))
}

fn stdout(env: &TestEnv, assert: &Assert) -> String {
    text(env, &assert.get_output().stdout)
}

fn stderr(env: &TestEnv, assert: &Assert) -> String {
    text(env, &assert.get_output().stderr)
}

const CARE: &str = "- acme/principle/care";
const RULES: &str = "- acme/policy/rules";
const TERMS: &str = "- acme/concept/terms";
const BETA: &str = "- beta";

/// What project `demo` documents: three domains (`acme` and `beta` with an
/// index, `gamma` without), fragments (one carrying a required read), a
/// third-party doc and two local snippets.
fn demo_files() -> Vec<(&'static str, String)> {
    vec![
        (
            "docs/domain/acme/principle/care.md",
            snippet("principle", "Care", "Why care matters.", "", &[], "Care.\n"),
        ),
        (
            "docs/domain/acme/policy/rules.md",
            snippet(
                "policy",
                "Rules",
                "The rules of the work.",
                "",
                &[CARE],
                "Follow the rules.\n\n<!-- @include: docs/snippets/shared.md -->\n\n\
                 <!-- @include: docs/snippets/missing.md -->\n\n\
                 A reference is written `@docs/project/design/y.md`, for example:\n\n\
                 ```\n- @~/.instrukt/docs/acme/principle/care.md\n```\n",
            ),
        ),
        (
            "docs/domain/acme/concept/terms.md",
            snippet(
                "concept",
                "Terms",
                "The words used.",
                "",
                &[CARE],
                "Terms.\n",
            ),
        ),
        (
            "docs/domain/acme/procedure/work.md",
            snippet(
                "procedure",
                "Work",
                "How the work is done.",
                "activation_trigger: When work arrives.\n",
                &[RULES, BETA],
                "Work.\n",
            ),
        ),
        (
            "docs/domain/acme/procedure/start.md",
            snippet(
                "procedure",
                "Start",
                "Starts a session.",
                "role: starter\n",
                &[RULES, "<!-- @include: docs/snippets/reads.md -->", BETA],
                "Start.\n",
            ),
        ),
        (
            "docs/domain/acme/procedure/greet.md",
            snippet(
                "procedure",
                "Greet",
                "Greets a newcomer.",
                "activation_trigger: When someone arrives.\n",
                &[],
                "Greet.\n",
            ),
        ),
        (
            "docs/domain/acme/index.yaml",
            "description: How acme works.\nexpanded: [acme/procedure/work]\n\
             listed: [acme/concept/terms]\n"
                .to_owned(),
        ),
        (
            "docs/domain/beta/spec/thing.md",
            snippet(
                "spec",
                "Thing",
                "What a thing is.",
                "",
                &[],
                "### Detail\n\nA thing.\n",
            ),
        ),
        (
            "docs/domain/beta/index.yaml",
            "description: What beta holds.\nexpanded: [beta/spec/thing]\n".to_owned(),
        ),
        (
            "docs/domain/gamma/concept/loose.md",
            snippet("concept", "Loose", "Not on any map.", "", &[], "Loose.\n"),
        ),
        (
            "docs/global/snippets/shared.md",
            "Shared text.\n".to_owned(),
        ),
        ("docs/global/snippets/reads.md", format!("{TERMS}\n")),
        (
            "docs/third-party/tool/guide.md",
            "---\ndescription: How the tool works.\n---\n\n# Tool guide\n\nUse it.\n".to_owned(),
        ),
        (
            "docs/project/spec/feature/x.md",
            snippet(
                "spec",
                "X",
                "The x feature.",
                "",
                &["- project/demo/design/y"],
                "The x feature.\n",
            ),
        ),
        (
            "docs/project/design/y.md",
            snippet("design", "Y", "How y is built.", "", &[], "Y.\n"),
        ),
    ]
}

/// Project `demo`, committed and published.
fn published_demo(env: &TestEnv) -> Repo {
    let demo = project(env, "demo", &demo_files());
    sync(env, &demo);
    demo
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_lists_what_its_task_concerns_and_reads_only_what_it_selects() {
    let env = TestEnv::new();
    let demo = published_demo(&env);

    let listed = env
        .run(
            demo.path(),
            [
                "docs",
                "index",
                "--domain",
                "acme",
                "--areas",
                "policy,procedure",
            ],
        )
        .success();
    insta::assert_snapshot!("filtered-index", stdout(&env, &listed));

    let read = env
        .run(demo.path(), ["docs", "get", "acme/procedure/work"])
        .success();
    insta::assert_snapshot!("procedure-with-reads-not-loaded", stdout(&env, &read));

    // A required read that was asked for is loaded, not listed; the
    // fragment, the missing fragment's directive and the stripped section
    // show the read pipeline.
    let both = env
        .run(
            demo.path(),
            ["docs", "get", "acme/policy/rules", "acme/principle/care"],
        )
        .success();
    insta::assert_snapshot!("policy-with-its-read", stdout(&env, &both));

    // A required read a fragment carries is named like any other.
    let start = env
        .run(demo.path(), ["docs", "get", "acme/procedure/start"])
        .success();
    insta::assert_snapshot!("fragment-required-read-not-loaded", stdout(&env, &start));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_sees_its_own_project_and_reads_another_projects_local_documentation() {
    let env = TestEnv::new();
    let demo = published_demo(&env);
    let other = project(
        &env,
        "other",
        &[(
            "docs/project/concept/notes.md",
            snippet("concept", "Notes", "Other's notes.", "", &[], "Notes.\n"),
        )],
    );
    sync(&env, &other);

    let in_demo = env.run(demo.path(), ["docs", "index"]).success();
    insta::assert_snapshot!("default-index-in-a-project", stdout(&env, &in_demo));

    let outside = env.path("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    let published_only = env.run(&outside, ["docs", "index"]).success();
    insta::assert_snapshot!(
        "default-index-outside-projects",
        stdout(&env, &published_only)
    );

    let others = env
        .run(&outside, ["docs", "index", "--project", "other"])
        .success();
    insta::assert_snapshot!("other-project-index", stdout(&env, &others));

    let read = env
        .run(&outside, ["docs", "get", "project/other/concept/notes"])
        .success();
    insta::assert_snapshot!("other-project-local-read", stdout(&env, &read));

    let local = env
        .run(demo.path(), ["docs", "get", "project/demo/spec/feature/x"])
        .success();
    insta::assert_snapshot!("own-local-read", stdout(&env, &local));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_adds_third_party_documentation() {
    let env = TestEnv::new();
    let demo = published_demo(&env);

    let with_third_party = env
        .run(demo.path(), ["docs", "index", "--third-party"])
        .success();
    insta::assert_snapshot!("third-party-index", stdout(&env, &with_third_party));

    let guide = env
        .run(demo.path(), ["docs", "get", "third-party/tool/guide"])
        .success();
    insta::assert_snapshot!("third-party-read", stdout(&env, &guide));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_reads_the_general_owners_third_party_documentation_as_global() {
    let env = TestEnv::new();
    let platform = project(
        &env,
        "platform",
        &[
            (
                "docs/general/principle/care.md",
                snippet("principle", "Care", "Why care matters.", "", &[], "Care.\n"),
            ),
            (
                "docs/third-party/tool/guide.md",
                "---\ndescription: How the tool works.\n---\n\n# Tool guide\n\nUse it.\n"
                    .to_owned(),
            ),
        ],
    );
    sync(&env, &platform);

    let guide = env
        .run(platform.path(), ["docs", "get", "third-party/tool/guide"])
        .success();
    insta::assert_snapshot!("general-owner-third-party-read", stdout(&env, &guide));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn the_general_owner_is_known_by_its_claim_even_without_snippets() {
    let env = TestEnv::new();
    // `general` is published with an empty index and no snippet: no
    // index entry names its owner.
    let platform = project(
        &env,
        "platform",
        &[
            ("docs/general/index.yaml", String::new()),
            (
                "docs/third-party/tool/guide.md",
                "---\ndescription: How the tool works.\n---\n\n# Tool guide\n\nUse it.\n"
                    .to_owned(),
            ),
        ],
    );
    sync(&env, &platform);

    let guide = env
        .run(platform.path(), ["docs", "get", "third-party/tool/guide"])
        .success();
    insta::assert_snapshot!(
        "general-owner-without-snippets-third-party-read",
        stdout(&env, &guide)
    );
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_is_warned_when_main_is_ahead_of_what_was_published() {
    let env = TestEnv::new();
    let demo = published_demo(&env);
    let published = demo.head();
    demo.write("README.md", "more\n");
    let main = demo.commit("docs: more");

    let listed = env
        .run(demo.path(), ["docs", "index", "--domain", "beta"])
        .success();
    let listed = stdout(&env, &listed)
        .replace(&main, "[MAIN]")
        .replace(&published, "[PUBLISHED]");
    insta::assert_snapshot!("index-behind-main", listed);

    let read = env
        .run(demo.path(), ["docs", "get", "beta/spec/thing"])
        .success();
    let read = stdout(&env, &read)
        .replace(&main, "[MAIN]")
        .replace(&published, "[PUBLISHED]");
    insta::assert_snapshot!("get-behind-main", read);
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_asking_for_what_is_not_there_is_told_so() {
    let env = TestEnv::new();
    let cwd = env.path("");

    let nothing = env.run(&cwd, ["docs", "index"]).success();
    insta::assert_snapshot!("nothing-published-index", stdout(&env, &nothing));
    let unpublished = env
        .run(&cwd, ["docs", "get", "acme/policy/rules"])
        .failure();
    insta::assert_snapshot!("nothing-published-get", stderr(&env, &unpublished));

    let demo = published_demo(&env);
    let unknown = env
        .run(
            demo.path(),
            ["docs", "get", "acme/policy/rules", "acme/policy/none"],
        )
        .failure();
    assert!(
        unknown.get_output().stdout.is_empty(),
        "known ids are not rendered"
    );
    insta::assert_snapshot!("unknown-id", stderr(&env, &unknown));

    let domain = env
        .run(
            demo.path(),
            ["docs", "index", "--domain", "acme", "--domain", "delta"],
        )
        .failure();
    insta::assert_snapshot!("unknown-domain", stderr(&env, &domain));

    let nobody = env
        .run(demo.path(), ["docs", "index", "--project", "nobody"])
        .success();
    insta::assert_snapshot!("unknown-project", stdout(&env, &nobody));

    std::fs::remove_file(demo.path().join("docs/project/design/y.md")).unwrap();
    let missing = env
        .run(demo.path(), ["docs", "get", "project/demo/design/y"])
        .failure();
    insta::assert_snapshot!("missing-local-file", stderr(&env, &missing));
}

/// Procedures with root frontmatter whose closures must refuse: one
/// requires a pending snippet, one requires through a fragment a pending
/// snippet, one requires through a fragment what is not published; and an
/// overview requiring a pending snippet, whose closure leaves it out.
fn refused_files() -> Vec<(&'static str, String)> {
    vec![
        (
            "docs/domain/acme/procedure/blocked.md",
            snippet(
                "procedure",
                "Blocked",
                "Cannot start yet.",
                "role: blocked\n",
                &["- acme/spec/coming"],
                "Wait.\n",
            ),
        ),
        (
            "docs/domain/acme/procedure/blocked-through-fragment.md",
            snippet(
                "procedure",
                "Blocked through fragment",
                "Cannot start yet either.",
                "role: blocked-too\n",
                &["<!-- @include: docs/snippets/coming.md -->"],
                "Wait.\n",
            ),
        ),
        (
            "docs/domain/acme/procedure/broken.md",
            snippet(
                "procedure",
                "Broken",
                "Requires what is not published.",
                "activation_trigger: When broken.\n",
                &["<!-- @include: docs/snippets/absent.md -->"],
                "No.\n",
            ),
        ),
        (
            "docs/domain/acme/spec/coming.md",
            snippet(
                "spec",
                "Coming",
                "Not delivered yet.",
                "delivered_by: [build-coming]\n",
                &[],
                "Soon.\n",
            ),
        ),
        (
            "docs/domain/acme/design/overview.md",
            snippet(
                "design",
                "Overview",
                "How acme fits together.",
                "",
                &["- acme/spec/coming", RULES],
                "Overview.\n",
            ),
        ),
        (
            "docs/global/snippets/coming.md",
            "- acme/spec/coming\n".to_owned(),
        ),
        (
            "docs/global/snippets/absent.md",
            "- acme/spec/absent\n".to_owned(),
        ),
    ]
}

/// Project `demo` with the procedures closures read and refuse, published.
fn published_closures(env: &TestEnv) -> Repo {
    let mut files = demo_files();
    files.extend(refused_files());
    let demo = project(env, "demo", &files);
    sync(env, &demo);
    demo
}

/// The refusal of the closure of `id`: nothing on stdout, the reason on
/// stderr.
fn refused_closure(env: &TestEnv, demo: &Repo, id: &str) -> String {
    let refused = env
        .run(demo.path(), ["docs", "get", "--closure", id])
        .failure();
    assert!(refused.get_output().stdout.is_empty(), "{id}");
    stderr(env, &refused)
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR2
#[test]
fn an_agent_starts_a_procedure_with_everything_it_requires_in_one_read() {
    let env = TestEnv::new();
    let demo = published_closures(&env);

    // `start` requires `rules`, through a fragment `terms`, and domain
    // `beta`; both snippets require `care`, which the closure holds once,
    // and `beta` brings its expanded `thing`.
    let closure = env
        .run(
            demo.path(),
            ["docs", "get", "--closure", "acme/procedure/start"],
        )
        .success();
    insta::assert_snapshot!("closure", stdout(&env, &closure));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR2
#[test]
fn a_closure_whose_required_read_is_pending_is_refused_naming_it() {
    let env = TestEnv::new();
    let demo = published_closures(&env);
    insta::assert_snapshot!(
        "closure-refused-pending",
        refused_closure(&env, &demo, "acme/procedure/blocked")
    );
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR2
#[test]
fn a_closure_whose_fragment_requires_a_pending_read_is_refused_naming_it() {
    let env = TestEnv::new();
    let demo = published_closures(&env);
    insta::assert_snapshot!(
        "closure-refused-pending-through-fragment",
        refused_closure(&env, &demo, "acme/procedure/blocked-through-fragment")
    );
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR2
#[test]
fn a_closure_whose_required_read_cannot_be_read_is_refused_naming_it() {
    let env = TestEnv::new();
    let demo = published_closures(&env);
    insta::assert_snapshot!(
        "closure-refused-unreadable",
        refused_closure(&env, &demo, "acme/procedure/broken")
    );
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR2
#[test]
fn a_closure_is_read_for_exactly_one_snippet() {
    let env = TestEnv::new();
    let demo = published_closures(&env);

    let two = env
        .run(
            demo.path(),
            [
                "docs",
                "get",
                "--closure",
                "acme/procedure/start",
                "acme/procedure/work",
            ],
        )
        .failure();
    insta::assert_snapshot!("closure-of-two-ids", stderr(&env, &two));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR2
#[test]
fn a_closure_of_a_third_party_doc_is_refused() {
    let env = TestEnv::new();
    let demo = published_closures(&env);
    insta::assert_snapshot!(
        "closure-of-a-third-party-doc",
        refused_closure(&env, &demo, "third-party/tool/guide")
    );
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR2
#[test]
fn a_closure_of_any_snippet_leaves_out_and_names_what_cannot_be_read() {
    let env = TestEnv::new();
    let demo = published_closures(&env);

    let overview = env
        .run(
            demo.path(),
            ["docs", "get", "--closure", "acme/design/overview"],
        )
        .success();
    insta::assert_snapshot!("closure-of-an-overview", stdout(&env, &overview));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR4
#[test]
fn an_agent_reads_a_domain_with_what_it_shows_in_full_and_what_it_lists() {
    let env = TestEnv::new();
    let demo = published_demo(&env);

    // `acme` expands `work`, which requires `rules` (requiring `care`) and
    // domain `beta`; it lists `terms` and has the triggered `greet`.
    let read = env
        .run(demo.path(), ["docs", "get", "--domain", "acme"])
        .success();
    insta::assert_snapshot!("domain-read", stdout(&env, &read));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR4
#[test]
fn an_agent_reading_a_domain_without_an_index_is_told_how_to_list_it() {
    let env = TestEnv::new();
    let demo = published_demo(&env);

    let read = env
        .run(demo.path(), ["docs", "get", "--domain", "gamma"])
        .success();
    insta::assert_snapshot!("domain-without-index", stdout(&env, &read));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR4
#[test]
fn an_agent_reading_a_reserved_name_as_a_domain_is_told_the_valid_ones() {
    let env = TestEnv::new();
    let demo = published_demo(&env);

    let reserved = env
        .run(demo.path(), ["docs", "get", "--domain", "project"])
        .failure();
    insta::assert_snapshot!("domain-reserved", stderr(&env, &reserved));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR7
#[test]
fn an_agent_finds_where_a_domain_is_authored() {
    let env = TestEnv::new();
    let demo = project(&env, "demo", &demo_files());
    demo.git([
        "remote",
        "add",
        "origin",
        "https://example.invalid/demo/docs.git",
    ]);
    sync(&env, &demo);

    let listed = env
        .run(
            demo.path(),
            ["docs", "index", "--domain", "beta", "--domain", "acme"],
        )
        .success();
    insta::assert_snapshot!("domain-sources", stdout(&env, &listed));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR8
#[test]
fn an_agent_finds_the_file_behind_a_snippet_it_read() {
    let env = TestEnv::new();
    published_demo(&env);
    let outside = env.path("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();

    let read = env
        .run(&outside, ["docs", "get", "beta/spec/thing"])
        .success();
    insta::assert_snapshot!("snippet-source", stdout(&env, &read));
}

const RUNTIMES: &str = "schema_version: 1
runtimes:
  - name: agent-cli
    agent: agent-cli
    session_id_env: AGENT_CLI_SESSION_ID
    start: agent-cli
    send: agent-cli send {runtime.session_id}
    alive: agent-cli alive {runtime.session_id}
    conversation: agent-cli history {runtime.session_id}
";

/// A machine whose runtime mapping knows one agent CLI.
fn machine(env: &TestEnv) {
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    std::fs::write(env.instrukt_home().join("instrukt.yaml"), RUNTIMES).unwrap();
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR3
#[test]
fn each_agent_reads_the_value_of_its_own_session_when_it_reads() {
    let mut env = TestEnv::new();
    machine(&env);
    let procedure = snippet(
        "procedure",
        "Report",
        "Reports into the session.",
        "",
        &[],
        // A structured value prints as YAML, as `instrukt config get` does.
        "Your session:\n\n<!-- @config: runtime.session_id -->\n\nThe modules:\n\n\
         <!-- @config: modules -->\n\nThen report.\n",
    );
    let demo = project(
        &env,
        "demo",
        &[("docs/domain/acme/procedure/report.md", procedure.clone())],
    );
    sync(&env, &demo);

    env.set_env("AGENT_CLI_SESSION_ID", "session-one");
    let first = env
        .run(demo.path(), ["docs", "get", "acme/procedure/report"])
        .success();
    insta::assert_snapshot!("first-session", stdout(&env, &first));
    env.set_env("AGENT_CLI_SESSION_ID", "session-two");
    let second = env
        .run(demo.path(), ["docs", "get", "acme/procedure/report"])
        .success();
    insta::assert_snapshot!("second-session", stdout(&env, &second));

    let published =
        std::fs::read_to_string(env.instrukt_home().join("docs/acme/procedure/report.md")).unwrap();
    assert_eq!(
        published, procedure,
        "the published procedure keeps the request"
    );
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR3
#[test]
fn each_agent_reads_the_value_of_the_project_it_stands_in() {
    let env = TestEnv::new();
    let procedure = snippet(
        "procedure",
        "Verify",
        "Runs the project's tests.",
        "",
        &[],
        "Run the tests:\n\n<!-- @config: actions.test -->\n",
    );
    let demo = project(
        &env,
        "demo",
        &[("docs/domain/acme/procedure/verify.md", procedure)],
    );
    demo.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: demo\n\
         actions:\n  test: make test\n",
    );
    demo.commit("chore: the test action");
    sync(&env, &demo);
    let other = project(&env, "other", &[]);
    other.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: other\nactions:\n  test: cargo test --workspace\n",
    );

    let in_demo = env
        .run(demo.path(), ["docs", "get", "acme/procedure/verify"])
        .success();
    insta::assert_snapshot!("value-of-demo", stdout(&env, &in_demo));
    let in_other = env
        .run(other.path(), ["docs", "get", "acme/procedure/verify"])
        .success();
    insta::assert_snapshot!("value-of-other", stdout(&env, &in_other));

    // Outside every project the global layer alone answers, and it has no
    // test action.
    let outside = env.path("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    let refused = env
        .run(&outside, ["docs", "get", "acme/procedure/verify"])
        .failure();
    insta::assert_snapshot!("value-outside-projects", stdout(&env, &refused));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR3
#[test]
fn a_value_that_cannot_be_resolved_is_named_with_why_and_the_read_fails() {
    let env = TestEnv::new();
    machine(&env);
    // Read outside every runtime session, and naming nothing configured.
    let fragile = snippet(
        "procedure",
        "Fragile",
        "Asks for values it cannot have.",
        "",
        &[],
        "Before.\n\n<!-- @config: runtime.session_id -->\n\n<!-- @config: no.such.setting -->\n\n\
         After.\n",
    );
    let demo = project(
        &env,
        "demo",
        &[("docs/domain/acme/procedure/fragile.md", fragile)],
    );
    sync(&env, &demo);

    let failed = env
        .run(demo.path(), ["docs", "get", "acme/procedure/fragile"])
        .failure();
    insta::assert_snapshot!("unresolved-values", stdout(&env, &failed));
}

// spec: project/instrukt/spec/feature/docs/progressive-retrieval UC-DR3
#[test]
fn a_snippet_that_runs_a_command_is_refused_at_publication_and_runs_nothing_when_read() {
    let env = TestEnv::new();
    machine(&env);
    let marker = env.path("command-ran");
    let runs = |title: &str| {
        snippet(
            "procedure",
            title,
            "Tries to run a command.",
            "",
            &[],
            &format!(
                "Before.\n\n<!-- @exec: touch {} -->\n\nAfter.\n",
                marker.display()
            ),
        )
    };
    let demo = project(
        &env,
        "demo",
        &[("docs/domain/acme/procedure/runs.md", runs("Runs"))],
    );
    let refused = env.run(demo.path(), ["sync"]).failure();
    insta::assert_snapshot!(
        "command-refused-at-publication",
        format!("{}{}", stdout(&env, &refused), stderr(&env, &refused))
            .replace(&demo.head(), "[SHA]")
    );
    assert!(!env.instrukt_home().join("docs/acme").exists());

    // A local snippet is read from the project's working tree, so a
    // command written there after publication is still met at read.
    demo.write(
        "docs/domain/acme/procedure/runs.md",
        &snippet("procedure", "Runs", "Runs nothing.", "", &[], "Nothing.\n"),
    )
    .write(
        "docs/project/procedure/later.md",
        &snippet(
            "procedure",
            "Later",
            "Written after publication.",
            "",
            &[],
            "Nothing yet.\n",
        ),
    );
    demo.commit("docs: no commands");
    sync(&env, &demo);
    demo.write("docs/project/procedure/later.md", &runs("Later"));
    let read = env
        .run(demo.path(), ["docs", "get", "project/demo/procedure/later"])
        .success();
    assert!(!marker.exists(), "the leftover command never ran");
    insta::assert_snapshot!("leftover-command-inert", stdout(&env, &read));
}
