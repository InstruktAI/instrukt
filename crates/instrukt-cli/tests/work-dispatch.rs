//! `project/instrukt/spec/feature/runner/work-dispatch`: a wake-up
//! dispatches eligible work into the procedure declaring its lane's
//! orchestrator role when the project chose automatic pickup, and leaves
//! it for a deliberate start otherwise; with both the todos and events
//! modules off, the runner's commands refuse and name both switches.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/flavor.rs"]
mod flavor;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod work;

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

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn work_claimed_while_github_is_unreachable_is_dispatched_once_github_answers() {
    let mut project = Tracked::new(true);
    create_change(&project);

    project.github.stop();
    project.tick();
    assert!(project.rt.calls().is_empty(), "{:?}", project.rt.calls());

    project.github.resume();
    project.tick();
    assert_eq!(project.rt.started_count(), 1, "{:?}", project.rt.calls());
    insta::assert_snapshot!(
        "dispatched-after-pending-claim",
        work::redact(&project.todo(["show", "tidy-imports"]))
    );
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn work_claimed_while_github_was_unreachable_is_dispatched_after_another_command_reflects_the_claim()
 {
    let mut project = Tracked::new(true);
    project.todo([
        "create",
        "tidy-imports",
        "Sort the imports of every module.",
        "--kind",
        "change",
        "--approved",
    ]);

    // The runner claims the work but cannot reflect the claim: no session.
    project.github.stop();
    project.tick();
    assert!(project.rt.calls().is_empty(), "{:?}", project.rt.calls());

    // A developer's command reflects the pending claim before the next tick.
    project.github.resume();
    project.todo([
        "artifacts",
        "write",
        "tidy-imports",
        "log",
        "Noted: imports are grouped std, external, internal.",
    ]);

    project.tick();

    assert_eq!(project.rt.started_count(), 1, "{:?}", project.rt.calls());
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

    assert!(project.rt.calls().is_empty(), "{:?}", project.rt.calls());
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD2
#[test]
fn with_work_tracking_and_event_responses_off_the_runner_refuses_and_names_both() {
    let env = TestEnv::new();
    let rt = FakeRuntime::new(&env, "rt", false);
    global_layer(&env, "todos: false, events: false", &[&rt]);
    let repo = env.git_repo("demo");
    repo.write("instrukt.yaml", "schema_version: 1\nproject: demo\n");
    repo.commit("the developer's project");
    for args in [vec!["tick"], vec!["run"], vec!["sessions", "list"]] {
        let refused = env.run(repo.path(), &args).failure();
        insta::assert_snapshot!(
            format!("refused-{}", args[0]),
            String::from_utf8_lossy(&refused.get_output().stderr)
        );
    }
    assert!(rt.calls().is_empty(), "{:?}", rt.calls());
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
        "default: {order: [claude], tier: light}\nroles: {prepare-orchestrator: {order: [antigravity], tier: strong}}\n",
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
fn confirmed_no_creation_allows_the_runner_to_retry_the_next_available_model() {
    let p = Tracked::new(true);
    selection_config(
        &p,
        "default: {order: [codex, claude], tier: strong}\nprofiles:\n  codex:\n    strong: [{model: gpt-6-astra, effort: high}, {model: gpt-6.1-sol, effort: high}]\n",
    );
    let fake = runtime::FakeT3::create(&p.env);
    fake.flag("empty-start", true);
    create_change(&p);
    p.tick();
    let key = p
        .todo(["show", "tidy-imports"])
        .lines()
        .find_map(|line| line.trim().strip_prefix("key: "))
        .unwrap()
        .to_owned();
    fake.flag("empty-start", false);
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
                "gpt-6-astra",
                "--until",
                "2099-01-01T00:00:00Z",
                "--reason",
                "quota",
            ],
        )
        .success();
    p.env
        .run(p.repo.path(), ["sessions", "resolve-start", &key, "--none"])
        .success();
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    insta::assert_snapshot!("confirmed-no-start-model-fallback", chosen(&p));
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

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn malformed_readiness_is_refused_and_its_token_is_revoked() {
    let p = Tracked::new(true);
    std::fs::write(p.env.path("t3-readiness/malformed"), "").unwrap();
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 0);
    assert!(
        p.rt.calls()
            .iter()
            .any(|line| line.starts_with("auth session revoke "))
    );
}

// spec: project/instrukt/spec/feature/runner/work-dispatch UC-RD1
#[test]
fn failed_readiness_token_revocation_refuses_thread_creation() {
    let p = Tracked::new(true);
    runtime::FakeT3::create(&p.env).flag("revoke-failure", true);
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 0);
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
fn an_unknown_readiness_descriptor_version_refuses_start_without_creating_a_thread() {
    let p = Tracked::new(true);
    let file = common::t3_readiness::state_file(&p.env);
    let mut state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    state["version"] = 2.into();
    std::fs::write(file, state.to_string()).unwrap();
    create_change(&p);
    p.tick();
    assert_eq!(p.rt.started_count(), 0);
    assert!(
        !p.rt
            .calls()
            .iter()
            .any(|line| line.starts_with("thread start "))
    );
}
