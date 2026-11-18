//! Rendered agent instructions from `instrukt sync`
//! (`docs/project/spec/feature/publication/agent-entrypoints.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};

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

#[test]
fn a_nested_master_renders_beside_itself_with_its_required_reads() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.home().join(".codex")).unwrap();
    let repo = demo_project(&env);

    env.run(repo.path(), ["sync"]).success();

    insta::assert_snapshot!("nested_agents_md", repo.read("tools/AGENTS.md"));
    assert!(!repo.path().join("tools/AGENTS.override.md").exists());
}

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

#[test]
fn a_global_file_keeping_a_reference_is_written_and_named() {
    let env = TestEnv::new();
    let home = env.home();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    // Another project publishes `ops`; its published snippet then reads as
    // an earlier release left it, with a visible reference validation
    // refuses today.
    let ops = env.git_repo("ops");
    ops.write("instrukt.yaml", &layer("ops"))
        .write("docs/domain/ops/index.yaml", "description: Operations\n")
        .write(
            "docs/domain/ops/policy/house.md",
            &snippet("policy", "House", "", &[]),
        );
    ops.commit("ops");
    env.run(ops.path(), ["sync"]).success();
    let published = env.instrukt_home().join("docs/ops/policy/house.md");
    let text = std::fs::read_to_string(&published).unwrap();
    std::fs::write(
        &published,
        text.replace("House: Rules.", "Read it (@notes/house.md) first."),
    )
    .unwrap();
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, "");
    platform
        .write(
            "docs/general/index.yaml",
            "expanded:\n  - general/policy/care\n  - ops/policy/house\n",
        )
        .commit("select ops");

    let synced = env.run(platform.path(), ["sync"]).success();

    insta::assert_snapshot!(
        "sync_keeping_a_reference",
        stdout(&env, &synced).replace(&platform.head(), "[SHA]")
    );
    insta::assert_snapshot!(
        "global_file_keeping_a_reference",
        std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap()
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

fn checkout_machine() -> TestEnv {
    let mut env = TestEnv::new();
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap();
    env.set_env(
        "PATH",
        &format!("{}:{}", bin.display(), env.injected("PATH").unwrap()),
    );
    env
}

fn checkout_project(env: &TestEnv) -> Repo {
    let repo = env.git_repo("checkout");
    repo.write(
        "instrukt.yaml",
        "schema_version: 6\nproject: checkout\nmodules: {history: false}\n",
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

// spec: project/instrukt/spec/feature/configuration/owned-files UC-OF3
#[test]
fn development_checkout_rendering_preserves_the_installed_checkouts_instructions() {
    let mut env = checkout_machine();
    let repo = checkout_project(&env);
    env.run(repo.path(), ["sync"]).success();
    env.run(repo.path(), ["hooks", "run", "post-checkout"])
        .success();
    let instructions = std::fs::read(repo.path().join("AGENTS.md")).unwrap();
    let published = publication_bytes(&env);
    let development = env.path("private-development-home");
    env.set_env("INSTRUKT_HOME", development.to_str().unwrap());
    env.set_env("INSTRUKT_DEVELOPMENT", "1");
    env.run(repo.path(), ["sync"]).success();
    env.run(repo.path(), ["hooks", "run", "post-checkout"])
        .success();
    assert_eq!(
        std::fs::read(repo.path().join("AGENTS.md")).unwrap(),
        instructions
    );
    assert_eq!(publication_bytes(&env), published);
}
