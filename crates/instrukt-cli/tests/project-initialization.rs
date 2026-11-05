//! `project/instrukt/spec/feature/repository/project-initialization`:
//! `instrukt init` adopts a git repository in one idempotent step and never
//! overwrites what exists.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use assert_cmd::assert::Assert;
use common::TestEnv;
use instrukt_core::repository::{Hook, HookRunRecord, SyncOutcome};

/// A test machine whose `PATH` holds the built `instrukt`, as an installed
/// one would: hook sections call it by name.
fn machine() -> TestEnv {
    let mut env = TestEnv::new();
    let bin = Path::new(env!("CARGO_BIN_EXE_instrukt")).parent().unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    env.set_env("PATH", &path);
    env
}

/// Emitted text with the test's temporary directory shown as `[TMP]`.
fn shown(env: &TestEnv, bytes: &[u8]) -> String {
    let base = env.path("").display().to_string();
    String::from_utf8_lossy(bytes).replace(base.trim_end_matches('/'), "[TMP]")
}

fn stdout(env: &TestEnv, assert: &Assert) -> String {
    shown(env, &assert.get_output().stdout)
}

fn stderr(env: &TestEnv, assert: &Assert) -> String {
    shown(env, &assert.get_output().stderr)
}

/// Waits, within a bound, until the detached worker of `hook`'s last run
/// has recorded how sync ended.
fn wait_for_run(env: &TestEnv, project: &str, hook: Hook) -> HookRunRecord {
    let path = env
        .instrukt_home()
        .join("state/hooks")
        .join(project)
        .join(format!("{}.yaml", hook.name()));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let record: HookRunRecord = serde_yaml_ng::from_str(&text).unwrap();
            if record.sync != Some(SyncOutcome::Running) {
                return record;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no finished run of {hook} at {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Every file below `dir` (the repository's git directory included) with
/// its bytes, permissions and modification time.
fn tree(dir: &Path) -> BTreeMap<PathBuf, (Vec<u8>, u32, SystemTime)> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let meta = entry.metadata().unwrap();
            (
                entry.path().to_path_buf(),
                (
                    std::fs::read(entry.path()).unwrap(),
                    meta.permissions().mode(),
                    meta.modified().unwrap(),
                ),
            )
        })
        .collect()
}

const OWN_POST_COMMIT: &str = "#!/bin/sh\necho \"the developer's own post-commit\"\n";

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN1
#[test]
fn one_init_adopts_the_repository_and_commits_nothing() {
    let env = machine();
    let repo = env.git_repo("My Demo");
    let own_hook = repo.path().join(".git/hooks/post-commit");
    std::fs::write(&own_hook, OWN_POST_COMMIT).unwrap();
    std::fs::set_permissions(&own_hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    repo.write("README.md", "# demo\n");
    let head = repo.commit("the developer's first commit");

    let init = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("first-init", stdout(&env, &init));

    assert_eq!(repo.head(), head, "nothing was committed");
    assert!(
        repo.git(["diff", "--cached", "--name-only"]).is_empty(),
        "nothing was staged"
    );
    let project = env.run(repo.path(), ["config", "get", "project"]).success();
    insta::assert_snapshot!(stdout(&env, &project), @"my-demo");
    assert!(repo.path().join("docs/project").is_dir(), "docs/project");
    insta::assert_snapshot!("seeded-master", repo.read("AGENTS.master.md"));
    insta::assert_snapshot!(
        "own-hook-beside-section",
        repo.read(".git/hooks/post-commit")
    );
    assert!(
        repo.read("docs/project/baseline.md").is_empty(),
        "the local baseline the master requires is seeded empty"
    );

    repo.commit("adopt instrukt");
    let adopted = wait_for_run(&env, "my-demo", Hook::PostCommit);
    assert!(
        matches!(
            adopted.sync,
            Some(SyncOutcome::Finished {
                exit_code: Some(0),
                ..
            })
        ),
        "the commit's publication succeeded: {:?}",
        adopted.sync
    );
    env.run(repo.path(), ["sync"]).success();
    let validated = env.run(repo.path(), ["docs", "validate"]).success();
    insta::assert_snapshot!("adopted-docs-validate", stdout(&env, &validated));
    insta::assert_snapshot!("compiled-seeded-master", repo.read("AGENTS.md"));
}

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN1
#[test]
fn a_folder_without_a_usable_slug_needs_one_supplied_and_outside_a_repository_init_is_refused() {
    let env = machine();
    let outside = env.path("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let refused = env.run(&outside, ["init"]).failure();
    insta::assert_snapshot!("outside-a-repository", stderr(&env, &refused));

    let repo = env.git_repo("___");
    let unnamed = env.run(repo.path(), ["init"]).failure();
    insta::assert_snapshot!("empty-slug", stderr(&env, &unnamed));
    assert!(
        !repo.path().join("instrukt.yaml").exists(),
        "a refused init writes no project layer"
    );

    env.run(repo.path(), ["init", "--slug", "named"]).success();
    let project = env.run(repo.path(), ["config", "get", "project"]).success();
    insta::assert_snapshot!(stdout(&env, &project), @"named");
}

const OWN_MASTER: &str = "## Purpose\n\nThe developer's own master.\n";
const OWN_BASELINE: &str = "@docs/project/policy/house-rules.md\n";
const OWN_ATTRIBUTES: &str = "*.png binary\ndocs/**/*.md text eol=lf\n";

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN2
#[test]
fn init_completes_a_partial_adoption_and_then_changes_nothing() {
    let env = machine();
    let repo = env.git_repo("demo");
    repo.write("AGENTS.master.md", OWN_MASTER);
    repo.write(".gitattributes", OWN_ATTRIBUTES);
    repo.write("docs/project/baseline.md", OWN_BASELINE);
    env.run(repo.path(), ["config", "init"]).success();
    repo.commit("the developer's partial adoption");
    let master = std::fs::read(repo.path().join("AGENTS.master.md")).unwrap();
    let baseline = std::fs::read(repo.path().join("docs/project/baseline.md")).unwrap();

    let completed = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("completing-init", stdout(&env, &completed));
    assert_eq!(
        std::fs::read(repo.path().join("AGENTS.master.md")).unwrap(),
        master,
        "the developer's master is unchanged"
    );
    assert_eq!(
        std::fs::read(repo.path().join("docs/project/baseline.md")).unwrap(),
        baseline,
        "the developer's local baseline is unchanged"
    );
    assert_eq!(
        repo.read(".gitattributes"),
        OWN_ATTRIBUTES,
        "init writes no git attributes"
    );

    let before = tree(repo.path());
    let again = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("repeated-init", stdout(&env, &again));
    assert_eq!(
        tree(repo.path()),
        before,
        "nothing in the repository changed"
    );
}

const HAND_WRITTEN: &str = "# Instructions\n\nWritten by hand.\n";

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN2
#[test]
fn a_hand_written_agents_file_holds_back_the_master_until_its_content_has_moved() {
    let env = machine();
    let repo = env.git_repo("demo");
    repo.write("AGENTS.md", HAND_WRITTEN);
    let hand_written = std::fs::read(repo.path().join("AGENTS.md")).unwrap();

    let first = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("init-beside-hand-written-agents", stdout(&env, &first));
    assert!(!repo.path().join("AGENTS.master.md").exists());
    assert_eq!(
        std::fs::read(repo.path().join("AGENTS.md")).unwrap(),
        hand_written,
        "the hand-written file is untouched"
    );

    repo.write("docs/project/instructions.md", HAND_WRITTEN);
    std::fs::remove_file(repo.path().join("AGENTS.md")).unwrap();
    let next = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("init-after-content-moved", stdout(&env, &next));
    assert!(repo.path().join("AGENTS.master.md").exists());

    let before = tree(repo.path());
    env.run(repo.path(), ["init"]).success();
    assert_eq!(
        tree(repo.path()),
        before,
        "nothing in the repository changed"
    );
}

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN2
#[test]
fn an_ignore_file_linked_outside_the_repository_refuses_init_and_nothing_changes() {
    let env = machine();
    let outside = env.path("elsewhere/ignore-list");
    std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
    std::fs::write(&outside, "secrets/\n").unwrap();
    let repo = env.git_repo("demo");
    std::os::unix::fs::symlink(&outside, repo.path().join(".gitignore")).unwrap();
    let outside_before = std::fs::read(&outside).unwrap();
    let before = tree(repo.path());

    let refused = env.run(repo.path(), ["init"]).failure();
    insta::assert_snapshot!("escaping-ignore-refused", stderr(&env, &refused));

    assert_eq!(
        std::fs::read(&outside).unwrap(),
        outside_before,
        "the outside file is untouched"
    );
    assert!(
        std::fs::symlink_metadata(repo.path().join(".gitignore"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link stays a link"
    );
    assert_eq!(
        tree(repo.path()),
        before,
        "nothing in the repository changed"
    );
    assert!(
        !env.instrukt_home().join("instrukt.yaml").exists(),
        "nothing was written in the home"
    );
}
