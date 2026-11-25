//! The harness's own guarantees: every command prints its contract on
//! `--help` without acting, subprocesses see only the environment the test
//! gives them, and an instrukt write outside the temporary home is refused
//! before it lands, directly or through a symbolic link.

#![allow(clippy::unwrap_used, reason = "isolated harness fixtures")]

use crate::common;
use std::collections::BTreeMap;

use crate::fixtures::documented;
use common::{Snapshot, TestEnv};
use instrukt_core::fsx::FENCE_EXIT;

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
    assert_eq!(
        seen.get("HOME").copied(),
        std::env::var("HOME").ok().as_deref()
    );
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

    /// The test machine of a session that ran with this stand-in as its
    /// home and a temporary directory inside its fenced `.t3`, as a T3
    /// provider session does, with live data beside the way down to that
    /// directory at its top and at an intermediate level.
    fn session(&self) -> TestEnv {
        for sibling in [TOP_SIBLING, INNER_SIBLING] {
            let dir = self.path().join(sibling);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("keep"), "live data\n").unwrap();
        }
        let temp_dir = self.path().join(SESSION_TEMP_DIR);
        std::fs::create_dir_all(&temp_dir).unwrap();
        TestEnv::in_stand_in_session(&self.path(), &temp_dir)
    }

    /// The stand-in's live data beside the session's temporary directory;
    /// the rest of the stand-in holds the test's own fixture.
    fn untouched(&self) -> Snapshot {
        let trees = [".instrukt", TOP_SIBLING, INNER_SIBLING]
            .iter()
            .map(|tree| self.path().join(tree))
            .collect();
        Snapshot::over(Vec::new(), trees)
    }
}

/// The temporary directory of a session under the stand-in home, inside
/// its fenced `.t3`, and the live data beside the way down to it.
const SESSION_TEMP_DIR: &str = ".t3/userdata/agent-tmp/run";
const TOP_SIBLING: &str = ".t3/worktrees";
const INNER_SIBLING: &str = ".t3/userdata/state";

