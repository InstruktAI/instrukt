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
    assert_eq!(
        stale,
        [(
            "general/policy/alpha".to_owned(),
            "docs/_compressed/general/policy/alpha.md".to_owned(),
            prose_digest(&alpha),
        )]
    );
    assert_eq!(
        missing,
        [(
            "general/policy/beta".to_owned(),
            "docs/_compressed/general/policy/beta.md".to_owned(),
            prose_digest(&beta),
        )]
    );
    assert_eq!(
        report["orphaned"][0]["unit"],
        "docs/_compressed/general/policy/gone.md"
    );
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
