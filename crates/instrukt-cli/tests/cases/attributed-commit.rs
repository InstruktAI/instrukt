//! Attributed authoring-worktree commits retain caller provenance and run hooks.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use std::io::{Read, Write as _};
use std::os::fd::AsFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Child, Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::config::CURRENT_SCHEMA_VERSION;

/// Emitted text with the test's temporary directory shown as `[TMP]`.
fn shown(env: &TestEnv, bytes: &[u8]) -> String {
    let base = env.path("").display().to_string();
    String::from_utf8_lossy(bytes).replace(base.trim_end_matches('/'), "[TMP]")
}

fn stderr(env: &TestEnv, assert: &Assert) -> String {
    shown(env, &assert.get_output().stderr)
}

/// The machine's selected adapter and configured agent author.
fn configure_machine(env: &TestEnv) {
    let path = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        format!(
            "schema_version: {CURRENT_SCHEMA_VERSION}
runner: {{runtime: t3}}
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

/// The clock when a held commit starts, and the time it is moved to while
/// the hook holds the commit: past the 120 seconds instrukt gives git's own
/// commands.
const STARTED: &str = "2026-11-03T12:00:00Z";
const PAST_GIT_LIMIT: &str = "2026-11-03T12:05:00Z";

/// How long, in real time, each step of a held commit may take before the
/// test fails. The commit's own limit runs on the file clock, which only the
/// test moves, so this bound never races it.
const STEP_BOUND: Duration = Duration::from_secs(30);
/// The pause between two looks at instrukt while a step is awaited.
const LOOK: Duration = Duration::from_millis(20);

/// The held commit's instrukt. Left running when the test fails, it is
/// interrupted, which stops the groups it owns, git and the hook among
/// them, and reaped.
struct Running {
    child: Child,
    stdout: Option<JoinHandle<Vec<u8>>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

impl Running {
    fn spawn(mut command: std::process::Command) -> Self {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let drain = |mut pipe: Box<dyn Read + Send>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                pipe.read_to_end(&mut bytes).unwrap();
                bytes
            })
        };
        let stdout = drain(Box::new(child.stdout.take().unwrap()));
        let stderr = drain(Box::new(child.stderr.take().unwrap()));
        Self {
            child,
            stdout: Some(stdout),
            stderr: Some(stderr),
        }
    }

    /// What instrukt left, once it has ended.
    fn ended(&mut self) -> Option<Output> {
        let status = self.child.try_wait().unwrap()?;
        Some(Output {
            status,
            stdout: self.stdout.take().unwrap().join().unwrap(),
            stderr: self.stderr.take().unwrap().join().unwrap(),
        })
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        let pid = rustix::process::Pid::from_child(&self.child);
        if let Err(error) = rustix::process::kill_process(pid, rustix::process::Signal::TERM) {
            eprintln!("cannot interrupt the held commit's instrukt: {error}");
        }
        let deadline = Instant::now() + STEP_BOUND;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(LOOK);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs `instrukt repo commit <args>` in `repo` while the project's
/// pre-commit hook holds the commit: once the hook's entry arrives, the
/// clock instrukt reads (`INSTRUKT_NOW=@<file>`) moves past git's limit, and
/// only then is the hook released to end.
fn commit_held_past_the_git_limit(env: &TestEnv, repo: &Repo, args: &[&str]) -> Assert {
    let held = env.path("held");
    std::fs::create_dir_all(&held).unwrap();
    let entered = held.join("entered");
    let release = held.join("release");
    for fifo in [&entered, &release] {
        nix::unistd::mkfifo(fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
    }
    let hook = repo.path().join(".git/hooks/pre-commit");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\necho entered > {}\nread -r _ < {}\n",
            instrukt_core::process::shell_quote(&entered.display().to_string()),
            instrukt_core::process::shell_quote(&release.display().to_string()),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let clock = held.join("now");
    std::fs::write(&clock, STARTED).unwrap();
    // Each pipe is opened for reading and writing: opening waits for no
    // one, reading the entry sees no end before the hook writes, and the
    // release line stays in its pipe until the hook reads it.
    let open = |fifo: &Path| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(fifo)
            .unwrap()
    };
    let mut entry = open(&entered);

    let mut command = env.instrukt_process(repo.path());
    command
        .args(["repo", "commit"])
        .args(args)
        .env("INSTRUKT_NOW", format!("@{}", clock.display()));
    let mut running = Running::spawn(command);
    let deadline = Instant::now() + STEP_BOUND;
    loop {
        let mut look = [PollFd::new(entry.as_fd(), PollFlags::POLLIN)];
        if poll(&mut look, PollTimeout::try_from(LOOK).unwrap()).unwrap() > 0 {
            break;
        }
        let early = running.ended().map(|output| {
            format!(
                "{}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
        });
        assert!(
            early.is_none(),
            "instrukt ended before the hook was entered ({early:?})"
        );
        assert!(
            Instant::now() < deadline,
            "the hook was not entered within {STEP_BOUND:?}"
        );
    }
    let mut arrival = [0_u8; 64];
    assert!(
        entry.read(&mut arrival).unwrap() > 0,
        "the hook's entry arrived"
    );

    let moved = held.join("now.moved");
    std::fs::write(&moved, PAST_GIT_LIMIT).unwrap();
    std::fs::rename(&moved, &clock).unwrap();
    let mut released = open(&release);
    released.write_all(b"go\n").unwrap();
    let deadline = Instant::now() + STEP_BOUND;
    let output = loop {
        if let Some(output) = running.ended() {
            break output;
        }
        assert!(
            Instant::now() < deadline,
            "instrukt did not end within {STEP_BOUND:?} after the hook was released"
        );
        std::thread::sleep(LOOK);
    };
    std::fs::remove_file(&hook).unwrap();
    Assert::new(output)
}

const MESSAGE: &str =
    "Teach the parser nested lists\n\nNested lists were flattened; they now keep their depth.";

// spec: project/instrukt/spec/feature/repository/attributed-commit UC-RC1
#[test]
fn a_native_session_commits_with_its_session_without_guessing_an_agent_author() {
    let mut env = TestEnv::new();
    configure_machine(&env);
    env.set_env("T3CODE_THREAD_ID", "thread-7");
    env.set_env("AGENT_EMAIL", "claude-agent@example.test");
    let repo = env.git_repo("demo");
    repo.write("README.md", "# demo\n");
    repo.commit("the developer's first commit");
    repo.git(["switch", "-c", "ad-hoc"]);
    repo.write(
        "staged-by-developer.md",
        "staged, not part of the agent's work\n",
    );
    repo.git(["add", "staged-by-developer.md"]);
    repo.write("parser.rs", "fn parse() {}\n");
    let staged_before = staged(&repo);

    let committed = commit_held_past_the_git_limit(
        &env,
        &repo,
        &[
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
                "Follow-up\n\nsession: t3:thread-7",
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
    repo.git(["switch", "-c", "ad-hoc"]);

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
    commit_held_past_the_git_limit(&env, &repo, &["--message", MESSAGE]).success();
    insta::assert_snapshot!("developer-commit", last_commit(&repo));
    insta::assert_snapshot!(commit_paths(&repo), @"notes.md");
}
