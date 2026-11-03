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
        std::env::var("PATH").unwrap_or_default()
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
    let git = which("git");
    std::os::unix::fs::symlink(&git, tools.join("git")).unwrap();
    let path = format!(
        "{}:{}:/usr/bin:/bin",
        tools.display(),
        instrukt_dir().display()
    );
    for dir in std::env::split_paths(&path) {
        assert!(
            !dir.join("graphify").exists(),
            "the test needs a PATH without graphify, but {} has one",
            dir.display()
        );
    }
    env.set_env("PATH", &path);
    env
}

fn instrukt_dir() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn which(program: &str) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap();
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("{program} is not on the test's PATH"))
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

/// A gated engine and the directory controlling it. For each run it marks
/// its start (`started-<rev>`) and the instrukt worker that started it
/// (`worker-<rev>`, its process id), fails when another run is in progress,
/// waits until the test opens the gate ([`Gate::open`]) — or ends without
/// output when the test aborts it ([`Gate::abort`]) — then prints
/// `map.json` and marks its end (`finished-<rev>`).
pub struct Gate {
    dir: PathBuf,
    pub engine: PathBuf,
}

impl Gate {
    pub fn new(env: &TestEnv) -> Self {
        let dir = env.path("gate");
        std::fs::create_dir_all(&dir).unwrap();
        let control = dir.display();
        let engine = script(
            env,
            "gated-engine",
            &format!(
                "rev=\"$(cat \"$1/rev\")\"\n\
                 p=$PPID\n\
                 while [ \"$p\" -gt 1 ]; do\n\
                 \x20 case \"$(ps -o comm= -p \"$p\")\" in\n\
                 \x20   *instrukt*) echo \"$p\" > \"{control}/worker-$rev\"; break ;;\n\
                 \x20 esac\n\
                 \x20 p=$(ps -o ppid= -p \"$p\" | tr -d ' ')\n\
                 done\n\
                 touch \"{control}/started-$rev\"\n\
                 mkdir \"{control}/running\" || exit 70\n\
                 trap 'rmdir \"{control}/running\"' EXIT\n\
                 i=0\n\
                 while [ ! -e \"{control}/open\" ]; do\n\
                 \x20 [ ! -e \"{control}/abort\" ] || exit 9\n\
                 \x20 i=$((i + 1)); [ \"$i\" -lt 6000 ] || exit 71; sleep 0.01\n\
                 done\n\
                 cat \"$1/map.json\"\n\
                 touch \"{control}/finished-$rev\"\n"
            ),
        );
        Self { dir, engine }
    }

    /// Whether the engine started for the revision named `rev`.
    pub fn started(&self, rev: &str) -> bool {
        self.dir.join(format!("started-{rev}")).exists()
    }

    /// Waits until the engine started for `rev`.
    pub fn wait_started(&self, rev: &str) {
        wait_until(&format!("the engine to start for {rev}"), || {
            self.started(rev)
        });
    }

    /// Waits until the engine run for `rev` printed its map.
    pub fn wait_finished(&self, rev: &str) {
        wait_until(&format!("the engine to finish for {rev}"), || {
            self.dir.join(format!("finished-{rev}")).exists()
        });
    }

    /// Kills the instrukt worker that started the engine for `rev`, as an
    /// interruption would; its engine keeps running.
    pub fn kill_worker(&self, rev: &str) {
        let marker = self.dir.join(format!("worker-{rev}"));
        wait_until(&format!("the worker of {rev} to be known"), || {
            std::fs::read_to_string(&marker).is_ok_and(|pid| !pid.trim().is_empty())
        });
        let pid = std::fs::read_to_string(&marker).unwrap();
        let killed = std::process::Command::new("kill")
            .args(["-s", "KILL", pid.trim()])
            .status()
            .unwrap();
        assert!(killed.success(), "the worker was killed");
    }

    pub fn open(&self) {
        std::fs::write(self.dir.join("open"), b"").unwrap();
    }

    /// Closes the gate again for the runs that start from now on.
    pub fn close(&self) {
        std::fs::remove_file(self.dir.join("open")).unwrap();
    }

    /// Makes a waiting run end without output.
    pub fn abort(&self) {
        std::fs::write(self.dir.join("abort"), b"").unwrap();
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

/// Polls `condition` until it holds, failing the test after a bound.
pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
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
    wait_until(&format!("the generation of {of} to end"), || {
        ended = last_generation(env).filter(|last| {
            last.sha == of && !matches!(last.state, instrukt_blueprint::GenerationState::Running)
        });
        ended.is_some()
    });
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
