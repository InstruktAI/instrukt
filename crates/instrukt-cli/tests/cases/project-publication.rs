//! `instrukt sync`
//! (`docs/project/spec/feature/publication/project-publication.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::index::{GlobalIndex, IndexEntry, IndexScope};
use instrukt_core::publication::{EvalFingerprint, GENERATED_LINE, PublishedRecord};
use instrukt_core::repository::Hook;

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
    insta::assert_snapshot!(text(&run.get_output().stdout, &[&sha]), @"
    published demo at [SHA]
    notice: docs/domain/acme/: domain-without-index: `acme` has no index.yaml: it is absent from the global agent files and found only through `instrukt docs index --domain acme`
    .instrukt/schemas/general-index.schema.json: schema written
    .instrukt/schemas/domain-index.schema.json: schema written
    .instrukt/schemas/project-index.schema.json: schema written
    .instrukt/schemas/company-index.schema.json: schema written
    .instrukt/schemas/snippet-schema.schema.json: schema written
    .instrukt/schemas/instrukt.schema.json: schema written
    ");

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
            PathBuf::from("project/demo/policy/house.md")
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
    insta::assert_snapshot!(text(&run.get_output().stdout, &[&main]), @"
    published demo at [SHA]
    notice: docs/domain/acme/: domain-without-index: `acme` has no index.yaml: it is absent from the global agent files and found only through `instrukt docs index --domain acme`
    .instrukt/schemas/general-index.schema.json: schema written
    .instrukt/schemas/domain-index.schema.json: schema written
    .instrukt/schemas/project-index.schema.json: schema written
    .instrukt/schemas/company-index.schema.json: schema written
    .instrukt/schemas/snippet-schema.schema.json: schema written
    .instrukt/schemas/instrukt.schema.json: schema written
    ");
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
    let env = hooked_machine();
    let other = project(&env, "other", &["shared"]);
    sync(&env, &other).success();
    crate::fixtures::t3::FakeT3::create(&env);
    common::t3_readiness::configure(
        &env,
        vec![common::t3_readiness::provider(
            "claudeAgent",
            "claudeAgent",
            true,
        )],
        serde_json::json!({}),
    );
    let repo = project(&env, "demo", &["acme", "legacy", "general"]);
    env.run(repo.path(), ["hooks", "install"]).success();
    repo.write("docs/domain/acme/policy/retired.md", &policy("Retired", ""))
        .write(
            "docs/project/index.yaml",
            "expanded:\n  - project/demo/policy/house\n",
        );
    repo.commit("retired");
    env.finished_hook_run(repo.path(), Hook::PostCommit);
    sync(&env, &repo).success();
    assert!(repo.read("AGENTS.md").starts_with(GENERATED_LINE));
    let kept_instructions = repo.read("AGENTS.md");
    let global_file = env.home().join(".claude/CLAUDE.md");
    let global_text = std::fs::read(&global_file).unwrap();
    let other_entries: Vec<IndexEntry> = owned_by(&index(&env), "other")
        .into_iter()
        .cloned()
        .collect();
    let other_tree: Vec<(PathBuf, Vec<u8>)> = published_tree(&env)
        .into_iter()
        .filter(|(path, _)| path.starts_with("shared"))
        .collect();

    // The commit retiring the snippet and the domains leaves the root
    // rendering as it is.
    std::fs::remove_file(repo.path().join("docs/domain/acme/policy/retired.md")).unwrap();
    std::fs::remove_dir_all(repo.path().join("docs/domain/legacy")).unwrap();
    std::fs::remove_dir_all(repo.path().join("docs/general")).unwrap();
    repo.write("instrukt.yaml", &layer("demo"));
    repo.commit("retire");
    env.finished_hook_run(repo.path(), Hook::PostCommit);
    assert_eq!(repo.read("AGENTS.md"), kept_instructions);

    // The record an earlier release kept of the files it wrote, which no
    // longer parses, beside the files it names.
    let record = env
        .instrukt_home()
        .join("state/publication/outputs/demo.yaml");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        "agents:\n- AGENTS.md\n- tools/AGENTS.md\nentrypoints:\n- path: .claude/CLAUDE.md\n  digest: sha256:00\n: not yaml [\n",
    )
    .unwrap();
    repo.write("tools/AGENTS.md", "# The developer's tools notes\n");
    let named = [
        repo.path().join("AGENTS.md"),
        repo.path().join("tools/AGENTS.md"),
        global_file.clone(),
    ];
    let named_before: Vec<Vec<u8>> = named
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    sync(&env, &repo).success();

    let docs = env.instrukt_home().join("docs");
    assert!(!docs.join("acme/policy/retired.md").exists());
    assert!(!docs.join("legacy").exists());
    assert!(!docs.join("general").exists());
    assert!(!record.exists(), "the earlier record is deleted");
    let named_after: Vec<Vec<u8>> = named
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    assert_eq!(
        named_after, named_before,
        "nothing the record names is touched"
    );
    assert_eq!(
        std::fs::read(&global_file).unwrap(),
        global_text,
        "a global file is never removed"
    );
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
    let other_after: Vec<IndexEntry> = owned_by(&after, "other").into_iter().cloned().collect();
    assert_eq!(other_after, other_entries);
    let other_tree_after: Vec<(PathBuf, Vec<u8>)> = published_tree(&env)
        .into_iter()
        .filter(|(path, _)| path.starts_with("shared"))
        .collect();
    assert_eq!(other_tree_after, other_tree);
}

