//! `project/instrukt/spec/feature/configuration/interpolation`:
//! `instrukt config get` expands the active runtime's session id at call
//! time. Cross-key and environment references, unset variables and cycles
//! are pure logic, covered by the interpolation unit tests.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;

use common::TestEnv;

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

fn machine(env: &TestEnv) {
    let path = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, RUNTIMES).unwrap();
}

fn get(env: &TestEnv, cwd: &std::path::Path, path: &str) -> String {
    let out = env.run(cwd, ["config", "get", path]).success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

// spec: project/instrukt/spec/feature/configuration/interpolation UC-VI1
#[test]
fn an_agent_receives_the_command_reading_its_own_conversation() {
    let mut env = TestEnv::new();
    machine(&env);
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", "schema_version: 1\nproject: demo\n");

    // Inside a thread-host session hosting an agent CLI, both variables are
    // set; the containing runtime is listed first and wins.
    env.set_env("THREAD_HOST_ID", "th-42");
    env.set_env("AGENT_CLI_SESSION_ID", "cli-7");
    insta::assert_snapshot!(
        get(&env, repo.path(), "runtime.conversation"),
        @"thread-host show th-42 --json"
    );

    env.remove_env("THREAD_HOST_ID");
    insta::assert_snapshot!(
        get(&env, repo.path(), "runtime.conversation"),
        @"agent-cli history cli-7"
    );
    insta::assert_snapshot!(get(&env, repo.path(), "runtime.session_id"), @"cli-7");
    assert_eq!(
        repo.read("instrukt.yaml"),
        "schema_version: 1\nproject: demo\n"
    );
}

// spec: project/instrukt/spec/feature/configuration/interpolation UC-VI1
#[test]
fn outside_every_mapped_runtime_no_command_is_given() {
    let env = TestEnv::new();
    machine(&env);
    let cwd = env.path("");
    let refused = env
        .run(&cwd, ["config", "get", "runtime.conversation"])
        .failure();
    let output = refused.get_output();
    assert!(output.stdout.is_empty(), "no command is given");
    insta::assert_snapshot!(
        String::from_utf8_lossy(&output.stderr),
        @"instrukt: no configured runtime's session variable is set; `runtime.*` has no value here"
    );
}
