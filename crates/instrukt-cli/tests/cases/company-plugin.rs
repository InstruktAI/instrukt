//! A company's knowledge as a Claude plugin, rendered by `instrukt docs
//! plugin` (`docs/project/spec/feature/publication/company-plugin.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use assert_cmd::assert::Assert;
use serde_json::Value;

use crate::common::{self, Repo, TestEnv};

const SKILL: &str = "skills/acme/SKILL.md";
const CATALOG: &str = ".claude-plugin/marketplace.json";

/// The catalog the repository already holds: another plugin, members
/// instrukt does not know, and a layout instrukt does not write.
const CATALOG_TEXT: &str = "{\"name\":\"acme-tools\",\"owner\":{\"name\":\"Acme Workshop\"},\"metadata\":{\"pluginRoot\":\"./plugins\"},\"plugins\":[{\"name\":\"workshop\",\"source\":\"./workshop\",\"category\":\"tools\"}]}\n";

/// A company's repository without a settings document: an index that
/// expands one document and lists two, each form of reference between
/// them, a document only a required read reaches, and the catalog.
fn company(env: &TestEnv) -> Repo {
    let repo = env.git_repo("acme");
    repo.write(
        "docs/index.yaml",
        "expanded:\n  - company.md\nlisted:\n  - systems.md\n  - teams/suppliers.md\n",
    )
    .write(
        "docs/company.md",
        "---\ndescription: Who Acme is.\n---\n\n# Acme\n\nAcme builds bicycles to order. Its workshop runs on [the systems](systems.md#workshop), and its frames come from @teams/suppliers.md.\n\n## Required reads\n\n- people.md\n\n## See Also\n\n- teams/suppliers.md — who supplies the frames\n",
    )
    .write(
        "docs/people.md",
        "---\ndescription: Who works at Acme.\n---\n\n# People\n\nEveryone at Acme builds; see [the systems][systems].\n\n[systems]: systems.md\n",
    )
    .write(
        "docs/systems.md",
        "---\ndescription: The systems Acme works with.\n---\n\n# Systems\n\nThe workshop runs on ERPNext.\n",
    )
    .write(
        "docs/teams/suppliers.md",
        "---\ndescription: >-\n  Who supplies Acme's frames,\n  and on what terms.\n---\n\n# Suppliers\n\nThe people who order frames are in [People](../people.md).\n\n## Required reads\n\n- contracts.md\n",
    )
    .write(
        "docs/teams/contracts.md",
        "---\ndescription: The terms Acme buys frames on.\n---\n\n# Contracts\n\nNet thirty days.\n",
    )
    .write(CATALOG, CATALOG_TEXT);
    repo
}

/// Runs `docs plugin` for Acme in `repo` with `extra` arguments.
fn plugin(env: &TestEnv, repo: &Repo, extra: &[&str]) -> Assert {
    let mut args = vec!["docs", "plugin", "--key", "acme", "--name", "Acme Bicycles"];
    args.extend_from_slice(extra);
    env.run(repo.path(), args)
}

