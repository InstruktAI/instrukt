//! `project/instrukt/spec/feature/runner/work-dispatch`: a wake-up
//! dispatches eligible work into the procedure declaring its lane's
//! orchestrator role when the project chose automatic pickup, and leaves
//! it for a deliberate start otherwise; with both the todos and events
//! modules off, the runner's commands refuse and name both switches.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::common;
use crate::fixtures::flavor;
use crate::fixtures::runtime;
use crate::fixtures::work;
use common::TestEnv;
use flavor::Tracked;
use runtime::{FakeRuntime, global_layer};

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn with_automatic_pickup_eligible_work_is_dispatched_to_an_agent() {
    let project = Tracked::new(true);
    project.todo([
        "create",
        "tidy-imports",
        "Sort the imports of every module.",
        "--kind",
        "change",
        "--approved",
    ]);

    project.tick();

    assert_eq!(project.rt.started_count(), 1, "{:?}", project.rt.calls());
    insta::assert_snapshot!("dispatched-opening", project.rt.opening("s1"));
    insta::assert_snapshot!(
        "dispatched-record",
        work::redact(&project.todo(["show", "tidy-imports"]))
    );
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

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn a_project_choosing_automatic_pickup_in_its_own_layer_is_dispatched_by_the_machines_wake_up() {
    // The machine's global layer leaves work tracking off; only the
    // project's own layer switches it on.
    let project = Tracked::with_global_modules(true, "todos: false");
    create_change(&project);

    project.tick();

    assert_eq!(project.rt.started_count(), 1, "{:?}", project.rt.calls());
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD3
#[test]
fn todos_are_picked_up_when_a_projects_event_responses_are_not_admitted() {
    // The project switches event responses on in its own layer; the
    // machine has no event transport, so only its todos are admitted.
    let project = Tracked::with_project_modules(true, "todos: true, events: true");
    create_change(&project);

    project.tick();

    assert_eq!(project.rt.started_count(), 1, "{:?}", project.rt.calls());
    // The computer's status names the project's event responses as not
    // admitted, with the setting they need.
    let out = project
        .env
        .run(project.env.home(), ["events", "status", "--json"])
        .success();
    let status: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let admission = serde_json::to_string_pretty(&status["admission"])
        .unwrap()
        .replace(&project.repo.path().display().to_string(), "[REPO]");
    insta::assert_snapshot!("events-not-admitted-in-status", admission);
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn work_claimed_while_github_is_unreachable_is_dispatched_at_once() {
    let mut project = Tracked::new(true);
    create_change(&project);

    project.github.stop();
    project.tick();
    assert_eq!(project.rt.started_count(), 1, "{:?}", project.rt.calls());

    project.github.resume();
    project.tick();
    assert_eq!(project.rt.started_count(), 1, "{:?}", project.rt.calls());
    insta::assert_snapshot!(
        "dispatched-while-github-unreachable",
        work::redact(&project.todo(["show", "tidy-imports"]))
    );
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn with_deliberate_pickup_eligible_work_awaits_the_developers_start() {
    let project = Tracked::new(false);
    project.todo([
        "create",
        "tidy-imports",
        "Sort the imports of every module.",
        "--kind",
        "change",
        "--approved",
    ]);

    project.tick();

    assert!(
        project.rt.thread_calls().is_empty(),
        "{:?}",
        project.rt.calls()
    );
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD2
#[test]
fn with_work_tracking_and_event_responses_off_the_runner_refuses_and_names_both() {
    let mut env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", false);
    global_layer(&env, "todos: false, events: false, history: false", &[&rt]);
    let repo = env.git_repo("demo");
    repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: {}\nproject: demo\n",
            instrukt_core::config::CURRENT_SCHEMA_VERSION
        ),
    );
    repo.commit("the developer's project");
    for args in [vec!["tick"], vec!["run"], vec!["sessions", "list"]] {
        let refused = env.run(repo.path(), &args).failure();
        insta::assert_snapshot!(
            format!("refused-{}", args[0]),
            String::from_utf8_lossy(&refused.get_output().stderr)
        );
    }
    assert!(rt.calls().is_empty(), "{:?}", rt.calls());

    // A company declaring a connector: the runner runs with work tracking
    // and event responses off; a tick still serves only those two and keeps
    // refusing.
    let slack = crate::fixtures::slack::FakeSlack::start();
    env.set_env("INSTRUKT_SLACK_API_URL", &slack.api_url);
    let _source = crate::fixtures::history::T3Store::new(&env);
    crate::fixtures::conversing::company_home(
        &env,
        "acme",
        crate::fixtures::slack::APP_TOKEN,
        crate::fixtures::slack::BOT_TOKEN,
        &crate::fixtures::conversing::slack_settings("{}", ""),
    );
    crate::fixtures::conversing::publish_company_procedure(&env);
    let refused = env.run(repo.path(), ["tick"]).failure();
    insta::assert_snapshot!(
        "refused-tick",
        String::from_utf8_lossy(&refused.get_output().stderr)
    );
    let _runner = crate::fixtures::conversing::Running::spawn(
        env.instrukt_process(repo.path())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
    );
    crate::fixtures::conversing::wait("the runner's connector connection", || {
        slack.connections() > 0
    });
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD2
#[test]
fn whether_run_has_a_company_to_serve_is_what_admission_decides() {
    let mut env = TestEnv::new();
    let slack = crate::fixtures::slack::FakeSlack::start();
    env.set_env("INSTRUKT_SLACK_API_URL", &slack.api_url);
    let _source = crate::fixtures::history::T3Store::new(&env);
    // Every module is off: only a company's connector gives `instrukt run`
    // something to serve.
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 1\nrunner: {runtime: t3}\nmodules: {todos: false, events: false, history: false}\n",
    )
    .unwrap();
    crate::fixtures::conversing::company_home(
        &env,
        "acme",
        crate::fixtures::slack::APP_TOKEN,
        crate::fixtures::slack::BOT_TOKEN,
        &crate::fixtures::conversing::slack_settings("{}", ""),
    );
    crate::fixtures::conversing::publish_company_procedure(&env);
    // Whether it has one is what the runner's admission decides for the
    // same files. A machine layer admission refuses is the command's error,
    // named, as the runner refuses to start on it, even where a module is on
    // that would otherwise let the command go on: `instrukt sessions`, which
    // asks admission only for this, says so.
    let machine = env.instrukt_home().join("instrukt.yaml");
    let valid_machine = std::fs::read_to_string(&machine).unwrap();
    std::fs::write(
        &machine,
        format!(
            "schema_version: {}\nrunner: {{runtime: t3}}\nmodules: {{todos: true}}\nbogus: 1\n",
            instrukt_core::config::CURRENT_SCHEMA_VERSION
        ),
    )
    .unwrap();
    let refused = env.run(env.home(), ["sessions", "list"]).failure().code(1);
    insta::assert_snapshot!(
        "unadmitted-run",
        String::from_utf8_lossy(&refused.get_output().stderr)
            .replace(&env.home().display().to_string(), "[HOME]")
    );
    std::fs::write(&machine, valid_machine).unwrap();
    // A company document admission refuses leaves that company out: there is
    // nothing to serve, as the runner would host nothing.
    let document = env.instrukt_home().join("companies/acme/instrukt.yaml");
    let valid = std::fs::read_to_string(&document).unwrap();
    std::fs::write(&document, format!("{valid}bogus: 1\n")).unwrap();
    let refused = env.run(env.home(), ["run"]).failure();
    insta::assert_snapshot!(
        "refused-run",
        String::from_utf8_lossy(&refused.get_output().stderr)
    );
    // Admitted, the company is served: the runner connects its connector.
    std::fs::write(&document, valid).unwrap();
    let _runner = crate::fixtures::conversing::Running::spawn(
        env.instrukt_process(env.home())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
    );
    crate::fixtures::conversing::wait("the runner's connector connection", || {
        slack.connections() > 0
    });
}

fn selection_config(project: &Tracked, settings: &str) {
    common::agent_profiles(&project.env, settings);
}
fn chosen(project: &Tracked) -> String {
    let result = project
        .env
        .run(project.repo.path(), ["sessions", "list", "--all", "--json"])
        .success();
    work::redact(
        &String::from_utf8(result.get_output().stdout.clone())
            .unwrap()
            .replace(
                project.env.path("").to_str().unwrap().trim_end_matches('/'),
                "[TMP]",
            ),
    )
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn sequential_selection_exhausts_the_first_family_chain_before_trying_another_family() {
    let p = Tracked::new(true);
    selection_config(
        &p,
        "default: {order: [claude, codex], tier: strong}\nprofiles:\n  claude:\n    strong: [{model: claude-sonnet-5-5, effort: high}, {model: claude-opus-5-5, effort: high}]\n",
    );
    p.env
        .run(
            p.repo.path(),
            [
                "agents",
                "status",
                "claude",
                "unavailable",
                "--model",
                "claude-sonnet-5-5",
                "--until",
                "2099-01-01T00:00:00Z",
                "--reason",
                "quota",
            ],
        )
        .success();
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    insta::assert_snapshot!("sequential-family-chain", chosen(&p));
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn exhausted_family_capacity_falls_back_to_the_next_family() {
    let p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude, codex], tier: strong}\n");
    p.env
        .run(
            p.repo.path(),
            [
                "agents",
                "status",
                "claude",
                "unavailable",
                "--reason",
                "capacity",
            ],
        )
        .success();
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    insta::assert_snapshot!("next-family-selection", chosen(&p));
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn an_expired_model_mark_lapses_before_dispatch() {
    let mut p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude, codex], tier: strong}\n");
    p.env.set_env("INSTRUKT_NOW", "2030-01-01T00:00:00Z");
    p.env
        .run(
            p.repo.path(),
            [
                "agents",
                "status",
                "claude",
                "unavailable",
                "--model",
                "claude-opus-5-5",
                "--until",
                "2030-01-02T00:00:00Z",
                "--reason",
                "quota",
            ],
        )
        .success();
    p.env.set_env("INSTRUKT_NOW", "2030-01-02T00:00:00Z");
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    insta::assert_snapshot!("expired-model-selection", chosen(&p));
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn a_task_preference_and_tier_override_the_procedure_and_global_preferences() {
    let mut p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude], tier: light}\n");
    p.authorized_task(
        "selected-task",
        "2030-01-06T09:00:00Z",
        &["--agent", "antigravity", "--tier", "strong"],
    );
    p.tick_at("2030-01-06T09:00:01Z");
    assert_eq!(p.rt.started_count(), 1);
    insta::assert_snapshot!("task-selection-precedence", chosen(&p));
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn runner_start_passes_the_resolved_provider_and_model_as_cli_arguments() {
    let p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude], tier: standard}\n");
    create_change(&p);
    p.tick();
    let command =
        p.rt.calls()
            .into_iter()
            .find(|line| line.starts_with("thread start "))
            .unwrap();
    insta::assert_snapshot!(
        "explicit-native-start-arguments",
        work::redact(&command.replace(
            p.env.path("").to_str().unwrap().trim_end_matches('/'),
            "[TMP]"
        ))
    );
}

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL1
#[test]
fn a_global_tier_override_replaces_that_chain_and_retains_other_shipped_tiers() {
    let p = Tracked::new(false);
    selection_config(
        &p,
        "profiles:\n  codex:\n    light: [{model: gpt-6.1-sol, effort: xhigh}]\n",
    );
    p.env
        .run(p.repo.path(), ["config", "validate", "--global"])
        .success();
    let result = p
        .env
        .run(p.repo.path(), ["config", "show", "agents"])
        .success();
    insta::assert_snapshot!(
        "chain-entry-override",
        serde_json::to_string_pretty(&serde_json::from_slice::<serde_json::Value>(&result.get_output().stdout).unwrap()["profiles"]["codex"]).unwrap()
    );
}

// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL1
#[test]
fn configured_unknown_option_values_are_reported_as_validation_findings() {
    let p = Tracked::new(false);
    selection_config(
        &p,
        "profiles:\n  codex:\n    light: [{model: gpt-6.1-sol, effort: high, serviceTier: imaginary}]\n",
    );
    let result = p
        .env
        .run(p.repo.path(), ["config", "validate", "--global"])
        .failure();
    insta::assert_snapshot!(
        "unknown-option-finding",
        String::from_utf8_lossy(&result.get_output().stdout).replace(
            p.env.path("").to_str().unwrap().trim_end_matches('/'),
            "[TMP]"
        )
    );
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn the_procedure_role_selects_its_configured_profile_order_and_tier() {
    let p = Tracked::new(true);
    selection_config(
        &p,
        "default: {order: [claude], tier: light}\nroles: {todo-orchestrator: {class: agent, can: [read, worker, record, orchestrate], order: [antigravity], tier: strong}}\n",
    );
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    insta::assert_snapshot!("procedure-selection-precedence", chosen(&p));
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn capacity_returning_after_total_exhaustion_starts_without_a_phantom_pending_request() {
    let p = Tracked::new(true);
    selection_config(&p, "default: {order: [codex], tier: strong}\n");
    p.env
        .run(
            p.repo.path(),
            [
                "agents",
                "status",
                "codex",
                "unavailable",
                "--reason",
                "capacity",
            ],
        )
        .success();
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 0);
    p.env
        .run(p.repo.path(), ["agents", "status", "codex", "available"])
        .success();
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    insta::assert_snapshot!("capacity-returned-selection", chosen(&p));
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS13
#[test]
fn a_captured_native_start_retries_the_same_selection_without_an_operator() {
    let p = Tracked::new(true);
    selection_config(
        &p,
        "default: {order: [codex, claude], tier: strong}\nprofiles:\n  codex:\n    strong: [{model: gpt-6.2-astra, effort: high}, {model: gpt-6.1-sol, effort: high}]\n",
    );
    let fake = runtime::FakeT3::create(&p.env);
    fake.flag("start-failure", true);
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 0);
    p.env
        .run(
            p.repo.path(),
            [
                "agents",
                "status",
                "codex",
                "unavailable",
                "--model",
                "gpt-6.2-astra",
                "--until",
                "2099-01-01T00:00:00Z",
                "--reason",
                "quota",
            ],
        )
        .success();
    fake.flag("start-failure", false);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    assert_eq!(fake.provider("s1").as_str(), "codex");
    let sessions: serde_json::Value = serde_json::from_str(&chosen(&p)).unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!(sessions[0]["model"], @r#""gpt-6.2-astra""#);
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn an_unready_profile_is_skipped_before_any_thread_is_created() {
    let p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude, codex], tier: strong}\n");
    common::t3_readiness::update(&p.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == "claudeAgent" {
                provider["installed"] = false.into();
            }
        }
    });
    create_change(&p);
    p.tick();
    let threads: serde_json::Value = serde_json::from_str(&chosen(&p)).unwrap();
    assert_eq!(threads.as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!("ready-profile-fallback", threads[0]["provider"]);
    assert!(p.env.path("t3-readiness/request.json").exists());
    assert!(
        p.rt.calls()
            .iter()
            .any(|line| line.starts_with("auth session revoke "))
    );
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn a_full_usage_window_marks_that_profiles_models_with_expiry() {
    let p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude, codex], tier: strong}\n");
    common::t3_readiness::update(&p.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == "claudeAgent" {
                provider["usageLimits"] = serde_json::json!({"checkedAt":"2030-01-01T00:00:00Z","windows":[{"id":"weekly","kind":"weekly","label":"Weekly","usedPercent":100,"resetsAt":"2099-01-01T00:00:00Z"}]});
            }
        }
    });
    create_change(&p);
    p.tick();
    let marks = p
        .env
        .run(p.repo.path(), ["agents", "availability"])
        .success();
    let marks: serde_json::Value = serde_json::from_slice(&marks.get_output().stdout).unwrap();
    assert!(!marks.as_array().unwrap().is_empty());
    insta::assert_json_snapshot!("snapshot-quota-marks", marks);
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn a_full_window_scoped_to_one_model_marks_only_that_models_family() {
    let p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude, codex], tier: strong}\n");
    // Fable's seven-day window is full; the session window for every model
    // is not.
    common::t3_readiness::update(&p.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == "claudeAgent" {
                provider["usageLimits"] = serde_json::json!({"checkedAt":"2030-01-01T00:00:00Z","windows":[
                    {"id":"five_hour","kind":"session","label":"Session","usedPercent":40,"resetsAt":"2099-01-01T00:00:00Z"},
                    {"id":"seven_day_fable","kind":"weekly","label":"Weekly · Fable","usedPercent":100,"resetsAt":"2099-01-01T00:00:00Z"}
                ]});
            }
        }
    });
    create_change(&p);
    p.tick();
    let marks = p
        .env
        .run(p.repo.path(), ["agents", "availability"])
        .success();
    let marks: serde_json::Value = serde_json::from_slice(&marks.get_output().stdout).unwrap();
    // Only Fable's model is unavailable: claude-opus-5-5, claude-sonnet-5-5
    // and claude-haiku-5-5 carry no mark.
    insta::assert_json_snapshot!(marks, @r#"
    [
      {
        "profile": "claude",
        "model": "claude-fable-5-2",
        "status": "unavailable",
        "until": "2099-01-01T00:00:00Z",
        "reason": "quota: window seven_day_fable at 100%"
      }
    ]
    "#);
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn automatic_quota_marks_follow_the_latest_snapshot() {
    let p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude, codex], tier: strong}\n");
    let windows = |windows: serde_json::Value| {
        common::t3_readiness::update(&p.env, |snapshot| {
            for provider in snapshot["providers"].as_array_mut().unwrap() {
                if provider["instanceId"] == "claudeAgent" {
                    provider["usageLimits"] =
                        serde_json::json!({"checkedAt":"2030-01-01T00:00:00Z","windows":windows});
                }
            }
        });
    };
    let marks = || {
        let out = p
            .env
            .run(p.repo.path(), ["agents", "availability"])
            .success();
        serde_json::from_slice::<serde_json::Value>(&out.get_output().stdout).unwrap()
    };
    // A snapshot marks every model of the profile.
    windows(serde_json::json!([
        {"id":"weekly","kind":"weekly","label":"Weekly","usedPercent":100,"resetsAt":"2099-01-01T00:00:00Z"}
    ]));
    create_change(&p);
    p.tick();
    assert_eq!(marks().as_array().unwrap().len(), 4);
    // The operator restricts one of them by hand meanwhile.
    p.env
        .run(
            p.repo.path(),
            [
                "agents",
                "status",
                "claude",
                "unavailable",
                "--model",
                "claude-opus-5-5",
                "--until",
                "2099-01-01T00:00:00Z",
                "--reason",
                "operator restriction",
            ],
        )
        .success();
    // The next snapshot has only Fable's window full: only Fable keeps its
    // automatic mark, and the manual restriction stays as set.
    windows(serde_json::json!([
        {"id":"weekly","kind":"weekly","label":"Weekly","usedPercent":40,"resetsAt":"2099-01-01T00:00:00Z"},
        {"id":"seven_day_fable","kind":"weekly","label":"Weekly · Fable","usedPercent":100,"resetsAt":"2099-01-01T00:00:00Z"}
    ]));
    p.tick();
    insta::assert_json_snapshot!("quota-marks-follow-the-snapshot", marks());
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn compatible_protocol_and_shape_admit_different_release_diagnostics() {
    let p = Tracked::new(true);
    let file = common::t3_readiness::state_file(&p.env);
    let mut state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    state["serverVersion"] = "99.0.0".into();
    std::fs::write(file, state.to_string()).unwrap();
    std::fs::write(p.env.path("t3-state/release"), "t3 v99.0.0-compatible").unwrap();
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    assert!(
        p.rt.calls()
            .iter()
            .any(|line| line.starts_with("thread start "))
    );
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn unreported_models_fall_through_the_same_ready_profiles_chain() {
    let p = Tracked::new(true);
    selection_config(
        &p,
        "default: {order: [claude, codex], tier: strong}\nprofiles:\n  claude:\n    strong: [{model: not-reported, effort: high}, {model: claude-opus-5-5, effort: high}]\n",
    );
    create_change(&p);
    p.tick();
    let threads: serde_json::Value = serde_json::from_str(&chosen(&p)).unwrap();
    assert_eq!(threads.as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!("reported-chain-fallback", threads[0]["model"]);
    let calls =
        p.rt.calls()
            .into_iter()
            .filter(|line| line.starts_with("auth ") || line == "--version")
            .collect::<Vec<_>>();
    insta::assert_json_snapshot!("readiness-command-contract", calls);
    let request: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(p.env.path("t3-readiness/request.json")).unwrap(),
    )
    .unwrap();
    insta::assert_json_snapshot!("readiness-rpc-envelope", request);
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn native_auto_selection_does_not_require_a_reported_model_slug() {
    let p = Tracked::new(true);
    selection_config(
        &p,
        "default: {order: [claude], tier: strong}\nprofiles:\n  claude:\n    strong: [{model: auto, effort: high}]\n",
    );
    common::t3_readiness::update(&p.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == "claudeAgent" {
                provider["models"] = serde_json::json!([]);
            }
        }
    });
    create_change(&p);
    p.tick();
    let threads: serde_json::Value = serde_json::from_str(&chosen(&p)).unwrap();
    assert_eq!(threads.as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!("native-auto-model", threads[0]["model"]);
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn quota_without_a_reset_records_one_hour_and_preserves_a_manual_restriction() {
    let mut p = Tracked::new(true);
    p.env.set_env("INSTRUKT_NOW", "2030-01-01T00:00:00Z");
    selection_config(&p, "default: {order: [claude, codex], tier: strong}\n");
    p.env
        .run(
            p.repo.path(),
            [
                "agents",
                "status",
                "claude",
                "unavailable",
                "--model",
                "claude-opus-5-5",
                "--until",
                "2099-01-01T00:00:00Z",
                "--reason",
                "operator restriction",
            ],
        )
        .success();
    common::t3_readiness::update(&p.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == "claudeAgent" {
                provider["usageLimits"] = serde_json::json!({"checkedAt":"2030-01-01T00:00:00Z","windows":[{"id":"session","kind":"session","label":"Session","usedPercent":100}]});
            }
        }
    });
    create_change(&p);
    p.tick();
    let marks = p
        .env
        .run(p.repo.path(), ["agents", "availability"])
        .success();
    let marks: serde_json::Value = serde_json::from_slice(&marks.get_output().stdout).unwrap();
    assert_eq!(marks.as_array().unwrap().len(), 5);
    insta::assert_json_snapshot!("quota-default-and-manual", marks);
}

