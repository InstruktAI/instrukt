//! `instrukt docs index|get`
//! (`docs/local/spec/feature/docs/progressive-retrieval.md`).
//!
//! The home is seeded as publication leaves it (`support/published.rs`):
//! publication is built separately, and these tests read its persisted
//! contract.

#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/published.rs"]
mod published;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use published::{doc, generated, publish};

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

/// A committed repository declaring project `slug`.
fn project(env: &TestEnv, slug: &str) -> Repo {
    let repo = env.git_repo(slug);
    repo.write(
        "instrukt.yaml",
        &format!("schema_version: 1\nproject: {slug}\n"),
    );
    repo.commit("chore: adopt instrukt");
    repo
}

const CARE: &str = "~/.instrukt/docs/acme/principle/care.md";
const RULES: &str = "~/.instrukt/docs/acme/policy/rules.md";
const BUNDLE: &str = "~/.instrukt/docs/acme/bundle.md";

/// The documentation project `demo` publishes: two domains, a fragment, a
/// baseline, a third-party doc and two local snippets.
fn demo_files() -> Vec<(&'static str, String)> {
    vec![
        (
            "docs/global/acme/principle/care.md",
            doc(
                "Care — Principle",
                "Why care matters.",
                "",
                "\n## Principle\n\nCare.\n",
            ),
        ),
        (
            "docs/global/acme/policy/rules.md",
            doc(
                "Rules — Policy",
                "The rules of the work.",
                "",
                &format!(
                    "\n## Required reads\n\n- @{CARE}\n\n## Rules\n\nFollow @{CARE} always.\n\n\
                     <!-- @include: docs/snippets/shared.md -->\n"
                ),
            ),
        ),
        (
            "docs/global/acme/procedure/work.md",
            doc(
                "Work — Procedure",
                "How the work is done.",
                "",
                &format!("\n## Required reads\n\n- @{RULES}\n- @{BUNDLE}\n\n## Steps\n\nWork.\n"),
            ),
        ),
        (
            "docs/global/beta/spec/thing.md",
            doc(
                "Thing — Spec",
                "What a thing is.",
                "",
                "\n## What it is\n\nA thing.\n",
            ),
        ),
        (
            "docs/global/snippets/shared.md",
            "Shared text.\n".to_owned(),
        ),
        ("docs/global/acme/baseline.md", format!("@{CARE}\n")),
        (
            "docs/third-party/tool/guide.md",
            "---\ndescription: How the tool works.\n---\n\n# Tool guide\n\nUse it.\n".to_owned(),
        ),
        (
            "docs/local/spec/feature/x.md",
            doc(
                "X — Spec",
                "The x feature.",
                "",
                "\n## Required reads\n\n- @docs/local/design/y.md\n\n## What it is\n\n\
                 See @docs/local/design/y.md.\n",
            ),
        ),
        (
            "docs/local/design/y.md",
            doc("Y — Design", "How y is built.", "", "\n## Purpose\n\nY.\n"),
        ),
        (
            "docs/local/baseline.md",
            "@docs/local/design/y.md\n".to_owned(),
        ),
    ]
}

/// Project `demo`, committed and published, with the `acme` bundle.
fn published_demo(env: &TestEnv) -> Repo {
    let demo = project(env, "demo");
    let files = demo_files();
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    publish(env, "demo", Some(&demo), &files);
    generated(
        env,
        "acme/bundle.md",
        "# Acme bundle\n\nEverything acme agents read.\n",
    );
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
    // fragment, the absolute reference and the stripped section show the
    // read pipeline.
    let both = env
        .run(
            demo.path(),
            ["docs", "get", "acme/policy/rules", "acme/principle/care"],
        )
        .success();
    insta::assert_snapshot!("policy-with-its-read", stdout(&env, &both));
}

