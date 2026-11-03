//! `local/projects/instrukt/spec/feature/runner/session-continuity`: the
//! sessions facade links conversations through the runtime mapping, so
//! a developer's clarification reaches the linked conversation as their
//! words; a standing agent whose start was interrupted waits until the
//! developer settles it. The runtime is a fake of the test's own
//! (`support/runtime.rs`).
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/fork.rs"]
mod fork;
#[path = "support/runtime.rs"]
mod runtime;

use common::TestEnv;
use fork::{STANDING_AGENT, push_upstream_change, tick, watched_fork};
use runtime::{FakeRuntime, global_layer, global_layer_with};

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

    // A session id is no command either, quoted or not.
    let hostile_id = "x$(touch${IFS}planted-id)`touch${IFS}planted-tick`'\";";
    rt.open_session(hostile_id);
    env.run(
        &cwd,
        [
            "sessions",
            "send",
            &format!("rt:{hostile_id}"),
            &clarification,
        ],
    )
    .success();
    assert_eq!(rt.sent(hostile_id, 1), clarification);
    let observed = env
        .run(&cwd, ["sessions", "alive", &format!("rt:{hostile_id}")])
        .success();
    insta::assert_snapshot!(
        String::from_utf8_lossy(&observed.get_output().stdout),
        @"needs-input"
    );
    env.run(
        &cwd,
        ["sessions", "conversation", &format!("rt:{hostile_id}")],
    )
    .success();
    for planted in ["planted-id", "planted-tick"] {
        assert!(!cwd.join(planted).exists(), "{planted} was executed");
    }
    for suffix in ["dollar", "tick", "pipe"] {
        let planted = std::path::PathBuf::from(format!("{}-{suffix}", canary.display()));
        assert!(!planted.exists(), "{} was executed", planted.display());
    }
}

/// A machine whose agent order is `alpha` then `beta`, each started by its
/// own agent CLI; `alpha`'s CLI creates sessions but prints no id for
/// them. A fork's upstream changed once its watch was set.
struct Interrupted {
    env: TestEnv,
    alpha: FakeRuntime,
    beta: FakeRuntime,
}

impl Interrupted {
    fn new() -> Self {
        let env = TestEnv::new();
        let alpha = FakeRuntime::new(&env, "alpha", false).starting_agent("alpha");
        let beta = FakeRuntime::new(&env, "beta", false).starting_agent("beta");
        global_layer_with(
            &env,
            "events: true",
            &[&alpha, &beta],
            "agents:\n  order: [alpha, beta]\n  thinking_mode: med\n",
        );
        let upstream = watched_fork(&env);
        let cwd = env.path("");
        tick(&env, &cwd);
        alpha.start_without_printing_ids();
        push_upstream_change(&upstream, "one.md");
        Self { env, alpha, beta }
    }

    fn cwd(&self) -> std::path::PathBuf {
        self.env.path("")
    }

    fn starts(&self) -> (usize, usize) {
        (self.alpha.started_count(), self.beta.started_count())
    }

    /// The developer is told of the interrupted start, and further
    /// wake-ups start nothing for the agent.
    fn told_and_waiting(&self) {
        tick(&self.env, &self.cwd());
        assert_eq!(self.starts(), (1, 0), "the next agent was not tried");
        tick(&self.env, &self.cwd());
        tick(&self.env, &self.cwd());
        assert_eq!(self.starts(), (1, 0), "no new session while unknown");
        insta::allow_duplicates! {
            insta::assert_snapshot!(
                self.alpha.notifications().join("\n"),
                @"instrukt sessions resolve-start fork/procedure/upstream-watch@fork --session <runtime:id> | --none"
            );
        }
    }

    fn resolve(&self, how: &[&str]) {
        let mut args = vec!["sessions", "resolve-start", STANDING_AGENT];
        args.extend_from_slice(how);
        self.env.run(&self.cwd(), args).success();
    }
}

// spec: local/projects/instrukt/spec/feature/runner/session-continuity UC-SS2
#[test]
fn when_no_session_was_created_the_standing_agent_starts_again() {
    let machine = Interrupted::new();
    machine.told_and_waiting();

    machine.resolve(&["--none"]);
    tick(&machine.env, &machine.cwd());

    assert_eq!(machine.starts(), (2, 0), "the agent started again");
}

// spec: local/projects/instrukt/spec/feature/runner/session-continuity UC-SS2
#[test]
fn when_the_created_session_is_named_it_is_bound_and_no_second_one_starts() {
    let machine = Interrupted::new();
    machine.told_and_waiting();

    machine.resolve(&["--session", "alpha:s1"]);
    machine.alpha.set_status("s1", "ended ok");
    tick(&machine.env, &machine.cwd());

    assert_eq!(machine.starts(), (1, 0), "no second session");
    assert_eq!(
        machine.alpha.sent_count("s1"),
        1,
        "the bound session got the change"
    );
}
