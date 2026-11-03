//! `local/projects/instrukt/spec/feature/configuration/interpolation`:
//! `instrukt config get` expands cross-key references, environment
//! variables and the active runtime's session id at call time.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;

use common::TestEnv;
use predicates::str::contains;

const RUNTIMES: &str = r#"schema_version: 1
runtimes:
  - name: thread-host
    session_id_env: THREAD_HOST_ID
    start: thread-host start
    send: thread-host send {runtime.session_id}
    alive: thread-host alive {runtime.session_id}
    conversation: "thread-host show {runtime.session_id} --json"
  - name: agent-cli
    agent: agent-cli
    session_id_env: AGENT_CLI_SESSION_ID
    start: agent-cli
    send: agent-cli send {runtime.session_id}
    alive: agent-cli alive {runtime.session_id}
    conversation: "agent-cli history {runtime.session_id}"
"#;

fn machine(env: &TestEnv, text: &str) {
    let path = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

// spec: local/projects/instrukt/spec/feature/configuration/interpolation UC-VI1
#[test]
fn an_agent_receives_the_command_reading_its_own_conversation() {
    let mut env = TestEnv::new();
    machine(&env, RUNTIMES);
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", "schema_version: 1\nproject: demo\n");

    // Inside a thread-host session hosting an agent CLI, both variables are
    // set; the containing runtime is listed first and wins.
    env.set_env("THREAD_HOST_ID", "th-42");
    env.set_env("AGENT_CLI_SESSION_ID", "cli-7");
    env.run(repo.path(), ["config", "get", "runtime.conversation"])
        .success()
        .stdout("thread-host show th-42 --json\n");

    env.remove_env("THREAD_HOST_ID");
    env.run(repo.path(), ["config", "get", "runtime.conversation"])
        .success()
        .stdout("agent-cli history cli-7\n");
    env.run(repo.path(), ["config", "get", "runtime.session_id"])
        .success()
        .stdout("cli-7\n");
}

// spec: local/projects/instrukt/spec/feature/configuration/interpolation UC-VI1
#[test]
fn outside_every_mapped_runtime_no_command_is_given() {
    let env = TestEnv::new();
    machine(&env, RUNTIMES);
    let cwd = env.path("");
    for path in ["runtime.conversation", "runtime.session_id"] {
        env.run(&cwd, ["config", "get", path])
            .failure()
            .stdout("")
            .stderr(contains("no configured runtime's session variable is set"));
    }
}

// spec: local/projects/instrukt/spec/feature/configuration/interpolation UC-VI1
#[test]
fn values_resolve_across_keys_and_the_environment_on_every_read() {
    let mut env = TestEnv::new();
    machine(&env, RUNTIMES);
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: demo\nactions:\n  \
         base: make -C ${WORK_ROOT}\n  \
         test: \"{actions.base} test {project}\"\n  \
         report: \"{runtime.conversation} > {project}.json\"\n  \
         loop: \"{actions.loop}\"\n",
    );
    env.set_env("WORK_ROOT", "/w");
    env.run(repo.path(), ["config", "get", "actions.test"])
        .success()
        .stdout("make -C /w test demo\n");
    env.set_env("WORK_ROOT", "/other");
    env.run(repo.path(), ["config", "get", "actions.test"])
        .success()
        .stdout("make -C /other test demo\n");

    env.set_env("AGENT_CLI_SESSION_ID", "cli-9");
    env.run(repo.path(), ["config", "get", "actions.report"])
        .success()
        .stdout("agent-cli history cli-9 > demo.json\n");

    env.remove_env("WORK_ROOT");
    env.run(repo.path(), ["config", "get", "actions.base"])
        .failure()
        .stderr(contains("`WORK_ROOT`"));
    env.run(repo.path(), ["config", "get", "actions.loop"])
        .failure()
        .stderr(contains("reference cycle: actions.loop -> actions.loop"));
    assert!(
        repo.read("instrukt.yaml").contains("${WORK_ROOT}"),
        "nothing written back"
    );
}
