//! Mapped projects for blueprint generation tests: a project with the
//! blueprint module on, instrukt's hooks installed and a bare remote to
//! push main to, mapped by a test engine — a small executable the test
//! writes into its temporary directory and configures as the machine's
//! `blueprint.engine` (global layer), the boundary a developer's own engine
//! sits at.
//!
//! Each revision commits the graph it should map to (`map.json`) and its
//! name (`rev`); the test engines print `map.json`, so answers reflect the
//! revision that was mapped.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use instrukt_blueprint::{GraphStore, LastGeneration};
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use serde_json::{Value, json};

use crate::common::{Repo, TestEnv};

/// How long a test waits for an observable condition before failing.
const WAIT: Duration = Duration::from_secs(60);
/// How long a push may take; a push that waited for a gated engine would
/// take longer.
const PUSH_BOUND: Duration = Duration::from_secs(30);

/// A test machine whose `PATH` holds the built `instrukt` ahead of the
/// test's own, as an installed one would: hook sections call it by name.
pub fn machine() -> TestEnv {
    let mut env = TestEnv::new();
    let path = format!(
        "{}:{}",
        instrukt_dir().display(),
        env.injected("PATH").unwrap()
    );
    env.set_env("PATH", &path);
    env
}

/// A test machine whose `PATH` holds only the built `instrukt`, `git` and
/// the system's base tools — no Graphify.
pub fn machine_without_graphify() -> TestEnv {
    let mut env = TestEnv::new();
    let tools = env.path("tools");
    std::fs::create_dir_all(&tools).unwrap();
    for tool in ["git", "kill", "cat", "rm", "mkdir", "mktemp", "dirname"] {
        std::os::unix::fs::symlink(crate::common::tool_executable(tool), tools.join(tool)).unwrap();
    }
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_instrukt"), tools.join("instrukt")).unwrap();
    let path = tools.display().to_string();
    env.set_env("PATH", &path);
    env
}

