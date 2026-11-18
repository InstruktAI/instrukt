//! `project/instrukt/spec/feature/publication/editor-schemas`: the binary
//! writes JSON Schemas of its own document types into the repository's
//! `.instrukt/schemas/`, and init and the pre-commit section keep a relative
//! modeline at the top of each file they describe.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};

/// The schema files instrukt generates.
const SCHEMAS: [&str; 5] = [
    "general-index.schema.json",
    "domain-index.schema.json",
    "project-index.schema.json",
    "snippet-schema.schema.json",
    "instrukt.schema.json",
];

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

/// `text` with each temporary file name (`.tmp` and six random characters)
/// shown as `.tmp[RANDOM]`.
fn without_temporary_names(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(".tmp") {
        out.push_str(&rest[..at]);
        out.push_str(".tmp[RANDOM]");
        let after = &rest[at + 4..];
        let random = after
            .char_indices()
            .take(6)
            .take_while(|(_, c)| c.is_ascii_alphanumeric())
            .map(|(i, c)| i + c.len_utf8())
            .last()
            .unwrap_or(0);
        rest = &after[random..];
    }
    out.push_str(rest);
    out
}

/// A repository holding a file of every kind an editor schema describes.
fn authored(env: &TestEnv, name: &str) -> Repo {
    let repo = env.git_repo(name);
    repo.write("docs/general/index.yaml", "expanded: []\n");
    repo.write(
        "docs/domain/sdlc/index.yaml",
        "description: How code is built.\n",
    );
    repo.write(
        "docs/domain/sdlc/schema.yaml",
        "spec/feature:\n  required: [Use cases]\n",
    );
    repo
}

/// The file a modeline in `file` (relative to `repo`) points at.
fn schema_of(repo: &Repo, file: &str) -> PathBuf {
    let text = repo.read(file);
    let first = text.lines().next().unwrap();
    let (_, target) = first.split_once("$schema=").unwrap();
    let dir = repo.path().join(file);
    dir.parent().unwrap().join(target)
}

/// The generated schemas below `repo`, by name.
fn schemas(repo: &Repo) -> Vec<(String, Vec<u8>)> {
    SCHEMAS
        .iter()
        .map(|name| {
            let path = repo.path().join(".instrukt/schemas").join(name);
            ((*name).to_owned(), std::fs::read(path).unwrap())
        })
        .collect()
}

const DESCRIBED: [&str; 5] = [
    "instrukt.yaml",
    "docs/general/index.yaml",
    "docs/domain/sdlc/index.yaml",
    "docs/domain/sdlc/schema.yaml",
    "docs/project/index.yaml",
];

// spec: project/instrukt/spec/feature/publication/editor-schemas UC-ES1
#[test]
fn each_file_names_its_generated_schema_by_a_path_inside_the_repository() {
    let env = TestEnv::new();
    let repo = authored(&env, "demo");

    env.run(repo.path(), ["init"]).success();

    let first_lines: Vec<String> = DESCRIBED
        .iter()
        .map(|file| format!("{file}: {}", repo.read(file).lines().next().unwrap()))
        .collect();
    insta::assert_snapshot!("modelines", first_lines.join("\n"));
    let root = repo.path().canonicalize().unwrap();
    for file in DESCRIBED {
        let schema = schema_of(&repo, file).canonicalize().unwrap();
        assert!(
            schema.starts_with(root.join(".instrukt/schemas")),
            "{file} names a schema inside the repository: {}",
            schema.display()
        );
    }
    for (name, bytes) in schemas(&repo) {
        insta::assert_snapshot!(name, String::from_utf8(bytes).unwrap());
    }
    let home = env.home().display().to_string();
    for entry in walkdir::WalkDir::new(repo.path())
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
    {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            let bytes = std::fs::read(entry.path()).unwrap();
            assert!(
                !String::from_utf8_lossy(&bytes).contains(&home),
                "{} names a path in the developer's home",
                entry.path().display()
            );
        }
    }
}

