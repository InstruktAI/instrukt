//! Rendered agent instructions from `instrukt sync`
//! (`docs/project/spec/feature/publication/agent-entrypoints.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use serde_json::Value;

/// A snippet of `taxonomy` (policy, principle or procedure) with every
/// required section, `extra` frontmatter lines and `reads` as its required
/// reads.
fn snippet(taxonomy: &str, title: &str, extra: &str, reads: &[&str]) -> String {
    let (suffix, sections): (&str, &[&str]) = match taxonomy {
        "policy" => (
            "Policy",
            &["Rules", "Rationale", "Scope", "Enforcement", "Exceptions"],
        ),
        "principle" => (
            "Principle",
            &["Principle", "Rationale", "Implications", "Tensions"],
        ),
        _ => (
            "Procedure",
            &["Goal", "Preconditions", "Steps", "Outputs", "Recovery"],
        ),
    };
    let mut text =
        format!("---\ndescription: About {title}.\n{extra}---\n\n# {title} — {suffix}\n");
    if !reads.is_empty() {
        text.push_str("\n## Required reads\n\n");
        for read in reads {
            text.push_str(&format!("- {read}\n"));
        }
    }
    for section in sections {
        text.push_str(&format!("\n## {section}\n\n{title}: {section}.\n"));
    }
    text
}