fn instrukt_dir() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Writes an executable shell script at `<test dir>/<name>`.
pub fn script(env: &TestEnv, name: &str, body: &str) -> PathBuf {
    let path = env.path(name);
    std::fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// An engine that prints the revision's `map.json`; a revision without one
/// fails it with a message of its own.
pub fn engine(env: &TestEnv) -> PathBuf {
    script(
        env,
        "engine",
        "if [ ! -e \"$1/map.json\" ]; then echo 'this revision carries no map' >&2; exit 3; fi\n\
         cat \"$1/map.json\"\n",
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GateState {
    Closed,
    Open,
    Aborted,
    Stopped,
}

/// A gated engine and the directory controlling it. It closes its standard
/// input first, as an engine may. For each run it marks its start
/// (`started-<rev>`) and the instrukt worker that started it
/// (`worker-<rev>`, its process id), fails when another run's process is
/// still alive, waits until the test opens the gate ([`Gate::open`]) — or ends without
/// output when the test aborts it ([`Gate::abort`]) — then prints
/// `map.json` and marks its end (`finished-<rev>`).
pub struct Gate {
    dir: PathBuf,
    pub engine: PathBuf,
    signal: std::sync::Arc<(std::sync::Mutex<GateState>, std::sync::Condvar)>,
    address: std::net::SocketAddr,
    server: Option<std::thread::JoinHandle<()>>,
}

impl Gate {
    pub fn new(env: &TestEnv) -> Self {
        let dir = env.path("gate");
        std::fs::create_dir_all(&dir).unwrap();
        let control = dir.display();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let signal = std::sync::Arc::new((
            std::sync::Mutex::new(GateState::Closed),
            std::sync::Condvar::new(),
        ));
        let shared = signal.clone();
        let server = std::thread::spawn(move || {
            let mut workers = Vec::new();
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                if *shared.0.lock().unwrap() == GateState::Stopped {
                    break;
                }
                let shared = shared.clone();
                workers.push(std::thread::spawn(move || {
                    let state = shared
                        .1
                        .wait_while(shared.0.lock().unwrap(), |state| {
                            *state == GateState::Closed
                        })
                        .unwrap();
                    let reply = if *state == GateState::Open {
                        b'o'
                    } else {
                        b'a'
                    };
                    // An interrupted engine may have disconnected already.
                    if let Err(error) = stream.write_all(&[reply]) {
                        assert!(
                            matches!(
                                error.kind(),
                                std::io::ErrorKind::BrokenPipe
                                    | std::io::ErrorKind::ConnectionReset
                            ),
                            "gate reply failed: {error}"
                        );
                    }
                }));
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });

        let ip = address.ip();
        let port = address.port();
        let engine = script(
            env,
            "gated-engine",
            &format!(
                "exec 0<&-\n\
                 rev=\"$(cat \"$1/rev\")\"\n\
                 p=$PPID\n\
                 while [ \"$p\" -gt 1 ]; do\n\
                 \x20 case \"$(ps -o comm= -p \"$p\")\" in\n\
                 \x20   *instrukt*) echo \"$p\" > \"{control}/worker-$rev\"; break ;;\n\
                 \x20 esac\n\
                 \x20 p=$(ps -o ppid= -p \"$p\" | tr -d ' ')\n\
                 done\n\
                 if [ -e \"{control}/running\" ] && kill -0 \"$(cat \"{control}/running\")\" 2>/dev/null; then exit 70; fi\n\
                 echo $$ > \"{control}/running\"\n\
                 touch \"{control}/started-$rev\"\n\
                 trap 'rm -f \"{control}/running\"' EXIT\n\
                 python3 -c \"import socket; s=socket.create_connection(('{ip}', {port})); raise SystemExit(0 if s.recv(1) == b'o' else 9)\"\n\
                 cat \"$1/map.json\"\n\
                 touch \"{control}/finished-$rev\"\n"
            ),
        );
        Self {
            dir,
            engine,
            signal,
            address,
            server: Some(server),
        }
    }

    /// Whether the engine started for the revision named `rev`.
    pub fn started(&self, rev: &str) -> bool {
        self.dir.join(format!("started-{rev}")).exists()
    }

    /// Waits until the engine started for `rev`.
    pub fn wait_started(&self, rev: &str) {
        wait_until(&self.dir, &format!("the engine to start for {rev}"), || {
            self.started(rev)
        });
    }

    /// Whether the engine run for `rev` printed its map.
    pub fn finished(&self, rev: &str) -> bool {
        self.dir.join(format!("finished-{rev}")).exists()
    }

    /// Waits until the engine run for `rev` printed its map.
    pub fn wait_finished(&self, rev: &str) {
        wait_until(
            &self.dir,
            &format!("the engine to finish for {rev}"),
            || self.dir.join(format!("finished-{rev}")).exists(),
        );
    }

    /// Kills the instrukt worker that started the engine for `rev`, as an
    /// interruption would; its engine keeps running.
    pub fn kill_worker(&self, rev: &str) {
        let marker = self.dir.join(format!("worker-{rev}"));
        wait_until(
            &self.dir,
            &format!("the worker of {rev} to be known"),
            || std::fs::read_to_string(&marker).is_ok_and(|pid| !pid.trim().is_empty()),
        );
        let pid = std::fs::read_to_string(&marker).unwrap();
        let killed = std::process::Command::new(crate::common::tool_executable("kill"))
            .args(["-s", "KILL", pid.trim()])
            .status()
            .unwrap();
        assert!(killed.success(), "the worker was killed");
        // Gone, not merely signalled: its locks are released only once it
        // has exited.
        wait_for_exit(pid.trim());
    }

    pub fn open(&self) {
        *self.signal.0.lock().unwrap() = GateState::Open;
        self.signal.1.notify_all();
    }

    /// Closes the gate again for the runs that start from now on.
    pub fn close(&self) {
        *self.signal.0.lock().unwrap() = GateState::Closed;
    }

    /// Makes a waiting run end without output.
    pub fn abort(&self) {
        let pid = std::fs::read_to_string(self.dir.join("running")).unwrap();
        *self.signal.0.lock().unwrap() = GateState::Aborted;
        self.signal.1.notify_all();
        // Interrupted status depends on the OS process group being gone,
        // even though no worker remains to write another generation record.
        wait_for_exit(pid.trim());
    }
}

/// A project `demo` mapped by `engine` — configured as the machine's engine
/// command; the shipped adapter when `None` — with instrukt's hooks
/// installed and `origin` a bare remote.
pub fn project(env: &TestEnv, engine: Option<&Path>) -> Repo {
    if let Some(engine) = engine {
        machine_engine(env, engine);
    }
    project_with_layer(env, "")
}

/// Configures `engine` as the machine's engine command, in the global
/// layer: only there may a command be configured.
pub fn machine_engine(env: &TestEnv, engine: &Path) {
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        format!(
            "schema_version: 1\nblueprint:\n  engine: {}\n",
            engine.display()
        ),
    )
    .unwrap();
}

/// A project `demo` with the blueprint module on, `extra` appended to its
/// project layer, instrukt's hooks installed and `origin` a bare remote.
pub fn project_with_layer(env: &TestEnv, extra: &str) -> Repo {
    let remote = env.bare_remote("origin");
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        &format!("schema_version: 1\nproject: demo\nmodules:\n  blueprint: true\n{extra}"),
    );
    repo.git([
        std::ffi::OsStr::new("remote"),
        std::ffi::OsStr::new("add"),
        std::ffi::OsStr::new("origin"),
        remote.as_os_str(),
    ]);
    env.run(repo.path(), ["hooks", "install"]).success();
    repo
}

/// Commits a revision named `rev` that maps to `map` (no `map.json` when
/// `None`); returns its SHA.
pub fn revision(repo: &Repo, rev: &str, map: Option<&Value>) -> String {
    repo.write("rev", rev);
    match map {
        Some(map) => {
            repo.write("map.json", &map.to_string());
        }
        None => {
            let path = repo.path().join("map.json");
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    repo.commit(&format!("feat: revision {rev}"))
}

/// Pushes main to `origin`, failing the test if the push takes longer than
/// its bound.
pub fn push(env: &TestEnv, repo: &Repo) {
    env.git_command(repo.path())
        .args(["push", "--quiet", "origin", "main"])
        .timeout(PUSH_BOUND)
        .assert()
        .success();
}

/// Rechecks the condition on filesystem progress, within a fixed bound.
pub fn wait_until(root: &Path, what: &str, mut condition: impl FnMut() -> bool) {
    let progress = crate::common::progress::Progress::new(root, WAIT);
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        progress.wait();
    }
}

fn home(env: &TestEnv) -> Home {
    Home::at_user_home(env.home().to_path_buf())
}

fn slug() -> ProjectSlug {
    ProjectSlug::parse("demo").unwrap()
}

pub fn store(env: &TestEnv) -> GraphStore {
    GraphStore::new(&home(env), &slug())
}

pub fn sha(sha: &str) -> CommitSha {
    CommitSha::parse(sha).unwrap()
}

/// The project's last generation.
pub fn last_generation(env: &TestEnv) -> Option<LastGeneration> {
    instrukt_blueprint::last_generation(&home(env), &slug()).unwrap()
}

/// Waits until the generation of `of` has ended, mapped or not.
pub fn wait_generated(env: &TestEnv, of: &str) -> LastGeneration {
    let of = sha(of);
    let mut ended = None;
    wait_until(
        store(env).dir(),
        &format!("the generation of {of} to end"),
        || {
            ended = last_generation(env).filter(|last| {
                last.sha == of
                    && !matches!(
                        last.state,
                        instrukt_blueprint::GenerationState::Running
                            | instrukt_blueprint::GenerationState::Orphaned
                    )
            });
            ended.is_some()
        },
    );
    ended.unwrap()
}

/// `instrukt <args>` in the repository, succeeding; its stdout with each
/// SHA in `names` shown as its name.
pub fn answer(env: &TestEnv, repo: &Repo, args: &[&str], names: &[(&str, &str)]) -> String {
    let output = env.run(repo.path(), args).success();
    let mut text = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    for (sha, name) in names {
        text = text.replace(sha, &format!("<{name}>"));
    }
    text
}

/// A graph document: `nodes` as `(id, source_file)`, `links` as
/// `(source, target, relation)`.
pub fn map(nodes: &[(&str, &str)], links: &[(&str, &str, &str)]) -> Value {
    json!({
        "nodes": nodes.iter().map(|(id, file)| json!({"id": id, "source_file": file})).collect::<Vec<_>>(),
        "links": links.iter().map(|(s, t, r)| json!({"source": s, "target": t, "relation": r})).collect::<Vec<_>>(),
    })
}

impl Drop for Gate {
    fn drop(&mut self) {
        *self.signal.0.lock().unwrap() = GateState::Stopped;
        self.signal.1.notify_all();
        std::net::TcpStream::connect(self.address).unwrap();
        self.server.take().unwrap().join().unwrap();
    }
}

fn wait_for_exit(pid: &str) {
    let deadline = Instant::now() + WAIT;
    while std::process::Command::new(crate::common::tool_executable("kill"))
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
    {
        assert!(Instant::now() < deadline, "process {pid} did not exit");
        std::thread::yield_now();
    }
}
