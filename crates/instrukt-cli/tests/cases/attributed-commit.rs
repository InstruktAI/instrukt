//! Attributed authoring-worktree commits retain caller provenance and run hooks.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use std::io::{Read, Write as _};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Child, ExitStatus, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// One of instrukt's output pipes, read to its end on a thread of its own.
/// The thread waits for its pipe at most one look at a time and stops when
/// asked, so it can always be stopped and joined. It is joined only once it
/// has finished, so looking at it never waits.
struct Reader {
    thread: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    read: Option<Result<Vec<u8>, String>>,
}

impl Reader {
    fn start(mut pipe: impl Read + AsFd + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let asked = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 4096];
            while !asked.load(Ordering::Acquire) {
                let mut look = [PollFd::new(pipe.as_fd(), PollFlags::POLLIN)];
                let timeout = PollTimeout::try_from(LOOK).map_err(std::io::Error::other)?;
                match poll(&mut look, timeout) {
                    Ok(0) | Err(nix::errno::Errno::EINTR) => continue,
                    Ok(_) => {}
                    Err(errno) => return Err(errno.into()),
                }
                // The pipe is ready: the read returns what it holds, or its end.
                match pipe.read(&mut chunk) {
                    Ok(0) => return Ok(bytes),
                    Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            Err(std::io::Error::other(
                "stopped before the end of instrukt's output",
            ))
        });
        Self {
            thread: Some(thread),
            stop,
            read: None,
        }
    }

    /// A reader that never started: finished, with nothing read.
    fn absent() -> Self {
        Self {
            thread: None,
            stop: Arc::new(AtomicBool::new(true)),
            read: Some(Err("instrukt's output was not read".to_owned())),
        }
    }

    /// Whether the reader has finished, joining it when it has.
    fn finished(&mut self) -> bool {
        if let Some(thread) = self.thread.take_if(|thread| thread.is_finished()) {
            self.read = Some(Self::joined(thread));
        }
        self.read.is_some()
    }

    /// Stops the reader if it still runs and joins it; its pipe closes as
    /// its thread ends. The thread sees the request within one look.
    fn retire(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.stop.store(true, Ordering::Release);
            self.read = Some(Self::joined(thread));
        }
    }

    fn joined(thread: JoinHandle<std::io::Result<Vec<u8>>>) -> Result<Vec<u8>, String> {
        match thread.join() {
            Ok(read) => read.map_err(|error| format!("cannot read instrukt's output: {error}")),
            Err(_) => Err("the reader of instrukt's output panicked".to_owned()),
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.retire();
    }
}

/// The held commit's instrukt and the readers of its output. Its status is
/// kept apart from its output, so looking for its end collects nothing.
/// Left running when the test fails, it is interrupted, which stops the
/// groups it owns, git and the hook among them, then reaped, and its
/// readers are retired; what cannot be done is reported, and the test's own
/// failure stands.
struct Running {
    child: Child,
    status: Option<ExitStatus>,
    stdout: Reader,
    stderr: Reader,
}

impl Running {
    fn spawn(mut command: std::process::Command) -> Self {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        // Owned before any reader starts, so a failure from here on still
        // interrupts and reaps instrukt.
        let mut running = Self {
            child,
            status: None,
            stdout: Reader::absent(),
            stderr: Reader::absent(),
        };
        running.stdout = Reader::start(stdout.unwrap());
        running.stderr = Reader::start(stderr.unwrap());
        running
    }

    /// instrukt's status, once it has ended.
    fn exited(&mut self) -> std::io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            self.status = self.child.try_wait()?;
        }
        Ok(self.status)
    }

    /// Whether instrukt has ended and both its readers have finished.
    fn retired(&mut self) -> std::io::Result<bool> {
        let exited = self.exited()?.is_some();
        let stdout = self.stdout.finished();
        let stderr = self.stderr.finished();
        Ok(exited && stdout && stderr)
    }

    /// What instrukt left, once it has ended and its output is read, looked
    /// for until `deadline`; otherwise what had not happened by then.
    fn output_by(&mut self, deadline: Instant) -> Result<Output, String> {
        while !self.retired().map_err(|error| error.to_string())? {
            if Instant::now() >= deadline {
                return Err(match self.status {
                    None => "instrukt had not ended".to_owned(),
                    Some(_) => {
                        "instrukt had ended, but its output was not read to its end".to_owned()
                    }
                });
            }
            std::thread::sleep(LOOK);
        }
        let read = |reader: &Reader| {
            reader
                .read
                .clone()
                .unwrap_or_else(|| Err("instrukt's output was not read".to_owned()))
        };
        Ok(Output {
            status: self.status.unwrap_or_default(),
            stdout: read(&self.stdout)?,
            stderr: read(&self.stderr)?,
        })
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let deadline = Instant::now() + STEP_BOUND;
        // Unless it is known to have ended.
        if !matches!(self.exited(), Ok(Some(_))) {
            let pid = rustix::process::Pid::from_child(&self.child);
            if let Err(error) = rustix::process::kill_process(pid, rustix::process::Signal::TERM) {
                eprintln!("cannot interrupt the held commit's instrukt: {error}");
            }
        }
        if let Err(missing) = self.output_by(deadline) {
            eprintln!("cleanup of the held commit: {missing}");
        }
        // Unless it is known to have ended.
        if !matches!(self.exited(), Ok(Some(_))) {
            if let Err(error) = self.child.kill() {
                eprintln!("cannot kill the held commit's instrukt: {error}");
            }
            if let Err(missing) = self.output_by(Instant::now() + STEP_BOUND) {
                eprintln!("cleanup of the held commit after killing it: {missing}");
            }
        }
        // Whatever was collected, no reader outlives the cleanup.
        self.stdout.retire();
        self.stderr.retire();
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
        let early = running
            .exited()
            .unwrap()
            .map(|status| match running.output_by(deadline) {
                Ok(output) => format!("{status}: {}", String::from_utf8_lossy(&output.stderr)),
                Err(missing) => format!("{status}; {missing}"),
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
    let output = running
        .output_by(Instant::now() + STEP_BOUND)
        .map_err(|missing| format!("{STEP_BOUND:?} after the hook was released, {missing}"))
        .unwrap();
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
