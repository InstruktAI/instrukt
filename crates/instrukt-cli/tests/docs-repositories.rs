//! Docs repositories: a computer's doctrine comes from adopted projects
//! that publish domains
//! (`docs/project/spec/feature/publication/docs-repositories.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::repository::{Hook, HookRunRecord, SyncOutcome};

/// A computer whose `PATH` holds the built `instrukt`, as an installed one
/// would: an adopted repository's hook sections and docs filter call it by
/// name.
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

/// A policy snippet with every required section.
fn policy(title: &str, rule: &str) -> String {
    format!(
        "---\ndescription: The {title} policy.\n---\n\n# {title} — Policy\n\n\
         ## Rules\n\n{rule}\n\n## Rationale\n\nBecause.\n\n## Scope\n\nAll work.\n\n\
         ## Enforcement\n\nReview.\n\n## Exceptions\n\nNone.\n"
    )
}

/// A project layer declaring `slug`, with `extra` lines.
fn layer(slug: &str, extra: &str) -> String {
    format!("schema_version: 1\nproject: {slug}\n{extra}")
}

/// A docs repository `name` on its remote, publishing `general` and
/// `sdlc` with a global master and a baseline per domain; `rule` is the
/// wording of its sdlc policy.
fn docs_repository(env: &TestEnv, name: &str, domains: &[&str], rule: &str) -> std::path::PathBuf {
    let remote = env.bare_remote(name);
    let author = env.git_repo(&format!("{name}-author"));
    author.write("instrukt.yaml", &layer(name, ""));
    for domain in domains {
        author
            .write(
                &format!("{}/policy/rules.md", common::position_dir(domain)),
                &policy(&format!("{domain} rules"), rule),
            )
            .write(
                &format!("{}/baseline.md", common::position_dir(domain)),
                &format!("@~/.instrukt/docs/{domain}/policy/rules.md\n"),
            );
    }
    if domains.contains(&"general") {
        author.write(
            "agents/AGENTS.global.md",
            "# Global baseline\n\n## Required reads\n\n@~/.instrukt/docs/general/bundle.md\n\n\
             ## Working here\n\nFollow the doctrine.\n",
        );
    }
    author.commit("doctrine");
    author.git(["remote", "add", "origin", remote.to_str().unwrap()]);
    author.git(["push", "--quiet", "origin", "main"]);
    remote
}

/// Clones the docs repository at `remote` as `name`, adopts it and runs its
/// publication.
fn adopt(env: &TestEnv, remote: &Path, name: &str) -> (Repo, Assert) {
    let clone = env.clone_repo(remote, name);
    env.run(clone.path(), ["init"]).success();
    let synced = env.run(clone.path(), ["sync"]);
    (clone, synced)
}

/// A project that is not a docs repository: its master requires the sdlc
/// bundle.
fn consumer(env: &TestEnv) -> Repo {
    let app = env.git_repo("app");
    app.write("instrukt.yaml", &layer("app", "")).write(
        "AGENTS.master.md",
        "## Purpose\n\nThe app.\n\n## Required reads\n\n@~/.instrukt/docs/sdlc/bundle.md\n",
    );
    app.commit("app");
    app
}

