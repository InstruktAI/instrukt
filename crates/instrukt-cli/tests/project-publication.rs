//! `instrukt sync`
//! (`docs/local/spec/feature/publication/project-publication.md`).
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

/// A project layer declaring `slug` and publishing `domains`.
fn layer(slug: &str, domains: &[&str]) -> String {
    let mut text = format!("schema_version: 1\nproject: {slug}\n");
    if !domains.is_empty() {
        text.push_str("publishes:\n");
        for domain in domains {
            text.push_str(&format!(
                "  - slug: {domain}\n    trigger: When working on {domain}.\n"
            ));
        }
    }
    text
}

/// A repository `name` declaring project `name`, publishing `domains`, with
/// one committed policy per domain and one local spec.
fn project(env: &TestEnv, name: &str, domains: &[&str]) -> Repo {
    let repo = env.git_repo(name);
    repo.write("instrukt.yaml", &layer(name, domains));
    for domain in domains {
        repo.write(
            &format!("docs/global/{domain}/policy/rules.md"),
            &policy("Rules", ""),
        );
    }
    repo.write(
        "docs/local/policy/house.md",
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

// spec: local/projects/instrukt/spec/feature/publication/project-publication UC-PUB1
#[test]
fn a_committed_snippet_is_published_and_indexed_while_uncommitted_changes_stay_out() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let committed = policy("Testing", "");
    repo.write("docs/global/acme/policy/testing.md", &committed);
    let sha = repo.commit("add testing");
    repo.write("docs/global/acme/policy/testing.md", &policy("Edited", ""))
        .write("docs/global/acme/policy/draft.md", &policy("Draft", ""));

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
        .find(|e| e.id.as_str() == "local/projects/demo/policy/house")
        .unwrap();
    assert_eq!(
        (local.scope, local.path.clone()),
        (
            IndexScope::Local,
            repo.path().join("docs/local/policy/house.md")
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

// spec: local/projects/instrukt/spec/feature/publication/project-publication UC-PUB1
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
        linked.join("docs/global/acme/policy/feature.md"),
        policy("Feature", ""),
    )
    .unwrap();
    env.git(&linked, ["add", "--all"]);
    env.git(&linked, ["commit", "--quiet", "-m", "feature"]);

    let run = env.run(&linked, ["sync"]).success();
    insta::assert_snapshot!(text(&run.get_output().stdout, &[&main]), @"published demo at [SHA]");
    assert!(!ids(&index(&env)).contains(&"acme/policy/feature"));
}

// spec: local/projects/instrukt/spec/feature/publication/project-publication UC-PUB2
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
            "local/projects/alpha/policy/house",
            "local/projects/beta/policy/house",
        ]
    );
    let alpha_after: Vec<IndexEntry> = owned_by(&both, "alpha").into_iter().cloned().collect();
    assert_eq!(alpha_after, alpha_entries);

    let before = published_tree(&env);
    beta.write(
        "instrukt.yaml",
        &layer("beta", &["beta-docs", "alpha-docs"]),
    )
    .write(
        "docs/global/alpha-docs/policy/taken.md",
        &policy("Taken", ""),
    );
    let sha = beta.commit("claim alpha-docs");
    let refused = sync(&env, &beta).failure();
    insta::assert_snapshot!(
        text(&refused.get_output().stderr, &[&sha]),
        @"instrukt: beta at [SHA] is not published: domain `alpha-docs` is owned by project `alpha`; it is free to claim once `alpha` stops publishing it"
    );
    assert_eq!(published_tree(&env), before);
    let record = PublishedRecord::load(&home(&env), &ProjectSlug::parse("beta").unwrap())
        .unwrap()
        .unwrap();
    assert_ne!(record.commit.as_str(), sha);
}

