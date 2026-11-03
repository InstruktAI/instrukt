//! `local/projects/instrukt/spec/feature/repository/hook-sections`:
//! `instrukt hooks install|remove|status|run` — one marked section per hook
//! file beside whatever else is there, post-hooks that start publication
//! without blocking git, and the status a hook manager's user needs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test: a failed step fails the test"
)]

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::repository::{Hook, HookRunRecord, SyncOutcome};

/// A test machine whose `PATH` holds the built `instrukt`, as an installed
/// one would: hook sections and the docs filter call it by name.
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

/// A repository with a project layer (slug `demo`) and a first commit.
fn project(env: &TestEnv, name: &str) -> Repo {
    let repo = env.git_repo(name);
    env.run(repo.path(), ["config", "init"]).success();
    repo.commit("adopt instrukt configuration");
    repo
}

fn hook_path(repo: &Repo, hook: &str) -> PathBuf {
    repo.path().join(".git/hooks").join(hook)
}

fn file_mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn write_hook(path: &Path, text: &str, mode: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// The recorded run of `hook` once its sync has ended: waits for the
/// detached worker, failing the test after a bound.
fn finished_run(env: &TestEnv, project: &str, hook: Hook) -> HookRunRecord {
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
            "no finished run of {hook} recorded at {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The exit status `instrukt sync` answers with in `repo` when run
/// directly: what a hook run's record must name.
fn sync_exit_code(env: &TestEnv, repo: &Repo) -> Option<i32> {
    env.instrukt(repo.path())
        .arg("sync")
        .output()
        .unwrap()
        .status
        .code()
}

fn assert_names_sync_exit(record: &HookRunRecord, hook: Hook, expected: Option<i32>) {
    assert_eq!(record.hook, hook);
    match &record.sync {
        Some(SyncOutcome::Finished { exit_code, .. }) => assert_eq!(*exit_code, expected),
        other => panic!("{hook}: sync did not finish: {other:?}"),
    }
}

/// A hook file holding the developer's own lines and another tool's
/// marked section.
const OWN_POST_COMMIT: &str = "#!/bin/sh\n\
# >>> other-tool >>>\n\
other-tool run post-commit\n\
# <<< other-tool <<<\n\
echo ran >> \"$(git rev-parse --git-dir)/own-hook-ran\"\n";

// spec: local/projects/instrukt/spec/feature/repository/hook-sections UC-HK1
#[test]
fn installing_keeps_the_developers_own_hook_and_a_commit_starts_publication() {
    let env = machine();
    let repo = project(&env, "demo");
    let post_commit = hook_path(&repo, "post-commit");
    write_hook(&post_commit, OWN_POST_COMMIT, 0o755);

    let installed = env.run(repo.path(), ["hooks", "install"]).success();
    insta::assert_snapshot!("install-beside-own-hook", stdout(&env, &installed));
    insta::assert_snapshot!(
        "own-hook-with-section",
        std::fs::read_to_string(&post_commit).unwrap()
    );

    repo.write("notes.md", "a change\n");
    repo.commit("a change on main");

    assert!(
        repo.path().join(".git/own-hook-ran").exists(),
        "the developer's own hook ran"
    );
    let record = finished_run(&env, "demo", Hook::PostCommit);
    assert_names_sync_exit(&record, Hook::PostCommit, sync_exit_code(&env, &repo));
}

// spec: local/projects/instrukt/spec/feature/repository/hook-sections UC-HK1
#[test]
fn with_blueprint_on_the_pre_push_section_is_installed_and_a_push_still_completes() {
    let env = machine();
    let remote = env.bare_remote("origin");
    let repo = project(&env, "demo");
    repo.git([
        std::ffi::OsStr::new("remote"),
        std::ffi::OsStr::new("add"),
        std::ffi::OsStr::new("origin"),
        remote.as_os_str(),
    ]);
    repo.write(
        "instrukt.yaml",
        &format!(
            "{}modules:\n  blueprint: true\n",
            repo.read("instrukt.yaml")
        ),
    );
    repo.commit("switch blueprint on");

    let installed = env.run(repo.path(), ["hooks", "install"]).success();
    insta::assert_snapshot!("install-with-blueprint", stdout(&env, &installed));
    repo.git(["push", "--quiet", "origin", "main"]);
    assert_eq!(env.git(&remote, ["rev-parse", "main"]), repo.head());

    let blueprint_on = repo.read("instrukt.yaml");
    repo.write(
        "instrukt.yaml",
        &blueprint_on.replace("blueprint: true", "blueprint: false"),
    );
    let reinstalled = env.run(repo.path(), ["hooks", "install"]).success();
    insta::assert_snapshot!("reinstall-with-blueprint-off", stdout(&env, &reinstalled));
    assert!(
        hook_path(&repo, "pre-push").exists(),
        "the pre-push file itself remains"
    );
}

// spec: local/projects/instrukt/spec/feature/repository/hook-sections UC-HK2
#[test]
fn a_pull_by_merge_or_rebase_completes_and_starts_publication_of_the_new_main() {
    let env = machine();
    let remote = env.bare_remote("origin");
    let upstream = env.clone_repo(&remote, "demo");
    env.run(upstream.path(), ["config", "init"]).success();
    upstream.commit("adopt instrukt configuration");
    upstream.git(["push", "--quiet", "origin", "main"]);

    let developer = env.clone_repo(&remote, "developer");
    env.run(developer.path(), ["hooks", "install"]).success();
    let expected = sync_exit_code(&env, &developer);

    upstream.write("a.md", "first upstream change\n");
    upstream.commit("upstream change by merge");
    upstream.git(["push", "--quiet", "origin", "main"]);
    developer.git(["pull", "--quiet", "--no-rebase"]);
    assert_eq!(developer.head(), upstream.head(), "the pull completed");
    let merged = finished_run(&env, "demo", Hook::PostMerge);
    assert_names_sync_exit(&merged, Hook::PostMerge, expected);

    developer.write("local.md", "a local commit\n");
    developer.commit("local work");
    finished_run(&env, "demo", Hook::PostCommit);
    upstream.write("b.md", "second upstream change\n");
    upstream.commit("upstream change by rebase");
    upstream.git(["push", "--quiet", "origin", "main"]);
    developer.git(["pull", "--quiet", "--rebase"]);
    assert_eq!(
        developer.git(["rev-parse", "HEAD~1"]),
        upstream.head(),
        "the rebase put the local commit on the new main"
    );
    let rebased = finished_run(&env, "demo", Hook::PostRewrite);
    assert_names_sync_exit(&rebased, Hook::PostRewrite, expected);
    finished_run(&env, "demo", Hook::PostCheckout);
}

// spec: local/projects/instrukt/spec/feature/repository/hook-sections UC-HK3
#[test]
fn status_names_the_hook_a_hook_manager_regenerated_and_the_line_to_restore_it() {
    let env = machine();
    let repo = project(&env, "demo");
    repo.git(["config", "core.hooksPath", ".manager-hooks"]);
    let hooks_path = repo.git(["config", "core.hooksPath"]);

    let installed = env.run(repo.path(), ["hooks", "install"]).success();
    insta::assert_snapshot!("install-into-hooks-path", stdout(&env, &installed));
    assert_eq!(
        repo.git(["config", "core.hooksPath"]),
        hooks_path,
        "core.hooksPath itself is never changed"
    );
    repo.write("notes.md", "a change\n");
    repo.commit("a change on main");
    finished_run(&env, "demo", Hook::PostCommit);

    write_hook(
        &repo.path().join(".manager-hooks/post-commit"),
        "#!/bin/sh\n# generated by the hook manager\nmanager run post-commit\n",
        0o755,
    );

    let status = env.run(repo.path(), ["hooks", "status"]).success();
    insta::assert_snapshot!("status-after-regeneration", stdout(&env, &status));
}

/// A hook that replaces itself with another program on its last line.
const OWN_POST_MERGE: &str = "#!/bin/sh\n\
exec 1>&2\n\
echo merging\n\
exec other-tool post-merge \"$@\"\n";

/// A hook the developer keeps switched off.
const OWN_POST_CHECKOUT: &str = "#!/bin/sh\necho disabled\n";

// spec: local/projects/instrukt/spec/feature/repository/hook-sections UC-HK4
#[test]
fn removing_leaves_every_byte_of_the_developers_hooks_as_it_was() {
    let env = machine();
    let repo = project(&env, "demo");
    let own = [
        ("post-commit", OWN_POST_COMMIT, 0o755),
        ("post-merge", OWN_POST_MERGE, 0o755),
        ("post-checkout", OWN_POST_CHECKOUT, 0o644),
    ];
    let mut before = Vec::new();
    for (hook, text, mode) in own {
        let path = hook_path(&repo, hook);
        write_hook(&path, text, mode);
        before.push((
            path.clone(),
            std::fs::read(&path).unwrap(),
            file_mode(&path),
        ));
    }

    let installed = env.run(repo.path(), ["hooks", "install"]).success();
    insta::assert_snapshot!(
        "install-beside-exec-and-disabled-hooks",
        stdout(&env, &installed)
    );
    insta::assert_snapshot!(
        "section-before-exec",
        std::fs::read_to_string(hook_path(&repo, "post-merge")).unwrap()
    );
    let reinstalled = env.run(repo.path(), ["hooks", "install"]).success();
    insta::assert_snapshot!("reinstall-is-idempotent", stdout(&env, &reinstalled));

    let removed = env.run(repo.path(), ["hooks", "remove"]).success();
    insta::assert_snapshot!("remove", stdout(&env, &removed));

    for (path, bytes, mode) in before {
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{}", path.display());
        assert_eq!(
            file_mode(&path),
            mode,
            "{} keeps its permissions",
            path.display()
        );
    }
    let created = hook_path(&repo, "post-rewrite");
    assert!(created.exists(), "a hook file instrukt created remains");
    let status = env.run(repo.path(), ["hooks", "status"]).success();
    insta::assert_snapshot!("status-after-remove", stdout(&env, &status));
}
