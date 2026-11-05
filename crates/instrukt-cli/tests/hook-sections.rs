//! `project/instrukt/spec/feature/repository/hook-sections`:
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
use instrukt_core::fsx::LockGuard;
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::publication::PublishedRecord;
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

/// The commit publication last recorded for `project`; `None` before the
/// first.
fn published(env: &TestEnv, project: &str) -> Option<String> {
    let home = Home::at_user_home(env.home().to_path_buf());
    PublishedRecord::load(&home, &ProjectSlug::parse(project).unwrap())
        .unwrap()
        .map(|record| record.commit.to_string())
}

/// Waits, within a bound, until publication has recorded `commit`.
fn wait_published(env: &TestEnv, project: &str, commit: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while published(env, project).as_deref() != Some(commit) {
        assert!(
            Instant::now() < deadline,
            "{commit} was never published (last published: {:?})",
            published(env, project)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Holds the home's docs lock, under which every sync publishes, so a sync
/// a hook run started cannot complete until the guard is dropped.
fn block_publication(env: &TestEnv) -> LockGuard {
    LockGuard::acquire(
        &env.instrukt_home().join("locks/docs.lock"),
        Duration::from_secs(30),
    )
    .unwrap()
}

/// Runs `git <args>` in `repo`, failing the test unless git returns within
/// a bound: a hook that waited for a blocked publication would not.
fn git_returns(env: &TestEnv, repo: &Repo, args: &[&str]) {
    env.git_command(repo.path())
        .args(args)
        .timeout(Duration::from_secs(30))
        .assert()
        .success();
}

/// A hook file holding the developer's own lines and another tool's
/// marked section.
const OWN_POST_COMMIT: &str = "#!/bin/sh\n\
# >>> other-tool >>>\n\
other-tool run post-commit\n\
# <<< other-tool <<<\n\
echo ran >> \"$(git rev-parse --git-dir)/own-hook-ran\"\n";

/// A developer's post-commit hook that ends by replacing the shell with
/// another program, an assignment prefixing its `exec`.
const OWN_EXEC_POST_COMMIT: &str = "#!/bin/sh\n\
# >>> other-tool >>>\n\
other-tool run post-commit\n\
# <<< other-tool <<<\n\
HOOK_NAME=post-commit exec sh -c 'echo ran >> \"$(git rev-parse --git-dir)/own-hook-ran\"'\n";

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK1
#[test]
fn installing_keeps_the_developers_own_hook_and_a_commit_starts_publication() {
    let env = machine();
    let repo = project(&env, "demo");
    let post_commit = hook_path(&repo, "post-commit");
    write_hook(&post_commit, OWN_EXEC_POST_COMMIT, 0o755);

    let installed = env.run(repo.path(), ["hooks", "install"]).success();
    insta::assert_snapshot!("install-beside-own-hook", stdout(&env, &installed));
    insta::assert_snapshot!(
        "own-hook-with-section",
        std::fs::read_to_string(&post_commit).unwrap()
    );

    let blocked = block_publication(&env);
    repo.write("notes.md", "a change\n");
    repo.git(["add", "notes.md"]);
    git_returns(
        &env,
        &repo,
        &["commit", "--quiet", "-m", "a change on main"],
    );
    let head = repo.head();
    assert!(
        repo.path().join(".git/own-hook-ran").exists(),
        "the developer's own hook ran"
    );
    assert_ne!(
        published(&env, "demo").as_deref(),
        Some(head.as_str()),
        "git returned while publication was still blocked"
    );

    drop(blocked);
    wait_published(&env, "demo", &head);
    finished_run(&env, "demo", Hook::PostCommit);
}

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK1
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

/// The names in the test machine's temporary directory.
fn temporary_files(env: &TestEnv) -> Vec<PathBuf> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(env.path("tmp"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    names.sort();
    names
}

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK1
#[test]
fn a_pre_push_section_after_instrukts_reads_the_full_ref_list() {
    let env = machine();
    let without = env.bare_remote("without-instrukt");
    let with = env.bare_remote("with-instrukt");
    let repo = project(&env, "demo");
    for remote in [&without, &with] {
        let name = remote.file_stem().unwrap();
        repo.git([
            std::ffi::OsStr::new("remote"),
            std::ffi::OsStr::new("add"),
            name,
            remote.as_os_str(),
        ]);
    }
    repo.write(
        "instrukt.yaml",
        &format!(
            "{}modules:\n  blueprint: true\n",
            repo.read("instrukt.yaml")
        ),
    );
    repo.commit("switch blueprint on");
    let captured = env.path("captured");
    std::fs::create_dir_all(&captured).unwrap();
    // The developer's own section: keeps the ref list git sent, per remote.
    let own_section = format!("cat > \"{}/$1\"\n", captured.display());
    let pre_push = hook_path(&repo, "pre-push");

    write_hook(&pre_push, &format!("#!/bin/sh\n{own_section}"), 0o755);
    repo.git(["push", "--quiet", "without-instrukt", "main"]);

    std::fs::remove_file(&pre_push).unwrap();
    env.run(repo.path(), ["hooks", "install"]).success();
    let with_section = std::fs::read_to_string(&pre_push).unwrap();
    insta::assert_snapshot!("pre-push-section", with_section);
    write_hook(&pre_push, &format!("{with_section}{own_section}"), 0o755);

    let temporary_before = temporary_files(&env);
    repo.git(["push", "--quiet", "with-instrukt", "main"]);

    let sent = std::fs::read(captured.join("without-instrukt")).unwrap();
    assert!(!sent.is_empty(), "git sent a ref list");
    assert_eq!(
        std::fs::read(captured.join("with-instrukt")).unwrap(),
        sent,
        "the section after instrukt's read the same full ref list"
    );
    assert_eq!(
        env.git(&with, ["rev-parse", "main"]),
        repo.head(),
        "the push completed"
    );
    assert_eq!(
        temporary_files(&env),
        temporary_before,
        "no temporary file stays behind"
    );
}

/// A step of the pre-push section made to fail from outside: a program on
/// `PATH` standing in for the one the step runs, or the temporary directory
/// the section is given.
enum Failure {
    /// `rm` cannot remove the temporary file.
    Removal,
    /// The temporary directory does not exist, so no file can be made.
    TemporaryFile,
    /// `cat` cannot copy the ref list.
    Copy,
    /// The temporary file is gone by the time stdin is restored from it.
    FileGone,
}

/// Pushes through a pre-push hook that runs under `set -eu`, holds
/// instrukt's section, and after it a developer section recording that it
/// ran, with `failure` arranged; asserts git's push result is that of a
/// push without failure and that the following section ran.
fn push_with_failing_step(failure: &Failure) {
    let mut env = machine();
    let fakes = env.path("fake-bin");
    std::fs::create_dir_all(&fakes).unwrap();
    let fake = |name: &str, script: &str| write_hook(&fakes.join(name), script, 0o755);
    match failure {
        Failure::Removal => fake("rm", "#!/bin/sh\nexit 1\n"),
        Failure::Copy => fake("cat", "#!/bin/sh\nexit 1\n"),
        Failure::FileGone => fake(
            "cat",
            "#!/bin/sh\nfor f in \"$TMPDIR\"/instrukt-pre-push.*; do /bin/rm -f \"$f\"; done\n",
        ),
        Failure::TemporaryFile => {
            env.set_env("TMPDIR", &env.path("no-such-dir").display().to_string());
        }
    }
    let bin = Path::new(env!("CARGO_BIN_EXE_instrukt")).parent().unwrap();
    let path = format!(
        "{}:{}:{}",
        fakes.display(),
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    env.set_env("PATH", &path);

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
    let pre_push = hook_path(&repo, "pre-push");
    write_hook(&pre_push, "#!/bin/sh\nset -eu\n", 0o755);
    env.run(repo.path(), ["hooks", "install"]).success();
    let ran = env.path("following-section-ran");
    let installed = std::fs::read_to_string(&pre_push).unwrap();
    write_hook(
        &pre_push,
        &format!("{installed}echo ran > \"{}\"\n", ran.display()),
        0o755,
    );

    git_returns(&env, &repo, &["push", "--quiet", "origin", "main"]);
    assert_eq!(
        env.git(&remote, ["rev-parse", "main"]),
        repo.head(),
        "the push landed"
    );
    assert!(ran.exists(), "the section after instrukt's ran");
}

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK1
#[test]
fn a_temporary_file_that_cannot_be_removed_leaves_the_push_and_later_sections_alone() {
    push_with_failing_step(&Failure::Removal);
}

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK1
#[test]
fn a_temporary_file_that_cannot_be_made_leaves_the_push_and_later_sections_alone() {
    push_with_failing_step(&Failure::TemporaryFile);
}

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK1
#[test]
fn a_failed_copy_of_the_ref_list_leaves_the_push_and_later_sections_alone() {
    push_with_failing_step(&Failure::Copy);
}

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK1
#[test]
fn a_temporary_file_gone_before_stdin_is_restored_leaves_the_push_and_later_sections_alone() {
    push_with_failing_step(&Failure::FileGone);
}

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK2
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

    upstream.write("a.md", "first upstream change\n");
    upstream.commit("upstream change by merge");
    upstream.git(["push", "--quiet", "origin", "main"]);
    let blocked = block_publication(&env);
    git_returns(&env, &developer, &["pull", "--quiet", "--no-rebase"]);
    let merged = developer.head();
    assert_eq!(merged, upstream.head(), "the pull completed");
    assert_ne!(
        published(&env, "demo").as_deref(),
        Some(merged.as_str()),
        "the pull returned while publication was still blocked"
    );
    drop(blocked);
    wait_published(&env, "demo", &merged);
    finished_run(&env, "demo", Hook::PostMerge);

    developer.write("local.md", "a local commit\n");
    developer.commit("local work");
    wait_published(&env, "demo", &developer.head());
    finished_run(&env, "demo", Hook::PostCommit);
    upstream.write("b.md", "second upstream change\n");
    upstream.commit("upstream change by rebase");
    upstream.git(["push", "--quiet", "origin", "main"]);
    let blocked = block_publication(&env);
    git_returns(&env, &developer, &["pull", "--quiet", "--rebase"]);
    let rebased = developer.head();
    assert_eq!(
        developer.git(["rev-parse", "HEAD~1"]),
        upstream.head(),
        "the rebase put the local commit on the new main"
    );
    assert_ne!(
        published(&env, "demo").as_deref(),
        Some(rebased.as_str()),
        "the pull returned while publication was still blocked"
    );
    drop(blocked);
    wait_published(&env, "demo", &rebased);
    finished_run(&env, "demo", Hook::PostRewrite);
    finished_run(&env, "demo", Hook::PostCheckout);
}

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK3
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
const OWN_POST_CHECKOUT: &str = "#!/bin/sh\necho disabled";

// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK4
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