// spec: project/instrukt/spec/feature/publication/editor-schemas UC-ES1
#[test]
fn the_modeline_changes_nothing_validation_reads_and_survives_a_configuration_write() {
    let env = TestEnv::new();
    let repo = authored(&env, "demo");
    env.run(repo.path(), ["init"]).success();

    env.run(repo.path(), ["config", "validate"]).success();
    env.run(repo.path(), ["docs", "validate"]).success();
    let out = env
        .run(repo.path(), ["config", "get", "--revision"])
        .success();
    let revision = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    env.run(
        repo.path(),
        [
            "config",
            "patch",
            "--revision",
            revision.trim(),
            "--yaml",
            "board: false",
        ],
    )
    .success();
    insta::assert_snapshot!("patched-layer", repo.read("instrukt.yaml"));
}

// spec: project/instrukt/spec/feature/publication/editor-schemas UC-ES1
#[cfg(unix)]
#[test]
fn linked_schemas_are_replaced_and_a_linked_file_is_left_and_named() {
    let env = TestEnv::new();
    let elsewhere = env.path("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let repo = env.git_repo("demo");
    let schemas_dir = repo.path().join(".instrukt/schemas");
    std::fs::create_dir_all(repo.path().join(".instrukt")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &schemas_dir).unwrap();
    // A domain index that would take a modeline, a link to a file kept
    // elsewhere.
    let index_copy = env.path("ops-index.yaml");
    let index_text = "description: Operations\n";
    std::fs::write(&index_copy, index_text).unwrap();
    let linked_index = repo.path().join("docs/domain/ops/index.yaml");
    std::fs::create_dir_all(linked_index.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&index_copy, &linked_index).unwrap();
    let is_dir = |path: &std::path::Path| std::fs::symlink_metadata(path).unwrap().is_dir();

    let init = env.run(repo.path(), ["init"]).success();
    insta::assert_snapshot!("init-beside-linked-schemas", stdout(&env, &init));
    assert!(
        is_dir(&schemas_dir),
        "the link is replaced by the directory"
    );
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "nothing was written through the link"
    );
    assert_eq!(std::fs::read_to_string(&index_copy).unwrap(), index_text);

    // The schemas linked again before a sync.
    std::fs::remove_dir_all(repo.path().join("docs/domain")).unwrap();
    let sha = repo.commit("adopt instrukt");
    std::fs::remove_dir_all(&schemas_dir).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &schemas_dir).unwrap();

    let synced = env.run(repo.path(), ["sync"]).success();
    insta::assert_snapshot!(
        "sync-beside-linked-schemas",
        stdout(&env, &synced).replace(&sha, "[SHA]")
    );
    assert!(is_dir(&schemas_dir));
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
}

/// Waits, within a bound, until the detached sync of `project`'s last
/// post-commit run has ended.
fn wait_for_post_commit_sync(env: &TestEnv, project: &str) {
    let record = env
        .instrukt_home()
        .join("state/hooks")
        .join(project)
        .join("post-commit.yaml");
    common::wait_until(
        &format!("the post-commit sync of {project}"),
        std::time::Duration::from_secs(60),
        || {
            std::fs::read_to_string(&record).is_ok_and(|text| {
                let record: instrukt_core::repository::HookRunRecord =
                    serde_yaml_ng::from_str(&text).unwrap();
                record.sync != Some(instrukt_core::repository::SyncOutcome::Running)
            })
        },
    );
}

