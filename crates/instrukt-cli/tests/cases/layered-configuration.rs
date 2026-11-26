//! `project/instrukt/spec/feature/configuration/layered-configuration`:
//! the global and the project layer, which key lives where, validation and
//! schema versions.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::common;
use crate::fixtures::{documented, mapped, t3};
use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};
use instrukt_core::config::{
    COMPANY_SCHEMA_VERSION, GLOBAL_SCHEMA_VERSION, PROJECT_SCHEMA_VERSION, Revision,
};

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
            instrukt_core::config::PROJECT_SCHEMA_VERSION
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

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL8
#[test]
fn thread_finishing_resolves_from_the_machine_entry() {
    let mut env = TestEnv::new();
    let fake = t3::FakeT3::create(&env);
    write_global(&env, MACHINE);
    // Every read below is made by an identified agent in its own session.
    fake.seed(
        "agent",
        env.home(),
        "agent",
        "Conclude the finished todo's threads.",
    );
    env.set_env("T3CODE_THREAD_ID", "agent");
    let session = fake
        .state_dir()
        .join(format!("{}.json", fake.native_id("agent")));
    let before = std::fs::read(&session).unwrap();
    let body = "<!-- @config: runtime.finish_thread -->\n";
    let repo = documented::project(
        &env,
        "demo",
        &[(
            "docs/domain/acme/procedure/conclude.md",
            documented::snippet(
                "procedure",
                "Conclude",
                "Settle a resting conversation or a finished todo's orchestrator.",
                "",
                &[],
                body,
            ),
        )],
    );
    documented::sync(&env, &repo);
    let rendered = || {
        common::mask_published_commits(&stdout(
            &env,
            &env.run(repo.path(), ["docs", "get", "acme/procedure/conclude"])
                .success(),
        ))
    };

    let shipped = rendered();
    insta::assert_snapshot!("thread-finishing-shipped", shipped);

    repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: {PROJECT_SCHEMA_VERSION}\nproject: demo\nruntimes:\n  t3:\n    finish_thread: Delete the thread and its transcript.\n"
        ),
    );
    let refused = env.run(repo.path(), ["config", "validate"]).failure();
    insta::assert_snapshot!(
        stdout(&env, &refused),
        @"[TMP]/demo/instrukt.yaml: runtimes: not permitted in the project layer (global layer only)"
    );
    assert_eq!(rendered(), shipped);

    // A current machine layer naming an action the runtime entry does not
    // define; an older one would be converted instead.
    write_global(
        &env,
        &format!(
            "schema_version: {GLOBAL_SCHEMA_VERSION}\nrunner: {{runtime: t3}}\nruntimes:\n  t3:\n    retire_thread: Archive the finished thread.\n"
        ),
    );
    let undefined = env
        .run(env.path("").as_path(), ["config", "validate", "--global"])
        .failure();
    // Only the machine layer's own finding; the agents file's findings are not this scenario's.
    let undefined = stdout(&env, &undefined);
    let layer: Vec<_> = undefined
        .lines()
        .filter(|line| line.contains("instrukt.yaml"))
        .collect();
    insta::assert_snapshot!(
        layer.join("\n"),
        @"[TMP]/home/.instrukt/instrukt.yaml: runtimes.t3.retire_thread: unknown key"
    );

    write_global(
        &env,
        &format!(
            "{MACHINE}runtimes:\n  t3:\n    finish_thread: Settle the conversation through the runtime's closest non-destructive operation.\n"
        ),
    );
    env.run(env.path("").as_path(), ["config", "validate", "--global"])
        .success();
    insta::assert_snapshot!("thread-finishing-overridden", rendered());

    assert_eq!(std::fs::read(&session).unwrap(), before);
    assert_eq!(fake.threads(), [fake.native_id("agent")]);
    assert_eq!(fake.calls().len(), 0);
}

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL9
#[test]
fn a_declared_regenerable_location_outside_the_checkout_is_reported_and_withholds_reclamation() {
    let env = TestEnv::new();
    write_global(&env, MACHINE);
    let repo = documented::project(&env, "demo", &[]);
    documented::sync(&env, &repo);
    let declare = |entry: &str| {
        repo.write(
            "instrukt.yaml",
            &format!(
                "schema_version: {PROJECT_SCHEMA_VERSION}\nproject: demo\nregenerable: [{entry}]\n"
            ),
        );
    };
    // The installation findings that name the declaration.
    let reclamation_findings = || {
        let checked = env.run(env.home(), ["install", "--check", "--json"]);
        let report: serde_json::Value =
            serde_json::from_slice(&checked.get_output().stdout).unwrap();
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| shown(&env, finding.as_str().unwrap().as_bytes()))
            .filter(|finding| finding.contains("`regenerable`"))
            .collect::<Vec<_>>()
    };

    declare("build/output");
    env.run(repo.path(), ["config", "validate"]).success();
    assert!(reclamation_findings().is_empty());

    declare("../elsewhere");
    let validated = env.run(repo.path(), ["config", "validate"]).failure();
    insta::assert_snapshot!(stdout(&env, &validated), @"[TMP]/demo/instrukt.yaml: regenerable: the entry `../elsewhere` steps to a parent directory; an entry names a location inside the checkout");
    insta::assert_json_snapshot!(reclamation_findings(), @r#"
    [
      "[TMP]/demo/instrukt.yaml `regenerable`: the entry `../elsewhere` steps to a parent directory; an entry names a location inside the checkout; reclaiming the declared output of this project's idle worktrees is withheld on this computer, and a delivered todo's worktree is still released"
    ]
    "#);
}

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL3
#[test]
fn configuration_written_by_an_earlier_instrukt_validates_and_keeps_working() {
    // Schema version 1 is the first schema; documents an earlier instrukt
    // wrote carry it. Validating the project before anything converts it
    // passes on the converted view and notes the committed file still to be
    // converted, leaving it as committed. Initializing again brings both
    // layers to the current schema with every setting kept, and reads answer
    // as before.
    let env = TestEnv::new();
    // The machine's runtime entry overrides a supported action beside one
    // the runtime entry no longer defines.
    let global = format!(
        "{MACHINE}ticks: 'off'\nmodules:\n  blueprint: true\n  conversations: false\nruntimes:\n  t3:\n    finish_thread: Settle it the machine's way.\n    retire_thread: Archive it.\n"
    );
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

    let validated = env.run(repo.path(), ["config", "validate"]).success();
    insta::assert_snapshot!(stdout(&env, &validated), @"");
    insta::assert_snapshot!(stderr(&env, &validated), @r"
    instrukt: [TMP]/home/.instrukt/instrukt.yaml: schema_version 1 inspected as 7; file migrates on the next writing command
    instrukt: [TMP]/kept/instrukt.yaml: schema_version 1 inspected as 6; file migrates on the next writing command
    ");
    assert_eq!(
        repo.read("instrukt.yaml"),
        layer,
        "the committed file is left as committed"
    );

    env.run(repo.path(), ["config", "init"]).success();
    env.run(repo.path(), ["config", "init", "--global"])
        .success();

    let stored: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&repo.read("instrukt.yaml")).unwrap();
    assert_eq!(
        stored["schema_version"].as_u64(),
        Some(PROJECT_SCHEMA_VERSION)
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
    // The action the runtime entry no longer defines is gone; the
    // supported override is kept as written.
    assert!(machine["runtimes"]["t3"].get("retire_thread").is_none());
    insta::assert_snapshot!(
        read(&["config", "get", "--global", "runtimes.t3.finish_thread"]),
        @"Settle it the machine's way."
    );
    assert_eq!(
        machine["schema_version"].as_u64(),
        Some(GLOBAL_SCHEMA_VERSION)
    );
    env.run(repo.path(), ["config", "validate", "--global"])
        .success();
    env.run(repo.path(), ["config", "init", "--company", "acme"])
        .success();
    env.run(repo.path(), ["config", "validate", "--company", "acme"])
        .success();
    let converted: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(company.join("company.yaml")).unwrap())
            .unwrap();
    assert!(converted["members"]["alex"].get("slack").is_none());
    assert_eq!(
        converted["schema_version"].as_u64(),
        Some(COMPANY_SCHEMA_VERSION)
    );
    insta::assert_snapshot!(read(&["config", "get", "--company", "acme", "members.alex.email"]), @"alex@example.com");
}

/// A project whose layer a newer instrukt wrote; returns it with the
/// layer's bytes.
fn written_by_a_newer_instrukt(env: &TestEnv) -> (Repo, Vec<u8>) {
    let layer = format!(
        "schema_version: {}\nproject: future\nboard: acme/7\n",
        PROJECT_SCHEMA_VERSION + 1
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
        GLOBAL_SCHEMA_VERSION + 1
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
    instrukt: the machine configuration [TMP]/home/.instrukt/instrukt.yaml cannot be read: its schema_version 8 is newer than this instrukt supports (7); run `instrukt config validate --global` to see what to fix
    config validate --global:
    stdout:
    stderr:
    instrukt: [TMP]/home/.instrukt/instrukt.yaml has schema_version 8, newer than this instrukt supports (7); upgrade with `curl -fsSL https://github.com/InstruktAI/instrukt/releases/latest/download/install.sh | sh`
    "
    );
    assert_eq!(std::fs::read(&path).unwrap(), machine_bytes);
}