/// Runs `instrukt config init --global` and returns its exit status and the
/// first line of its stderr, the fence's own refusal, with the stand-in live
/// home and the test's home named. The test's home is named first: in a
/// stand-in session it lies inside the stand-in.
fn initialize(env: &TestEnv, live: &Live) -> (Option<i32>, String) {
    let output = env
        .instrukt(env.home())
        .args(["config", "init", "--global"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .next()
        .unwrap_or_default()
        .replace(&env.home().display().to_string(), "[TEST HOME]")
        .replace(&live.path().display().to_string(), "[LIVE]");
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
    assert_eq!(status, Some(i32::from(FENCE_EXIT)));
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
    assert_eq!(status, Some(i32::from(FENCE_EXIT)));
    insta::assert_snapshot!(stderr, @"instrukt: write refused at the fence: [TEST HOME]/.instrukt/locks resolves to [LIVE]/.instrukt/locks, under [LIVE] which INSTRUKT_WRITE_FENCE keeps off limits");
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

#[test]
fn a_write_where_a_fenced_symbolic_link_leads_is_refused_before_it_lands() {
    let live = Live::new();
    let target = live.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, live.path().join("link")).unwrap();
    let untouched = Snapshot::over(Vec::new(), vec![live.path()]);
    let mut env = TestEnv::new();
    env.fence(&live.path().join("link"));
    std::os::unix::fs::symlink(&target, env.instrukt_home()).unwrap();
    let (status, stderr) = initialize(&env, &live);
    assert_eq!(status, Some(i32::from(FENCE_EXIT)));
    insta::assert_snapshot!(stderr, @"instrukt: write refused at the fence: [TEST HOME]/.instrukt/locks resolves to [LIVE]/target/locks, under [LIVE]/link which INSTRUKT_WRITE_FENCE keeps off limits");
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

#[test]
fn a_write_to_a_hard_link_beside_a_fenced_file_is_admitted() {
    let live = Live::new();
    let mut env = TestEnv::new();
    assert_eq!(initialize(&env, &live).0, Some(0));
    // The lock every initialization takes, and a fenced second name for
    // the same file in the same directory.
    let lock = env.instrukt_home().join("locks/agents.lock");
    let fenced = env.instrukt_home().join("locks/fenced");
    std::fs::hard_link(&lock, &fenced).unwrap();
    env.fence(&fenced);
    assert_eq!(initialize(&env, &live).0, Some(0));
    assert!(env.refusals().is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn a_write_inside_a_fenced_location_spelled_otherwise_than_stored_is_refused() {
    let live = Live::new();
    let composed = live.path().join("caf\u{e9}");
    std::fs::create_dir(&composed).unwrap();
    let untouched = Snapshot::over(Vec::new(), vec![live.path()]);
    let mut env = TestEnv::new();
    env.fence(&live.path().join("cafe\u{301}"));
    std::os::unix::fs::symlink(&composed, env.instrukt_home()).unwrap();
    let (status, stderr) = initialize(&env, &live);
    assert_eq!(status, Some(i32::from(FENCE_EXIT)));
    insta::assert_snapshot!(stderr, @"instrukt: write refused at the fence: [TEST HOME]/.instrukt/locks resolves to [LIVE]/caf\u{e9}/locks, under [LIVE]/cafe\u{301} which INSTRUKT_WRITE_FENCE keeps off limits");
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

#[test]
fn a_test_whose_temporary_directory_lies_under_a_fenced_location_runs() {
    let live = Live::new();
    let env = live.session();
    let untouched = live.untouched();
    let care = documented::snippet("policy", "Care", "Care.", "", &[], "First.\n");
    let repo = documented::project(
        &env,
        "platform",
        &[
            (
                "docs/general/index.yaml",
                "expanded:\n  - general/policy/care\n".to_owned(),
            ),
            ("docs/general/policy/care.md", care),
        ],
    );
    // Publishing works in a temporary checkout under the session's
    // temporary directory.
    documented::sync(&env, &repo);
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert!(env.refusals().is_empty());
}

#[test]
fn a_write_beside_the_temporary_directory_at_the_top_of_a_fenced_location_is_refused() {
    let live = Live::new();
    let env = live.session();
    let untouched = live.untouched();
    std::os::unix::fs::symlink(live.path().join(TOP_SIBLING), env.instrukt_home()).unwrap();
    let (status, stderr) = initialize(&env, &live);
    assert_eq!(status, Some(i32::from(FENCE_EXIT)));
    insta::assert_snapshot!(stderr, @"instrukt: write refused at the fence: [TEST HOME]/.instrukt/locks resolves to [LIVE]/.t3/worktrees/locks, under [LIVE]/.t3/worktrees which INSTRUKT_WRITE_FENCE keeps off limits");
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

#[test]
fn a_write_beside_the_temporary_directory_inside_a_fenced_location_is_refused() {
    let live = Live::new();
    let env = live.session();
    let untouched = live.untouched();
    std::os::unix::fs::symlink(live.path().join(INNER_SIBLING), env.instrukt_home()).unwrap();
    let (status, stderr) = initialize(&env, &live);
    assert_eq!(status, Some(i32::from(FENCE_EXIT)));
    insta::assert_snapshot!(stderr, @"instrukt: write refused at the fence: [TEST HOME]/.instrukt/locks resolves to [LIVE]/.t3/userdata/state/locks, under [LIVE]/.t3/userdata/state which INSTRUKT_WRITE_FENCE keeps off limits");
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

#[test]
fn the_loggers_own_files_are_refused_before_they_land() {
    let live = Live::new();
    let untouched = Snapshot::over(Vec::new(), vec![live.path()]);
    let mut env = TestEnv::new();
    env.fence(&live.path());
    // The selected log directory leads into the fenced live fixture.
    let logs = env.home().join("logs");
    env.set_env("INSTRUKT_LOG_DIR", logs.to_str().unwrap());
    std::os::unix::fs::symlink(live.path(), logs).unwrap();
    let (status, stderr) = initialize(&env, &live);
    assert_eq!(status, Some(i32::from(FENCE_EXIT)));
    insta::assert_snapshot!(stderr, @"instrukt: write refused at the fence: [TEST HOME]/logs resolves to [LIVE], under [LIVE] which INSTRUKT_WRITE_FENCE keeps off limits");
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

/// A live database that keeps a write-ahead log; closed, it has no sidecars.
fn live_database(live: &Live) -> std::path::PathBuf {
    let database = live.path().join("mirrors.db");
    let db = instrukt_core::state_db::rusqlite::Connection::open(&database).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.execute_batch("CREATE TABLE kept(id INTEGER)").unwrap();
    drop(db);
    database
}

/// Imports the legacy store at `source` and returns the exit status.
fn import(env: &TestEnv, source: &std::path::Path) -> Option<i32> {
    env.instrukt(env.home())
        .args(["history", "import-teleclaude", "--source"])
        .arg(source)
        .output()
        .unwrap()
        .status
        .code()
}

#[test]
fn a_read_that_would_write_beside_a_live_database_is_refused_before_it_lands() {
    let live = Live::new();
    let database = live_database(&live);
    let untouched = Snapshot::over(Vec::new(), vec![live.path()]);
    let mut env = TestEnv::new();
    env.fence(&live.path());
    assert_eq!(import(&env, &database), Some(i32::from(FENCE_EXIT)));
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

#[test]
fn a_read_through_a_link_to_a_live_database_is_refused_before_it_lands() {
    let live = Live::new();
    let database = live_database(&live);
    let untouched = Snapshot::over(Vec::new(), vec![live.path()]);
    let mut env = TestEnv::new();
    env.fence(&live.path());
    // A link in the test's own directory, which SQLite follows to the live
    // file and keeps its side files beside.
    let link = env.path("source.db");
    std::os::unix::fs::symlink(&database, &link).unwrap();
    assert_eq!(import(&env, &link), Some(i32::from(FENCE_EXIT)));
    assert!(untouched.changed().is_empty(), "{:?}", untouched.changed());
    assert_eq!(env.refusals().len(), 1);
}

/// A project published once while the Claude CLI is installed, then
/// changed, with the CLI's entrypoint fenced: publishing it again writes the
/// home's own publication before it reaches the entrypoint, and is refused
/// there.
struct Republication {
    repo: common::Repo,
    roots: Vec<std::path::PathBuf>,
    previous: BTreeMap<std::path::PathBuf, Vec<u8>>,
}

fn republication(env: &mut TestEnv) -> Republication {
    let claude = env.home().join(".claude");
    crate::fixtures::t3::FakeT3::create(env);
    common::t3_readiness::configure(
        env,
        vec![common::t3_readiness::provider(
            "claudeAgent",
            "claudeAgent",
            true,
        )],
        serde_json::json!({}),
    );
    let care = |body: &str| documented::snippet("policy", "Care", "Care.", "", &[], body);
    let repo = documented::project(
        env,
        "platform",
        &[
            (
                "docs/general/index.yaml",
                "expanded:\n  - general/policy/care\n".to_owned(),
            ),
            ("docs/general/policy/care.md", care("First.\n")),
        ],
    );
    documented::sync(env, &repo);
    let home = instrukt_core::home::Home::at_user_home(env.home().to_path_buf());
    let roots = vec![
        home.index_file(),
        home.docs_dir(),
        home.claims_dir(),
        home.publication_state_dir(),
        home.publication_outputs_dir(),
        claude.clone(),
    ];
    let previous = published_files(&roots);
    repo.write("docs/general/policy/care.md", &care("Second.\n"));
    repo.commit("docs: change the care policy");
    env.fence(&claude);
    Republication {
        repo,
        roots,
        previous,
    }
}

#[test]
fn a_publication_refused_part_way_keeps_the_previous_one() {
    let mut env = TestEnv::new();
    let republication = republication(&mut env);
    let output = env
        .instrukt(republication.repo.path())
        .arg("sync")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(i32::from(FENCE_EXIT)));
    assert_eq!(
        published_files(&republication.roots),
        republication.previous
    );
    assert_eq!(env.refusals().len(), 1);
}

#[test]
fn a_refusal_the_record_cannot_keep_fails_the_test_and_is_rolled_back() {
    let mut env = TestEnv::new();
    let republication = republication(&mut env);
    // The record cannot grow: it is made as long as the file system lets a
    // file be, found by halving the gap between a length it accepts and one
    // it refuses. The file stays sparse, so this takes no room on the volume.
    let record = env.path("fence").join(instrukt_core::fsx::FENCE_RECORD);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&record)
        .unwrap();
    let (mut accepted, mut refused) = (0_u64, 1_u64 << 63);
    while refused - accepted > 1 {
        let length = accepted + (refused - accepted) / 2;
        if file.set_len(length).is_ok() {
            accepted = length;
        } else {
            refused = length;
        }
    }
    file.set_len(accepted).unwrap();
    drop(file);
    // An append of the test's own fails before instrukt starts.
    let appended = std::io::Write::write_all(
        &mut std::fs::OpenOptions::new()
            .append(true)
            .open(&record)
            .unwrap(),
        b"\n",
    );
    assert!(appended.is_err());
    let output = env
        .instrukt(republication.repo.path())
        .arg("sync")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(i32::from(FENCE_EXIT)));
    assert_eq!(
        published_files(&republication.roots),
        republication.previous
    );
    // The refusal removed the record it could not append to, which fails
    // the test as an unreadable record does.
    assert!(!record.exists());
    let verdict = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| env.refusals()));
    assert!(verdict.is_err());
    // This test of the fence lets its own verdict pass.
    std::fs::write(&record, "").unwrap();
}

/// Every file under `roots`, with its content.
fn published_files(roots: &[std::path::PathBuf]) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    roots
        .iter()
        .flat_map(|root| walkdir::WalkDir::new(root).follow_links(true))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            (
                entry.path().to_path_buf(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}
