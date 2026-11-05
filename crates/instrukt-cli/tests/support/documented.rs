//! Documented projects for retrieval tests: valid snippets committed in a
//! temporary repository and published into the temporary home by the real
//! `instrukt sync`.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use crate::common::{Repo, TestEnv};

/// A snippet of `taxonomy` with every required section: `extra`
/// frontmatter lines, a `Required reads` section holding `reads` (one
/// line each, as written) when any, and `body` as the
/// first section's text.
pub fn snippet(
    taxonomy: &str,
    title: &str,
    description: &str,
    extra: &str,
    reads: &[&str],
    body: &str,
) -> String {
    let (suffix, sections): (&str, &[&str]) = match taxonomy {
        "principle" => (
            "Principle",
            &["Principle", "Rationale", "Implications", "Tensions"],
        ),
        "concept" => ("Concept", &["What", "Why"]),
        "policy" => (
            "Policy",
            &["Rules", "Rationale", "Scope", "Enforcement", "Exceptions"],
        ),
        "procedure" => (
            "Procedure",
            &["Goal", "Preconditions", "Steps", "Outputs", "Recovery"],
        ),
        "design" => (
            "Design",
            &[
                "Purpose",
                "Inputs/Outputs",
                "Invariants",
                "Primary flows",
                "Failure modes",
            ],
        ),
        "spec" => ("Spec", &["What it is", "Canonical fields"]),
        other => panic!("no builder for {other}"),
    };
    let mut text = format!("---\ndescription: {description}\n{extra}---\n\n# {title} — {suffix}\n");
    if !reads.is_empty() {
        text.push_str("\n## Required reads\n\n");
        for read in reads {
            text.push_str(&format!("{read}\n"));
        }
    }
    for (index, section) in sections.iter().enumerate() {
        let content = if index == 0 { body } else { "Text.\n" };
        text.push_str(&format!("\n## {section}\n\n{content}"));
    }
    text
}

/// A project layer declaring `slug`.
fn layer(slug: &str) -> String {
    format!("schema_version: 1\nproject: {slug}\n")
}

/// A repository `slug` declaring that project, with `files` committed on
/// main.
pub fn project(env: &TestEnv, slug: &str, files: &[(&str, String)]) -> Repo {
    let repo = env.git_repo(slug);
    repo.write("instrukt.yaml", &layer(slug));
    for (path, text) in files {
        repo.write(path, text);
    }
    repo.commit("docs: document the project");
    repo
}

/// Publishes the project with `instrukt sync`, which must succeed.
pub fn sync(env: &TestEnv, repo: &Repo) {
    let run = env.run(repo.path(), ["sync"]);
    let output = run.get_output();
    assert!(
        output.status.success(),
        "sync failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