// spec: local/projects/instrukt/spec/feature/publication/project-publication UC-PUB3
#[test]
fn a_deleted_snippet_and_a_dropped_domain_are_retracted_and_nothing_else() {
    let env = TestEnv::new();
    let other = project(&env, "other", &["shared"]);
    sync(&env, &other).success();
    let repo = project(&env, "demo", &["acme", "legacy"]);
    repo.write("docs/global/acme/policy/retired.md", &policy("Retired", ""));
    repo.commit("retired");
    sync(&env, &repo).success();
    let other_entries: Vec<IndexEntry> = owned_by(&index(&env), "other")
        .into_iter()
        .cloned()
        .collect();
    let other_tree: Vec<(PathBuf, Vec<u8>)> = published_tree(&env)
        .into_iter()
        .filter(|(path, _)| path.starts_with("shared"))
        .collect();

    std::fs::remove_file(repo.path().join("docs/global/acme/policy/retired.md")).unwrap();
    std::fs::remove_dir_all(repo.path().join("docs/global/legacy")).unwrap();
    repo.write("instrukt.yaml", &layer("demo", &["acme"]));
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
            "local/projects/demo/policy/house",
            "local/projects/other/policy/house",
            "shared/policy/rules",
        ]
    );
    let other_after: Vec<IndexEntry> = owned_by(&after, "other").into_iter().cloned().collect();
    assert_eq!(other_after, other_entries);
    let other_tree_after: Vec<(PathBuf, Vec<u8>)> = published_tree(&env)
        .into_iter()
        .filter(|(path, _)| path.starts_with("shared"))
        .collect();
    assert_eq!(other_tree_after, other_tree);
}

// Behaviors of the spec's canonical fields that no use case states: the
// no-op, `--force`, the validation refusal, failure and AGENTS.md
// retraction. Unbound: no use case realizes them.

#[test]
fn main_unchanged_since_its_publication_is_a_no_op_until_forced() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let sha = repo.head();
    sync(&env, &repo).success();
    let before = published_tree(&env);

    let again = sync(&env, &repo).success();
    insta::assert_snapshot!(text(&again.get_output().stdout, &[&sha]), @"demo is already published at [SHA]");
    assert_eq!(published_tree(&env), before);

    let forced = env.run(repo.path(), ["sync", "--force"]).success();
    insta::assert_snapshot!(text(&forced.get_output().stdout, &[&sha]), @"published demo at [SHA]");
    assert_eq!(published_tree(&env), before);
}

#[test]
fn committed_docs_with_findings_are_refused_and_the_previous_publication_stays() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let published = repo.head();
    sync(&env, &repo).success();
    let before = published_tree(&env);
    repo.write(
        "docs/global/acme/policy/bare.md",
        "---\nvisibility: public\n---\n\n# Bare — Policy\n\n## Rules\n\nx\n",
    );
    let sha = repo.commit("broken");

    let refused = sync(&env, &repo).failure();
    insta::assert_snapshot!(text(&refused.get_output().stdout, &[&sha]), @r"
    docs/global/acme/policy/bare.md: frontmatter-description-missing
    docs/global/acme/policy/bare.md: structure-section-missing: Enforcement
    docs/global/acme/policy/bare.md: structure-section-missing: Exceptions
    docs/global/acme/policy/bare.md: structure-section-missing: Rationale
    docs/global/acme/policy/bare.md: structure-section-missing: Scope
    ");
    insta::assert_snapshot!(text(&refused.get_output().stderr, &[&sha]), @"instrukt: demo at [SHA] is not published: the committed documentation has 5 validation finding(s)");
    assert_eq!(published_tree(&env), before);
    let record = PublishedRecord::load(&home(&env), &ProjectSlug::parse("demo").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.commit.as_str(), published);
}

#[test]
fn a_failing_publish_time_command_fails_the_sync_and_keeps_the_previous_publication() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    let published = repo.head();
    sync(&env, &repo).success();
    let before = published_tree(&env);
    repo.write("docs/global/acme/policy/later.md", &policy("Later", ""))
        .write("AGENTS.master.md", "## Purpose\n\n<!-- @exec: exit 3 -->\n");
    repo.commit("failing command");

    let failed = sync(&env, &repo).failure();
    insta::assert_snapshot!(text(&failed.get_output().stderr, &[]), @"instrukt: cannot compile AGENTS.master.md: the read-time command `exit 3` failed: exit status 3");
    assert_eq!(published_tree(&env), before);
    assert!(!repo.path().join("AGENTS.md").exists());
    let record = PublishedRecord::load(&home(&env), &ProjectSlug::parse("demo").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.commit.as_str(), published);
}

#[test]
fn an_agents_md_whose_master_is_gone_is_removed() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &[]);
    repo.write("tools/AGENTS.master.md", "## Tools\n\nOnly the tools.\n");
    repo.commit("master");
    sync(&env, &repo).success();
    assert!(repo.path().join("tools/AGENTS.md").is_file());

    std::fs::remove_file(repo.path().join("tools/AGENTS.master.md")).unwrap();
    repo.commit("drop master");
    sync(&env, &repo).success();
    assert!(!repo.path().join("tools/AGENTS.md").exists());
}
