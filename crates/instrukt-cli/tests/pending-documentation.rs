//! The read side of pending documentation: `instrukt docs index|get` with
//! and without a todo's lens
//! (`docs/project/spec/feature/docs/pending-documentation.md`).
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
    common::mask_published_commits(
        &String::from_utf8_lossy(&assert.get_output().stdout).replace(&base, "[TMP]"),
    )
}

/// Configured values inside the todos' planned blocks. `build-it`'s
/// resolves (the GitHub API address); `other-work`'s names nothing configured, so
/// resolving it would be refused with a notice and a failed read.
const BUILD_VALUE: &str = "github.api_url";
const OTHER_VALUE: &str = "no.such.setting";

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
             <!-- @config: {BUILD_VALUE} -->\n<!-- /planned:build-it -->\n\
             <!-- planned:other-work -->\nAdded by other-work.\n<!-- @config: {OTHER_VALUE} -->\n\
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
        // Named only where the body is shown: a withheld snippet's
        // required reads are not named.
        &["- acme/spec/later"],
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
        &[
            ("docs/domain/acme/spec/live.md", live),
            ("docs/domain/acme/policy/reworked.md", reworked),
            ("docs/domain/acme/policy/reworked.build-it.md", next),
            ("docs/domain/acme/spec/fresh.md", fresh),
            ("docs/domain/acme/spec/later.md", later),
            ("docs/domain/acme/concept/old.md", old),
            ("docs/domain/acme/concept/old.build-it.md", String::new()),
        ],
    );
    sync(env, &demo);
    demo
}

// spec: project/instrukt/spec/feature/docs/pending-documentation UC-PD1
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
    // Hidden pending content is never resolved: the read succeeds with no
    // refusal notice (the snapshot).

    let withheld = env
        .run(demo.path(), ["docs", "get", "acme/spec/fresh"])
        .success();
    insta::assert_snapshot!("pending-withheld", stdout(&env, &withheld));
}

// spec: project/instrukt/spec/feature/docs/pending-documentation UC-PD2
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
    // The lens's own material is read and its value resolved; another
    // todo's is never resolved, so the read succeeds without its refusal.

    let not_mine = env
        .run(
            demo.path(),
            ["docs", "get", "--target", "build-it", "acme/spec/later"],
        )
        .success();
    insta::assert_snapshot!("other-todos-snippet-withheld", stdout(&env, &not_mine));
}
