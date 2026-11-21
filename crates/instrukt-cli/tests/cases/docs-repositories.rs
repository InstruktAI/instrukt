//! Docs repositories: a computer's doctrine comes from adopted projects
//! that publish domains
//! (`docs/project/spec/feature/publication/docs-repositories.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use std::path::Path;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::repository::{Hook, SyncOutcome};

/// A computer whose `PATH` holds the built `instrukt`, as an installed one
/// would: an adopted repository's hook sections and docs filter call it by
/// name.
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
    format!("schema_version: 6\nproject: {slug}\n{extra}")
}

/// A docs repository `name` on its remote, publishing each of `domains`
/// with an index that expands its one policy; `rule` is the wording of
/// those policies.
fn docs_repository(env: &TestEnv, name: &str, domains: &[&str], rule: &str) -> std::path::PathBuf {
    let remote = env.bare_remote(name);
    let author = env.git_repo(&format!("{name}-author"));
    author.write("instrukt.yaml", &layer(name, ""));
    for domain in domains {
        let description = if *domain == "general" {
            String::new()
        } else {
            format!("description: The {domain} doctrine\n")
        };
        author
            .write(
                &format!("{}/policy/rules.md", common::position_dir(domain)),
                &policy(&format!("{domain} rules"), rule),
            )
            .write(
                &format!("{}/index.yaml", common::position_dir(domain)),
                &format!("{description}expanded:\n  - {domain}/policy/rules\n"),
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

/// A project that is not a docs repository: its index expands a concept
/// of its own that requires the sdlc domain.
fn consumer(env: &TestEnv) -> Repo {
    let app = env.git_repo("app");
    app.write("instrukt.yaml", &layer("app", ""))
        .write("AGENTS.master.md", "## Purpose\n\nThe app.\n")
        .write(
            "docs/project/index.yaml",
            "expanded:\n  - project/app/concept/way\n",
        )
        .write(
            "docs/project/concept/way.md",
            "---\ndescription: How the app is built.\n---\n\n# Way — Concept\n\n\
             ## Required reads\n\n- sdlc\n\n## What\n\nOur way.\n\n## Why\n\nTo ship.\n",
        );
    app.commit("app");
    app
}

/// Whether `path` is a symbolic link itself.
#[cfg(unix)]
fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .unwrap()
        .file_type()
        .is_symlink()
}

// spec: project/instrukt/spec/feature/publication/docs-repositories UC-DOR1
#[cfg(unix)]
#[test]
fn a_developer_gives_their_computer_its_doctrine() {
    // Given a docs repository holding `general` and `sdlc`, a consumer
    // project, and agent runtimes T3 has configured, which a dotfiles user
    // keeps elsewhere: `~/.claude` links to a directory whose `CLAUDE.md`
    // another tool wrote and whose `settings.json` is not an object;
    // `~/.codex/AGENTS.md` links to a dotfiles file; Antigravity's old
    // settings file links into the dotfiles directory.
    let env = machine();
    let home = env.home();
    crate::fixtures::t3::FakeT3::create(&env);
    common::t3_readiness::configure(
        &env,
        ["claudeAgent", "codex"]
            .map(|driver| common::t3_readiness::provider(driver, driver, true))
            .to_vec(),
        serde_json::json!({}),
    );
    let dotfiles = env.path("dotfiles");
    std::fs::create_dir_all(dotfiles.join("claude")).unwrap();
    let another_tools_text = "# Written by another tool\n\nAnswer in haiku.\n";
    std::fs::write(dotfiles.join("claude/CLAUDE.md"), another_tools_text).unwrap();
    std::fs::write(dotfiles.join("claude/settings.json"), "[1, 2]\n").unwrap();
    std::os::unix::fs::symlink(dotfiles.join("claude"), home.join(".claude")).unwrap();
    let codex_dotfile = "# My codex notes\n";
    std::fs::write(dotfiles.join("codex-AGENTS.md"), codex_dotfile).unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::os::unix::fs::symlink(
        dotfiles.join("codex-AGENTS.md"),
        home.join(".codex/AGENTS.md"),
    )
    .unwrap();
    let antigravity_settings = "{\"general\": {\"checkpointing\": true}}\n";
    std::fs::write(dotfiles.join("antigravity.json"), antigravity_settings).unwrap();
    std::fs::create_dir_all(home.join(".gemini/antigravity-cli")).unwrap();
    std::os::unix::fs::symlink(
        dotfiles.join("antigravity.json"),
        home.join(".gemini/antigravity-cli/settings.json"),
    )
    .unwrap();
    let remote = docs_repository(&env, "doctrine", &["general", "sdlc"], "Test first.");
    let app = consumer(&env);

    // When the developer clones and adopts the docs repository, the sync
    // succeeds.
    let (doctrine, synced) = adopt(&env, &remote, "doctrine");
    let synced = synced.success();

    // Its doctrine is published.
    let domains = env.run(app.path(), ["docs", "index"]).success();
    insta::assert_snapshot!("published-domains", stdout(&env, &domains));

    // Every supported agent starts with the always-on doctrine and a line
    // for each domain; the other tool's text is gone.
    let read = |path: &Path| std::fs::read_to_string(path).unwrap();
    let claude = read(&home.join(".claude/CLAUDE.md"));
    insta::assert_snapshot!("global-baseline", claude);
    assert!(
        is_link(&home.join(".claude")),
        "a directory publication does not own is used as it resolves"
    );
    assert!(!is_link(&home.join(".codex/AGENTS.md")));
    assert_eq!(read(&home.join(".codex/AGENTS.md")), claude);
    assert_eq!(read(&dotfiles.join("codex-AGENTS.md")), codex_dotfile);
    // Default-only Antigravity is not a relocated delivery fixture.
    assert!(!home.join(".gemini/AGENTS.md").exists());

    // Settings: nothing is written to either, nor through the link.
    assert_eq!(read(&dotfiles.join("claude/settings.json")), "[1, 2]\n");
    assert!(is_link(&home.join(".gemini/antigravity-cli/settings.json")));
    assert_eq!(
        read(&dotfiles.join("antigravity.json")),
        antigravity_settings
    );

    // The report names each destination written with its program, the
    // replaced link and each settings file left with its reason; a second
    // sync of the unchanged main names every destination as current.
    let head = doctrine.head();
    insta::assert_snapshot!(
        "first-sync-report",
        stdout(&env, &synced).replace(&head, "[SHA]")
    );
    let again = env.run(doctrine.path(), ["sync"]).success();
    insta::assert_snapshot!(
        "second-sync-report",
        stdout(&env, &again).replace(&head, "[SHA]")
    );

    // Agents in another project read that doctrine.
    let got = env
        .run(app.path(), ["docs", "get", "sdlc/policy/rules"])
        .success();
    insta::assert_snapshot!("consumer-reads-doctrine", stdout(&env, &got));
    env.run(app.path(), ["hooks", "install"]).success();
    app.commit("instructions");
    env.finished_hook_run(app.path(), Hook::PostCommit);
    insta::assert_snapshot!("consumer-instructions", app.git(["show", "HEAD:AGENTS.md"]));
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
    let retracted = env.finished_hook_run(first.path(), Hook::PostCommit);
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
fn validating_a_project_whose_master_needs_an_unpublished_domain_names_it() {
    let env = machine();
    let app = env.git_repo("app");
    app.write("instrukt.yaml", &layer("app", "")).write(
        "tools/AGENTS.master.md",
        "## Tools\n\nThe tools.\n\n## Required reads\n\n- sdlc\n",
    );
    app.commit("app");

    let refused = env.run(app.path(), ["docs", "validate"]).failure();
    insta::assert_snapshot!(stdout(&env, &refused), @"tools/AGENTS.master.md: reference-domain-unpublished: sdlc: domain `sdlc` is not published on this computer; adopt a docs repository that publishes it");
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
    app.write("instrukt.yaml", &layer("app", ""))
        .write(
            "docs/project/concept/flow.md",
            "---\ndescription: How work flows here.\n---\n\n# Flow — Concept\n\n\
         ## Required reads\n\n- flows/policy/rules\n\n\
         ## What\n\nOur flow.\n\n## Why\n\nTo ship.\n",
        )
        .write(
            "docs/project/index.yaml",
            "listed: [project/app/concept/flow]\n",
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