/// What a command printed, standard output then standard error.
fn printed(assert: &Assert) -> String {
    let output = assert.get_output();
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A change detector over the whole repository.
fn untouched(repo: &Repo) -> common::Snapshot {
    common::Snapshot::over(vec![], vec![repo.path().to_path_buf()])
}

/// The catalog's entry for Acme's plugin, which is the plugin's manifest.
fn entry(repo: &Repo) -> Value {
    let catalog: Value = serde_json::from_str(&repo.read(CATALOG)).unwrap();
    catalog["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "acme")
        .unwrap()
        .clone()
}

/// The plugin's version, as its catalog entry holds it: three numbers.
fn version(repo: &Repo) -> [u64; 3] {
    let numbers: Vec<u64> = entry(repo)["version"]
        .as_str()
        .unwrap()
        .split('.')
        .map(|number| number.parse().unwrap())
        .collect();
    numbers.try_into().unwrap()
}

// spec: project/instrukt/spec/feature/publication/company-plugin UC-CP1
#[test]
fn an_operator_gives_people_who_work_with_claude_the_companys_own_knowledge() {
    let env = TestEnv::new();
    let repo = company(&env);
    let rendered = plugin(&env, &repo, &[]).success();
    insta::assert_snapshot!("company_plugin_rendered", printed(&rendered));
    // The skill carries the passage and each selected document in full; each
    // listed document is a file of its own, named with its description; each
    // reference names where the plugin carries what it names.
    insta::assert_snapshot!("company_plugin_skill", repo.read(SKILL));
    insta::assert_snapshot!(
        "company_plugin_listed_document",
        repo.read("skills/acme/references/teams/suppliers.md")
    );
    // The skill stands at the repository's root: no other folder holds the
    // plugin and no other manifest defines it.
    assert!(!repo.path().join("plugins").exists());

    // The catalog lists the plugin with that skill; every other entry and
    // member keeps its value and its place.
    let before: Value = serde_json::from_str(CATALOG_TEXT).unwrap();
    let after: Value = serde_json::from_str(&repo.read(CATALOG)).unwrap();
    let members =
        |value: &Value| -> Vec<String> { value.as_object().unwrap().keys().cloned().collect() };
    assert_eq!(members(&after), members(&before));
    for member in ["name", "owner", "metadata"] {
        assert_eq!(after[member], before[member], "{member}");
    }
    let plugins = after["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0], before["plugins"][0]);
    insta::assert_snapshot!(
        "company_plugin_entry",
        serde_json::to_string_pretty(&plugins[1]).unwrap()
    );
    assert_eq!(plugins[1]["source"], "./");
    assert_eq!(plugins[1]["skills"], serde_json::json!(["./skills/acme"]));

    // A document refers to something outside the company's own documents:
    // the rendering names the document and the reference, and changes
    // nothing.
    repo.write(
        "docs/systems.md",
        "---\ndescription: The systems Acme works with.\n---\n\n# Systems\n\nThe workshop runs on ERPNext, as [the old notes](../notes/erp.md) say.\n",
    );
    let before = untouched(&repo);
    let refused = plugin(&env, &repo, &[]).code(1);
    insta::assert_snapshot!("company_plugin_reference_outside", printed(&refused));
    assert_eq!(before.changed(), Vec::<std::path::PathBuf>::new());
}

// spec: project/instrukt/spec/feature/publication/company-plugin UC-CP2
#[test]
fn an_operator_publishes_a_new_version_exactly_when_the_knowledge_changed() {
    let env = TestEnv::new();
    let repo = company(&env);
    plugin(&env, &repo, &[]).success();
    // The repository's plugin is already in a release series of its own,
    // as one an earlier renderer left: its entry holds another version.
    let rendered = version(&repo);
    let first = [rendered[0] + 2, rendered[1] + 3, rendered[2] + 4];
    let catalog = repo.read(CATALOG).replace(
        &format!("\"{}.{}.{}\"", rendered[0], rendered[1], rendered[2]),
        &format!("\"{}.{}.{}\"", first[0], first[1], first[2]),
    );
    repo.write(CATALOG, &catalog);
    assert_eq!(version(&repo), first);

    // Nothing changed: no file is changed and the version is as it was.
    let before = untouched(&repo);
    plugin(&env, &repo, &[]).success();
    assert_eq!(before.changed(), Vec::<std::path::PathBuf>::new());
    assert_eq!(version(&repo), first);

    // A document changes: the plugin holds the change, one step higher.
    repo.write(
        "docs/systems.md",
        "---\ndescription: The systems Acme works with.\n---\n\n# Systems\n\nThe workshop runs on ERPNext and orders arrive by email.\n",
    );
    plugin(&env, &repo, &[]).success();
    insta::assert_snapshot!(
        "company_plugin_changed_document",
        repo.read("skills/acme/references/systems.md")
    );
    assert_eq!(version(&repo), [first[0], first[1], first[2] + 1]);

    // Rendering once more changes nothing.
    let before = untouched(&repo);
    plugin(&env, &repo, &[]).success();
    assert_eq!(before.changed(), Vec::<std::path::PathBuf>::new());
}

// spec: project/instrukt/spec/feature/publication/company-plugin UC-CP3
#[test]
fn an_operator_learns_before_a_merge_whether_the_knowledge_still_renders() {
    let env = TestEnv::new();
    let repo = company(&env);
    plugin(&env, &repo, &[]).success();
    repo.commit("the company's knowledge and its plugin");

    // A branch changes a document without rendering the plugin: the
    // validation passes, and nothing changes.
    repo.git(["checkout", "--quiet", "-b", "suppliers"]);
    repo.write(
        "docs/teams/contracts.md",
        "---\ndescription: The terms Acme buys frames on.\n---\n\n# Contracts\n\nNet sixty days.\n",
    );
    repo.commit("longer terms");
    let before = untouched(&repo);
    plugin(&env, &repo, &["--validate"]).success();
    assert_eq!(before.changed(), Vec::<std::path::PathBuf>::new());
    plugin(&env, &repo, &["--check"]).code(1);

    // The change makes a public document depend on an internal one: the
    // validation names both, and nothing changes.
    repo.write(
        "docs/teams/suppliers.md",
        "---\ndescription: Who supplies Acme's frames.\nvisibility: public\n---\n\n# Suppliers\n\nThe people who order frames are in [People](../people.md).\n\n## Required reads\n\n- contracts.md\n",
    );
    let before = untouched(&repo);
    let refused = plugin(&env, &repo, &["--validate"]).code(1);
    insta::assert_snapshot!("company_plugin_public_on_internal", printed(&refused));
    assert_eq!(before.changed(), Vec::<std::path::PathBuf>::new());
}

// spec: project/instrukt/spec/feature/publication/company-plugin UC-CP4
#[test]
fn an_operator_learns_whether_the_held_plugin_matches_its_documents() {
    let env = TestEnv::new();
    let repo = company(&env);
    plugin(&env, &repo, &[]).success();

    // The plugin matches its documents: the check passes, and nothing
    // changes.
    let before = untouched(&repo);
    plugin(&env, &repo, &["--check"]).success();
    assert_eq!(before.changed(), Vec::<std::path::PathBuf>::new());

    // A document changes: the check names each file of the plugin that is
    // behind, and nothing changes.
    repo.write(
        "docs/people.md",
        "---\ndescription: Who works at Acme.\n---\n\n# People\n\nEveryone at Acme builds and rides.\n",
    );
    let before = untouched(&repo);
    let behind = plugin(&env, &repo, &["--check"]).code(1);
    insta::assert_snapshot!("company_plugin_behind", printed(&behind));
    assert_eq!(before.changed(), Vec::<std::path::PathBuf>::new());
}
