//! `local/projects/instrukt/spec/feature/repository/attributed-commit`:
//! `instrukt repo commit` and `instrukt repo stamp` — the acting agent as
//! author, its session and the caller's declaration of human involvement,
//! nothing guessed, and commit-review notes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;

use std::os::unix::fs::PermissionsExt as _;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::config::CURRENT_SCHEMA_VERSION;
use instrukt_core::repository::REVIEW_NOTES_REF;

/// Emitted text with the test's temporary directory shown as `[TMP]`.
fn shown(env: &TestEnv, bytes: &[u8]) -> String {
    let base = env.path("").display().to_string();
    String::from_utf8_lossy(bytes).replace(base.trim_end_matches('/'), "[TMP]")
}

fn stderr(env: &TestEnv, assert: &Assert) -> String {
    shown(env, &assert.get_output().stderr)
}

/// The machine's runtime mapping: a runtime hosting agent CLIs first, then
/// an agent CLI with its configured git author.
fn configure_machine(env: &TestEnv) {
    let path = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        format!(
            "schema_version: {CURRENT_SCHEMA_VERSION}
runtimes:
  - name: thread-host
    session_id_env: THREAD_HOST_ID
    start: thread-host start
    send: thread-host send {{runtime.session_id}}
    alive: thread-host alive {{runtime.session_id}}
    conversation: thread-host show {{runtime.session_id}}
  - name: claude-cli
    agent: claude
    session_id_env: CLAUDE_SESSION_ID
    start: claude
    send: claude send {{runtime.session_id}}
    alive: claude alive {{runtime.session_id}}
    conversation: claude history {{runtime.session_id}}
agents:
  claude:
    git_author: Claude Agent <claude-agent@example.test>
"
        ),
    )
    .unwrap();
}

/// The last commit as git reads it back: author, committer and message.
fn last_commit(repo: &Repo) -> String {
    repo.git([
        "log",
        "-1",
        "--format=author: %an <%ae>%ncommitter: %cn <%ce>%n%n%B",
    ])
}

fn commit_paths(repo: &Repo) -> String {
    repo.git(["show", "--name-only", "--format=", "HEAD"])
}

fn staged(repo: &Repo) -> String {
    repo.git(["diff", "--cached", "--name-only"])
}

const MESSAGE: &str =
    "Teach the parser nested lists\n\nNested lists were flattened; they now keep their depth.";

// spec: local/projects/instrukt/spec/feature/repository/attributed-commit UC-RC1
#[test]
fn an_agent_commits_its_work_under_its_own_name_with_its_session() {
    let mut env = TestEnv::new();
    configure_machine(&env);
    env.set_env("THREAD_HOST_ID", "thread-7");
    env.set_env("CLAUDE_SESSION_ID", "claude-3");
    let repo = env.git_repo("demo");
    repo.write("README.md", "# demo\n");
    repo.commit("the developer's first commit");
    repo.write(
        "staged-by-developer.md",
        "staged, not part of the agent's work\n",
    );
    repo.git(["add", "staged-by-developer.md"]);
    repo.write("parser.rs", "fn parse() {}\n");
    let staged_before = staged(&repo);

    let committed = env
        .run(
            repo.path(),
            [
                "repo",
                "commit",
                "--message",
                MESSAGE,
                "--human-involved",
                "true",
                "--todo",
                "nested-lists",
                "--only",
                "parser.rs",
            ],
        )
        .success();
    let printed = String::from_utf8(committed.get_output().stdout.clone()).unwrap();
    assert_eq!(printed.trim_end(), repo.head(), "prints the commit's id");
    insta::assert_snapshot!("agent-commit", last_commit(&repo));
    insta::assert_snapshot!(commit_paths(&repo), @"parser.rs");
    assert_eq!(
        staged(&repo),
        staged_before,
        "the developer's index is as it was"
    );

    let head = repo.head();
    repo.write("parser.rs", "fn parse() { todo }\n");
    let refused = env
        .run(
            repo.path(),
            [
                "repo",
                "commit",
                "--message",
                "Follow-up\n\nsession: thread-host:thread-7",
                "--only",
                "parser.rs",
            ],
        )
        .failure();
    insta::assert_snapshot!("session-line-refused", stderr(&env, &refused));
    assert_eq!(repo.head(), head, "nothing was committed");
    assert_eq!(staged(&repo), staged_before, "the index is untouched");
}

