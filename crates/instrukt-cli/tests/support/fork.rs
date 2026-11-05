//! A developer's fork watching its upstream, published with a standing
//! `upstream-watch` procedure subscribed to `runner.upstream_changed`.
#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::path::Path;

use crate::common::{Repo, TestEnv};
use crate::documented::{project, snippet, sync};

/// The standing agent, as `resolve-start` and its anomaly name it.
pub const STANDING_AGENT: &str = "fork/procedure/upstream-watch@fork";

const FORK_LAYER: &str = "schema_version: 1\nproject: fork\n\
events:\n  watch:\n    - {remote: upstream, ref: refs/heads/main}\n";

/// A published fork whose watched upstream has one commit; returns the
/// upstream's working clone, to push changes from.
pub fn watched_fork(env: &TestEnv) -> Repo {
    let bare = env.bare_remote("upstream");
    let upstream = env.clone_repo(&bare, "upstream-work");
    upstream.write("README.md", "upstream\n");
    upstream.commit("feat: the upstream's first release");
    upstream.git(["push", "--quiet", "origin", "main"]);
    let fork = project(
        env,
        "fork",
        &[
            ("instrukt.yaml", FORK_LAYER.to_owned()),
            (
                "docs/domain/fork/procedure/upstream-watch.md",
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
    sync(env, &fork);
    upstream
}

/// Pushes an upstream change touching `file`.
pub fn push_upstream_change(upstream: &Repo, file: &str) -> String {
    upstream.write(file, "changed upstream\n");
    let sha = upstream.commit("feat: an upstream change");
    upstream.git(["push", "--quiet", "origin", "main"]);
    sha
}

/// A wake-up, which must succeed; returns its standard error.
pub fn tick(env: &TestEnv, cwd: &Path) -> String {
    let run = env.run(cwd, ["tick"]);
    let output = run.get_output();
    assert!(
        output.status.success(),
        "tick failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}
