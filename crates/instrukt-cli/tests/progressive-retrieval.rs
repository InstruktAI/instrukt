//! `instrukt docs index|get`
//! (`docs/local/spec/feature/docs/progressive-retrieval.md`).
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
    String::from_utf8_lossy(bytes).replace(&base, "[TMP]")
}

fn stdout(env: &TestEnv, assert: &Assert) -> String {
    text(env, &assert.get_output().stdout)
}

fn stderr(env: &TestEnv, assert: &Assert) -> String {
    text(env, &assert.get_output().stderr)
}

const CARE: &str = "- @~/.instrukt/docs/acme/principle/care.md";
const RULES: &str = "- @~/.instrukt/docs/acme/policy/rules.md";
const TERMS: &str = "- @~/.instrukt/docs/acme/concept/terms.md";
const BUNDLE: &str = "- @~/.instrukt/docs/acme/bundle.md";

/// What project `demo` documents: two domains, fragments (one carrying a
/// required read), a baseline, a third-party doc and two local snippets.
fn demo_files() -> Vec<(&'static str, String)> {
    vec![
        (
            "docs/global/acme/principle/care.md",
            snippet("principle", "Care", "Why care matters.", "", &[], "Care.\n"),
        ),
        (
            "docs/global/acme/policy/rules.md",
            snippet(
                "policy",
                "Rules",
                "The rules of the work.",
                "",
                &[CARE],
                "Follow the rules.\n\n<!-- @include: docs/snippets/shared.md -->\n\n\
                 <!-- @include: docs/snippets/missing.md -->\n",
            ),
        ),
        (
            "docs/global/acme/concept/terms.md",
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
            "docs/global/acme/procedure/work.md",
            snippet(
                "procedure",
                "Work",
                "How the work is done.",
                "",
                &[RULES, BUNDLE],
                "Work.\n",
            ),
        ),
        (
            "docs/global/acme/procedure/start.md",
            snippet(
                "procedure",
                "Start",
                "Starts a session.",
                "role: starter\n",
                &[RULES, "<!-- @include: docs/snippets/reads.md -->", BUNDLE],
                "Start.\n",
            ),
        ),
        (
            "docs/global/beta/spec/thing.md",
            snippet("spec", "Thing", "What a thing is.", "", &[], "A thing.\n"),
        ),
        (
            "docs/global/snippets/shared.md",
            "Shared text.\n".to_owned(),
        ),
        ("docs/global/snippets/reads.md", format!("{TERMS}\n")),
        (
            "docs/global/acme/baseline.md",
            "@~/.instrukt/docs/acme/principle/care.md\n".to_owned(),
        ),
        (
            "docs/third-party/tool/guide.md",
            "---\ndescription: How the tool works.\n---\n\n# Tool guide\n\nUse it.\n".to_owned(),
        ),
        (
            "docs/local/spec/feature/x.md",
            snippet(
                "spec",
                "X",
                "The x feature.",
                "",
                &["- @docs/local/design/y.md"],
                "The x feature.\n",
            ),
        ),
        (
            "docs/local/design/y.md",
            snippet("design", "Y", "How y is built.", "", &[], "Y.\n"),
        ),
        (
            "docs/local/baseline.md",
            "@docs/local/design/y.md\n".to_owned(),
        ),
    ]
}

/// Project `demo`, committed and published.
fn published_demo(env: &TestEnv) -> Repo {
    let demo = project(env, "demo", &["acme", "beta"], &demo_files());
    sync(env, &demo);
    demo
}

// spec: local/projects/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
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
                "--domains",
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

// spec: local/projects/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_sees_its_own_project_and_reads_another_projects_local_documentation() {
    let env = TestEnv::new();
    let demo = published_demo(&env);
    let other = project(
        &env,
        "other",
        &[],
        &[(
            "docs/local/concept/notes.md",
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
        .run(
            &outside,
            ["docs", "get", "local/projects/other/concept/notes"],
        )
        .success();
    insta::assert_snapshot!("other-project-local-read", stdout(&env, &read));

    let local = env
        .run(
            demo.path(),
            ["docs", "get", "local/projects/demo/spec/feature/x"],
        )
        .success();
    insta::assert_snapshot!("own-local-read", stdout(&env, &local));
}

// spec: local/projects/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_narrows_to_baselines_and_adds_third_party_documentation() {
    let env = TestEnv::new();
    let demo = published_demo(&env);

    let baseline = env
        .run(demo.path(), ["docs", "index", "--baseline-only"])
        .success();
    insta::assert_snapshot!("baseline-index", stdout(&env, &baseline));

    let with_third_party = env
        .run(demo.path(), ["docs", "index", "--third-party"])
        .success();
    insta::assert_snapshot!("third-party-index", stdout(&env, &with_third_party));

    let guide = env
        .run(demo.path(), ["docs", "get", "third-party/tool/guide"])
        .success();
    insta::assert_snapshot!("third-party-read", stdout(&env, &guide));
}

// spec: local/projects/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_is_warned_when_main_is_ahead_of_what_was_published() {
    let env = TestEnv::new();
    let demo = published_demo(&env);
    let published = demo.head();
    demo.write("README.md", "more\n");
    let main = demo.commit("docs: more");

    let listed = env
        .run(demo.path(), ["docs", "index", "--domains", "beta"])
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

// spec: local/projects/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
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
        .run(demo.path(), ["docs", "index", "--domains", "acme,gamma"])
        .failure();
    insta::assert_snapshot!("unknown-domain", stderr(&env, &domain));

    let nobody = env
        .run(demo.path(), ["docs", "index", "--project", "nobody"])
        .success();
    insta::assert_snapshot!("unknown-project", stdout(&env, &nobody));

    std::fs::remove_file(demo.path().join("docs/local/design/y.md")).unwrap();
    let missing = env
        .run(demo.path(), ["docs", "get", "local/projects/demo/design/y"])
        .failure();
    insta::assert_snapshot!("missing-local-file", stderr(&env, &missing));
}

