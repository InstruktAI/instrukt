//! `local/projects/instrukt/spec/feature/runner/event-activation`: a tick
//! reads the project's watched upstream refs with `git ls-remote`, logs a
//! change as `runner.upstream_changed`, and starts — or wakes — the
//! standing agent of the procedure subscribed to it. The runtime is a fake
//! of the test's own (`support/runtime.rs`).
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/runtime.rs"]
mod runtime;

use common::{Repo, TestEnv};
use documented::{project, snippet, sync};
use runtime::{FakeRuntime, global_layer};

const FORK_LAYER: &str = "schema_version: 1\nproject: fork\n\
publishes:\n  - slug: fork\n    trigger: When working on the fork.\n\
events:\n  watch:\n    - {remote: upstream, ref: refs/heads/main}\n";

/// An upstream repository at `<test>/upstream.git` with one commit.
fn upstream(env: &TestEnv) -> (std::path::PathBuf, Repo) {
    let bare = env.bare_remote("upstream");
    let work = env.clone_repo(&bare, "upstream-work");
    work.write("README.md", "upstream\n");
    work.commit("feat: the upstream's first release");
    work.git(["push", "--quiet", "origin", "main"]);
    (bare, work)
}

fn push_upstream_change(work: &Repo, file: &str) -> String {
    work.write(file, "changed upstream\n");
    let sha = work.commit("feat: an upstream change");
    work.git(["push", "--quiet", "origin", "main"]);
    sha
}

fn tick(env: &TestEnv, cwd: &std::path::Path) {
    let run = env.run(cwd, ["tick"]);
    let output = run.get_output();
    assert!(
        output.status.success(),
        "tick failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// spec: local/projects/instrukt/spec/feature/runner/event-activation UC-EA1
#[test]
fn an_upstream_change_starts_then_wakes_the_subscribed_standing_agent() {
    let env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", false);
    global_layer(&env, "events: true", &[&rt]);
    let (bare, upstream_work) = upstream(&env);
    let fork = project(
        &env,
        "fork",
        &[],
        &[
            ("instrukt.yaml", FORK_LAYER.to_owned()),
            (
                "docs/global/fork/procedure/upstream-watch.md",
                snippet(
                    "procedure",
                    "Upstream Watch",
                    "Report upstream changes and offer a reconciliation that keeps local changes.",
                    "role: upstream-watcher\nsubscriptions:\n  - runner.upstream_changed\n",
                    &[],
                    "Report the upstream change and offer reconciliation.\n",
                ),
            ),
        ],
    );
    fork.git(["remote", "add", "upstream", bare.to_str().unwrap()]);
    sync(&env, &fork);
    let cwd = env.path("");

    // Enabling the watch records where upstream stands; nothing changed yet.
    tick(&env, &cwd);
    assert!(rt.calls().is_empty(), "{:?}", rt.calls());

    let first = push_upstream_change(&upstream_work, "one.md");
    tick(&env, &cwd);
    assert_eq!(rt.calls().len(), 1, "{:?}", rt.calls());
    insta::assert_snapshot!(
        "standing-agent-started",
        rt.opening("s1").replace(&first, "<revision>")
    );

    // Nothing new upstream: the agent is not started or woken again.
    tick(&env, &cwd);
    assert_eq!(rt.calls().len(), 1, "{:?}", rt.calls());

    // The agent's run ended; the next change wakes that same session.
    rt.set_status("s1", "ended ok");
    let second = push_upstream_change(&upstream_work, "two.md");
    tick(&env, &cwd);
    assert_eq!(rt.calls().len(), 2, "{:?}", rt.calls());
    insta::assert_snapshot!(
        "standing-agent-woken",
        rt.sent("s1", 1).replace(&second, "<revision>")
    );
}
