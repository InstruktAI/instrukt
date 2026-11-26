//! Compressed units: the inspection, and the renderings that hold units
//! (`docs/project/spec/feature/publication/compressed-units.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use common::{Repo, TestEnv};
use sha2::{Digest, Sha256};

const LAYER: &str = "schema_version: 6\nproject: demo\n";

/// A general policy titled `title` whose one rule is `rule`.
fn policy(title: &str, rule: &str) -> String {
    format!(
        "---\ndescription: About {title}.\n---\n\n# {title} — Policy\n\n## Rules\n\n{rule}\n\n## Rationale\n\n{title}: why.\n\n## Scope\n\nAll.\n\n## Enforcement\n\nReview.\n\n## Exceptions\n\nNone.\n"
    )
}

/// The SHA-256, in hex, of `text`'s body after its frontmatter: the prose
/// a unit of it is written from.
fn prose_digest(text: &str) -> String {
    let body = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(text, |(_, body)| body);
    hex::encode(Sha256::digest(body.as_bytes()))
}

/// A unit declaring `digest`.
fn unit(digest: &str, body: &str) -> String {
    format!("---\nsource_sha256: {digest}\nmodel: test-model\n---\n{body}")
}

/// Every file below `dir` outside `.git`, with its bytes.
fn files_below(dir: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path().strip_prefix(dir).unwrap().to_path_buf(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

/// `instrukt docs units` with `args` in `repo`, with its exit status and
/// standard output.
fn units(env: &TestEnv, repo: &Repo, args: &[&str]) -> (Option<i32>, String) {
    let mut all = vec!["docs", "units"];
    all.extend_from_slice(args);
    let output = env.instrukt(repo.path()).args(all).output().unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

// spec: project/instrukt/spec/feature/publication/compressed-units UC-CU3
#[test]
fn a_pipeline_learns_offline_exactly_which_units_to_write() {
    let mut env = TestEnv::new();
    let home = env.path("no-home");
    let logs = env.path("no-logs");
    env.set_env("INSTRUKT_HOME", home.to_str().unwrap())
        .set_env("INSTRUKT_LOG_DIR", logs.to_str().unwrap());
    let origin = env.git_repo("origin");
    let old_alpha = policy("Alpha", "Do alpha.");
    let alpha = policy("Alpha", "Do alpha, and say so.");
    let beta = policy("Beta", "Do beta.");
    let gone = policy("Gone", "Do what is gone.");
    origin
        .write("instrukt.yaml", LAYER)
        .write(
            "docs/general/index.yaml",
            "expanded:\n  - general/policy/alpha\n  - general/policy/beta\n",
        )
        .write("docs/general/policy/alpha.md", &alpha)
        .write("docs/general/policy/beta.md", &beta)
        .write(
            "docs/_compressed/general/policy/alpha.md",
            &unit(&prose_digest(&old_alpha), "- do alpha\n"),
        )
        .write(
            "docs/_compressed/general/policy/gone.md",
            &unit(&prose_digest(&gone), "- do what is gone\n"),
        );
    origin.commit("documentation");
    let clone = env.clone_repo(origin.path(), "clone");
    let before = files_below(clone.path());

    let (code, text) = units(&env, &clone, &[]);
    assert_eq!(code, Some(3));
    insta::assert_snapshot!("units_name_the_work", text);
    let (code, json) = units(&env, &clone, &["--json"]);
    assert_eq!(code, Some(3));
    insta::assert_snapshot!("units_name_the_work_as_json", json);
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    let digests = |list: &str| -> Vec<(String, String, String)> {
        report[list]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["source"].as_str().unwrap().to_owned(),
                    item["unit"].as_str().unwrap().to_owned(),
                    item["source_sha256"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let stale = digests("stale");
    let missing = digests("missing");
    // Each renewed unit declares the digest of the prose this test wrote.
    let declared = |work: &[(String, String, String)]| -> Vec<String> {
        work.iter().map(|(_, _, digest)| digest.clone()).collect()
    };
    assert_eq!(declared(&stale), [prose_digest(&alpha)]);
    assert_eq!(declared(&missing), [prose_digest(&beta)]);
    assert!(!home.exists(), "the inspection created a home");
    assert!(!logs.exists(), "the inspection wrote a log");
    assert_eq!(files_below(clone.path()), before, "the inspection wrote");

    // Every inlined source gets a current unit at the place named, with the
    // digest named, and the orphan is removed.
    for (_, place, digest) in stale.iter().chain(&missing) {
        clone.write(place, &unit(digest, "- the rule; its reason\n"));
    }
    std::fs::remove_file(clone.path().join("docs/_compressed/general/policy/gone.md")).unwrap();
    let (code, text) = units(&env, &clone, &[]);
    assert_eq!(code, Some(0));
    insta::assert_snapshot!("units_find_nothing_to_compress", text);
}

/// A general principle titled `title`.
fn principle(title: &str) -> String {
    format!(
        "---\ndescription: About {title}.\n---\n\n# {title} — Principle\n\n## Principle\n\n{title}: the principle.\n\n## Rationale\n\n{title}: why.\n\n## Implications\n\n{title}: what follows.\n\n## Tensions\n\n{title}: none.\n"
    )
}

/// `text` with a `Required reads` section naming `read` after its title.
fn requiring(text: &str, read: &str) -> String {
    let (head, rest) = text.split_once(" — Policy\n").unwrap();
    format!("{head} — Policy\n\n## Required reads\n\n- {read}\n{rest}")
}

/// Emitted text with the test's temporary directory shown as `[TMP]`.
fn shown(env: &TestEnv, text: &str) -> String {
    let base = env.path("").display().to_string();
    text.replace(base.trim_end_matches('/'), "[TMP]")
}

/// Each of `paths` with its text, for one snapshot.
fn files_at(env: &TestEnv, paths: &[&std::path::Path]) -> String {
    paths
        .iter()
        .map(|path| {
            format!(
                "== {}\n{}",
                shown(env, &path.display().to_string()),
                shown(env, &std::fs::read_to_string(path).unwrap())
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Configures Claude and Codex through the T3 fake, Codex in `codex_home`.
fn claude_and_codex(env: &TestEnv, codex_home: &std::path::Path) {
    crate::fixtures::t3::FakeT3::create(env);
    common::t3_readiness::configure(
        env,
        vec![
            common::t3_readiness::provider("claudeAgent", "claudeAgent", true),
            common::t3_readiness::provider("codex", "codex", true),
        ],
        serde_json::json!({"codex": {"driver":"codex","config":{"homePath":codex_home}}}),
    );
}

// spec: project/instrukt/spec/feature/publication/compressed-units UC-CU1
#[test]
fn a_developers_agents_receive_the_always_on_doctrine_compressed() {
    let env = TestEnv::new();
    let codex_home = env.path("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    claude_and_codex(&env, &codex_home);
    let care = requiring(&policy("Care", "Take care."), "general/principle/attention");
    let attention = principle("Attention");
    let claude = policy("Claude", "Claude: write plainly.");
    let platform = env.git_repo("platform");
    platform
        .write("instrukt.yaml", "schema_version: 6\nproject: platform\n")
        .write(
            "docs/general/index.yaml",
            "expanded:\n  - general/policy/care\nagents:\n  claudeAgent:\n    - general/policy/agents/claude\n",
        )
        .write("docs/general/policy/care.md", &care)
        .write("docs/general/principle/attention.md", &attention)
        .write("docs/general/policy/agents/claude.md", &claude)
        .write(
            "docs/_compressed/general/policy/care.md",
            &unit(&prose_digest(&care), "- MUST take care -- work lasts\n"),
        )
        .write(
            "docs/_compressed/general/principle/attention.md",
            &unit(&prose_digest(&attention), "- MUST attend -- it is the principle\n"),
        )
        .write(
            "docs/_compressed/general/policy/agents/claude.md",
            &unit(&prose_digest(&claude), "- PREFER plain words -- Claude reads them\n"),
        );
    platform.commit("general");

    env.run(platform.path(), ["sync"]).success();

    insta::assert_snapshot!(
        "global_files_holding_units",
        files_at(
            &env,
            &[
                &env.home().join(".claude/CLAUDE.md"),
                &codex_home.join("AGENTS.md")
            ]
        )
    );
}

/// A test machine whose `PATH` holds the built `instrukt`, as an installed
/// one would: a repository's hook sections call it by name.
fn hooked_machine() -> TestEnv {
    let mut env = TestEnv::new();
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap();
    let path = format!("{}:{}", bin.display(), env.injected("PATH").unwrap());
    env.set_env("PATH", &path);
    env
}

/// Commits everything through the hooks, waits for the post-commit run's
/// sync and returns its exit code.
fn commit_through_hook(env: &TestEnv, repo: &Repo, message: &str) -> Option<i32> {
    repo.commit(message);
    let record = env.finished_hook_run(repo.path(), instrukt_core::repository::Hook::PostCommit);
    let finished = match &record.sync {
        Some(instrukt_core::repository::SyncOutcome::Finished { exit_code, .. }) => {
            Some(*exit_code)
        }
        _ => None,
    };
    assert!(
        finished.is_some(),
        "the hook's sync did not finish: {:?}",
        record.sync
    );
    finished.flatten()
}

// spec: project/instrukt/spec/feature/publication/compressed-units UC-CU2
#[test]
fn a_maintainers_prose_change_reaches_agents_before_its_compression_catches_up() {
    let env = hooked_machine();
    let codex_home = env.path("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    claude_and_codex(&env, &codex_home);
    let repo = env.git_repo("demo");
    repo.write("README.md", "# demo\n");
    repo.commit("the maintainer's first commit");
    env.run(repo.path(), ["init"]).success();
    let care = policy("Care", "Take care.");
    let kind = policy("Kind", "Be kind.");
    let house = requiring(&policy("House", "Keep the house."), "general/policy/care");
    repo.write(
        "docs/general/index.yaml",
        "expanded:\n  - general/policy/care\n  - general/policy/kind\n",
    )
    .write("docs/general/policy/care.md", &care)
    .write("docs/general/policy/kind.md", &kind)
    .write(
        "docs/project/index.yaml",
        "expanded:\n  - project/demo/policy/house\n",
    )
    .write("docs/project/policy/house.md", &house)
    .write(
        "docs/_compressed/general/policy/care.md",
        &unit(&prose_digest(&care), "- MUST take care -- work lasts\n"),
    )
    .write(
        "docs/_compressed/general/policy/kind.md",
        &unit(&prose_digest(&kind), "- MUST be kind -- people work here\n"),
    )
    .write(
        "docs/_compressed/project/policy/house.md",
        &unit(
            &prose_digest(&house),
            "- MUST keep the house -- it holds us\n",
        ),
    );
    assert_eq!(commit_through_hook(&env, &repo, "documentation"), Some(0));
    let claude = env.home().join(".claude/CLAUDE.md");
    let root = repo.path().join("AGENTS.md");

    // The maintainer changes `care` and merges it without renewing its unit.
    let changed = policy("Care", "Take care, and say so.");
    repo.write("docs/general/policy/care.md", &changed);
    assert_eq!(commit_through_hook(&env, &repo, "care says more"), Some(0));
    assert!(repo.git(["status", "--porcelain"]).is_empty());
    env.run(repo.path(), ["docs", "validate"]).success();
    env.run(repo.path(), ["docs", "render", "--check"])
        .success();
    insta::assert_snapshot!(
        "a_changed_source_in_prose",
        files_at(&env, &[&claude, &root])
    );

    // A renewed unit is committed: `care` is held compressed again.
    repo.write(
        "docs/_compressed/general/policy/care.md",
        &unit(
            &prose_digest(&changed),
            "- MUST take care and say so -- work lasts\n",
        ),
    );
    assert_eq!(
        commit_through_hook(&env, &repo, "renew care's unit"),
        Some(0)
    );
    env.run(repo.path(), ["docs", "render", "--check"])
        .success();
    insta::assert_snapshot!(
        "a_renewed_unit_compressed_again",
        files_at(&env, &[&claude, &root])
    );
}