/// A computer whose `PATH` holds the built `instrukt`, as an installed one
/// would: a repository's hook sections call it by name.
fn hooked_machine() -> TestEnv {
    let mut env = TestEnv::new();
    let bin = Path::new(env!("CARGO_BIN_EXE_instrukt")).parent().unwrap();
    let path = format!("{}:{}", bin.display(), env.injected("PATH").unwrap());
    env.set_env("PATH", &path);
    env
}

/// Every durable output of publication, by path, with its content and
/// modification time: in the home the published tree, the global index, the
/// claims and published records, both eval-fingerprint records; in `repo`
/// what its sync writes there, the editor schemas.
fn durable_outputs(
    env: &TestEnv,
    repo: &Repo,
) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    let state = env.instrukt_home().join("state");
    let roots = [
        env.instrukt_home().join("docs"),
        state.join("index.yaml"),
        state.join("publication"),
        state.join("eval-fingerprint"),
        repo.path().join(".instrukt/schemas"),
    ];
    let mut out = BTreeMap::new();
    for root in roots {
        assert!(root.exists(), "{} is published", root.display());
        for entry in walkdir::WalkDir::new(&root) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let meta = entry.metadata().unwrap();
                out.insert(
                    entry.path().to_path_buf(),
                    (
                        std::fs::read(entry.path()).unwrap(),
                        meta.modified().unwrap(),
                    ),
                );
            }
        }
    }
    out
}

