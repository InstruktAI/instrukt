//! `project/instrukt/spec/feature/configuration/layered-configuration`:
//! the global and the project layer, which key lives where, validation and
//! schema versions.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::common;
use crate::fixtures::mapped;
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

const MACHINE: &str =
    "schema_version: 1\nrunner: {runtime: t3}\nagents: {order: [claude], tier: standard}\n";

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL1
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

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL2
#[test]
fn a_cloned_project_cannot_declare_what_runs_on_the_machine() {
    let env = TestEnv::new();
    write_global(&env, MACHINE);
    let remote = env.bare_remote("hostile");
    let author = env.git_repo("author");
    author.write(
        "instrukt.yaml",
        &format!(
            "schema_version: {}\nproject: hostile\nrunner: {{runtime: hostile}}\nagents:\n  order: [project-agent]\n  tier: strong\n",
            instrukt_core::config::CURRENT_SCHEMA_VERSION
        ),
    );
    author.commit("feat: configuration");
    author.git(["remote", "add", "origin", remote.to_str().unwrap()]);
    author.git(["push", "--quiet", "origin", "main"]);
    let clone = env.clone_repo(&remote, "clone");

    let validated = env.run(clone.path(), ["config", "validate"]).failure();
    insta::assert_snapshot!("cloned-project-findings", stdout(&env, &validated));

    let read = |args: &[&str]| stdout(&env, &env.run(clone.path(), args).success());
    insta::assert_snapshot!(
        "machine-runtime",
        read(&["config", "get", "runner.runtime"])
    );
    insta::assert_snapshot!("whole-configuration", read(&["config", "get"]));

    env.run(env.path("").as_path(), ["config", "validate", "--global"])
        .success();
}

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL2
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

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL3
#[test]
fn configuration_written_by_an_earlier_instrukt_keeps_working_after_init() {
    // Schema version 1 is the first schema; documents an earlier instrukt
    // wrote carry it. Initializing again brings both layers to the current
    // schema with every setting kept, and reads answer as before.
    let env = TestEnv::new();
    let global =
        format!("{MACHINE}ticks: 'off'\nmodules:\n  blueprint: true\n  conversations: false\n");
    write_global(&env, &global);
    // A company home an earlier instrukt wrote, whose member named a Slack
    // identity.
    let company = env.instrukt_home().join("companies/acme");
    std::fs::create_dir_all(&company).unwrap();
    std::fs::write(
        company.join("instrukt.yaml"),
        "schema_version: 5\ncompany: acme\nmembers:\n  alex: {email: alex@example.com, slack: U1}\n",
    )
    .unwrap();
    let layer = "schema_version: 1\nproject: kept\nboard: acme/7\nactions:\n  test: make test\n";
    let repo = project(&env, "kept", layer);

    env.run(repo.path(), ["config", "init"]).success();
    env.run(repo.path(), ["config", "init", "--global"])
        .success();

    let stored: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&repo.read("instrukt.yaml")).unwrap();
    assert_eq!(
        stored["schema_version"].as_u64(),
        Some(CURRENT_SCHEMA_VERSION)
    );
    assert_eq!(stored["board"].as_str(), Some("acme/7"));
    let read = |args: &[&str]| stdout(&env, &env.run(repo.path(), args).success());
    assert!(stored.get("actions").is_none());
    insta::assert_snapshot!(read(&["config", "get", "board"]), @"acme/7");
    insta::assert_snapshot!(read(&["config", "get", "ticks"]), @"off");
    env.run(repo.path(), ["config", "validate"]).success();
    // The machine's conversations switch is gone, whatever its value; the
    // member keeps their address and loses the Slack identity.
    let machine: serde_yaml_ng::Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(env.instrukt_home().join("instrukt.yaml")).unwrap(),
    )
    .unwrap();
    assert!(machine["modules"].get("conversations").is_none());
    assert_eq!(machine["modules"]["blueprint"].as_bool(), Some(true));
    env.run(repo.path(), ["config", "init", "--company", "acme"])
        .success();
    env.run(repo.path(), ["config", "validate", "--company", "acme"])
        .success();
    let converted: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(company.join("instrukt.yaml")).unwrap())
            .unwrap();
    assert!(converted["members"]["alex"].get("slack").is_none());
    assert_eq!(
        converted["schema_version"].as_u64(),
        Some(CURRENT_SCHEMA_VERSION)
    );
    insta::assert_snapshot!(read(&["config", "get", "--company", "acme", "members.alex.email"]), @"alex@example.com");
}

