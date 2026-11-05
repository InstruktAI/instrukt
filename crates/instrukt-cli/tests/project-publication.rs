//! `instrukt sync`
//! (`docs/project/spec/feature/publication/project-publication.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::index::{GlobalIndex, IndexEntry, IndexScope};
use instrukt_core::publication::{EvalFingerprint, PublishedRecord};

/// A policy snippet with every required section.
fn policy(title: &str, extra: &str) -> String {
    format!(
        "---\ndescription: The {title} policy.\n{extra}---\n\n# {title} — Policy\n\n\
         ## Rules\n\nFollow {title}.\n\n## Rationale\n\nBecause.\n\n## Scope\n\nAll.\n\n\
         ## Enforcement\n\nReview.\n\n## Exceptions\n\nNone.\n"
    )
}

/// A project layer declaring `slug`.
fn layer(slug: &str) -> String {
    format!("schema_version: 1\nproject: {slug}\n")
}

/// A repository `name` declaring project `name`, publishing `domains`, with
/// one committed policy per domain and one local spec.
fn project(env: &TestEnv, name: &str, domains: &[&str]) -> Repo {
    let repo = env.git_repo(name);
    repo.write("instrukt.yaml", &layer(name));
    for domain in domains {
        repo.write(
            &format!("{}/policy/rules.md", common::position_dir(domain)),
            &policy("Rules", ""),
        );
    }
    repo.write(
        "docs/project/policy/house.md",
        &policy("House", "visibility: public\n"),
    );
    repo.commit("docs");
    repo
}

fn sync(env: &TestEnv, repo: &Repo) -> Assert {
    env.run(repo.path(), ["sync"])
}

fn home(env: &TestEnv) -> Home {
    Home::at_user_home(env.home().to_path_buf())
}

fn index(env: &TestEnv) -> GlobalIndex {
    GlobalIndex::load(&home(env)).unwrap().unwrap()
}

fn ids(index: &GlobalIndex) -> Vec<&str> {
    index.snippets.iter().map(|e| e.id.as_str()).collect()
}

fn owned_by<'a>(index: &'a GlobalIndex, project: &str) -> Vec<&'a IndexEntry> {
    index
        .snippets
        .iter()
        .filter(|e| e.source_project.as_str() == project)
        .collect()
}