fn layer(slug: &str) -> String {
    format!("schema_version: 1\nproject: {slug}\n")
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

/// A project whose root master states its purpose and whose index selects
/// one snippet in full (with a required read of its own), lists another,
/// beside a triggered procedure and a nested master.
fn demo_project(env: &TestEnv) -> Repo {
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", &layer("demo"))
        .write(
            "AGENTS.master.md",
            "<!-- LOCKED_FILE: edit only on request. -->\n\n## Purpose\n\nThe demo project.\n\n\
             ## Notes\n\n<!-- @config: project -->\n\n\
             Written as ``@docs/project/policy/house.md`` it is quoted.\n\n\
             ```\n@docs/project/policy/not-a-snippet.md\n```\n",
        )
        .write(
            "tools/AGENTS.master.md",
            "## Tools\n\nOnly the tools.\n\n## Required reads\n\n- project/demo/policy/style\n",
        )
        .write(
            "docs/project/index.yaml",
            "expanded:\n  - project/demo/policy/house\nlisted:\n  - project/demo/policy/style\n",
        )
        .write(
            "docs/project/policy/house.md",
            &snippet("policy", "House", "", &["project/demo/principle/care"]),
        )
        .write(
            "docs/project/principle/care.md",
            &snippet("principle", "Care", "", &[]),
        )
        .write(
            "docs/project/policy/style.md",
            &snippet("policy", "Style", "", &[]),
        )
        .write(
            "docs/project/procedure/release.md",
            &snippet(
                "procedure",
                "Release",
                "activation_trigger: When cutting a release.\n",
                &[],
            ),
        );
    repo.commit("instructions");
    repo
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE1
#[test]
fn an_agent_opened_in_the_project_reads_its_purpose_and_its_selection() {
    let env = TestEnv::new();
    // Codex is installed: it reads the project override file.
    std::fs::create_dir_all(env.home().join(".codex")).unwrap();
    let repo = demo_project(&env);

    env.run(repo.path(), ["sync"]).success();

    insta::assert_snapshot!("root_agents_md", repo.read("AGENTS.md"));
    assert_eq!(repo.read("AGENTS.override.md"), repo.read("AGENTS.md"));
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE1
#[test]
fn a_nested_master_renders_beside_itself_with_its_required_reads() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.home().join(".codex")).unwrap();
    let repo = demo_project(&env);

    env.run(repo.path(), ["sync"]).success();

    insta::assert_snapshot!("nested_agents_md", repo.read("tools/AGENTS.md"));
    assert!(!repo.path().join("tools/AGENTS.override.md").exists());
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE1
#[test]
fn a_root_master_with_required_reads_is_refused_naming_the_project_index() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", &layer("demo"))
        .write(
            "AGENTS.master.md",
            "## Purpose\n\nThe demo project.\n\n## Required reads\n\n- project/demo/policy/house\n",
        )
        .write(
            "docs/project/policy/house.md",
            &snippet("policy", "House", "", &[]),
        );
    repo.commit("master with reads");

    let refused = env.run(repo.path(), ["sync"]).failure();

    insta::assert_snapshot!("root_master_with_required_reads", stdout(&env, &refused));
    assert!(!repo.path().join("AGENTS.md").exists());
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE1
#[test]
fn without_a_project_index_the_root_agents_md_holds_the_purpose_and_procedures() {
    let env = TestEnv::new();
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", &layer("demo"))
        .write("AGENTS.master.md", "## Purpose\n\nThe demo project.\n")
        .write(
            "docs/project/procedure/release.md",
            &snippet(
                "procedure",
                "Release",
                "activation_trigger: When cutting a release.\n",
                &[],
            ),
        );
    repo.commit("no index");

    let synced = env.run(repo.path(), ["sync"]).success();

    insta::assert_snapshot!(
        "sync_without_project_index",
        stdout(&env, &synced).replace(&repo.head(), "[SHA]")
    );
    insta::assert_snapshot!("root_agents_md_without_index", repo.read("AGENTS.md"));
}

/// A project publishing `sdlc` with a description, a triggered procedure
/// and a snippet the general owner's index selects.
fn engineering(env: &TestEnv) -> Repo {
    let repo = env.git_repo("engineering");
    repo.write("instrukt.yaml", &layer("engineering"))
        .write(
            "docs/domain/sdlc/index.yaml",
            "description: Writing, reviewing and shipping code\n",
        )
        .write(
            "docs/domain/sdlc/policy/tests.md",
            &snippet("policy", "Tests", "", &[]),
        )
        .write(
            "docs/domain/sdlc/procedure/ship.md",
            &snippet(
                "procedure",
                "Ship",
                "activation_trigger: When the human asks to ship.\n",
                &[],
            ),
        );
    repo.commit("sdlc");
    repo
}

/// The owner of `docs/general/`: its index selects a general snippet (with
/// its closure), an sdlc snippet and another general snippet in full, lists
/// one, maps an agent policy to `agents`, and it has a triggered procedure.
fn general_owner(env: &TestEnv, agents: &str) -> Repo {
    let repo = env.git_repo("platform");
    repo.write("instrukt.yaml", &layer("platform"))
        .write(
            "docs/general/index.yaml",
            &format!(
                "expanded:\n  - general/policy/care\n  - sdlc/policy/tests\n  - general/policy/kind\n\
                 listed:\n  - general/policy/style\n{agents}"
            ),
        )
        .write(
            "docs/general/policy/care.md",
            &snippet("policy", "Care", "", &["general/principle/attention"]),
        )
        .write(
            "docs/general/principle/attention.md",
            &snippet("principle", "Attention", "", &[]),
        )
        .write(
            "docs/general/policy/kind.md",
            &snippet("policy", "Kind", "", &[]),
        )
        .write(
            "docs/general/policy/style.md",
            &snippet("policy", "Style", "", &[]),
        )
        .write(
            "docs/general/policy/agents/claude.md",
            &snippet("policy", "Claude", "", &[]),
        )
        .write(
            "docs/general/procedure/onboard.md",
            &snippet(
                "procedure",
                "Onboard",
                "activation_trigger: When the human asks to onboard someone.\n",
                &[],
            ),
        );
    repo.commit("general");
    repo
}

const CLAUDE_AGENTS: &str = "agents:\n  claude:\n    - general/policy/agents/claude\n";

/// Whether every member of `before` is in `after` with the same value,
/// objects compared member by member.
fn keeps_every_member(before: &Value, after: &Value) -> bool {
    match (before, after) {
        (Value::Object(before), Value::Object(after)) => before.iter().all(|(key, value)| {
            after
                .get(key)
                .is_some_and(|kept| keeps_every_member(value, kept))
        }),
        _ => before == after,
    }
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE2
#[cfg(unix)]
#[test]
fn every_installed_agent_cli_starts_with_the_general_section_and_keeps_its_settings() {
    let env = TestEnv::new();
    let home = env.home();
    let claude_settings = serde_json::json!({
        "theme": "dark",
        "pluginConfigs": {"other@market": {"enabled": true}},
        "permissions": {"defaultMode": "ask"}
    });
    let gemini_settings = serde_json::json!({
        "context": {"includeDirectories": ["/"]},
        "general": {"checkpointing": {"enabled": true}}
    });
    // A dotfiles user: `~/.claude` links to a directory kept elsewhere, and
    // the Gemini settings file links to a file kept elsewhere.
    let dotfiles = env.path("dotfiles");
    std::fs::create_dir_all(dotfiles.join("claude")).unwrap();
    std::fs::write(
        dotfiles.join("claude/settings.json"),
        serde_json::to_vec_pretty(&claude_settings).unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(dotfiles.join("claude"), home.join(".claude")).unwrap();
    std::fs::write(
        dotfiles.join("gemini-settings.json"),
        serde_json::to_vec_pretty(&gemini_settings).unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".gemini/antigravity-cli")).unwrap();
    std::os::unix::fs::symlink(
        dotfiles.join("gemini-settings.json"),
        home.join(".gemini/antigravity-cli/settings.json"),
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, "");

    env.run(platform.path(), ["sync"]).success();

    let read = |rel: &str| std::fs::read_to_string(home.join(rel)).unwrap();
    insta::assert_snapshot!("global_file", read(".claude/CLAUDE.md"));
    assert_eq!(read(".codex/AGENTS.md"), read(".claude/CLAUDE.md"));
    assert_eq!(
        read(".gemini/antigravity-cli/AGENTS.md"),
        read(".claude/CLAUDE.md")
    );
    assert!(
        !home.join(".dsh").exists(),
        "an agent CLI not installed is skipped"
    );
    let is_link = |path: &std::path::Path| {
        std::fs::symlink_metadata(path)
            .unwrap()
            .file_type()
            .is_symlink()
    };
    assert!(
        is_link(&home.join(".claude")),
        "the user's link stays a link"
    );
    assert!(is_link(&home.join(".gemini/antigravity-cli/settings.json")));
    let in_dotfiles = |rel: &str| std::fs::read_to_string(dotfiles.join(rel)).unwrap();
    assert_eq!(in_dotfiles("claude/CLAUDE.md"), read(".claude/CLAUDE.md"));
    let claude_after = in_dotfiles("claude/settings.json");
    let gemini_after = in_dotfiles("gemini-settings.json");
    insta::assert_snapshot!("claude_settings", claude_after);
    insta::assert_snapshot!("gemini_settings", gemini_after);
    assert!(keeps_every_member(
        &claude_settings,
        &serde_json::from_str(&claude_after).unwrap()
    ));
    assert!(keeps_every_member(
        &gemini_settings,
        &serde_json::from_str(&gemini_after).unwrap()
    ));
    assert!(!home.join(".codex/settings.json").exists());
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE3
#[test]
fn an_agents_snippet_ends_only_its_own_clis_global_file() {
    let env = TestEnv::new();
    let home = env.home();
    for dir in [".claude", ".codex"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, CLAUDE_AGENTS);

    env.run(platform.path(), ["sync"]).success();

    let read = |rel: &str| std::fs::read_to_string(home.join(rel)).unwrap();
    insta::assert_snapshot!("claude_global_file", read(".claude/CLAUDE.md"));
    insta::assert_snapshot!("codex_global_file", read(".codex/AGENTS.md"));
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE3
#[test]
fn mapping_a_snippet_to_an_unsupported_agent_cli_is_refused_naming_the_supported_ones() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.home().join(".claude")).unwrap();
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(
        &env,
        "agents:\n  cursor:\n    - general/policy/agents/claude\n",
    );

    let refused = env.run(platform.path(), ["sync"]).failure();

    insta::assert_snapshot!("unsupported_agent_cli", stdout(&env, &refused));
    assert!(!env.home().join(".claude/CLAUDE.md").exists());
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE4
#[test]
fn a_global_file_the_developer_wrote_stops_publication_until_it_is_moved() {
    let env = TestEnv::new();
    let home = env.home();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    let own = "# My own rules\n\nAlways answer in haiku.\n";
    std::fs::write(home.join(".claude/CLAUDE.md"), own).unwrap();
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, "");

    let refused = env.run(platform.path(), ["sync"]).failure();

    insta::assert_snapshot!(
        "foreign_global_file",
        stderr(&env, &refused).replace(&platform.head(), "[SHA]")
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap(),
        own
    );
    assert!(!env.instrukt_home().join("docs/general").exists());

    std::fs::rename(home.join(".claude/CLAUDE.md"), env.path("CLAUDE.mine.md")).unwrap();
    env.run(platform.path(), ["sync"]).success();
    insta::assert_snapshot!(
        "global_file_after_moving_own",
        std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap()
    );
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE4
#[test]
fn a_global_file_edited_after_instrukt_wrote_it_stops_the_next_publication() {
    let env = TestEnv::new();
    let home = env.home();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, "");
    env.run(platform.path(), ["sync"]).success();
    let edited = format!(
        "{}\nA line of my own.\n",
        std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap()
    );
    std::fs::write(home.join(".claude/CLAUDE.md"), &edited).unwrap();

    let refused = env.run(platform.path(), ["sync", "--force"]).failure();

    insta::assert_snapshot!(
        "edited_global_file",
        stderr(&env, &refused).replace(&platform.head(), "[SHA]")
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap(),
        edited
    );
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE5
#[test]
fn a_domain_without_an_index_is_published_but_absent_from_the_global_files() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.home().join(".claude")).unwrap();
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, "");
    platform
        .write(
            "docs/domain/ops/policy/oncall.md",
            &snippet("policy", "Oncall", "", &[]),
        )
        .commit("ops without index");

    let synced = env.run(platform.path(), ["sync"]).success();

    insta::assert_snapshot!(
        "sync_domain_without_index",
        stdout(&env, &synced).replace(&platform.head(), "[SHA]")
    );
    insta::assert_snapshot!(
        "global_file_without_unindexed_domain",
        std::fs::read_to_string(env.home().join(".claude/CLAUDE.md")).unwrap()
    );
    let listed = env
        .run(platform.path(), ["docs", "index", "--domain", "ops"])
        .success();
    insta::assert_snapshot!(
        "unindexed_domain_listing",
        common::mask_published_commits(&stdout(&env, &listed))
    );
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE4
#[test]
fn retracting_the_global_files_keeps_one_the_developer_edited_and_names_it() {
    let env = TestEnv::new();
    let home = env.home();
    for dir in [".claude", ".codex"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, "");
    env.run(platform.path(), ["sync"]).success();
    let edited = "# Mine now\n";
    std::fs::write(home.join(".claude/CLAUDE.md"), edited).unwrap();
    std::fs::remove_dir_all(platform.path().join("docs/general")).unwrap();
    platform.git(["add", "--all", "docs"]);
    platform.git(["commit", "--quiet", "-m", "stop publishing general"]);

    let synced = env.run(platform.path(), ["sync"]).success();

    insta::assert_snapshot!(
        "retraction_keeps_edited_global_file",
        stdout(&env, &synced).replace(&platform.head(), "[SHA]")
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap(),
        edited
    );
    assert!(!home.join(".codex/AGENTS.md").exists());
}

fn checkout_machine() -> TestEnv {
    let mut env = TestEnv::new();
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap();
    env.set_env(
        "PATH",
        &format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    env
}

fn checkout_project(env: &TestEnv) -> Repo {
    let repo = env.git_repo("checkout");
    repo.write(
        "instrukt.yaml",
        "schema_version: 5\nproject: checkout\nmodules: {history: false}\n",
    )
    .write("AGENTS.master.md", "## Purpose\n\nPublished purpose.\n")
    .write(
        "docs/project/index.yaml",
        "expanded: [project/checkout/policy/rules]\n",
    )
    .write(
        "docs/project/policy/rules.md",
        &snippet("policy", "Published rules", "", &[]),
    )
    .write(".gitignore", "AGENTS.md\n");
    repo.commit("docs: main baseline");
    env.run(repo.path(), ["hooks", "install"]).success();
    repo
}

fn publication_bytes(env: &TestEnv) -> Vec<u8> {
    std::fs::read(env.instrukt_home().join("state/publication/checkout.yaml")).unwrap()
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE6
// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK5
#[test]
fn linked_checkout_renders_published_baseline_before_return_without_publication() {
    let env = checkout_machine();
    let repo = checkout_project(&env);
    env.run(repo.path(), ["sync"]).success();
    let published = publication_bytes(&env);
    let baseline = std::fs::read(repo.path().join("AGENTS.md")).unwrap();
    let blocked = instrukt_core::fsx::LockGuard::acquire(
        &env.instrukt_home().join("locks/docs.lock"),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let linked = env.path("linked");
    repo.git([
        std::ffi::OsStr::new("worktree"),
        std::ffi::OsStr::new("add"),
        linked.as_os_str(),
        std::ffi::OsStr::new("-b"),
        std::ffi::OsStr::new("proposal"),
    ]);
    assert_eq!(std::fs::read(linked.join("AGENTS.md")).unwrap(), baseline);
    std::fs::write(
        linked.join("AGENTS.master.md"),
        "## Purpose\n\nProposed purpose.\n",
    )
    .unwrap();
    std::fs::write(
        linked.join("docs/project/policy/rules.md"),
        snippet("policy", "Proposed rules", "", &[]),
    )
    .unwrap();
    env.git(&linked, ["add", "--all"]);
    env.git(
        &linked,
        ["commit", "--quiet", "-m", "docs: propose a new baseline"],
    );
    std::fs::remove_file(linked.join("AGENTS.md")).unwrap();
    env.git(&linked, ["checkout", "--detach"]);
    assert_eq!(std::fs::read(linked.join("AGENTS.md")).unwrap(), baseline);
    drop(blocked);
    assert_eq!(publication_bytes(&env), published);
    assert_eq!(
        std::fs::read(repo.path().join("AGENTS.md")).unwrap(),
        baseline
    );
    let status = instrukt_core::repository::last_run(
        &instrukt_core::home::Home::at_user_home(env.home().to_path_buf()),
        &instrukt_core::ids::ProjectSlug::parse("checkout").unwrap(),
        instrukt_core::repository::Hook::PostCheckout,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        status.checkout,
        Some(instrukt_core::repository::CheckoutOutcome::Rendered)
    );
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE7
// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK5
#[test]
fn primary_checkout_bootstraps_once_and_later_renders_published_state() {
    let env = checkout_machine();
    let repo = checkout_project(&env);
    let own = repo.path().join(".git/hooks/post-checkout");
    let mut hook = std::fs::read_to_string(&own).unwrap();
    hook.push_str("if [ -f AGENTS.md ]; then echo ran > own-checkout-ran; fi\n");
    std::fs::write(&own, hook).unwrap();
    repo.git(["checkout", "-b", "feature"]);
    assert!(repo.path().join("AGENTS.md").is_file());
    assert!(repo.path().join("own-checkout-ran").is_file());
    let published = publication_bytes(&env);
    let baseline = std::fs::read(repo.path().join("AGENTS.md")).unwrap();
    repo.write("docs/project/policy/rules.md", "invalid proposed snippet");
    std::fs::remove_file(repo.path().join("AGENTS.md")).unwrap();
    repo.git(["checkout", "-b", "another"]);
    assert_eq!(
        std::fs::read(repo.path().join("AGENTS.md")).unwrap(),
        baseline
    );
    assert_eq!(publication_bytes(&env), published);
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE7
// spec: project/instrukt/spec/feature/repository/hook-sections UC-HK5
#[test]
fn unpublished_linked_checkout_reports_prerequisite_without_publishing_or_failing_git() {
    let env = checkout_machine();
    let repo = checkout_project(&env);
    let linked = env.path("unpublished");
    repo.git([
        std::ffi::OsStr::new("worktree"),
        std::ffi::OsStr::new("add"),
        linked.as_os_str(),
        std::ffi::OsStr::new("-b"),
        std::ffi::OsStr::new("proposal"),
    ]);
    assert!(!linked.join("AGENTS.md").exists());
    assert!(
        !env.instrukt_home()
            .join("state/publication/checkout.yaml")
            .exists()
    );
    let result = env
        .run(&linked, ["hooks", "run", "post-checkout"])
        .success();
    assert!(!result.get_output().stderr.is_empty());
    let status = instrukt_core::repository::last_run(
        &instrukt_core::home::Home::at_user_home(env.home().to_path_buf()),
        &instrukt_core::ids::ProjectSlug::parse("checkout").unwrap(),
        instrukt_core::repository::Hook::PostCheckout,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        status.checkout,
        Some(instrukt_core::repository::CheckoutOutcome::Failed { .. })
    ));
    let status = env.run(&linked, ["hooks", "status"]).success();
    insta::assert_snapshot!(stdout(&env, &status));
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE6
#[test]
fn a_linked_checkout_retracts_its_own_withdrawn_generated_instructions() {
    let env = checkout_machine();
    let repo = checkout_project(&env);
    repo.write(
        "nested/AGENTS.master.md",
        "## Nested\n\nPublished nested instructions.\n",
    );
    repo.commit("docs: add nested instructions");
    env.run(repo.path(), ["sync"]).success();
    let linked = env.path("withdrawal-linked");
    env.git(
        repo.path(),
        ["worktree", "add", linked.to_str().unwrap(), "-b", "linked"],
    );
    assert!(linked.join("nested/AGENTS.md").is_file());
    // A user-owned AGENTS.md never written by publication is not a withdrawal target.
    std::fs::create_dir_all(linked.join("manual")).unwrap();
    std::fs::write(
        linked.join("manual/AGENTS.md"),
        "developer owned instructions\n",
    )
    .unwrap();
    std::fs::remove_file(repo.path().join("nested/AGENTS.master.md")).unwrap();
    repo.commit("docs: withdraw nested instructions");
    env.run(repo.path(), ["sync"]).success();
    assert!(!repo.path().join("nested/AGENTS.md").exists());
    let published = publication_bytes(&env);
    env.git(&linked, ["checkout", "--detach", &repo.head()]);
    assert!(!linked.join("nested/AGENTS.md").exists());
    assert!(linked.join("manual/AGENTS.md").is_file());
    assert_eq!(publication_bytes(&env), published);
}