// spec: project/instrukt/spec/feature/publication/editor-schemas UC-ES2
#[test]
fn publication_brings_older_schemas_and_modelines_to_the_installed_instrukt() {
    // The hook sections call the built `instrukt` by name.
    let mut env = TestEnv::new();
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap();
    let path = format!("{}:{}", bin.display(), env.injected("PATH").unwrap());
    env.set_env("PATH", &path);
    let current = authored(&env, "current");
    env.run(current.path(), ["init"]).success();
    let repo = authored(&env, "demo");
    env.run(repo.path(), ["init"]).success();
    repo.commit("adopt instrukt");
    // The commit's post-commit section started a detached sync; it ends
    // before anything below changes the schemas.
    wait_for_post_commit_sync(&env, "demo");
    env.run(repo.path(), ["sync"]).success();

    // What an older instrukt left: other schema content and a modeline
    // naming another path.
    repo.write(".instrukt/schemas/domain-index.schema.json", "{}\n");
    let layer =
        repo.read("instrukt.yaml")
            .replacen("instrukt.schema.json", "config.schema.json", 1);
    repo.write("instrukt.yaml", &layer);
    let synced = env.run(repo.path(), ["sync"]).success();
    insta::assert_snapshot!(
        "refresh-after-upgrade",
        stdout(&env, &synced).replace(&repo.head(), "[SHA]")
    );

    assert_eq!(
        schemas(&repo),
        schemas(&current),
        "the schemas are the installed instrukt's"
    );
    assert_eq!(
        repo.read("instrukt.yaml"),
        layer,
        "sync writes no committed file"
    );

    // The next commit brings each described file's modeline back.
    repo.commit("keep working");
    for file in DESCRIBED {
        assert_eq!(
            repo.git(["show", &format!("HEAD:{file}")]).lines().next(),
            current.read(file).lines().next(),
            "{file} points at its schema"
        );
    }
    assert_eq!(repo.git(["status", "--porcelain"]), "");
}

// spec: project/instrukt/spec/feature/publication/editor-schemas UC-ES2
#[test]
fn a_refresh_that_cannot_write_fails_the_sync_and_the_next_sync_tries_again() {
    let env = TestEnv::new();
    let repo = authored(&env, "demo");
    env.run(repo.path(), ["init"]).success();
    repo.commit("adopt instrukt");
    let dir = repo.path().join(".instrukt/schemas");
    std::fs::write(dir.join("instrukt.schema.json"), "{}\n").unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let failed = env.run(repo.path(), ["sync"]).failure();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    insta::assert_snapshot!(
        "unwritable-schemas",
        without_temporary_names(&stderr(&env, &failed))
    );
    let record = env.instrukt_home().join("state/publication/demo.yaml");
    assert!(!record.exists(), "no published commit is recorded");

    let next = env.run(repo.path(), ["sync"]).success();
    insta::assert_snapshot!(
        "next-sync-refreshes",
        stdout(&env, &next).replace(&repo.head(), "[SHA]")
    );
    assert!(record.exists(), "the next sync publishes");

    // A refresh that fails part-way puts back what it changed before: the
    // schema it replaced is the link again, pointing where it pointed.
    let moved = env.path("general-index.schema.json");
    std::fs::rename(dir.join("general-index.schema.json"), &moved).unwrap();
    std::os::unix::fs::symlink(&moved, dir.join("general-index.schema.json")).unwrap();
    let moved_bytes = std::fs::read(&moved).unwrap();
    std::fs::remove_file(dir.join("domain-index.schema.json")).unwrap();
    std::fs::create_dir(dir.join("domain-index.schema.json")).unwrap();
    env.run(repo.path(), ["sync"]).failure();
    assert_eq!(
        std::fs::read_link(dir.join("general-index.schema.json")).unwrap(),
        moved
    );
    assert_eq!(std::fs::read(&moved).unwrap(), moved_bytes);
    std::fs::remove_dir(dir.join("domain-index.schema.json")).unwrap();
    std::fs::remove_file(dir.join("general-index.schema.json")).unwrap();
    std::fs::rename(&moved, dir.join("general-index.schema.json")).unwrap();
    env.run(repo.path(), ["sync"]).success();

    // A sync that fails after the refresh puts the schemas' directory back
    // as the link it was.
    let elsewhere = env.path("schemas-elsewhere");
    std::fs::rename(&dir, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
    let kept: Vec<Vec<u8>> = SCHEMAS
        .iter()
        .map(|name| std::fs::read(elsewhere.join(name)).unwrap())
        .collect();
    std::fs::remove_file(&record).unwrap();
    std::fs::create_dir(&record).unwrap();
    env.run(repo.path(), ["sync"]).failure();
    assert_eq!(std::fs::read_link(&dir).unwrap(), elsewhere);
    let after: Vec<Vec<u8>> = SCHEMAS
        .iter()
        .map(|name| std::fs::read(elsewhere.join(name)).unwrap())
        .collect();
    assert_eq!(after, kept);
}
