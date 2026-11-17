//! The harness's own guarantees: every command prints its contract on
//! `--help` without acting, subprocesses see only the environment the test
//! gives them, and an instrukt write outside the temporary home is refused
//! before it lands, directly or through a symbolic link.

#![allow(clippy::unwrap_used, reason = "isolated harness fixtures")]

use crate::common;
use std::collections::BTreeMap;

use common::{Snapshot, TestEnv};
use instrukt_core::fsx::{FENCE_EXIT, FENCE_VARIABLE};

const COMMANDS: [&str; 15] = [
    "config",
    "docs",
    "sync",
    "init",
    "hooks",
    "repo",
    "specs",
    "blueprint",
    "todo",
    "eval",
    "run",
    "tick",
    "ticks",
    "sessions",
    "install",
];

#[test]
fn every_command_prints_its_contract_with_help_and_does_nothing_else() {
    let env = TestEnv::new();
    let cwd = env.path("");
    for command in COMMANDS {
        let output = env.run(&cwd, [command, "--help"]).success();
        let help = String::from_utf8(output.get_output().stdout.clone()).unwrap();
        insta::assert_snapshot!(format!("help-{command}"), help);
    }
    assert!(
        !env.instrukt_home().exists(),
        "--help must not create the home"
    );
}

/// An environment variable the test process itself carries (cargo sets it
/// for every test run), which no child may inherit.
const AMBIENT: &str = "CARGO_MANIFEST_DIR";

/// A variable the test injects, and the value it injects.
const INJECTED: &str = "FAKE_SESSION_ID";
const SENTINEL: &str = "s-1";

#[test]
fn git_children_see_only_the_injected_environment() {
    assert!(
        std::env::var_os(AMBIENT).is_some(),
        "precondition: {AMBIENT} is ambient"
    );
    let mut env = TestEnv::new();
    env.set_env(INJECTED, SENTINEL);
    let repo = env.git_repo("demo");
    let dump = repo.git(["-c", "alias.envdump=!env", "envdump"]);
    let seen: BTreeMap<&str, &str> = dump
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    insta::assert_snapshot!(seen.get(INJECTED).copied().unwrap_or_default(), @"s-1");
    assert_eq!(seen.get("HOME").map(std::path::Path::new), Some(env.home()));
    assert!(!seen.contains_key(AMBIENT));
}

#[test]
fn instrukt_children_see_only_the_injected_environment() {
    assert!(
        std::env::var_os(AMBIENT).is_some(),
        "precondition: {AMBIENT} is ambient"
    );
    let mut env = TestEnv::new();
    env.set_env(INJECTED, SENTINEL);
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: 1\nproject: demo\ngithub:\n  repository: \"${{{INJECTED}}}\"\n  approvers: [\"${{{AMBIENT}}}\"]\n"
        ),
    );
    let injected = env
        .run(repo.path(), ["config", "get", "github.repository"])
        .success();
    let value = String::from_utf8_lossy(&injected.get_output().stdout).into_owned();
    insta::assert_snapshot!(value, @"s-1");
    env.run(repo.path(), ["config", "get", "github.approvers"])
        .failure();
}

#[test]
fn commits_carry_the_isolated_git_identity() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    repo.write("README.md", "demo\n");
    let sha = repo.commit("feat: start");
    assert_eq!(sha.len(), 40);
    insta::assert_snapshot!(
        repo.git(["log", "-1", "--format=%an <%ae>"]),
        @"Test Developer <developer@example.test>"
    );
}

/// A directory standing in for the live user's home: the real one is never
/// written by these tests, not even to show it is protected.
struct Live {
    dir: tempfile::TempDir,
}
impl Live {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".instrukt")).unwrap();
        std::fs::write(dir.path().join(".instrukt/keep"), "live data\n").unwrap();
        Self { dir }
    }
    fn path(&self) -> std::path::PathBuf {
        self.dir.path().canonicalize().unwrap()
    }
}

/// Runs `instrukt config init --global` and returns its exit status and
/// stderr, with the stand-in live home and the test's directory named.
fn initialize(env: &TestEnv, live: &Live) -> (Option<i32>, String) {
    let output = env
        .instrukt(env.home())
        .args(["config", "init", "--global"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr)
        .replace(&live.path().display().to_string(), "[LIVE]")
        .replace(&env.home().display().to_string(), "[TEST HOME]");
    (output.status.code(), stderr)
}

#[test]
fn a_write_outside_the_temporary_home_is_refused_before_it_lands() {
    let live = Live::new();
    let untouched = Snapshot::over(Vec::new(), vec![live.path()]);
    let mut env = TestEnv::new();
    env.fence(&live.path());
    env.set_env(
        "INSTRUKT_HOME",
        &live.path().join(".instrukt").display().to_string(),
    );
    let (status, stderr) = initialize(&env, &live);
    assert_eq!(status, Some(FENCE_EXIT));
    insta::assert_snapshot!(stderr, @"instrukt: write refused at the fence: [LIVE]/.instrukt/locks resolves to [LIVE]/.instrukt/locks, under [LIVE] which INSTRUKT_WRITE_FENCE keeps off limits");
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

#[test]
fn a_write_through_a_symbolic_link_is_refused_before_it_lands() {
    let live = Live::new();
    let untouched = Snapshot::over(Vec::new(), vec![live.path()]);
    let mut env = TestEnv::new();
    env.fence(&live.path());
    std::os::unix::fs::symlink(live.path().join(".instrukt"), env.instrukt_home()).unwrap();
    let (status, stderr) = initialize(&env, &live);
    assert_eq!(status, Some(FENCE_EXIT));
    insta::assert_snapshot!(stderr, @"instrukt: write refused at the fence: [TEST HOME]/.instrukt/locks resolves to [LIVE]/.instrukt/locks, under [LIVE] which INSTRUKT_WRITE_FENCE keeps off limits");
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

#[test]
fn every_instrukt_process_is_fenced_from_the_live_home() {
    let env = TestEnv::new();
    let live = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let fenced: Vec<_> = std::env::split_paths(env.injected(FENCE_VARIABLE).unwrap()).collect();
    assert!(fenced.contains(&live.join(".instrukt")), "{fenced:?}");
    assert!(fenced.contains(&live.join(".teleclaude")), "{fenced:?}");
}