// spec: local/projects/instrukt/spec/feature/repository/attributed-commit UC-RC2
#[test]
fn a_developer_outside_any_session_commits_as_themselves() {
    let env = TestEnv::new();
    configure_machine(&env);
    let repo = env.git_repo("plain");
    repo.write("README.md", "# plain\n");
    repo.commit("the developer's first commit");

    let refusing = repo.path().join(".git/hooks/pre-commit");
    std::fs::write(&refusing, "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&refusing, std::fs::Permissions::from_mode(0o755)).unwrap();
    repo.write("notes.md", "a new file\n");
    let head = repo.head();
    env.run(
        repo.path(),
        ["repo", "commit", "--message", MESSAGE, "--only", "notes.md"],
    )
    .failure();
    assert_eq!(
        repo.head(),
        head,
        "the project's hook refused: nothing committed"
    );
    insta::assert_snapshot!(
        repo.git(["status", "--porcelain", "--untracked-files=all"]),
        @"?? notes.md"
    );
    std::fs::remove_file(&refusing).unwrap();

    repo.git(["add", "notes.md"]);
    env.run(repo.path(), ["repo", "commit", "--message", MESSAGE])
        .success();
    insta::assert_snapshot!("developer-commit", last_commit(&repo));
    insta::assert_snapshot!(commit_paths(&repo), @"notes.md");
}

// spec: local/projects/instrukt/spec/feature/repository/attributed-commit UC-RC3
#[test]
fn review_outcomes_and_exemptions_are_noted_on_each_commit_in_order() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    let mut commits = Vec::new();
    for name in ["one", "two", "three", "four"] {
        repo.write(&format!("{name}.md"), &format!("{name}\n"));
        commits.push(repo.commit(&format!("add {name}")));
    }
    let report = env.path("review-report.md");
    std::fs::write(&report, "# Review\n\nNo findings.\n").unwrap();

    let approve = env
        .instrukt(repo.path())
        .args([
            "repo",
            "stamp",
            &commits[0],
            "--verdict",
            "approve",
            "--report",
        ])
        .arg(&report)
        .assert()
        .success();
    let exempt = env
        .run(
            repo.path(),
            [
                "repo",
                "stamp",
                &commits[1],
                "--verdict",
                "exempt",
                "--reason",
                "typo fix",
            ],
        )
        .success();
    let needs_work = env
        .instrukt(repo.path())
        .args([
            "repo",
            "stamp",
            &commits[3],
            "--verdict",
            "needs_work",
            "--report",
        ])
        .arg(&report)
        .assert()
        .success();
    let without_reason = env
        .run(
            repo.path(),
            ["repo", "stamp", &commits[2], "--verdict", "exempt"],
        )
        .failure();

    let label = |text: String| {
        commits.iter().enumerate().fold(text, |text, (index, sha)| {
            text.replace(sha, &format!("<commit {}>", index + 1))
        })
    };
    let printed = [approve, exempt, needs_work]
        .iter()
        .map(|assert| String::from_utf8_lossy(&assert.get_output().stdout).into_owned())
        .collect::<String>();
    insta::assert_snapshot!("stamps-printed", label(printed));
    insta::assert_snapshot!("exempt-without-reason", stderr(&env, &without_reason));

    let notes_arg = format!("--notes={REVIEW_NOTES_REF}");
    let notes = repo.git(["log", "--reverse", "--format=%H%n%N--", notes_arg.as_str()]);
    insta::assert_snapshot!("notes-in-order", label(notes));
}
