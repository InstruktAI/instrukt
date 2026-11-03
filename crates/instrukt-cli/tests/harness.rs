//! The harness's own guarantees: the binary runs against the temporary
//! home, subprocesses see no live environment, and writes outside the
//! temporary home are detected.

mod common;

use common::{LiveGuard, TestEnv};
use predicates::str::contains;

#[test]
fn every_command_prints_its_contract_with_help() {
    let env = TestEnv::new();
    let cwd = env.path("");
    for command in [
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
    ] {
        env.run(&cwd, [command, "--help"])
            .success()
            .stdout(contains("Usage: instrukt"));
    }
}

#[test]
fn subprocesses_see_the_temporary_home_and_no_live_session() {
    let mut env = TestEnv::new();
    env.set_env("FAKE_SESSION_ID", "s-1");
    let repo = env.git_repo("demo");
    repo.write("README.md", "demo\n");
    let sha = repo.commit("feat: start");
    assert_eq!(sha.len(), 40);
    let author = repo.git(["log", "-1", "--format=%an <%ae>"]);
    assert_eq!(author, "Test Developer <developer@example.test>");
    let home_seen = env.git(
        repo.path(),
        ["config", "--global", "--list", "--show-origin"],
    );
    assert!(
        home_seen.contains(&env.home().display().to_string()),
        "{home_seen}"
    );
}

#[test]
fn a_write_to_a_guarded_path_is_detected() {
    let live = tempfile::tempdir().unwrap();
    let settings = live.path().join("settings.json");
    std::fs::write(&settings, "{}").unwrap();
    let tree = live.path().join(".instrukt");
    let guard = LiveGuard::over(vec![settings.clone()], vec![tree.clone()]);
    assert!(guard.changed().is_empty());

    std::fs::create_dir_all(tree.join("docs")).unwrap();
    std::fs::write(&settings, "{\"changed\": true}").unwrap();
    let changed = guard.changed();
    assert!(changed.contains(&settings), "{changed:?}");
    assert!(changed.contains(&tree), "{changed:?}");
}