/// Waits, within a bound, until the detached sync of `hook`'s last run in
/// `project` has ended.
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

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR1
#[test]
fn adopting_a_docs_repository_publishes_its_domains() {
    let env = machine();
    let remote = docs_repository(&env, "doctrine", &["general", "sdlc"], "Test first.");
    let (_, synced) = adopt(&env, &remote, "doctrine");
    synced.success();

    let app = consumer(&env);
    let domains = env.run(app.path(), ["docs", "index"]).success();
    insta::assert_snapshot!("published-domains", stdout(&env, &domains));
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR1
#[test]
fn every_installed_agent_starts_with_the_doctrines_global_baseline() {
    let env = machine();
    for dir in [".claude", ".codex"] {
        std::fs::create_dir_all(env.home().join(dir)).unwrap();
    }
    let remote = docs_repository(&env, "doctrine", &["general", "sdlc"], "Test first.");
    adopt(&env, &remote, "doctrine").1.success();

    let claude = std::fs::read_to_string(env.home().join(".claude/CLAUDE.md")).unwrap();
    insta::assert_snapshot!("global-baseline", claude);
    let codex = std::fs::read_to_string(env.home().join(".codex/AGENTS.md")).unwrap();
    assert_eq!(
        codex, claude,
        "without addenda every agent reads the same baseline"
    );
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR1
#[test]
fn another_project_reads_the_adopted_doctrine() {
    let env = machine();
    let remote = docs_repository(&env, "doctrine", &["general", "sdlc"], "Test first.");
    adopt(&env, &remote, "doctrine").1.success();
    let app = consumer(&env);

    env.run(app.path(), ["sync"]).success();
    insta::assert_snapshot!("consumer-instructions", app.read("AGENTS.md"));
    let read = env
        .run(app.path(), ["docs", "get", "sdlc/policy/rules"])
        .success();
    insta::assert_snapshot!("consumer-reads-doctrine", stdout(&env, &read));
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR2
#[test]
fn a_second_docs_repository_claiming_a_published_domain_is_refused_naming_the_owner() {
    let env = machine();
    let doctrine = docs_repository(&env, "doctrine", &["general", "sdlc"], "Test first.");
    adopt(&env, &doctrine, "doctrine").1.success();
    let own = docs_repository(&env, "mydocs", &["sdlc"], "Ship daily.");

    let (mine, refused) = adopt(&env, &own, "mydocs");
    let refused = refused.failure();
    insta::assert_snapshot!(
        "claim-refused",
        stderr(&env, &refused).replace(&mine.head(), "[SHA]")
    );
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR2
#[test]
fn after_the_first_owner_retracts_a_domain_projects_read_the_new_owners_version() {
    let env = machine();
    let doctrine = docs_repository(&env, "doctrine", &["general", "sdlc"], "Test first.");
    let (first, synced) = adopt(&env, &doctrine, "doctrine");
    synced.success();
    let own = docs_repository(&env, "mydocs", &["sdlc"], "Ship daily.");
    let (mine, refused) = adopt(&env, &own, "mydocs");
    refused.failure();

    first.write("instrukt.yaml", &layer("doctrine", ""));
    std::fs::remove_dir_all(first.path().join("docs/domain/sdlc")).unwrap();
    // Only the retirement is committed; what init added stays uncommitted.
    first.git(["add", "--all", "instrukt.yaml", "docs"]);
    first.git(["commit", "--quiet", "-m", "stop publishing sdlc"]);
    let retracted = wait_for_run(&env, "doctrine", Hook::PostCommit);
    assert!(
        matches!(
            retracted.sync,
            Some(SyncOutcome::Finished {
                exit_code: Some(0),
                ..
            })
        ),
        "{:?}",
        retracted.sync
    );
    env.run(mine.path(), ["sync"]).success();

    let app = consumer(&env);
    let read = env
        .run(app.path(), ["docs", "get", "sdlc/policy/rules"])
        .success();
    insta::assert_snapshot!("consumer-reads-own-doctrine", stdout(&env, &read));
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR3
#[test]
fn publishing_a_project_whose_master_needs_an_unpublished_domain_names_it() {
    let env = machine();
    let app = consumer(&env);

    let refused = env.run(app.path(), ["sync"]).failure();
    insta::assert_snapshot!(stderr(&env, &refused), @"instrukt: domain `sdlc` is not published on this computer, but AGENTS.master.md needs it; adopt a docs repository that publishes it");
}

/// A project with `todos` on whose `workflow.domain` names `flows`, which
/// no repository on the computer publishes.
fn workflow_consumer(env: &TestEnv) -> Repo {
    let app = env.git_repo("app");
    app.write(
        "instrukt.yaml",
        &layer("app", "modules: {todos: true}\nworkflow: {domain: flows}\n"),
    )
    .write("docs/project/policy/house.md", &policy("House", "Be kind."));
    app.commit("app");
    app
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR3
#[test]
fn validating_a_project_whose_workflow_domain_is_unpublished_names_it() {
    let env = machine();
    let app = workflow_consumer(&env);

    let refused = env.run(app.path(), ["docs", "validate"]).failure();
    insta::assert_snapshot!(stderr(&env, &refused), @"instrukt: domain `flows` is not published on this computer, but `workflow.domain` needs it; adopt a docs repository that publishes it");
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR3
#[test]
fn publishing_a_project_whose_workflow_domain_is_unpublished_names_it() {
    let env = machine();
    let app = workflow_consumer(&env);

    let refused = env.run(app.path(), ["sync"]).failure();
    insta::assert_snapshot!(stderr(&env, &refused), @"instrukt: domain `flows` is not published on this computer, but `workflow.domain` needs it; adopt a docs repository that publishes it");
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR3
#[test]
fn publishing_reads_the_todos_switch_from_committed_main_not_the_working_tree() {
    let env = machine();
    let app = workflow_consumer(&env);
    app.write(
        "instrukt.yaml",
        &layer("app", "workflow: {domain: flows}\n"),
    );

    let refused = env.run(app.path(), ["sync"]).failure();
    insta::assert_snapshot!(stderr(&env, &refused), @"instrukt: domain `flows` is not published on this computer, but `workflow.domain` needs it; adopt a docs repository that publishes it");
}

/// A project whose local concept requires a snippet of `flows`, which no
/// repository on the computer publishes.
fn dependent_consumer(env: &TestEnv) -> Repo {
    let app = env.git_repo("app");
    app.write("instrukt.yaml", &layer("app", "")).write(
        "docs/project/concept/flow.md",
        "---\ndescription: How work flows here.\n---\n\n# Flow — Concept\n\n\
         ## Required reads\n\n- flows/policy/rules\n\n\
         ## What\n\nOur flow.\n\n## Why\n\nTo ship.\n",
    );
    app.commit("app");
    app
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR3
#[test]
fn validating_a_snippet_that_requires_an_unpublished_domain_names_it() {
    let env = machine();
    let app = dependent_consumer(&env);

    let refused = env.run(app.path(), ["docs", "validate"]).failure();
    insta::assert_snapshot!(stdout(&env, &refused), @"docs/project/concept/flow.md: reference-domain-unpublished: flows/policy/rules: domain `flows` is not published on this computer; adopt a docs repository that publishes it");
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR3
#[test]
fn publishing_a_snippet_that_requires_an_unpublished_domain_names_it() {
    let env = machine();
    let app = dependent_consumer(&env);

    let refused = env.run(app.path(), ["sync"]).failure();
    insta::assert_snapshot!(stdout(&env, &refused), @"docs/project/concept/flow.md: reference-domain-unpublished: flows/policy/rules: domain `flows` is not published on this computer; adopt a docs repository that publishes it");
}
