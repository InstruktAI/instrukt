//! Compiled agent instructions from `instrukt sync`
//! (`docs/project/spec/feature/publication/agent-entrypoints.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;

use common::{Repo, TestEnv};
use serde_json::Value;

/// A snippet of `taxonomy` (policy or procedure) with every required
/// section and `extra` frontmatter lines.
fn snippet(taxonomy: &str, title: &str, extra: &str) -> String {
    let (suffix, sections): (&str, &[&str]) = match taxonomy {
        "policy" => (
            "Policy",
            &["Rules", "Rationale", "Scope", "Enforcement", "Exceptions"],
        ),
        _ => (
            "Procedure",
            &["Goal", "Preconditions", "Steps", "Outputs", "Recovery"],
        ),
    };
    let mut text =
        format!("---\ndescription: About {title}.\n{extra}---\n\n# {title} — {suffix}\n");
    for section in sections {
        text.push_str(&format!("\n## {section}\n\n{title}: {section}.\n"));
    }
    text
}

fn layer(slug: &str) -> String {
    format!("schema_version: 1\nproject: {slug}\n")
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE1
#[test]
fn an_agent_opened_in_the_project_reads_one_self_contained_instruction_file() {
    let env = TestEnv::new();
    // Codex is installed: it reads the project override file.
    std::fs::create_dir_all(env.home().join(".codex")).unwrap();
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", &layer("demo"))
        .write(
            "AGENTS.master.md",
            "<!-- LOCKED_FILE: edit only on request. -->\n\n## Purpose\n\nThe demo project.\n\n\
             ## Required reads\n\n@docs/project/policy/house.md\n@docs/project/policy/future.md\n\n\
             ## Notes\n\n<!-- @config: project -->\n\n\
             Written as ``@docs/project/policy/house.md`` it is quoted.\n\n\
             ```\n@docs/project/policy/not-a-snippet.md\n```\n\n\
             <!-- @docs/project/policy/house.md stays a comment -->\n",
        )
        .write("tools/AGENTS.master.md", "## Tools\n\nOnly the tools.\n")
        .write(
            "docs/project/policy/house.md",
            &snippet("policy", "House", ""),
        )
        .write(
            "docs/project/policy/future.md",
            &snippet("policy", "Future", "delivered_by: [build-future]\n"),
        )
        .write(
            "docs/project/policy/style.md",
            &snippet("policy", "Style", ""),
        )
        .write(
            "docs/project/procedure/release.md",
            &snippet(
                "procedure",
                "Release",
                "activation_trigger: When cutting a release.\n",
            ),
        )
        .write(
            "docs/project/baseline.md",
            "@docs/project/policy/style.md\n",
        );
    repo.commit("instructions");

    env.run(repo.path(), ["sync"]).success();

    insta::assert_snapshot!("root_agents_md", repo.read("AGENTS.md"));
    insta::assert_snapshot!("nested_agents_md", repo.read("tools/AGENTS.md"));
    assert_eq!(repo.read("AGENTS.override.md"), repo.read("AGENTS.md"));
    assert!(!repo.path().join("tools/AGENTS.override.md").exists());
}

/// The general owner: a global master inlining the general bundle, one
/// addendum per agent, and a general baseline.
fn general_owner(env: &TestEnv) -> Repo {
    let repo = env.git_repo("platform");
    repo.write("instrukt.yaml", &layer("platform"))
        .write(
            "agents/AGENTS.global.md",
            "# Global baseline\n\n## Required reads\n\n@~/.instrukt/docs/general/bundle.md\n\n\
             ## Who you are\n\nA careful agent.\n",
        )
        .write(
            "agents/CLAUDE.addendum.md",
            "## Claude\n\nClaude-specific guidance.\n",
        )
        .write(
            "agents/CODEX.addendum.md",
            "## Codex\n\nCodex-specific guidance.\n",
        )
        .write(
            "agents/GEMINI.addendum.md",
            "## Gemini\n\nGemini-specific guidance.\n",
        )
        .write(
            "docs/general/policy/care.md",
            &snippet("policy", "Care", ""),
        )
        .write(
            "docs/general/baseline.md",
            "@~/.instrukt/docs/general/policy/care.md\n",
        );
    repo.commit("global baseline");
    repo
}

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
fn every_installed_agent_cli_starts_with_the_global_baseline_and_keeps_its_settings() {
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
    let other = env.git_repo("engineering");
    other.write("instrukt.yaml", &layer("engineering")).write(
        "docs/domain/sdlc/policy/tests.md",
        &snippet("policy", "Tests", ""),
    );
    other.commit("sdlc");
    env.run(other.path(), ["sync"]).success();
    let platform = general_owner(&env);

    env.run(platform.path(), ["sync"]).success();

    let read = |rel: &str| std::fs::read_to_string(home.join(rel)).unwrap();
    insta::assert_snapshot!("claude_entrypoint", read(".claude/CLAUDE.md"));
    insta::assert_snapshot!("codex_entrypoint", read(".codex/AGENTS.md"));
    insta::assert_snapshot!(
        "gemini_entrypoint",
        read(".gemini/antigravity-cli/AGENTS.md")
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
