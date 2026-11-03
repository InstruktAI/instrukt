//! The read side of pending documentation: `instrukt docs index|get` with
//! and without a todo's lens
//! (`docs/local/spec/feature/docs/pending-documentation.md`).

#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/published.rs"]
mod published;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use published::{doc, publish};

fn stdout(env: &TestEnv, assert: &Assert) -> String {
    let base = env.home().parent().unwrap().display().to_string();
    String::from_utf8_lossy(&assert.get_output().stdout).replace(&base, "[TMP]")
}

/// Project `demo` publishing domain `acme` with pending material of two
/// todos, `build-it` and `other-work`: new snippets, a next version, a
/// retirement, and additions and a replacement inside a live snippet.
fn published_demo(env: &TestEnv) -> Repo {
    let demo = env.git_repo("demo");
    demo.write("instrukt.yaml", "schema_version: 1\nproject: demo\n");
    demo.commit("chore: adopt instrukt");
    let live = doc(
        "Live — Spec",
        "A live contract with pending additions.",
        "",
        "\n## What it is\n\nThe current text.\n\n<!-- planned:build-it -->\nAdded by build-it.\n\
         <!-- /planned:build-it -->\n<!-- planned:other-work -->\nAdded by other-work.\n\
         <!-- /planned:other-work -->\n\n## Canonical fields\n\n<!-- planned-change:build-it -->\n\
         - field: current\n<!-- change:build-it -->\n- field: replaced by build-it\n\
         <!-- /planned-change:build-it -->\n",
    );
    let reworked = doc(
        "Reworked — Policy",
        "A live policy with a next version.",
        "",
        "\n## Rules\n\nThe current rules.\n",
    );
    let next = doc(
        "Reworked — Policy",
        "A live policy with a next version.",
        "",
        "\n## Rules\n\nThe rules as build-it delivers them.\n",
    );
    let fresh = doc(
        "Fresh — Spec",
        "A new contract build-it delivers.",
        "delivered_by: [build-it]\n",
        "\n## What it is\n\nNew with build-it.\n",
    );
    let later = doc(
        "Later — Spec",
        "A new contract other-work delivers.",
        "delivered_by: [other-work]\n",
        "\n## What it is\n\nNew with other-work.\n",
    );
    let old = doc(
        "Old — Concept",
        "A concept build-it retires.",
        "",
        "\n## What\n\nOld.\n",
    );
    publish(
        env,
        "demo",
        Some(&demo),
        &[
            ("docs/global/acme/spec/live.md", &live),
            ("docs/global/acme/policy/reworked.md", &reworked),
            ("docs/global/acme/policy/reworked.build-it.md", &next),
            ("docs/global/acme/spec/fresh.md", &fresh),
            ("docs/global/acme/spec/later.md", &later),
            ("docs/global/acme/concept/old.md", &old),
            ("docs/global/acme/concept/old.build-it.md", ""),
        ],
    );
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

    let not_mine = env
        .run(
            demo.path(),
            ["docs", "get", "--target", "build-it", "acme/spec/later"],
        )
        .success();
    insta::assert_snapshot!("other-todos-snippet-withheld", stdout(&env, &not_mine));
}
