//! The read side of pending documentation: `instrukt docs index|get` with
//! and without a todo's lens
//! (`docs/local/spec/feature/docs/pending-documentation.md`).
//!
//! The home is published by the real `instrukt sync`.

#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/documented.rs"]
mod documented;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use documented::{project, snippet, sync};

fn stdout(env: &TestEnv, assert: &Assert) -> String {
    let base = env.home().parent().unwrap().display().to_string();
    String::from_utf8_lossy(&assert.get_output().stdout).replace(&base, "[TMP]")
}

/// Files a read-time command inside a todo's planned block creates in the
/// reader's working directory when that block is read.
const BUILD_RAN: &str = "build-it-ran";
const OTHER_RAN: &str = "other-work-ran";

/// Project `demo` publishing domain `acme` with pending material of two
/// todos, `build-it` and `other-work`: new snippets, a next version, a
/// retirement, and additions and a replacement inside a live snippet.
fn published_demo(env: &TestEnv) -> Repo {
    let live = snippet(
        "spec",
        "Live",
        "A live contract with pending additions.",
        "",
        &[],
        &format!(
            "The current text.\n\n<!-- planned:build-it -->\nAdded by build-it.\n\
             <!-- @exec: touch {BUILD_RAN} -->\n<!-- /planned:build-it -->\n\
             <!-- planned:other-work -->\nAdded by other-work.\n<!-- @exec: touch {OTHER_RAN} -->\n\
             <!-- /planned:other-work -->\n\n<!-- planned-change:build-it -->\n- field: current\n\
             <!-- change:build-it -->\n- field: replaced by build-it\n\
             <!-- /planned-change:build-it -->\n"
        ),
    );
    let reworked = snippet(
        "policy",
        "Reworked",
        "A live policy with a next version.",
        "",
        &[],
        "The current rules.\n",
    );
    let next = snippet(
        "policy",
        "Reworked",
        "A live policy with a next version.",
        "",
        &[],
        "The rules as build-it delivers them.\n",
    );
    let fresh = snippet(
        "spec",
        "Fresh",
        "A new contract build-it delivers.",
        "delivered_by: [build-it]\n",
        &[],
        "New with build-it.\n",
    );
    let later = snippet(
        "spec",
        "Later",
        "A new contract other-work delivers.",
        "delivered_by: [other-work]\n",
        &[],
        "New with other-work.\n",
    );
    let old = snippet(
        "concept",
        "Old",
        "A concept build-it retires.",
        "",
        &[],
        "Old.\n",
    );
    let demo = project(
        env,
        "demo",
        &["acme"],
        &[
            ("docs/global/acme/spec/live.md", live),
            ("docs/global/acme/policy/reworked.md", reworked),
            ("docs/global/acme/policy/reworked.build-it.md", next),
            ("docs/global/acme/spec/fresh.md", fresh),
            ("docs/global/acme/spec/later.md", later),
            ("docs/global/acme/concept/old.md", old),
            ("docs/global/acme/concept/old.build-it.md", String::new()),
        ],
    );
    sync(env, &demo);
    demo
}

// spec: local/projects/instrukt/spec/feature/docs/pending-documentation UC-PD1
#[test]
fn a_reader_of_the_current_contract_is_told_pending_changes_exist_and_who_owns_them() {
    let env = TestEnv::new();
    let demo = published_demo(&env);

    let listed = env.run(demo.path(), ["docs", "index"]).success();
    insta::assert_snapshot!("index-marks-pending", stdout(&env, &listed));

    let current = env
        .run(
            demo.path(),
            ["docs", "get", "acme/spec/live", "acme/policy/reworked"],
        )
        .success();
    insta::assert_snapshot!("current-with-awareness", stdout(&env, &current));
    // Hidden pending content never runs.
    assert!(!demo.path().join(BUILD_RAN).exists());
    assert!(!demo.path().join(OTHER_RAN).exists());

    let withheld = env
        .run(demo.path(), ["docs", "get", "acme/spec/fresh"])
        .success();
    insta::assert_snapshot!("pending-withheld", stdout(&env, &withheld));
}

// spec: local/projects/instrukt/spec/feature/docs/pending-documentation UC-PD2
#[test]
fn a_builder_reads_the_documentation_as_its_todo_will_deliver_it() {
    let env = TestEnv::new();
    let demo = published_demo(&env);

    let listed = env
        .run(demo.path(), ["docs", "index", "--target", "build-it"])
        .success();
    insta::assert_snapshot!("index-for-a-todo", stdout(&env, &listed));

    let lens = env
        .run(
            demo.path(),
            [
                "docs",
                "get",
                "--target",
                "build-it",
                "acme/spec/fresh",
                "acme/policy/reworked",
                "acme/spec/live",
                "acme/concept/old",
            ],
        )
        .success();
    insta::assert_snapshot!("todo-lens", stdout(&env, &lens));
    // The lens's own material is read, commands included; another todo's
    // is not.
    assert!(demo.path().join(BUILD_RAN).exists());
    assert!(!demo.path().join(OTHER_RAN).exists());

    let not_mine = env
        .run(
            demo.path(),
            ["docs", "get", "--target", "build-it", "acme/spec/later"],
        )
        .success();
    insta::assert_snapshot!("other-todos-snippet-withheld", stdout(&env, &not_mine));
}
