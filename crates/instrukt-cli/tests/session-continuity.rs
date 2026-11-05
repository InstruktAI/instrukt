//! `project/instrukt/spec/feature/runner/session-continuity`: the
//! sessions facade links conversations through the runtime mapping, so
//! a developer's clarification reaches the linked conversation as their
//! words; a standing agent whose start was interrupted waits until the
//! developer settles it. A developer who chose T3 Code has agents work in
//! T3 threads through instrukt's shipped entry; a feature the chosen
//! runtime cannot carry is refused; history lookups and endings an agent
//! owes are handed to it as instructions. The runtimes are fakes of the
//! test's own (`support/runtime.rs`, and `support/t3.rs` for T3's `t3`).
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/flavor.rs"]
mod flavor;
#[path = "support/fork.rs"]
mod fork;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/t3.rs"]
mod t3;

use std::path::Path;

use assert_cmd::assert::Assert;
use common::TestEnv;
use flavor::Tracked;
use fork::{STANDING_AGENT, push_upstream_change, tick, watched_fork};
use runtime::{FakeRuntime, global_layer, global_layer_with};
use t3::FakeT3;

/// The exit status of a call that is the agent's to do.
const YOURS_TO_DO: i32 = 3;

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS1
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

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS1
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

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS2
#[test]
fn when_no_session_was_created_the_standing_agent_starts_again() {
    let machine = Interrupted::new();
    machine.told_and_waiting();

    machine.resolve(&["--none"]);
    tick(&machine.env, &machine.cwd());

    assert_eq!(machine.starts(), (2, 0), "the agent started again");
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS2
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

fn stdout(assert: &Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

/// Standard error with the test's temporary directory shown as `[TMP]`.
fn stderr(env: &TestEnv, assert: &Assert) -> String {
    let base = env.path("").display().to_string();
    String::from_utf8_lossy(&assert.get_output().stderr)
        .replace(base.trim_end_matches('/'), "[TMP]")
}

/// The machine's global layer as a developer who chose T3 Code writes it:
/// `runner.runtime: t3` and nothing mapped of their own.
fn chose_t3(env: &TestEnv, modules: &str) {
    write_global(
        env,
        &format!("schema_version: 1\nmodules: {{{modules}}}\nrunner: {{runtime: t3}}\n"),
    );
}

fn write_global(env: &TestEnv, text: &str) {
    let path = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn global_text(env: &TestEnv) -> String {
    std::fs::read_to_string(env.instrukt_home().join("instrukt.yaml")).unwrap()
}

fn global_revision(env: &TestEnv, cwd: &Path) -> String {
    stdout(
        &env.run(cwd, ["config", "get", "--global", "--revision"])
            .success(),
    )
    .trim()
    .to_owned()
}

fn patch_global(env: &TestEnv, cwd: &Path, yaml: &str) -> Assert {
    let revision = global_revision(env, cwd);
    env.run(
        cwd,
        [
            "config",
            "patch",
            "--global",
            "--revision",
            &revision,
            "--yaml",
            yaml,
        ],
    )
}

/// A tracked project on a machine that chose T3 Code, with work tracking
/// switched on there; `t3` is the fake on the test's `PATH`.
fn tracked_on_t3(autostart: bool) -> (Tracked, FakeT3) {
    let mut project = Tracked::new(autostart);
    let t3 = FakeT3::install(&mut project.env);
    chose_t3(&project.env, "todos: true");
    project.github.configure(&project.env);
    (project, t3)
}

fn create_change(project: &Tracked) {
    project.todo([
        "create",
        "tidy-imports",
        "Sort the imports of every module.",
        "--kind",
        "change",
        "--approved",
    ]);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn work_dispatched_on_a_machine_that_chose_t3_runs_in_a_t3_thread_of_the_project() {
    let (project, t3) = tracked_on_t3(true);
    create_change(&project);

    project.tick();

    assert_eq!(t3.threads(), ["thread:1"]);
    assert_eq!(t3.folder("thread:1"), project.repo.path());
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn further_work_reaches_the_same_t3_thread_and_reads_back_through_instrukt() {
    let (project, t3) = tracked_on_t3(true);
    create_change(&project);
    project.tick();
    let cwd = project.repo.path();

    project
        .env
        .run(
            cwd,
            ["sessions", "send", "t3:thread:1", "Also sort the tests."],
        )
        .success();
    let read = project
        .env
        .run(cwd, ["sessions", "conversation", "t3:thread:1"])
        .success();

    assert_eq!(t3.threads().len(), 1, "no second thread");
    assert_eq!(t3.message_count("thread:1"), 2);
    assert_eq!(stdout(&read), t3.messages("thread:1"));
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn a_worktree_request_on_t3_bases_the_thread_on_the_default_branch_and_prints_its_path() {
    let mut env = TestEnv::new();
    let t3 = FakeT3::install(&mut env);
    chose_t3(&env, "events: true");
    let origin = env.bare_remote("demo");
    let seed = env.git_repo("seed");
    seed.write("README.md", "# demo\n");
    seed.commit("the developer's project");
    seed.git(["push", "--quiet", origin.to_str().unwrap(), "main"]);
    let repo = env.clone_repo(&origin, "demo");

    let run = env
        .instrukt(repo.path())
        .args(["sessions", "run", "--worktree", "feature-x", "--brief", "-"])
        .write_stdin("Prepare the workspace.")
        .assert()
        .success();

    assert_eq!(t3.worktree_base("thread:1"), "main");
    assert_eq!(
        stdout(&run),
        format!("t3:thread:1\n{}\n", t3.worktree("thread:1").display())
    );
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn a_t3_threads_status_is_observed_as_one_of_the_five_states() {
    let mut env = TestEnv::new();
    let t3 = FakeT3::install(&mut env);
    chose_t3(&env, "events: true");
    let cwd = env.path("");
    let id = t3.developer_starts_thread();
    let observe = |id: &str| {
        let run = env
            .run(&cwd, ["sessions", "alive", &format!("t3:{id}")])
            .success();
        stdout(&run).trim().to_owned()
    };

    let mut observed = Vec::new();
    for status in [
        "preparing",
        "queued",
        "starting",
        "running",
        "waiting",
        "idle",
        "completed",
        "failed",
        "interrupted",
        "cancelled",
        "rolled_back",
    ] {
        t3.set_status(&id, status);
        observed.push((status.to_owned(), observe(&id)));
    }
    t3.set_status(&id, "running");
    t3.set_pending(&id, 1);
    observed.push(("running, a pending request".into(), observe(&id)));
    t3.archive(&id);
    observed.push(("archived".into(), observe(&id)));
    observed.push(("unknown thread".into(), observe("thread:404")));

    let expected: Vec<(String, String)> = [
        ("preparing", "running"),
        ("queued", "running"),
        ("starting", "running"),
        ("running", "running"),
        ("waiting", "needs-input"),
        ("idle", "ended ok"),
        ("completed", "ended ok"),
        ("failed", "ended failed"),
        ("interrupted", "ended failed"),
        ("cancelled", "ended failed"),
        ("rolled_back", "ended failed"),
        ("running, a pending request", "needs-input"),
        ("archived", "gone"),
        ("unknown thread", "gone"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect();
    assert_eq!(observed, expected);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn a_t3_server_that_cannot_be_reached_yields_no_state_rather_than_gone() {
    let mut env = TestEnv::new();
    let t3 = FakeT3::install(&mut env);
    chose_t3(&env, "events: true");
    let id = t3.developer_starts_thread();
    t3.server_down();

    let run = env
        .run(&env.path(""), ["sessions", "alive", &format!("t3:{id}")])
        .failure();

    assert_eq!(stdout(&run), "unobservable\n");
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS4
#[test]
fn an_agent_in_a_t3_thread_is_told_how_to_search_with_its_own_tools() {
    let mut env = TestEnv::new();
    let t3 = FakeT3::install(&mut env);
    chose_t3(&env, "events: true");
    let caller = t3.developer_starts_thread();
    env.set_env("T3CODE_THREAD_ID", &caller);
    let before = t3.calls().len();

    let run = env
        .run(
            &env.path(""),
            ["sessions", "history", "search", "flaky test"],
        )
        .code(YOURS_TO_DO);

    insta::assert_snapshot!("t3-history-search", stdout(&run));
    assert_eq!(t3.calls().len(), before, "instrukt ran nothing");
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS4
#[test]
fn an_agent_in_a_t3_thread_reads_a_conversation_the_search_named() {
    let mut env = TestEnv::new();
    let t3 = FakeT3::install(&mut env);
    chose_t3(&env, "events: true");
    let earlier = t3.developer_starts_thread();
    let caller = t3.developer_starts_thread();
    env.set_env("T3CODE_THREAD_ID", &caller);

    let run = env
        .run(&env.path(""), ["sessions", "history", "show", &earlier])
        .success();

    assert_eq!(stdout(&run), t3.messages(&earlier));
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS4
#[test]
fn a_runtime_whose_search_is_a_command_is_searched_by_instrukt() {
    let mut env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", false).with_history();
    global_layer(&env, "events: true", &[&rt]);
    rt.open_session("s1");
    rt.open_session("s2");
    rt.developer_types("s1", "the flaky test fails on CI");
    rt.developer_types("s2", "rename the parser");
    env.set_env("RT_SESSION", "s3");

    let run = env
        .run(&env.path(""), ["sessions", "history", "search", "flaky"])
        .success();

    assert_eq!(stdout(&run), "s1\n");
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS4
#[test]
fn a_runtime_leaving_history_search_empty_refuses_naming_the_verb() {
    let mut env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", false);
    global_layer(&env, "events: true", &[&rt]);
    env.set_env("RT_SESSION", "s1");

    let run = env
        .run(&env.path(""), ["sessions", "history", "search", "flaky"])
        .failure();

    insta::assert_snapshot!("history-search-missing", stderr(&env, &run));
    assert!(rt.calls().is_empty(), "no search was made");
}

/// A machine whose only runtime starts, reads and observes sessions but
/// cannot send to them.
const CANNOT_SEND: &str = "schema_version: 1\nruntimes:\n  - name: rt\n    session_id_env: RT_SESSION\n    start: rt start\n    alive: rt alive\n    conversation: rt show\n";

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS5
#[test]
fn switching_on_work_the_runtime_cannot_carry_is_refused_and_changes_nothing() {
    let env = TestEnv::new();
    write_global(&env, CANNOT_SEND);
    let cwd = env.path("");

    let run = patch_global(&env, &cwd, "modules: {todos: true}").failure();

    insta::assert_snapshot!("feature-refused-missing-verb", stderr(&env, &run));
    assert_eq!(global_text(&env), CANNOT_SEND);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS5
#[test]
fn switching_on_work_a_runtime_fully_carries_is_accepted() {
    let env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", false);
    global_layer(&env, "", &[&rt]);
    let cwd = env.path("");

    patch_global(&env, &cwd, "modules: {todos: true, events: true}").success();

    let on = env
        .run(&cwd, ["config", "get", "--global", "modules.todos"])
        .success();
    assert_eq!(stdout(&on).trim(), "true");
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS6
#[test]
fn with_t3_installed_and_no_runtime_chosen_switching_on_work_is_refused_and_none_is_picked() {
    let mut env = TestEnv::new();
    let t3 = FakeT3::install(&mut env);
    write_global(&env, "schema_version: 1\n");
    let cwd = env.path("");

    let run = patch_global(&env, &cwd, "modules: {todos: true, events: true}").failure();

    insta::assert_snapshot!("feature-refused-no-runtime", stderr(&env, &run));
    assert_eq!(global_text(&env), "schema_version: 1\n");
    env.run(&cwd, ["config", "get", "--global", "runtimes"])
        .failure();
    assert!(t3.calls().is_empty(), "T3 was never consulted");
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS7
#[test]
fn on_t3_ending_the_todos_sessions_and_its_own_is_handed_to_the_agent() {
    let (mut project, t3) = tracked_on_t3(false);
    create_change(&project);
    let cwd = project.repo.path().to_path_buf();
    for procedure in ["acme/procedure/prepare-overview", "acme/procedure/plan"] {
        project
            .env
            .run(
                &cwd,
                [
                    "sessions",
                    "run",
                    "--procedure",
                    procedure,
                    "--args",
                    "tidy-imports",
                    "--todo",
                    "tidy-imports",
                ],
            )
            .success();
    }
    // The orchestrator, in thread 1, ends what the todo bound, then itself.
    project.env.set_env("T3CODE_THREAD_ID", "thread:1");
    let before = t3.calls().len();

    let children = project
        .env
        .run(&cwd, ["sessions", "end", "--todo", "tidy-imports"])
        .code(YOURS_TO_DO);
    let own = project
        .env
        .run(&cwd, ["sessions", "end", "self"])
        .code(YOURS_TO_DO);

    insta::assert_snapshot!("t3-end-todo", stdout(&children));
    insta::assert_snapshot!("t3-end-self", stdout(&own));
    assert_eq!(t3.calls().len(), before, "instrukt ran nothing in T3");
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS7
#[test]
fn a_runtime_that_needs_no_ending_is_said_so_and_nothing_more_is_owed() {
    let mut env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", false);
    global_layer(&env, "events: true", &[&rt]);
    rt.open_session("s1");
    env.set_env("RT_SESSION", "s1");

    let run = env
        .run(&env.path(""), ["sessions", "end", "self"])
        .success();

    insta::assert_snapshot!("end-not-needed", stdout(&run));
    assert!(rt.calls().is_empty(), "nothing was done");
}
