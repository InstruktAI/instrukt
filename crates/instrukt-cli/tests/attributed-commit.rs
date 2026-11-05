//! `project/instrukt/spec/feature/repository/attributed-commit`:
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
    git_author: Claude Agent <${{AGENT_EMAIL}}>
"
        ),
    )
    .unwrap();
}

/// `text` with each stamp time (`at: <RFC 3339>`) shown as `<time N>`,
/// numbered in time order, so a snapshot keeps which stamp came first.
fn timeless(text: &str) -> String {
    const MARK: &str = "at: ";
    let mut times: Vec<&str> = text
        .match_indices(MARK)
        .filter_map(|(at, _)| text.get(at + MARK.len()..)?.split(';').next())
        .collect();
    times.sort_unstable();
    times.dedup();
    times
        .iter()
        .enumerate()
        .fold(text.to_owned(), |text, (index, time)| {
            text.replace(time, &format!("<time {}>", index + 1))
        })
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

// spec: project/instrukt/spec/feature/repository/attributed-commit UC-RC1
#[test]
fn an_agent_commits_its_work_under_its_own_name_with_its_session() {
    let mut env = TestEnv::new();
    configure_machine(&env);
    env.set_env("THREAD_HOST_ID", "thread-7");
    env.set_env("CLAUDE_SESSION_ID", "claude-3");
    env.set_env("AGENT_EMAIL", "claude-agent@example.test");
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

// spec: project/instrukt/spec/feature/repository/attributed-commit UC-RC2
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

// spec: project/instrukt/spec/feature/repository/attributed-commit UC-RC3
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
    insta::assert_snapshot!("notes-in-order", timeless(&label(notes)));
}

/// A machine's clone of `remote` at `<test dir>/<name>`.
fn clone_of(env: &TestEnv, remote: &std::path::Path, name: &str) -> Repo {
    env.clone_repo(remote, name)
}

fn stdout(env: &TestEnv, assert: &Assert) -> String {
    shown(env, &assert.get_output().stdout)
}

/// The review note of `commit` as the remote holds it.
fn remote_note(env: &TestEnv, remote: &std::path::Path, commit: &str) -> String {
    env.git(remote, ["notes", "--ref", REVIEW_NOTES_REF, "show", commit])
}

fn stamp_exempt(env: &TestEnv, repo: &Repo, commit: &str, reason: &str) -> Assert {
    env.run(
        repo.path(),
        [
            "repo",
            "stamp",
            commit,
            "--verdict",
            "exempt",
            "--reason",
            reason,
        ],
    )
}

// spec: project/instrukt/spec/feature/repository/attributed-commit UC-RC3
#[test]
fn a_second_machine_stamping_after_the_first_merges_both_outcomes_onto_the_remote() {
    let env = TestEnv::new();
    let remote = env.bare_remote("origin");
    let first = clone_of(&env, &remote, "first");
    first.write("one.md", "one\n");
    let commit = first.commit("add one");
    first.git(["push", "--quiet", "--set-upstream", "origin", "main"]);
    let second = clone_of(&env, &remote, "second");

    let report = env.path("review-report.md");
    std::fs::write(&report, "# Review\n\nNo findings.\n").unwrap();
    let approved = env
        .instrukt(first.path())
        .args(["repo", "stamp", &commit, "--verdict", "approve", "--report"])
        .arg(&report)
        .assert()
        .success();
    let merged = stamp_exempt(&env, &second, &commit, "generated file").success();

    let label = |text: String| text.replace(&commit, "<commit>");
    insta::assert_snapshot!("first-machine-stamp", label(stdout(&env, &approved)));
    insta::assert_snapshot!(
        "second-machine-stamp-after-rejection",
        label(stdout(&env, &merged))
    );
    insta::assert_snapshot!(
        "remote-note-with-both-outcomes",
        timeless(&remote_note(&env, &remote, &commit))
    );
}

// spec: project/instrukt/spec/feature/repository/attributed-commit UC-RC3
#[test]
fn a_stamp_made_while_the_remote_is_unreachable_is_delivered_by_the_next_stamp() {
    let env = TestEnv::new();
    let remote = env.bare_remote("origin");
    let repo = clone_of(&env, &remote, "demo");
    let mut commits = Vec::new();
    for name in ["one", "two"] {
        repo.write(&format!("{name}.md"), &format!("{name}\n"));
        commits.push(repo.commit(&format!("add {name}")));
    }
    repo.git(["push", "--quiet", "--set-upstream", "origin", "main"]);
    let label = |text: String| {
        commits.iter().enumerate().fold(text, |text, (index, sha)| {
            text.replace(sha, &format!("<commit {}>", index + 1))
        })
    };

    let away = env.path("origin-away.git");
    std::fs::rename(&remote, &away).unwrap();
    let pending = stamp_exempt(&env, &repo, &commits[0], "typo fix").success();
    insta::assert_snapshot!("stamp-while-unreachable", label(stdout(&env, &pending)));
    let local = repo.git(["notes", "--ref", REVIEW_NOTES_REF, "show", &commits[0]]);
    insta::assert_snapshot!("note-kept-locally", timeless(&local));

    std::fs::rename(&away, &remote).unwrap();
    let delivered = stamp_exempt(&env, &repo, &commits[1], "comment only").success();
    insta::assert_snapshot!("next-stamp-delivers", label(stdout(&env, &delivered)));
    assert_eq!(
        remote_note(&env, &remote, &commits[0]),
        local,
        "the pending note arrived"
    );
    insta::assert_snapshot!(
        "delivered-second-note",
        timeless(&remote_note(&env, &remote, &commits[1]))
    );
}

// spec: project/instrukt/spec/feature/repository/attributed-commit UC-RC3
#[test]
fn a_later_approval_supersedes_an_earlier_needs_work_and_both_stamps_stay() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    repo.write("one.md", "one\n");
    let commit = repo.commit("add one");
    let first = env.path("first-review.md");
    std::fs::write(&first, "# Review\n\nOne finding.\n").unwrap();
    let second = env.path("second-review.md");
    std::fs::write(&second, "# Review\n\nNo findings.\n").unwrap();

    for (verdict, report) in [("needs_work", &first), ("approve", &second)] {
        env.instrukt(repo.path())
            .args(["repo", "stamp", &commit, "--verdict", verdict, "--report"])
            .arg(report)
            .assert()
            .success();
    }

    let note = repo.git(["notes", "--ref", REVIEW_NOTES_REF, "show", &commit]);
    insta::assert_snapshot!("needs-work-then-approve", timeless(&note));
}