// spec: local/projects/instrukt/spec/feature/docs/progressive-retrieval UC-DR1
#[test]
fn an_agent_sees_its_own_project_and_reads_another_projects_local_documentation() {
    let env = TestEnv::new();
    let demo = published_demo(&env);
    let other = project(&env, "other");
    publish(
        &env,
        "other",
        Some(&other),
        &[(
            "docs/local/concept/notes.md",
            &doc(
                "Notes — Concept",
                "Other's notes.",
                "",
                "\n## What\n\nNotes.\n",
            ),
        )],
    );

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

/// A procedure with root frontmatter whose reads meet in one snippet, so
/// the closure must include it once.
fn closure_files() -> Vec<(&'static str, String)> {
    vec![
        (
            "docs/global/acme/procedure/start.md",
            doc(
                "Start — Procedure",
                "Starts a session.",
                "role: starter\n",
                &format!(
                    "\n## Required reads\n\n- @{RULES}\n- @~/.instrukt/docs/acme/concept/terms.md\n\
                     - @{BUNDLE}\n\n## Goal\n\nStart.\n"
                ),
            ),
        ),
        (
            "docs/global/acme/concept/terms.md",
            doc(
                "Terms — Concept",
                "The words used.",
                "",
                &format!("\n## Required reads\n\n- @{CARE}\n\n## What\n\nTerms.\n"),
            ),
        ),
        (
            "docs/global/acme/procedure/blocked.md",
            doc(
                "Blocked — Procedure",
                "Cannot start yet.",
                "role: blocked\n",
                "\n## Required reads\n\n- @~/.instrukt/docs/acme/spec/coming.md\n\n## Goal\n\nWait.\n",
            ),
        ),
        (
            "docs/global/acme/procedure/broken.md",
            doc(
                "Broken — Procedure",
                "Requires what is not published.",
                "activation_trigger: When broken.\n",
                "\n## Required reads\n\n- @~/.instrukt/docs/acme/spec/absent.md\n\n## Goal\n\nNo.\n",
            ),
        ),
        (
            "docs/global/acme/spec/coming.md",
            doc(
                "Coming — Spec",
                "Not delivered yet.",
                "delivered_by: [build-coming]\n",
                "\n## What it is\n\nSoon.\n",
            ),
        ),
    ]
}

// spec: local/projects/instrukt/spec/feature/docs/progressive-retrieval UC-DR2
#[test]
fn an_agent_starts_a_procedure_with_everything_it_requires_in_one_read() {
    let env = TestEnv::new();
    let demo = project(&env, "demo");
    let mut files = demo_files();
    files.extend(closure_files());
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    publish(&env, "demo", Some(&demo), &files);
    generated(
        &env,
        "acme/bundle.md",
        "# Acme bundle\n\nEverything acme agents read.\n",
    );

    let closure = env
        .run(
            demo.path(),
            ["docs", "get", "--closure", "acme/procedure/start"],
        )
        .success();
    insta::assert_snapshot!("closure", stdout(&env, &closure));

    let pending = env
        .run(
            demo.path(),
            ["docs", "get", "--closure", "acme/procedure/blocked"],
        )
        .failure();
    assert!(pending.get_output().stdout.is_empty());
    insta::assert_snapshot!("closure-refused-pending", stderr(&env, &pending));

    let unreadable = env
        .run(
            demo.path(),
            ["docs", "get", "--closure", "acme/procedure/broken"],
        )
        .failure();
    insta::assert_snapshot!("closure-refused-unreadable", stderr(&env, &unreadable));

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

const SESSION_COMMAND: &str = "<!-- @exec: instrukt config get runtime.session_id -->";

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
    let demo = project(&env, "demo");
    let procedure = doc(
        "Report — Procedure",
        "Reports into the session.",
        "",
        &format!("\n## Steps\n\nYour session:\n\n{SESSION_COMMAND}\n\nThen report.\n"),
    );
    let failing = doc(
        "Fragile — Procedure",
        "Asks a command that fails.",
        "",
        "\n## Steps\n\nBefore.\n\n<!-- @exec: echo partial; exit 3 -->\n\nAfter.\n",
    );
    publish(
        &env,
        "demo",
        Some(&demo),
        &[
            ("docs/global/acme/procedure/report.md", &procedure),
            ("docs/global/acme/procedure/fragile.md", &failing),
        ],
    );

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
