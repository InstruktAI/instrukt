//! `local/projects/instrukt/spec/feature/runner/session-continuity`: the
//! sessions facade links conversations through the runtime mapping, so
//! a developer's clarification reaches the linked conversation as their
//! words. The runtime is a fake of the test's own (`support/runtime.rs`).
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/runtime.rs"]
mod runtime;

use common::TestEnv;
use runtime::{FakeRuntime, global_layer};

// spec: local/projects/instrukt/spec/feature/runner/session-continuity UC-SS1
#[test]
fn a_clarification_typed_in_one_conversation_reaches_the_linked_one_as_the_developers_words() {
    let env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", true);
    global_layer(&env, "events: true", &[&rt]);
    rt.open_session("s1");
    rt.open_session("s2");
    let cwd = env.path("");

    let linked = env
        .run(&cwd, ["sessions", "link", "rt:s1", "rt:s2"])
        .success();
    insta::assert_snapshot!(
        String::from_utf8_lossy(&linked.get_output().stdout),
        @"linked rt:s1 and rt:s2"
    );

    rt.developer_types("s1", "Use the staging database, not production.");

    let read = env
        .run(&cwd, ["sessions", "conversation", "rt:s2"])
        .success();
    insta::assert_snapshot!(
        String::from_utf8_lossy(&read.get_output().stdout),
        @"developer (typed in s1): Use the staging database, not production."
    );
}

// spec: local/projects/instrukt/spec/feature/runner/session-continuity UC-SS1
#[test]
fn a_clarification_reaches_the_runtime_byte_for_byte_and_executes_nothing() {
    let env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", true);
    global_layer(&env, "events: true", &[&rt]);
    rt.open_session("s2");
    let cwd = env.path("");
    let canary = env.path("canary");
    let clarification = format!(
        "Use the staging database; not production.\n\
         Don't run $(touch {c}-dollar) or `touch {c}-tick` or \"quoted\" 'text' | cat > {c}-pipe & echo",
        c = canary.display()
    );

    // A collaborator's session opens with the clarification as its brief,
    // and the clarification is sent to the developer's other session.
    let started = env
        .instrukt(&cwd)
        .args(["sessions", "run", "--worktree", "shared", "--brief", "-"])
        .write_stdin(clarification.clone())
        .assert()
        .success();
    insta::assert_snapshot!(
        String::from_utf8_lossy(&started.get_output().stdout)
            .replace(rt.state_dir().to_str().unwrap(), "<sessions>"),
        @r"
    rt:s1
    <sessions>/worktrees/shared
    "
    );
    env.run(&cwd, ["sessions", "send", "rt:s2", &clarification])
        .success();

    assert_eq!(rt.opening("s1"), clarification);
    assert_eq!(rt.sent("s2", 1), clarification);
    for suffix in ["dollar", "tick", "pipe"] {
        let planted = std::path::PathBuf::from(format!("{}-{suffix}", canary.display()));
        assert!(!planted.exists(), "{} was executed", planted.display());
    }
}