/// Every file of the published tree with its content, by path below
/// `~/.instrukt/docs`.
fn published_tree(env: &TestEnv) -> BTreeMap<PathBuf, Vec<u8>> {
    let docs = env.instrukt_home().join("docs");
    walkdir::WalkDir::new(&docs)
        .into_iter()
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path().strip_prefix(&docs).unwrap().to_path_buf(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

/// Emitted text with commit ids shown as `[SHA]`.
fn text(bytes: &[u8], shas: &[&str]) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    for sha in shas {
        text = text.replace(sha, "[SHA]");
    }
    text
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB1
#[test]
fn a_committed_snippet_is_published_and_indexed_while_uncommitted_changes_stay_out() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let committed = policy("Testing", "");
    repo.write("docs/domain/acme/policy/testing.md", &committed);
    let sha = repo.commit("add testing");
    repo.write("docs/domain/acme/policy/testing.md", &policy("Edited", ""))
        .write("docs/domain/acme/policy/draft.md", &policy("Draft", ""));

    let run = sync(&env, &repo).success();
    insta::assert_snapshot!(text(&run.get_output().stdout, &[&sha]), @"published demo at [SHA]");

    let docs = env.instrukt_home().join("docs");
    assert_eq!(
        std::fs::read_to_string(docs.join("acme/policy/testing.md")).unwrap(),
        committed
    );
    assert!(!docs.join("acme/policy/draft.md").exists());
    let index = index(&env);
    let entry = index
        .snippets
        .iter()
        .find(|e| e.id.as_str() == "acme/policy/testing")
        .unwrap();
    assert_eq!(
        (
            entry.scope,
            entry.path.as_path(),
            entry.source_project.as_str()
        ),
        (
            IndexScope::Global,
            Path::new("acme/policy/testing.md"),
            "demo"
        )
    );
    assert!(!ids(&index).contains(&"acme/policy/draft"));
    let local = index
        .snippets
        .iter()
        .find(|e| e.id.as_str() == "project/demo/policy/house")
        .unwrap();
    assert_eq!(
        (local.scope, local.path.clone()),
        (
            IndexScope::Local,
            repo.path().join("docs/project/policy/house.md")
        )
    );
    let record = PublishedRecord::load(&home(&env), &entry.source_project)
        .unwrap()
        .unwrap();
    assert_eq!(
        (record.commit.as_str(), record.checkout.as_path()),
        (sha.as_str(), repo.path())
    );
    let fingerprint = EvalFingerprint::current(&home(&env)).unwrap().unwrap();
    assert_eq!(
        Some(fingerprint),
        EvalFingerprint::for_commit(&home(&env), &record.commit).unwrap()
    );
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB1
#[test]
fn a_linked_worktree_publishes_the_committed_main_of_its_project() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let main = repo.head();
    let linked = env.path("demo-feature");
    repo.git([
        "worktree",
        "add",
        "--quiet",
        "-b",
        "feature",
        linked.to_str().unwrap(),
    ]);
    std::fs::write(
        linked.join("docs/domain/acme/policy/feature.md"),
        policy("Feature", ""),
    )
    .unwrap();
    env.git(&linked, ["add", "--all"]);
    env.git(&linked, ["commit", "--quiet", "-m", "feature"]);

    let run = env.run(&linked, ["sync"]).success();
    insta::assert_snapshot!(text(&run.get_output().stdout, &[&main]), @"published demo at [SHA]");
    assert!(!ids(&index(&env)).contains(&"acme/policy/feature"));
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB2
#[test]
fn two_projects_publish_side_by_side_and_a_claim_on_anothers_domain_is_refused() {
    let env = TestEnv::new();
    let alpha = project(&env, "alpha", &["alpha-docs"]);
    let beta = project(&env, "beta", &["beta-docs"]);
    sync(&env, &alpha).success();
    let alpha_entries: Vec<IndexEntry> = owned_by(&index(&env), "alpha")
        .into_iter()
        .cloned()
        .collect();
    sync(&env, &beta).success();

    let both = index(&env);
    assert_eq!(
        ids(&both),
        [
            "alpha-docs/policy/rules",
            "beta-docs/policy/rules",
            "project/alpha/policy/house",
            "project/beta/policy/house",
        ]
    );
    let alpha_after: Vec<IndexEntry> = owned_by(&both, "alpha").into_iter().cloned().collect();
    assert_eq!(alpha_after, alpha_entries);

    let before = published_tree(&env);
    beta.write("instrukt.yaml", &layer("beta")).write(
        "docs/domain/alpha-docs/policy/taken.md",
        &policy("Taken", ""),
    );
    let sha = beta.commit("claim alpha-docs");
    let refused = sync(&env, &beta).failure();
    insta::assert_snapshot!(
        text(&refused.get_output().stderr, &[&sha]),
        @"instrukt: beta at [SHA] is not published: `docs/domain/alpha-docs/` is owned by project `alpha`; it is free to claim once `alpha` stops publishing it"
    );
    assert_eq!(published_tree(&env), before);
    let record = PublishedRecord::load(&home(&env), &ProjectSlug::parse("beta").unwrap())
        .unwrap()
        .unwrap();
    assert_ne!(record.commit.as_str(), sha);
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB9
#[test]
fn a_domain_named_after_a_reserved_name_is_refused_naming_its_directory_and_the_rule() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let published = repo.head();
    sync(&env, &repo).success();
    let before = published_tree(&env);
    repo.write("docs/domain/general/policy/x.md", &policy("X", ""))
        .write("docs/domain/project/policy/y.md", &policy("Y", ""));
    let sha = repo.commit("reserved names");

    let refused = sync(&env, &repo).failure();
    insta::assert_snapshot!(
        text(&refused.get_output().stderr, &[&sha]),
        @"instrukt: demo at [SHA] is not published: `docs/domain/general/` and `docs/domain/project/` are named after a reserved name: `general` and `project` are reserved and `third-party` keeps its special case, so no domain may take them"
    );
    assert_eq!(published_tree(&env), before);
    assert_eq!(published_commit(&env, "demo"), published);
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB9
#[test]
fn a_domain_named_after_no_reserved_name_publishes_and_is_read() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["snippets"]);
    sync(&env, &repo).success();
    let read = env
        .run(repo.path(), ["docs", "get", "snippets/policy/rules"])
        .success();
    insta::assert_snapshot!(
        "unreserved_domain_read",
        without_tmp(&env, &read.get_output().stdout, &[])
    );
}

/// Every file of the project's working tree outside `.git`, with its
/// content.
fn project_tree(repo: &Repo) -> BTreeMap<PathBuf, Vec<u8>> {
    walkdir::WalkDir::new(repo.path())
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path().strip_prefix(repo.path()).unwrap().to_path_buf(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

fn published_commit(env: &TestEnv, project: &str) -> String {
    PublishedRecord::load(&home(env), &ProjectSlug::parse(project).unwrap())
        .unwrap()
        .unwrap()
        .commit
        .to_string()
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB3
#[test]
fn retired_docs_and_instructions_are_retracted_and_nothing_else() {
    let env = TestEnv::new();
    let other = project(&env, "other", &["shared"]);
    sync(&env, &other).success();
    let repo = project(&env, "demo", &["acme", "legacy"]);
    repo.write("docs/domain/acme/policy/retired.md", &policy("Retired", ""))
        .write("AGENTS.master.md", "## Purpose\n\nThe demo project.\n")
        .write("tools/AGENTS.master.md", "## Tools\n\nOnly the tools.\n");
    repo.commit("retired");
    sync(&env, &repo).success();
    assert!(repo.path().join("tools/AGENTS.md").is_file());
    let kept_instructions = repo.read("AGENTS.md");
    let other_entries: Vec<IndexEntry> = owned_by(&index(&env), "other")
        .into_iter()
        .cloned()
        .collect();
    let other_tree: Vec<(PathBuf, Vec<u8>)> = published_tree(&env)
        .into_iter()
        .filter(|(path, _)| path.starts_with("shared"))
        .collect();

    std::fs::remove_file(repo.path().join("docs/domain/acme/policy/retired.md")).unwrap();
    std::fs::remove_dir_all(repo.path().join("docs/domain/legacy")).unwrap();
    std::fs::remove_file(repo.path().join("tools/AGENTS.master.md")).unwrap();
    repo.write("instrukt.yaml", &layer("demo"));
    repo.commit("retire");
    sync(&env, &repo).success();

    let docs = env.instrukt_home().join("docs");
    assert!(!docs.join("acme/policy/retired.md").exists());
    assert!(!docs.join("legacy").exists());
    let after = index(&env);
    assert_eq!(
        ids(&after),
        [
            "acme/policy/rules",
            "project/demo/policy/house",
            "project/other/policy/house",
            "shared/policy/rules",
        ]
    );
    assert!(!repo.path().join("tools/AGENTS.md").exists());
    assert_eq!(repo.read("AGENTS.md"), kept_instructions);
    let other_after: Vec<IndexEntry> = owned_by(&after, "other").into_iter().cloned().collect();
    assert_eq!(other_after, other_entries);
    let other_tree_after: Vec<(PathBuf, Vec<u8>)> = published_tree(&env)
        .into_iter()
        .filter(|(path, _)| path.starts_with("shared"))
        .collect();
    assert_eq!(other_tree_after, other_tree);
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB4
#[test]
fn main_unchanged_since_its_publication_is_a_no_op_until_forced() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    repo.write("AGENTS.master.md", "## Purpose\n\nThe demo project.\n");
    let sha = repo.commit("instructions");
    sync(&env, &repo).success();
    let before = published_tree(&env);
    let project_before = project_tree(&repo);

    let again = sync(&env, &repo).success();
    insta::assert_snapshot!(text(&again.get_output().stdout, &[&sha]), @"demo is already published at [SHA]");
    assert_eq!(published_tree(&env), before);
    assert_eq!(project_tree(&repo), project_before);

    let forced = env.run(repo.path(), ["sync", "--force"]).success();
    insta::assert_snapshot!(text(&forced.get_output().stdout, &[&sha]), @"published demo at [SHA]");
    assert_eq!(published_tree(&env), before);
    assert_eq!(published_commit(&env, "demo"), sha);
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB5
#[test]
fn committed_docs_with_findings_are_refused_and_retried_by_the_next_publication() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let published = repo.head();
    sync(&env, &repo).success();
    let before = published_tree(&env);
    repo.write(
        "docs/domain/acme/policy/bare.md",
        "---\nvisibility: public\n---\n\n# Bare — Policy\n\n## Rules\n\nx\n",
    );
    let sha = repo.commit("broken");

    let refused = sync(&env, &repo).failure();
    insta::assert_snapshot!(text(&refused.get_output().stdout, &[&sha]), @r"
    docs/domain/acme/policy/bare.md: frontmatter-description-missing
    docs/domain/acme/policy/bare.md: structure-section-missing: Enforcement
    docs/domain/acme/policy/bare.md: structure-section-missing: Exceptions
    docs/domain/acme/policy/bare.md: structure-section-missing: Rationale
    docs/domain/acme/policy/bare.md: structure-section-missing: Scope
    ");
    insta::assert_snapshot!(text(&refused.get_output().stderr, &[&sha]), @"instrukt: demo at [SHA] is not published: the committed documentation has 5 validation finding(s)");
    assert_eq!(published_tree(&env), before);
    assert_eq!(published_commit(&env, "demo"), published);

    let retried = sync(&env, &repo).failure();
    assert_eq!(
        text(&retried.get_output().stderr, &[&sha]),
        text(&refused.get_output().stderr, &[&sha])
    );
    assert_eq!(published_commit(&env, "demo"), published);
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB6
#[test]
fn an_unresolvable_configured_value_fails_the_sync_and_keeps_the_previous_publication() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let published = repo.head();
    sync(&env, &repo).success();
    let before = published_tree(&env);
    repo.write("docs/domain/acme/policy/later.md", &policy("Later", ""))
        .write(
            "AGENTS.master.md",
            "## Purpose\n\n<!-- @config: no.such.setting -->\n",
        );
    repo.commit("unresolvable value");

    let failed = sync(&env, &repo).failure();
    insta::assert_snapshot!(text(&failed.get_output().stderr, &[]));
    assert_eq!(published_tree(&env), before);
    assert!(!repo.path().join("AGENTS.md").exists());
    assert_eq!(published_commit(&env, "demo"), published);
}

/// Emitted text with the test's temporary directory shown as `[TMP]`.
fn without_tmp(env: &TestEnv, bytes: &[u8], shas: &[&str]) -> String {
    let base = env.path("").display().to_string();
    text(bytes, shas).replace(base.trim_end_matches('/'), "[TMP]")
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB6
#[test]
fn an_instruction_file_left_with_a_reference_fails_the_sync() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let published = repo.head();
    sync(&env, &repo).success();
    let before = published_tree(&env);
    repo.write(
        "AGENTS.master.md",
        "## Purpose\n\nRead it (@notes/house.md) first.\n",
    )
    .write("notes/house.md", "House notes.\n");
    repo.commit("noted");
    env.run(repo.path(), ["docs", "validate"]).success();

    let failed = sync(&env, &repo).failure();
    insta::assert_snapshot!(without_tmp(&env, &failed.get_output().stderr, &[]), @"instrukt: cannot compile AGENTS.master.md: the compiled text still references `@[TMP]/demo/notes/house.md`");
    assert_eq!(published_tree(&env), before);
    assert_eq!(published_commit(&env, "demo"), published);
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB7
#[cfg(unix)]
#[test]
fn an_output_that_is_a_symbolic_link_is_refused_and_its_target_left_alone() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let published = repo.head();
    sync(&env, &repo).success();
    let before = published_tree(&env);
    let git_config = repo.read(".git/config");
    repo.write("AGENTS.master.md", "## Purpose\n\nThe demo project.\n")
        .write("docs/domain/acme/policy/later.md", &policy("Later", ""));
    std::os::unix::fs::symlink(".git/config", repo.path().join("AGENTS.md")).unwrap();
    repo.commit("aliased output");

    let refused = sync(&env, &repo).failure();
    insta::assert_snapshot!(without_tmp(&env, &refused.get_output().stderr, &[]), @"instrukt: [TMP]/demo/AGENTS.md is a symbolic link or lies behind one; publication writes only inside its root");
    assert_eq!(repo.read(".git/config"), git_config);
    assert_eq!(published_tree(&env), before);
    assert_eq!(published_commit(&env, "demo"), published);
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB8
#[cfg(unix)]
#[test]
fn a_replaced_publication_that_cannot_be_deleted_does_not_fail_the_sync() {
    use std::os::unix::fs::PermissionsExt;

    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    sync(&env, &repo).success();
    let locked = env.instrukt_home().join("docs/acme/policy");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    repo.write("docs/domain/acme/policy/later.md", &policy("Later", ""));
    let sha = repo.commit("later");

    let run = sync(&env, &repo).success();
    let leftovers = || -> Vec<PathBuf> {
        std::fs::read_dir(env.instrukt_home())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".sync-")
            })
            .collect()
    };
    let left = leftovers();
    assert_eq!(left.len(), 1, "the copy that could not be removed is left");
    let stdout = text(&run.get_output().stdout, &[&sha])
        .replace(&left[0].display().to_string(), "[LEFTOVER]");
    insta::assert_snapshot!(stdout, @r"
    published demo at [SHA]
    leftover [LEFTOVER] could not be removed; a later sync removes it
    ");
    assert_eq!(published_commit(&env, "demo"), sha);
    assert!(
        env.instrukt_home()
            .join("docs/acme/policy/later.md")
            .is_file()
    );

    // Still not removable: an unchanged main is a no-op that reports it.
    let still = sync(&env, &repo).success();
    let stdout = text(&still.get_output().stdout, &[&sha])
        .replace(&left[0].display().to_string(), "[LEFTOVER]");
    insta::assert_snapshot!(stdout, @r"
    demo is already published at [SHA]
    leftover [LEFTOVER] could not be removed; a later sync removes it
    ");

    for dir in walkdir::WalkDir::new(&left[0])
        .into_iter()
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_dir())
    {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let removed = sync(&env, &repo).success();
    insta::assert_snapshot!(text(&removed.get_output().stdout, &[&sha]), @"demo is already published at [SHA]");
    assert!(leftovers().is_empty(), "the next sync removes the leftover");
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB7
#[cfg(unix)]
#[test]
fn a_linked_later_output_is_refused_before_anything_is_written() {
    use std::os::unix::fs::MetadataExt;

    /// Every entry below `root` (outside `.git`) with its identity: inode,
    /// modification time, length and whether it is a link.
    fn identities(root: &Path) -> BTreeMap<PathBuf, (u64, i64, i64, u64, bool)> {
        walkdir::WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| e.file_name() != ".git")
            .map(Result::unwrap)
            .map(|e| {
                let meta = std::fs::symlink_metadata(e.path()).unwrap();
                (
                    e.path().to_path_buf(),
                    (
                        meta.ino(),
                        meta.mtime(),
                        meta.mtime_nsec(),
                        meta.len(),
                        meta.file_type().is_symlink(),
                    ),
                )
            })
            .collect()
    }

    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    repo.write("AGENTS.master.md", "## Purpose\n\nThe demo project.\n");
    let published = repo.commit("instructions");
    sync(&env, &repo).success();
    let current = env
        .instrukt_home()
        .join("state/eval-fingerprint/current.yaml");
    let elsewhere = env.path("elsewhere.yaml");
    std::fs::rename(&current, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &current).unwrap();
    let elsewhere_text = std::fs::read_to_string(&elsewhere).unwrap();
    repo.write("docs/domain/acme/policy/later.md", &policy("Later", ""))
        .write(
            "AGENTS.master.md",
            "## Purpose\n\nThe changed demo project.\n",
        );
    repo.commit("later");
    let home_before = identities(&env.instrukt_home());
    let project_before = identities(repo.path());

    let refused = sync(&env, &repo).failure();
    insta::assert_snapshot!(without_tmp(&env, &refused.get_output().stderr, &[]), @"instrukt: [TMP]/home/.instrukt/state/eval-fingerprint/current.yaml is a symbolic link or lies behind one; publication writes only inside its root");
    assert_eq!(identities(&env.instrukt_home()), home_before);
    assert_eq!(identities(repo.path()), project_before);
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), elsewhere_text);
    assert_eq!(published_commit(&env, "demo"), published);
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB8
#[cfg(unix)]
#[test]
fn a_refused_sync_still_reports_the_leftover_it_could_not_remove() {
    use std::os::unix::fs::PermissionsExt;

    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    sync(&env, &repo).success();
    let locked = env.instrukt_home().join("docs/acme/policy");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    repo.write("docs/domain/acme/policy/later.md", &policy("Later", ""));
    repo.commit("later");
    sync(&env, &repo).success();
    let leftover: Vec<PathBuf> = std::fs::read_dir(env.instrukt_home())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".sync-")
        })
        .collect();
    assert_eq!(leftover.len(), 1);

    let current = env
        .instrukt_home()
        .join("state/eval-fingerprint/current.yaml");
    let elsewhere = env.path("elsewhere.yaml");
    std::fs::rename(&current, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &current).unwrap();
    repo.write("docs/domain/acme/policy/again.md", &policy("Again", ""));
    repo.commit("again");

    let refused = sync(&env, &repo).failure();
    let stdout = text(&refused.get_output().stdout, &[])
        .replace(&leftover[0].display().to_string(), "[LEFTOVER]");
    insta::assert_snapshot!(stdout, @"leftover [LEFTOVER] could not be removed; a later sync removes it");
    let error: Vec<String> = without_tmp(&env, &refused.get_output().stderr, &[])
        .lines()
        .filter(|line| line.starts_with("instrukt:"))
        .map(str::to_owned)
        .collect();
    insta::assert_snapshot!(error.join("\n"), @"instrukt: [TMP]/home/.instrukt/state/eval-fingerprint/current.yaml is a symbolic link or lies behind one; publication writes only inside its root");
    assert!(leftover[0].exists());

    for dir in walkdir::WalkDir::new(&leftover[0])
        .into_iter()
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_dir())
    {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