/// Whether `instrukt events status` shows the readiness refresh failing.
fn readiness_failing(p: &Tracked) -> bool {
    let out = p
        .env
        .run(p.env.home(), ["events", "status", "--json"])
        .success();
    let status: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    status["workers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["worker"] == "readiness")
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn malformed_readiness_is_a_failed_refresh_the_start_proceeds_and_its_token_is_revoked() {
    let p = Tracked::new(true);
    std::fs::write(p.env.path("t3-readiness/malformed"), "").unwrap();
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    // The tick's readiness ended with it: no worker is shown.
    assert!(!readiness_failing(&p));
    assert!(
        p.rt.calls()
            .iter()
            .any(|line| line.starts_with("auth session revoke "))
    );
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn a_failed_readiness_token_revocation_is_logged_and_the_start_proceeds() {
    let p = Tracked::new(true);
    runtime::FakeT3::create(&p.env).flag("revoke-failure", true);
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    assert!(!readiness_failing(&p));
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn an_incomplete_claude_authentication_probe_skips_its_bundled_models() {
    let p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude, codex], tier: strong}\n");
    common::t3_readiness::update(&p.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == "claudeAgent" {
                provider["status"] = "warning".into();
                provider["auth"]["status"] = "unknown".into();
            }
        }
    });
    create_change(&p);
    p.tick();
    let threads: serde_json::Value = serde_json::from_str(&chosen(&p)).unwrap();
    insta::assert_json_snapshot!("incomplete-claude-probe-fallback", threads[0]["provider"]);
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn a_successfully_probed_codex_custom_configuration_needs_no_openai_login() {
    let p = Tracked::new(true);
    selection_config(&p, "default: {order: [codex], tier: strong}\n");
    common::t3_readiness::update(&p.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == "codex" {
                provider["status"] = "ready".into();
                provider["auth"]["status"] = "unknown".into();
            }
        }
    });
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn compatible_readiness_releases_do_not_block_start() {
    let p = Tracked::new(true);
    let file = common::t3_readiness::state_file(&p.env);
    let mut state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    state["serverVersion"] = "future-compatible".into();
    std::fs::write(file, state.to_string()).unwrap();
    runtime::FakeT3::create(&p.env).release("future-cli");
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn an_unknown_native_descriptor_keeps_start_pending_until_supported() {
    let p = Tracked::new(true);
    let file = common::t3_readiness::state_file(&p.env);
    let mut state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    state["version"] = 2.into();
    std::fs::write(&file, state.to_string()).unwrap();
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 0);
    state["version"] = 1.into();
    std::fs::write(&file, state.to_string()).unwrap();
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    // The tick's readiness ended with it: no worker is shown.
    assert!(!readiness_failing(&p));
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn a_reused_snapshot_never_renews_a_lapsed_quota_restriction() {
    let mut p = Tracked::new(true);
    selection_config(&p, "default: {order: [claude, codex], tier: strong}\n");
    // The operator's clock, replaced whole at each move.
    let clock = p.env.path("clock");
    let mut now: jiff::Timestamp = "2030-01-01T00:00:00Z".parse().unwrap();
    let set_clock = |at: jiff::Timestamp| {
        let next = clock.with_extension("next");
        std::fs::write(&next, at.to_string()).unwrap();
        std::fs::rename(&next, &clock).unwrap();
    };
    set_clock(now);
    p.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    let agents =
        instrukt_core::config::Agents::parse(instrukt_core::config::SHIPPED_AGENTS).unwrap();
    let profile =
        |name: &str| &agents.profiles[&instrukt_core::ids::ProfileName::parse(name).unwrap()];
    // A manual restriction on a model of another profile stays as set.
    let manual = profile("antigravity").strong[0]["model"]
        .as_str()
        .unwrap()
        .to_owned();
    p.env
        .run(
            p.repo.path(),
            [
                "agents",
                "status",
                "antigravity",
                "unavailable",
                "--model",
                &manual,
                "--until",
                "2099-01-01T00:00:00Z",
                "--reason",
                "operator restriction",
            ],
        )
        .success();
    // At t=0 the snapshot reports the first profile's window full, with no
    // reset given.
    let claude = profile("claude").instance.clone();
    common::t3_readiness::update(&p.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == claude.as_str() {
                provider["usageLimits"] = serde_json::json!({"checkedAt":"2030-01-01T00:00:00Z","windows":[{"id":"session","kind":"session","label":"Session","usedPercent":100}]});
            }
        }
    });
    let fake = runtime::FakeT3::create(&p.env);
    // The runner reads T3's snapshot twice as it starts: once on its own,
    // once for its first wake. Each read is held until the test lets it
    // through.
    std::fs::write(p.env.path("t3-readiness/hold-read"), "").unwrap();
    let _runner = crate::fixtures::conversing::Running::spawn(
        p.env
            .instrukt_process(p.env.home())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
    );
    crate::common::wait_until(
        "the first snapshot read",
        std::time::Duration::from_secs(20),
        || common::t3_readiness::held_read_open(&p.env, 1),
    );
    common::t3_readiness::release_read(&p.env, 1);
    // The snapshot's marks are applied once its refresh is done: the first
    // profile shows its restriction.
    let availability = || {
        let out = p
            .env
            .run(p.repo.path(), ["agents", "availability"])
            .success();
        serde_json::from_slice::<serde_json::Value>(&out.get_output().stdout).unwrap()
    };
    crate::common::wait_until(
        "the first snapshot's marks",
        std::time::Duration::from_secs(20),
        || {
            availability()
                .as_array()
                .unwrap()
                .iter()
                .any(|mark| mark["profile"] == "claude")
        },
    );
    // Every later read fails, the second one included: the snapshot of t=0
    // is the only one the runner publishes. That read is let through once
    // it stands held, and the runner shows its refresh failing; no refresh
    // is then under way, and every later one fails too, its token not
    // renewed and its read failing.
    std::fs::write(p.env.path("t3-readiness/malformed"), "").unwrap();
    fake.flag("issue-failure", true);
    crate::common::wait_until(
        "the second snapshot read",
        std::time::Duration::from_secs(20),
        || common::t3_readiness::held_read_open(&p.env, 2),
    );
    common::t3_readiness::release_read(&p.env, 2);
    crate::common::wait_until(
        "the second refresh failing",
        std::time::Duration::from_secs(20),
        || readiness_failing(&p),
    );
    // The runner has woken at `at` once its last admission reads it: every
    // wake admits the configuration first and takes its own time after, and
    // wakes run one after another, so none that began earlier is still
    // running and work created from now on is dispatched at `at` or later.
    let woken_at = |at: jiff::Timestamp| {
        crate::common::wait_until(
            "the runner's wake at the moved clock",
            std::time::Duration::from_secs(20),
            || {
                let out = p
                    .env
                    .run(p.env.home(), ["events", "status", "--json"])
                    .success();
                let status: serde_json::Value =
                    serde_json::from_slice(&out.get_output().stdout).unwrap();
                status["admission"]["runner"]["at"]
                    .as_str()
                    .and_then(|last| last.parse::<jiff::Timestamp>().ok())
                    .is_some_and(|last| last >= at)
            },
        );
    };
    // At t=30 min the first profile is still restricted: work created then
    // starts on the second, at t=30 or the minute after.
    now += jiff::SignedDuration::from_mins(30);
    set_clock(now);
    woken_at(now);
    create_change(&p);
    // Creating work wakes no runner here: a wake already under way, or the
    // one the next minute on the clock brings, dispatches it.
    now += jiff::SignedDuration::from_mins(1);
    set_clock(now);
    crate::common::wait_until(
        "the start at t=30",
        std::time::Duration::from_secs(20),
        || p.rt.started_count() == 1,
    );
    assert_eq!(fake.provider("s1"), profile("codex").instance);
    // The start reused the snapshot of t=0: the restriction still expires
    // at t=60 min, and the manual one is as set.
    insta::assert_json_snapshot!("reused-snapshot-quota-marks", availability());
    // At t=61 min its restriction has lapsed: work created then starts on the
    // first profile, at t=61 or the minute after.
    now += jiff::SignedDuration::from_mins(30);
    set_clock(now);
    woken_at(now);
    p.todo([
        "create",
        "sort-docs",
        "Sort the documentation index.",
        "--kind",
        "change",
        "--approved",
    ]);
    now += jiff::SignedDuration::from_mins(1);
    set_clock(now);
    crate::common::wait_until(
        "the start at t=61",
        std::time::Duration::from_secs(20),
        || p.rt.started_count() == 2,
    );
    assert_eq!(fake.provider("s2"), claude);
}