/// A project whose layer a newer instrukt wrote; returns it with the
/// layer's bytes.
fn written_by_a_newer_instrukt(env: &TestEnv) -> (Repo, Vec<u8>) {
    let layer = format!(
        "schema_version: {}\nproject: future\nboard: acme/7\n",
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

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL4
#[test]
fn reading_configuration_written_by_a_newer_instrukt_is_refused() {
    reading_the_revision_of_configuration_written_by_a_newer_instrukt_is_refused();
    changing_configuration_written_by_a_newer_instrukt_is_refused();
    validating_configuration_written_by_a_newer_instrukt_is_refused();
    let env = TestEnv::new();
    let (repo, bytes) = written_by_a_newer_instrukt(&env);
    let refused = env.run(repo.path(), ["config", "get", "board"]).failure();
    insta::assert_snapshot!("newer-schema-get", stderr(&env, &refused));
    unchanged(&repo, &bytes);
}

fn reading_the_revision_of_configuration_written_by_a_newer_instrukt_is_refused() {
    let env = TestEnv::new();
    let (repo, bytes) = written_by_a_newer_instrukt(&env);
    let refused = env
        .run(repo.path(), ["config", "get", "--revision"])
        .failure();
    insta::assert_snapshot!("newer-schema-revision", stderr(&env, &refused));
    unchanged(&repo, &bytes);
}

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

fn validating_configuration_written_by_a_newer_instrukt_is_refused() {
    let env = TestEnv::new();
    let (repo, bytes) = written_by_a_newer_instrukt(&env);
    let refused = env.run(repo.path(), ["config", "validate"]).failure();
    insta::assert_snapshot!("newer-schema-validate", stderr(&env, &refused));
    unchanged(&repo, &bytes);

    // The machine's layer written by a newer instrukt stops every command:
    // a repository command names the door, and the door says to upgrade.
    let machine = format!(
        "schema_version: {}\nrunner: {{runtime: t3}}\n",
        CURRENT_SCHEMA_VERSION + 1
    );
    write_global(&env, &machine);
    let path = env.instrukt_home().join("instrukt.yaml");
    let machine_bytes = std::fs::read(&path).unwrap();
    // A repository command stops on it with one line naming the door.
    let refused = env.run(repo.path(), ["docs", "render"]).failure().code(1);
    // The door says to upgrade.
    let validated = env
        .run(repo.path(), ["config", "validate", "--global"])
        .failure()
        .code(1);
    insta::assert_snapshot!(
        format!(
            "docs render:\nstdout:\n{}stderr:\n{}config validate --global:\nstdout:\n{}stderr:\n{}",
            stdout(&env, &refused),
            stderr(&env, &refused),
            stdout(&env, &validated),
            stderr(&env, &validated)
        ),
        @r"
    docs render:
    stdout:
    stderr:
    instrukt: the machine configuration [TMP]/home/.instrukt/instrukt.yaml cannot be read: its schema_version 7 is newer than this instrukt supports (6); run `instrukt config validate --global` to see what to fix
    config validate --global:
    stdout:
    stderr:
    instrukt: [TMP]/home/.instrukt/instrukt.yaml has schema_version 7, newer than this instrukt supports (6); upgrade with `curl -fsSL https://github.com/InstruktAI/instrukt/releases/latest/download/install.sh | sh`
    "
    );
    assert_eq!(std::fs::read(&path).unwrap(), machine_bytes);
}

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL7
#[test]
fn a_runner_keeps_dispatching_after_its_clock_file_disappears() {
    for closed_stderr in [false, true] {
        dispatch_after_clock_loss(closed_stderr);
    }
}

fn dispatch_after_clock_loss(closed_stderr: bool) {
    use crate::fixtures::{conversing::Running, documented, flavor::Tracked, t3::FakeT3};
    use std::{
        net::Shutdown, os::fd::OwnedFd, os::unix::net::UnixStream, process::Stdio, time::Duration,
    };

    let p = Tracked::new(true);
    let redis = common::redis::RedisServer::new(&p.env);
    let config = p.env.instrukt_home().join("instrukt.yaml");
    let mut settings: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    settings["modules"]["events"] = true.into();
    settings["redis"] =
        serde_yaml_ng::from_str(&format!("cluster: {{url: '{}'}}", redis.url)).unwrap();
    std::fs::write(&config, serde_yaml_ng::to_string(&settings).unwrap()).unwrap();
    p.repo.write(
        "docs/domain/clockwake/procedure/observe.md",
        &documented::snippet(
            "procedure", "Observe", "Accept the wake event.",
            "role: event-handler\nsubscription_home: project\nsubscriptions: [example.clock_wake]\n",
            &[], "Observe the change.",
        ),
    );
    p.repo.commit("docs: publish clock wake procedure");
    p.repo.git(["push", "--quiet", "origin", "main"]);
    documented::sync(&p.env, &p.repo);
    let runtime = FakeT3::create(&p.env);
    let machine = instrukt_core::machine::machine_id(&instrukt_core::home::Home::at_user_home(
        p.env.home().into(),
    ))
    .unwrap();
    let at = "2030-01-06T00:00:00Z";
    let clock = p.env.path("runner-clock");
    std::fs::write(&clock, at).unwrap();
    let stderr = p.env.path("runner.stderr");
    let mut command = p.env.instrukt_process(p.env.home());
    // The independent GitHub fixture can validate tokens even after the
    // runner's clock file disappears.
    common::github::sync_auth_clock(p.env.home(), Some(at));
    let (sink, peer) = if closed_stderr {
        let (writer, peer) = UnixStream::pair().unwrap();
        (Stdio::from(OwnedFd::from(writer)), Some(peer))
    } else {
        (Stdio::from(std::fs::File::create(&stderr).unwrap()), None)
    };
    let mut runner = Running::spawn(
        command
            .arg("run")
            .env("INSTRUKT_NOW", format!("@{}", clock.display()))
            .env("INSTRUKT_LOG_LEVEL", "DEBUG")
            .stdout(Stdio::null())
            .stderr(sink),
    );
    let log = p
        .env
        .home()
        .join(".local/state/instrukt-ai/instrukt/run.log");
    common::wait_until(
        "the first runner wake finishes",
        Duration::from_secs(20),
        || std::fs::read_to_string(&log).is_ok_and(|text| text.contains("runner_wakeup_done")),
    );
    p.todo([
        "create",
        "clock-fallback",
        "Keep dispatching.",
        "--kind",
        "change",
        "--approved",
    ]);
    assert!(runtime.threads().is_empty());
    common::github::sync_auth_clock(p.env.home(), Some(at));

    if let Some(peer) = peer {
        peer.shutdown(Shutdown::Both).unwrap();
    }
    std::fs::rename(&clock, clock.with_extension("removed")).unwrap();
    let event = serde_json::json!({
        "id":"clock-removed","type":"example.clock_wake","source":machine.to_string(),
        "subject":"todo:clock-fallback","home":"project:demo","home_owner":"unresolved",
        "time":at,"data":{}
    });
    redis::cmd("XADD")
        .arg(instrukt_runner::transport::EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg(event.to_string())
        .query::<String>(&mut redis.connection())
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    common::wait_until(
        "the next wake dispatches a todo with the clock absent",
        Duration::from_secs(30),
        || {
            assert!(
                runner.exited().is_none(),
                "{}",
                std::fs::read_to_string(&stderr).unwrap_or_default()
            );
            assert!(
                std::time::Instant::now() < deadline,
                "runner did not dispatch after clock loss
{}
{}",
                std::fs::read_to_string(&stderr).unwrap_or_default(),
                std::fs::read_to_string(&log)
                    .unwrap_or_default()
                    .lines()
                    .rev()
                    .take(24)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            runtime
                .threads()
                .iter()
                .any(|id| runtime.key(id).starts_with("instrukt:todo:"))
        },
    );
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let wake: String = db
        .query_row("SELECT last_at FROM wakeups", [], |row| row.get(0))
        .unwrap();
    let wake = wake.parse::<jiff::Timestamp>().unwrap();
    assert!(wake > at.parse::<jiff::Timestamp>().unwrap());
    assert!(!clock.exists());
    if closed_stderr {
        assert!(runner.exited().is_none());
        return;
    }
    let diagnostics = std::fs::read_to_string(&stderr).unwrap();
    assert_eq!(
        diagnostics
            .matches("continuing from the last clock reading")
            .count(),
        1,
        "{diagnostics}"
    );
}
