//! `project/instrukt/spec/feature/repository/project-initialization`:
//! `instrukt init` adopts a git repository in one idempotent step and never
//! overwrites what exists.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
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
    let path = format!("{}:{}", bin.display(), env.injected("PATH").unwrap());
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
    let progress =
        crate::common::progress::Progress::new(path.parent().unwrap(), Duration::from_secs(30));
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
        progress.wait();
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
    insta::assert_snapshot!("seeded-index", repo.read("docs/project/index.yaml"));
    insta::assert_snapshot!("seeded-master", repo.read("AGENTS.master.md"));
    insta::assert_snapshot!(
        "own-hook-beside-section",
        repo.read(".git/hooks/post-commit")
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
const OWN_INDEX: &str = "expanded: [project/demo/policy/house-rules]\n";
const OWN_IGNORES: &str = "/build\n*.log";
const OWN_ATTRIBUTES: &str = "*.png binary\ndocs/**/*.md text eol=lf\n";

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN2
#[test]
fn init_completes_a_partial_adoption_and_then_changes_nothing() {
    let env = machine();
    let repo = env.git_repo("demo");
    repo.write("AGENTS.master.md", OWN_MASTER);
    repo.write(".gitattributes", OWN_ATTRIBUTES);
    repo.write(".gitignore", OWN_IGNORES);
    repo.write("docs/project/index.yaml", OWN_INDEX);
    env.run(repo.path(), ["config", "init"]).success();
    repo.commit("the developer's partial adoption");

    let completed = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("completing-init", stdout(&env, &completed));
    assert_eq!(
        repo.read("AGENTS.master.md"),
        OWN_MASTER,
        "the developer's master is unchanged"
    );
    insta::assert_snapshot!("index-with-modeline", repo.read("docs/project/index.yaml"));
    insta::assert_snapshot!("completed-ignores", repo.read(".gitignore"));
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

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN5
#[test]
fn a_hand_written_agents_file_is_kept_aside_and_left_until_the_first_publication_renders_it() {
    let env = machine();
    let repo = env.git_repo("demo");
    repo.write("AGENTS.md", HAND_WRITTEN);
    repo.commit("the developer's own instructions");

    let first = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("init-beside-hand-written-agents", stdout(&env, &first));
    assert_eq!(
        repo.read(".instrukt/adopted/AGENTS.md"),
        HAND_WRITTEN,
        "the hand-written instructions are kept byte for byte"
    );
    insta::assert_snapshot!(
        "master-beside-kept-instructions",
        repo.read("AGENTS.master.md")
    );
    assert_eq!(
        repo.read("AGENTS.md"),
        HAND_WRITTEN,
        "the hand-written file is left as it was"
    );

    let before = tree(repo.path());
    env.run(repo.path(), ["init"]).success();
    assert_eq!(
        tree(repo.path()),
        before,
        "nothing in the repository changed"
    );

    repo.git(["rm", "--cached", "--quiet", "AGENTS.md"]);
    repo.commit("adopt instrukt");
    wait_for_run(&env, "demo", Hook::PostCommit);
    env.run(repo.path(), ["sync"]).success();
    insta::assert_snapshot!("first-rendered-agents", repo.read("AGENTS.md"));
    assert_eq!(
        repo.read(".instrukt/adopted/AGENTS.md"),
        HAND_WRITTEN,
        "the kept copy outlives the first publication"
    );
    let after = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("init-after-first-publication", stdout(&env, &after));
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

/// A policy snippet with every required section.
fn policy(title: &str) -> String {
    format!(
        "---\ndescription: The {title} policy.\n---\n\n# {title} — Policy\n\n\
         ## Rules\n\nFollow it.\n\n## Rationale\n\nBecause.\n\n## Scope\n\nAll work.\n\n\
         ## Enforcement\n\nReview.\n\n## Exceptions\n\nNone.\n"
    )
}

/// Another project on the computer, `name`, publishing `domain` with
/// `files` (repository path, contents) beside one policy snippet.
fn publisher(env: &TestEnv, name: &str, domain: &str, files: &[(&str, &str)]) -> common::Repo {
    let repo = env.git_repo(name);
    repo.write(
        "instrukt.yaml",
        &format!("schema_version: 1\nproject: {name}\n"),
    );
    repo.write(
        &format!("{}/policy/rules.md", common::position_dir(domain)),
        &policy(&format!("{domain} rules")),
    );
    for (path, contents) in files {
        repo.write(path, contents);
    }
    repo.commit("publish");
    env.run(repo.path(), ["sync"]).success();
    repo
}

/// Every file below `dir`, with its bytes.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    tree(dir)
        .into_iter()
        .map(|(path, (bytes, _, _))| (path, bytes))
        .collect()
}

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN3
#[test]
fn adopting_a_docs_repository_reports_its_claims_before_anything_is_published() {
    let env = machine();
    publisher(&env, "other", "ops", &[]);
    std::fs::create_dir_all(env.home().join(".claude")).unwrap();
    std::fs::write(
        env.home().join(".claude/CLAUDE.md"),
        "My own instructions.\n",
    )
    .unwrap();
    std::fs::create_dir_all(env.home().join(".codex")).unwrap();
    let published = |env: &TestEnv| {
        let home = env.instrukt_home();
        (files(&home.join("docs")), files(&home.join("state")))
    };
    let before = published(&env);

    let repo = env.git_repo("doctrine");
    repo.write("docs/general/policy/always.md", &policy("Always"));
    repo.write("docs/domain/free/policy/free.md", &policy("Free"));
    repo.write("docs/domain/ops/policy/ops.md", &policy("Ops"));
    repo.write(
        "docs/domain/project/policy/reserved.md",
        &policy("Reserved"),
    );
    repo.commit("the developer's doctrine");

    let init = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("claims-report", stdout(&env, &init));
    assert_eq!(published(&env), before, "nothing was published or claimed");
    assert_eq!(
        std::fs::read_to_string(env.home().join(".claude/CLAUDE.md")).unwrap(),
        "My own instructions.\n",
        "no global agent file was written"
    );
    assert!(
        !env.home().join(".codex/AGENTS.md").exists(),
        "no global agent file was written"
    );
}

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN3
#[test]
fn a_repository_holding_only_domains_writes_no_global_agent_files() {
    let env = machine();
    std::fs::create_dir_all(env.home().join(".claude")).unwrap();
    let repo = env.git_repo("ops-docs");
    repo.write("docs/domain/ops/policy/ops.md", &policy("Ops"));

    let init = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("claims-without-general", stdout(&env, &init));
}

const SDLC_SCHEMA: &str = "spec/feature:\n  required: [Use cases]\n";
const CHANGED_SDLC_SCHEMA: &str = "spec/feature:\n  required: [Use cases, Known caveats]\n";

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN4
#[test]
fn a_new_project_is_given_its_own_copy_of_the_published_sdlc_schema() {
    let env = machine();
    let sdlc = publisher(
        &env,
        "doctrine",
        "sdlc",
        &[("docs/domain/sdlc/schema.yaml", SDLC_SCHEMA)],
    );
    let repo = env.git_repo("demo");

    let init = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("given-the-sdlc-schema", stdout(&env, &init));
    insta::assert_snapshot!("project-schema-copy", repo.read("docs/project/schema.yaml"));

    sdlc.write("docs/domain/sdlc/schema.yaml", CHANGED_SDLC_SCHEMA);
    sdlc.commit("change the sdlc schema");
    env.run(sdlc.path(), ["sync"]).success();
    let own = std::fs::read(repo.path().join("docs/project/schema.yaml")).unwrap();
    env.run(repo.path(), ["init"]).success();
    assert_eq!(
        std::fs::read(repo.path().join("docs/project/schema.yaml")).unwrap(),
        own,
        "the project's own schema is unchanged"
    );
}

// spec: project/instrukt/spec/feature/repository/project-initialization UC-IN4
#[test]
fn without_a_published_sdlc_no_project_schema_is_written() {
    let env = machine();
    let repo = env.git_repo("demo");

    let init = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("without-sdlc", stdout(&env, &init));
    assert!(
        !repo.path().join("docs/project/schema.yaml").exists(),
        "no project schema"
    );
}