/// Both eval-fingerprint records as `project`'s published commit has them.
fn fingerprints(env: &TestEnv, project: &str) -> (EvalFingerprint, EvalFingerprint) {
    let commit = PublishedRecord::load(&home(env), &ProjectSlug::parse(project).unwrap())
        .unwrap()
        .unwrap()
        .commit;
    (
        EvalFingerprint::for_commit(&home(env), &commit)
            .unwrap()
            .unwrap(),
        EvalFingerprint::current(&home(env)).unwrap().unwrap(),
    )
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB4
#[test]
fn publishing_again_restores_what_is_missing_without_a_new_commit() {
    let mut env = TestEnv::new();
    let other = project(&env, "other", &["ops"]);
    other
        .write(
            "docs/domain/ops/policy/rules.md",
            &policy("Rules", "eval: true\n"),
        )
        .commit("evaluated");
    let repo = project(&env, "demo", &["acme"]);
    repo.write("README.md", "# The demo project\n");
    let sha = repo.commit("readme");
    let t1: jiff::Timestamp = "2030-01-01T08:00:00Z".parse().unwrap();
    env.set_env("INSTRUKT_NOW", &t1.to_string());
    sync(&env, &other).success();
    sync(&env, &repo).success();
    let after_t1 = durable_outputs(&env, &repo);
    let (per_commit_t1, current_t1) = fingerprints(&env, "demo");
    assert_eq!(per_commit_t1.recorded_at, Some(t1));
    assert_eq!(current_t1.recorded_at, Some(t1));

    // A second sync of the same main later writes nothing.
    env.set_env("INSTRUKT_NOW", "2030-01-02T08:00:00Z");
    let again = sync(&env, &repo).success();
    insta::assert_snapshot!(text(&again.get_output().stdout, &[&sha]), @r"
    published demo at [SHA], nothing to change
    notice: docs/domain/acme/: domain-without-index: `acme` has no index.yaml: it is absent from the global agent files and found only through `instrukt docs index --domain acme`
    ");
    assert_eq!(durable_outputs(&env, &repo), after_t1);
    assert_eq!(
        fingerprints(&env, "demo"),
        (per_commit_t1.clone(), current_t1)
    );

    // A published snippet deleted by hand is restored, and nothing else
    // changes.
    let deleted = env.instrukt_home().join("docs/acme/policy/rules.md");
    std::fs::remove_file(&deleted).unwrap();
    env.set_env("INSTRUKT_NOW", "2030-01-03T08:00:00Z");
    sync(&env, &repo).success();
    let after_t3 = durable_outputs(&env, &repo);
    assert_eq!(
        after_t3.get(&deleted).map(|(bytes, _)| bytes),
        after_t1.get(&deleted).map(|(bytes, _)| bytes)
    );
    let without = |outputs: &BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>| {
        let mut outputs = outputs.clone();
        outputs.remove(&deleted);
        outputs
    };
    assert_eq!(without(&after_t3), without(&after_t1));

    // `--force` is gone.
    let unknown = env.run(repo.path(), ["sync", "--force"]).failure();
    insta::assert_snapshot!(text(&unknown.get_output().stderr, &[]), @r"
    error: unexpected argument '--force' found

    Usage: instrukt sync

    For more information, try '--help'.
    ");

    // Another owner changes an evaluated snippet; the unchanged main's next
    // sync records the new fingerprint for its commit and leaves the
    // current record as that owner wrote it.
    other
        .write(
            "docs/domain/ops/policy/rules.md",
            &policy("Rules", "eval: true\n").replace("Follow Rules.", "Follow the rules."),
        )
        .commit("evaluated, changed");
    let t4: jiff::Timestamp = "2030-01-04T08:00:00Z".parse().unwrap();
    env.set_env("INSTRUKT_NOW", &t4.to_string());
    sync(&env, &other).success();
    let current_path = env
        .instrukt_home()
        .join("state/eval-fingerprint/current.yaml");
    let current_t4_bytes = std::fs::read(&current_path).unwrap();
    let current_t4 = EvalFingerprint::current(&home(&env)).unwrap().unwrap();
    assert_eq!(current_t4.recorded_at, Some(t4));
    let t5: jiff::Timestamp = "2030-01-05T08:00:00Z".parse().unwrap();
    env.set_env("INSTRUKT_NOW", &t5.to_string());
    sync(&env, &repo).success();
    let (per_commit_t5, current_t5) = fingerprints(&env, "demo");
    assert_ne!(per_commit_t5.fingerprint, per_commit_t1.fingerprint);
    assert_eq!(per_commit_t5.fingerprint, current_t4.fingerprint);
    assert_eq!(per_commit_t5.recorded_at, Some(t5));
    assert_eq!(current_t5, current_t4);
    assert_eq!(std::fs::read(&current_path).unwrap(), current_t4_bytes);
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
    docs/domain/acme/policy/bare.md: structure-section-missing: Enforcement, required by instrukt root rules
    docs/domain/acme/policy/bare.md: structure-section-missing: Exceptions, required by instrukt root rules
    docs/domain/acme/policy/bare.md: structure-section-missing: Rationale, required by instrukt root rules
    docs/domain/acme/policy/bare.md: structure-section-missing: Scope, required by instrukt root rules
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
    crate::fixtures::t3::FakeT3::create(&env);
    common::t3_readiness::configure(
        &env,
        vec![common::t3_readiness::provider(
            "claudeAgent",
            "claudeAgent",
            true,
        )],
        serde_json::json!({}),
    );
    let repo = project(&env, "demo", &["acme"]);
    let published = repo.head();
    sync(&env, &repo).success();
    let before = published_tree(&env);
    // A snippet the global agent files hold in full reads a value the
    // configuration does not hold.
    repo.write("docs/domain/acme/policy/later.md", &policy("Later", ""))
        .write(
            "docs/general/policy/house.md",
            &policy("House", "").replace("Follow House.", "<!-- @config: no.such.setting -->"),
        )
        .write(
            "docs/general/index.yaml",
            "expanded:\n  - general/policy/house\n",
        );
    repo.commit("unresolvable value");

    let failed = sync(&env, &repo).failure();
    insta::assert_snapshot!(text(&failed.get_output().stderr, &[]));
    assert_eq!(published_tree(&env), before);
    assert!(!env.home().join(".claude/CLAUDE.md").exists());
    assert_eq!(published_commit(&env, "demo"), published);
}

/// Emitted text with the test's temporary directory shown as `[TMP]`.
fn without_tmp(env: &TestEnv, bytes: &[u8], shas: &[&str]) -> String {
    let base = env.path("").display().to_string();
    text(bytes, shas).replace(base.trim_end_matches('/'), "[TMP]")
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB7
#[cfg(unix)]
#[test]
fn an_output_that_is_a_symbolic_link_is_replaced_and_its_target_left_alone() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    sync(&env, &repo).success();
    let schemas = repo.path().join(".instrukt/schemas");
    let elsewhere = env.path("schemas-elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("mine.json"), "{}\n").unwrap();
    std::fs::remove_dir_all(&schemas).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &schemas).unwrap();
    repo.write("docs/domain/acme/policy/later.md", &policy("Later", ""));
    let sha = repo.commit("later");

    let synced = sync(&env, &repo).success();
    insta::assert_snapshot!(without_tmp(&env, &synced.get_output().stdout, &[&sha]), @r"
    published demo at [SHA]
    notice: docs/domain/acme/: domain-without-index: `acme` has no index.yaml: it is absent from the global agent files and found only through `instrukt docs index --domain acme`
    .instrukt/schemas: a symbolic link, replaced; what it pointed to is left as it is
    .instrukt/schemas/general-index.schema.json: schema written
    .instrukt/schemas/domain-index.schema.json: schema written
    .instrukt/schemas/project-index.schema.json: schema written
    .instrukt/schemas/company-index.schema.json: schema written
    .instrukt/schemas/snippet-schema.schema.json: schema written
    .instrukt/schemas/instrukt.schema.json: schema written
    ");
    assert!(
        std::fs::symlink_metadata(&schemas).unwrap().is_dir(),
        "the link is replaced by the directory"
    );
    let left: Vec<_> = std::fs::read_dir(&elsewhere)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, ["mine.json"]);
    assert!(
        env.instrukt_home()
            .join("docs/acme/policy/later.md")
            .is_file()
    );
    assert_eq!(published_commit(&env, "demo"), sha);
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
    notice: docs/domain/acme/: domain-without-index: `acme` has no index.yaml: it is absent from the global agent files and found only through `instrukt docs index --domain acme`
    leftover [LEFTOVER] could not be removed; a later sync removes it
    ");
    assert_eq!(published_commit(&env, "demo"), sha);
    assert!(
        env.instrukt_home()
            .join("docs/acme/policy/later.md")
            .is_file()
    );

    // Still not removable: an unchanged main writes nothing and reports it.
    let still = sync(&env, &repo).success();
    let stdout = text(&still.get_output().stdout, &[&sha])
        .replace(&left[0].display().to_string(), "[LEFTOVER]");
    insta::assert_snapshot!(stdout, @r"
    published demo at [SHA], nothing to change
    notice: docs/domain/acme/: domain-without-index: `acme` has no index.yaml: it is absent from the global agent files and found only through `instrukt docs index --domain acme`
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
    insta::assert_snapshot!(text(&removed.get_output().stdout, &[&sha]), @r"
    published demo at [SHA], nothing to change
    notice: docs/domain/acme/: domain-without-index: `acme` has no index.yaml: it is absent from the global agent files and found only through `instrukt docs index --domain acme`
    ");
    assert!(leftovers().is_empty(), "the next sync removes the leftover");
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB7
#[cfg(unix)]
#[test]
fn a_linked_later_output_is_replaced_by_the_file() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &["acme"]);
    repo.write("README.md", "# The demo project\n");
    repo.commit("readme");
    sync(&env, &repo).success();
    let current = env
        .instrukt_home()
        .join("state/eval-fingerprint/current.yaml");
    let elsewhere = env.path("elsewhere.yaml");
    std::fs::rename(&current, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &current).unwrap();
    let elsewhere_text = std::fs::read_to_string(&elsewhere).unwrap();
    repo.write("docs/domain/acme/policy/later.md", &policy("Later", ""))
        .write("README.md", "# The changed demo project\n");
    let sha = repo.commit("later");

    let synced = sync(&env, &repo).success();
    insta::assert_snapshot!(without_tmp(&env, &synced.get_output().stdout, &[&sha]), @r"
    published demo at [SHA]
    notice: docs/domain/acme/: domain-without-index: `acme` has no index.yaml: it is absent from the global agent files and found only through `instrukt docs index --domain acme`
    [TMP]/home/.instrukt/state/eval-fingerprint/current.yaml: a symbolic link, replaced; what it pointed to is left as it is
    ");
    assert!(
        std::fs::symlink_metadata(&current)
            .unwrap()
            .file_type()
            .is_file()
    );
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), elsewhere_text);
    assert!(
        env.instrukt_home()
            .join("docs/acme/policy/later.md")
            .is_file()
    );
    assert_eq!(published_commit(&env, "demo"), sha);
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

    repo.write(
        "docs/domain/acme/policy/bare.md",
        "---\nvisibility: public\n---\n\n# Bare — Policy\n\n## Rules\n\nx\n",
    );
    let sha = repo.commit("broken");

    let refused = sync(&env, &repo).failure();
    let stdout = text(&refused.get_output().stdout, &[])
        .replace(&leftover[0].display().to_string(), "[LEFTOVER]");
    insta::assert_snapshot!(stdout, @r"
    docs/domain/acme/policy/bare.md: frontmatter-description-missing
    docs/domain/acme/policy/bare.md: structure-section-missing: Enforcement, required by instrukt root rules
    docs/domain/acme/policy/bare.md: structure-section-missing: Exceptions, required by instrukt root rules
    docs/domain/acme/policy/bare.md: structure-section-missing: Rationale, required by instrukt root rules
    docs/domain/acme/policy/bare.md: structure-section-missing: Scope, required by instrukt root rules
    leftover [LEFTOVER] could not be removed; a later sync removes it
    ");
    let error: Vec<String> = without_tmp(&env, &refused.get_output().stderr, &[&sha])
        .lines()
        .filter(|line| line.starts_with("instrukt:"))
        .map(str::to_owned)
        .collect();
    insta::assert_snapshot!(error.join("\n"), @"instrukt: demo at [SHA] is not published: the committed documentation has 5 validation finding(s)");
    assert!(leftover[0].exists());

    for dir in walkdir::WalkDir::new(&leftover[0])
        .into_iter()
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_dir())
    {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB1
#[test]
fn project_positions_publish_committed_files_and_pending_companions() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &[]);
    let current = policy("House", "");
    let companion = policy("Next", "");
    repo.write("docs/project/policy/house.md", &current)
        .write("docs/project/policy/house.rework.md", &companion)
        .write("docs/project/index.yaml", "expanded: []\n")
        .write("docs/project/schema.yaml", "{}\n");
    repo.commit("project position");
    repo.write("docs/project/policy/house.md", &policy("Uncommitted", ""))
        .write("docs/project/policy/draft.md", &policy("Draft", ""));
    sync(&env, &repo).success();
    let root = env.instrukt_home().join("docs/project/demo");
    assert_eq!(
        std::fs::read_to_string(root.join("policy/house.md")).unwrap(),
        current
    );
    assert_eq!(
        std::fs::read_to_string(root.join("policy/house.rework.md")).unwrap(),
        companion
    );
    for file in ["index.yaml", "schema.yaml"] {
        let committed = env
            .git_command(repo.path())
            .args(["show", &format!("HEAD:docs/project/{file}")])
            .assert()
            .success();
        assert_eq!(
            std::fs::read(root.join(file)).unwrap(),
            committed.get_output().stdout
        );
    }
    assert!(!root.join("policy/draft.md").exists());
    assert!(
        env.instrukt_home()
            .join("state/publication/claims/project/demo.yaml")
            .is_file()
    );
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB3
#[test]
fn retiring_the_project_position_retracts_only_its_own_store_and_claim() {
    let env = TestEnv::new();
    let first = project(&env, "first", &[]);
    let second = project(&env, "second", &[]);
    sync(&env, &first).success();
    sync(&env, &second).success();
    let other = env
        .instrukt_home()
        .join("docs/project/second/policy/house.md");
    let before = std::fs::read(&other).unwrap();
    std::fs::remove_dir_all(first.path().join("docs/project")).unwrap();
    first.commit("retire project docs");
    sync(&env, &first).success();
    assert!(!env.instrukt_home().join("docs/project/first").exists());
    assert!(
        !env.instrukt_home()
            .join("state/publication/claims/project/first.yaml")
            .exists()
    );
    assert_eq!(std::fs::read(other).unwrap(), before);
    let retired = env
        .run(first.path(), ["docs", "index", "--project", "first"])
        .success();
    let remaining = env
        .run(second.path(), ["docs", "index", "--project", "second"])
        .success();
    insta::assert_snapshot!(
        "project-retraction-indexes",
        common::mask_published_commits(&format!(
            "{}{}",
            without_tmp(&env, &retired.get_output().stdout, &[]),
            without_tmp(&env, &remaining.get_output().stdout, &[])
        ))
    );
}

// spec: project/instrukt/spec/feature/publication/project-publication UC-PUB7
#[cfg(unix)]
#[test]
fn a_linked_project_store_is_replaced_without_writing_through_it() {
    let env = TestEnv::new();
    let repo = project(&env, "demo", &[]);
    sync(&env, &repo).success();
    let store = env.instrukt_home().join("docs/project/demo");
    let outside = env.path("project-docs-elsewhere");
    std::fs::rename(&store, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &store).unwrap();
    let previous = std::fs::read(outside.join("policy/house.md")).unwrap();
    repo.write("docs/project/policy/house.md", &policy("Changed", ""));
    let sha = repo.commit("change project docs");
    let synced = sync(&env, &repo).success();
    insta::assert_snapshot!(without_tmp(&env, &synced.get_output().stdout, &[&sha]), @r"
    published demo at [SHA]
    [TMP]/home/.instrukt/docs/project/demo: a symbolic link, replaced; what it pointed to is left as it is
    ");
    assert!(
        std::fs::symlink_metadata(&store)
            .unwrap()
            .file_type()
            .is_dir()
    );
    assert_eq!(
        std::fs::read(outside.join("policy/house.md")).unwrap(),
        previous
    );
    assert_eq!(
        std::fs::read(store.join("policy/house.md")).unwrap(),
        repo.read("docs/project/policy/house.md").into_bytes()
    );
    assert_eq!(published_commit(&env, "demo"), sha);
}
