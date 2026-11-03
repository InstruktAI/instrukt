//! The harness's own guarantees: every command prints its contract on
//! `--help` without acting, subprocesses see only the environment the test
//! gives them, and writes to guarded live paths are detected.

mod common;

use common::{LiveGuard, TestEnv};

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

#[test]
fn git_children_see_only_the_injected_environment() {
    assert!(
        std::env::var_os(AMBIENT).is_some(),
        "precondition: {AMBIENT} is ambient"
    );
    let mut env = TestEnv::new();
    env.set_env("FAKE_SESSION_ID", "s-1");
    let repo = env.git_repo("demo");
    let seen = repo.git(["-c", "alias.envdump=!env", "envdump"]);
    let vars: Vec<&str> = seen.lines().collect();
    assert!(vars.contains(&"FAKE_SESSION_ID=s-1"), "{seen}");
    assert!(
        vars.contains(&format!("HOME={}", env.home().display()).as_str()),
        "{seen}"
    );
    assert!(
        !vars.iter().any(|v| v.starts_with(&format!("{AMBIENT}="))),
        "{seen}"
    );
}

#[test]
fn instrukt_children_see_only_the_injected_environment() {
    assert!(
        std::env::var_os(AMBIENT).is_some(),
        "precondition: {AMBIENT} is ambient"
    );
    let mut env = TestEnv::new();
    env.set_env("FAKE_SESSION_ID", "s-1");
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: 1\nproject: demo\nactions:\n  injected: \"${{FAKE_SESSION_ID}}\"\n  ambient: \"${{{AMBIENT}}}\"\n"
        ),
    );
    let injected = env
        .run(repo.path(), ["config", "get", "actions.injected"])
        .success();
    assert_eq!(
        String::from_utf8_lossy(&injected.get_output().stdout).trim(),
        "s-1"
    );
    env.run(repo.path(), ["config", "get", "actions.ambient"])
        .failure();
}

#[test]
fn commits_carry_the_isolated_git_identity() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    repo.write("README.md", "demo\n");
    let sha = repo.commit("feat: start");
    assert_eq!(sha.len(), 40);
    let author = repo.git(["log", "-1", "--format=%an <%ae>"]);
    assert_eq!(author, "Test Developer <developer@example.test>");
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

#[test]
fn a_write_through_a_guarded_symlink_is_detected() {
    let live = tempfile::tempdir().unwrap();
    let real = live.path().join("dotfiles/settings.json");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, "{}").unwrap();
    let link = live.path().join("settings.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let guard = LiveGuard::over(vec![link.clone()], Vec::new());
    assert!(guard.changed().is_empty());

    std::fs::write(&real, "{\"written\": \"through the link\"}").unwrap();
    assert_eq!(guard.changed(), vec![link.clone()]);

    let other = live.path().join("dotfiles/other.json");
    std::fs::write(&other, "{\"written\": \"through the link\"}").unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&other, &link).unwrap();
    assert_eq!(guard.changed(), vec![link]);
}