/// Procedures with root frontmatter whose closures must refuse: one
/// requires a pending snippet, one requires through a fragment a pending
/// snippet, one requires through a fragment what is not published.
fn refused_files() -> Vec<(&'static str, String)> {
    vec![
        (
            "docs/global/acme/procedure/blocked.md",
            snippet(
                "procedure",
                "Blocked",
                "Cannot start yet.",
                "role: blocked\n",
                &["- @~/.instrukt/docs/acme/spec/coming.md"],
                "Wait.\n",
            ),
        ),
        (
            "docs/global/acme/procedure/blocked-through-fragment.md",
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
            "docs/global/acme/procedure/broken.md",
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
            "docs/global/acme/spec/coming.md",
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
            "docs/global/snippets/coming.md",
            "- @~/.instrukt/docs/acme/spec/coming.md\n".to_owned(),
        ),
        (
            "docs/global/snippets/absent.md",
            "- @~/.instrukt/docs/acme/spec/absent.md\n".to_owned(),
        ),
    ]
}

// spec: local/projects/instrukt/spec/feature/docs/progressive-retrieval UC-DR2
#[test]
fn an_agent_starts_a_procedure_with_everything_it_requires_in_one_read() {
    let env = TestEnv::new();
    let mut files = demo_files();
    files.extend(refused_files());
    let demo = project(&env, "demo", &["acme", "beta"], &files);
    sync(&env, &demo);

    // `start` requires `rules` and, through a fragment, `terms`; both
    // require `care`, which the closure holds once.
    let closure = env
        .run(
            demo.path(),
            ["docs", "get", "--closure", "acme/procedure/start"],
        )
        .success();
    insta::assert_snapshot!("closure", stdout(&env, &closure));

    for (name, id) in [
        ("closure-refused-pending", "acme/procedure/blocked"),
        (
            "closure-refused-pending-through-fragment",
            "acme/procedure/blocked-through-fragment",
        ),
        ("closure-refused-unreadable", "acme/procedure/broken"),
    ] {
        let refused = env
            .run(demo.path(), ["docs", "get", "--closure", id])
            .failure();
        assert!(refused.get_output().stdout.is_empty(), "{id}");
        insta::assert_snapshot!(name, stderr(&env, &refused));
    }

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

    let not_a_procedure = env
        .run(
            demo.path(),
            ["docs", "get", "--closure", "acme/policy/rules"],
        )
        .failure();
    insta::assert_snapshot!("closure-of-a-policy", stderr(&env, &not_a_procedure));
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

// spec: local/projects/instrukt/spec/feature/docs/progressive-retrieval UC-DR3
#[test]
fn each_agent_reads_the_value_of_its_own_session_when_it_reads() {
    let mut env = TestEnv::new();
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    std::fs::write(env.instrukt_home().join("instrukt.yaml"), RUNTIMES).unwrap();
    // Read-time commands find `instrukt` where agents find it: on PATH.
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap()
        .display()
        .to_string();
    env.set_env(
        "PATH",
        &format!("{bin}:{}", std::env::var("PATH").unwrap_or_default()),
    );
    let procedure = snippet(
        "procedure",
        "Report",
        "Reports into the session.",
        "",
        &[],
        "Your session:\n\n<!-- @exec: instrukt config get runtime.session_id -->\n\nThen report.\n",
    );
    let failing = snippet(
        "procedure",
        "Fragile",
        "Asks a command that fails.",
        "",
        &[],
        "Before.\n\n<!-- @exec: echo partial; exit 3 -->\n\nAfter.\n",
    );
    let demo = project(
        &env,
        "demo",
        &["acme"],
        &[
            ("docs/global/acme/procedure/report.md", procedure.clone()),
            ("docs/global/acme/procedure/fragile.md", failing),
        ],
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
        "the published procedure keeps the command"
    );

    let failed = env
        .run(demo.path(), ["docs", "get", "acme/procedure/fragile"])
        .failure();
    insta::assert_snapshot!("failed-command", stdout(&env, &failed));
}
