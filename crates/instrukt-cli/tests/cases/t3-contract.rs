//! One release evaluation, shared by pinned CI and nightly branch dispatch.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "isolated contract acceptance test"
)]
#[path = "../support/t3_contract.rs"]
mod release;
use crate::{
    common::{self, TestEnv},
    fixtures::t3::FakeT3,
};
use instrukt_core::home::Home;
use serde_json::{Value, json};
use std::{fs, path::Path};

// spec: project/instrukt/spec/feature/runner/t3-contract UC-TC1
#[test]
fn a_release_and_its_fake_are_checked_without_a_model_account() {
    if let Ok(stage) = std::env::var("T3_CONTRACT_STAGE") {
        let root = std::env::var("T3_CONTRACT_ROOT").unwrap();
        let root = Path::new(&root);
        assert_eq!(
            Home::from_env().unwrap().root(),
            root.join("home/.instrukt"),
            "contract worker requires its disposable Instrukt root"
        );
        assert_eq!(
            std::env::var_os("T3CODE_HOME").map(std::path::PathBuf::from),
            Some(root.join("home/.t3")),
            "contract worker requires its disposable base"
        );
        assert!(
            root.join("runtime-bin/t3")
                .canonicalize()
                .unwrap()
                .starts_with(root),
            "contract worker cannot use an installed T3"
        );
        let result = match stage.as_str() {
            "real" => release::real(root),
            "overlay" => release::overlay(root),
            "fake" => (|| -> anyhow::Result<()> {
                let readiness = release::readiness()?;
                release::cli(
                    &["thread", "send", "--", "contract-thread", "-"],
                    Some("contract queued input"),
                )?;
                let result = release::observe("contract-thread")?;
                let websocket = release::websocket_dispatch("contract-thread")?;
                let shown: Value = serde_json::from_str(&release::cli(
                    &["thread", "show", "--json", "contract-thread"],
                    None,
                )?)?;
                let native_launch = release::native_launch_contract(
                    root,
                    shown["thread"]["projectId"].as_str().unwrap(),
                )?;
                let retention = release::retention(&root.join("project"))?;
                let projects = release::project_boundary(root, &root.join("project"))?;
                fs::write(
                    root.join("fake.json"),
                    json!({"adapter":result,"readiness":readiness,"steering":release::fake_steering(root)?,"websocket_dispatch":websocket,"native_launch":native_launch,"retention":retention,"projects":projects}).to_string(),
                )?;
                Ok(())
            })(),
            other => Err(anyhow::anyhow!("unknown contract stage: {other}")),
        };
        assert!(result.is_ok(), "{stage} boundary failed: {result:?}");
        return;
    }
    let version = release::VERSION.trim();
    let mut manifest = json!({"version":version,"instrukt_revision":option_env!("GITHUB_SHA"),"upstream_sha":std::env::var("T3_CONTRACT_UPSTREAM_SHA").ok(),"outcome":"failure","operation":"fixture prerequisites","exclusions":["provider execution and successful CLI start","ready-thread consumption and completed-turn relay","provider-generated assistant/reasoning/tool/diff/subagent payloads","authenticated provider quota/account behavior","worktree launch with an opening model turn"],"fake_operations":{"readiness translation":"not verified","version":"not verified","auth session issue/revoke":"not verified","thread list/show":"not verified","thread send":"not verified","thread start":"provider-dependent success path unverified","consume/finish":"provider-entry/completion fixture control, not live verified","fault flags":"fault injection, not vendor behavior","worktree preparation":"fixture control: a new-worktree launch answers preparing and T3 finishes only at the test's explicit step, never on reads; completed preparation is not live verified"}});
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut env = TestEnv::new();
        manifest["operation"] = json!("release acquisition/checksum");
        let binary = release::download(&env, version).expect("release acquisition and checksum");
        manifest["release"] =
            serde_json::from_slice(&fs::read(env.path("release.json")).unwrap()).unwrap();
        fs::remove_file(env.path("runtime-bin/t3")).unwrap();
        std::os::unix::fs::symlink(&binary, env.path("runtime-bin/t3")).unwrap();
        let t3_base = env.home().join(".t3");
        env.set_env("T3CODE_HOME", t3_base.to_str().unwrap());
        env.set_env("T3CODE_NO_BROWSER", "true");
        env.set_env("T3CODE_TELEMETRY_ENABLED", "false");
        let project = env.git_repo("project");
        project.write("instrukt.yaml", "schema_version: 6\nproject: contract\n");
        project.commit("feat: disposable contract project");
        fs::create_dir_all(env.instrukt_home()).unwrap();
        fs::write(
            env.instrukt_home().join("instrukt.yaml"),
            "schema_version: 8\nrunner: {runtime: t3}\nmodules: {history: true}\n",
        )
        .unwrap();
        let mut overlay_env = TestEnv::new();
        release::stage_binary(&binary, &overlay_env).expect("overlay release binary");
        let overlay_t3_base = overlay_env.home().join(".t3");
        overlay_env.set_env("T3CODE_HOME", overlay_t3_base.to_str().unwrap());
        overlay_env.set_env("T3CODE_NO_BROWSER", "true");
        overlay_env.set_env("T3CODE_TELEMETRY_ENABLED", "false");
        let overlay_project = overlay_env.git_repo("project");
        overlay_project.write("instrukt.yaml", "schema_version: 6\nproject: contract\n");
        overlay_project.commit("feat: disposable overlay contract project");
        // The owner of `docs/general/` the overlay-link check publishes.
        crate::fixtures::documented::project(
            &overlay_env,
            "docs-owner",
            &[
                (
                    "docs/general/index.yaml",
                    "expanded:\n  - general/policy/contract\n".to_owned(),
                ),
                (
                    "docs/general/policy/contract.md",
                    crate::fixtures::documented::snippet(
                        "policy",
                        "Contract",
                        "The contract check's doctrine.",
                        "",
                        &[],
                        "Every agent reads this.\n",
                    ),
                ),
            ],
        );
        fs::create_dir_all(overlay_env.instrukt_home()).unwrap();
        fs::write(
            overlay_env.instrukt_home().join("instrukt.yaml"),
            "schema_version: 8\nrunner: {runtime: t3}\n",
        )
        .unwrap();
        manifest["operation"] =
            json!("real CLI, readiness, projection, input and overlay-link boundaries");
        let real_command = release::worker_command(&env, "real", &env.path("")).unwrap();
        let overlay_command =
            release::worker_command(&overlay_env, "overlay", &overlay_env.path("")).unwrap();
        let (real_result, overlay_result) = std::thread::scope(|scope| {
            let real_handle = scope.spawn(|| release::run_worker(real_command, "real"));
            let overlay_handle = scope.spawn(|| release::run_worker(overlay_command, "overlay"));
            (
                real_handle.join().expect("real contract worker thread"),
                overlay_handle
                    .join()
                    .expect("overlay contract worker thread"),
            )
        });
        match (real_result, overlay_result) {
            (Ok(()), Ok(())) => {}
            (Err(real_err), Ok(())) => {
                manifest["operation"] =
                    json!("real CLI, readiness, projection and input boundaries");
                panic!("real boundary failed: {real_err:?}");
            }
            (Ok(()), Err(overlay_err)) => {
                manifest["operation"] = json!("real overlay-link boundaries");
                panic!("overlay boundary failed: {overlay_err:?}");
            }
            (Err(real_err), Err(overlay_err)) => {
                panic!(
                    "real and overlay boundaries failed:\nreal: {real_err:?}\noverlay: {overlay_err:?}"
                );
            }
        }
        let real: Value =
            serde_json::from_slice(&fs::read(env.path("real.json")).unwrap()).unwrap();
        let overlay_links: Value =
            serde_json::from_slice(&fs::read(overlay_env.path("overlay.json")).unwrap()).unwrap();
        let mut fake_env = TestEnv::new();
        let fake_base = fake_env.home().join(".t3");
        fake_env.set_env("T3CODE_HOME", fake_base.to_str().unwrap());
        let fake = FakeT3::create(&fake_env);
        fake.release(&format!("t3 v{version}"));
        common::t3_readiness::update(&fake_env, |value| {
            value["providers"] = real["readiness"]["providers"].clone()
        });
        let project = fake_env.git_repo("project");
        project.commit("feat: fake boundary project");
        fake.register_project(
            project.path(),
            real["fixture"]["thread"]["projectId"].as_str().unwrap(),
        );
        fake.seed(
            "contract-thread",
            project.path(),
            "contract · instrukt:contract",
            "contract deferred input",
        );
        for (name, value) in real["fixture"]["thread"].as_object().unwrap() {
            fake.change("contract-thread", name, value.clone());
        }
        fake.change(
            "contract-thread",
            "messages",
            real["fixture"]["messages"].clone(),
        );
        // The retention fixture thread: settled long enough ago and opened
        // since, through the fake's controls for what the release reaches
        // with management commands.
        fake.migrate_projection();
        fake.next_thread_id(release::RETENTION_THREAD);
        let retained = fake.developer_starts_in(project.path());
        fake.settle_at(&retained, "2026-11-10T12:00:00Z");
        fake.opened_at(&retained, "2026-11-10T12:30:00Z");
        manifest["operation"] =
            json!("fake readiness, queued input, transcript and steering boundaries");
        release::worker(&fake_env, "fake", &fake_env.path("")).unwrap();
        let emulated: Value =
            serde_json::from_slice(&fs::read(fake_env.path("fake.json")).unwrap()).unwrap();
        manifest["operation"] = json!("fake thread show transcript comparison");
        assert!(
            real["adapter"]["transcript"] == emulated["adapter"]["transcript"],
            "fake thread show transcript differs in content or native run relationships"
        );
        manifest["operation"] = json!("fake persisted input/run relationship comparison");
        assert!(
            real["adapter"]["relationships"] == emulated["adapter"]["relationships"],
            "fake persisted input/run relationships differ"
        );
        manifest["operation"] = json!("project snapshot, project add and ancestor-only launch");
        assert_eq!(real["projects"], emulated["projects"]);
        manifest["projects"] = real["projects"].clone();
        manifest["fake_operations"]["project add/snapshot"] = json!(
            "compared check, ensure with the title T3 gives a root and a company home's repository-name title, duplicate refusal, reuse and ancestor-only pending launch"
        );
        manifest["operation"] = json!("WebSocket queued dispatch, replay, and required fields");
        assert_eq!(real["native_launch"], emulated["native_launch"]);
        manifest["native_launch"] = real["native_launch"].clone();
        assert_eq!(real["websocket_dispatch"], emulated["websocket_dispatch"]);
        manifest["websocket_dispatch"] = real["websocket_dispatch"].clone();
        manifest["fake_operations"]["WebSocket dispatch"] = json!(
            "compared queued acceptance, duplicate suppression and rejection of omitted provenance"
        );
        manifest["operation"] = json!("thread archiving through finished threads and archive");
        assert_eq!(real["retention"], emulated["retention"]);
        manifest["retention"] = real["retention"].clone();
        manifest["fake_operations"]["thread archiving"] = json!(
            "compared the finished, quiet, pinned and opening facts the listing and projection decode to, the archive with its table re-read, the listing afterwards, and the refused second archive with the archived row"
        );
        manifest["fake_operations"]["settle/pin/open"] = json!(
            "fixture controls for what the release reaches with thread.settle, thread.pin and thread.visit; their projection is compared through the archiving check"
        );
        let mut observed = real["adapter"].clone();
        let mut fake_observed = emulated["adapter"].clone();
        for value in [&mut observed, &mut fake_observed] {
            value.as_object_mut().unwrap().remove("transcript");
            value.as_object_mut().unwrap().remove("relationships");
        }
        assert!(
            observed == fake_observed,
            "fake input observation or selected thread metadata differs"
        );
        manifest["operation"] =
            json!("fake readiness translation and revoked-credential denial comparison");
        assert!(
            real["readiness"]["translated"] == emulated["readiness"]["translated"],
            "fake translated readiness profiles differ"
        );
        assert!(
            real["readiness"]["revoked_denied"] == emulated["readiness"]["revoked_denied"],
            "fake revoked credential denial differs"
        );
        observed["transcript_messages_compared"] = json!(
            real["adapter"]["transcript"]["messages"]
                .as_array()
                .unwrap()
                .len()
        );
        observed["input_relationships_compared"] =
            json!(real["adapter"]["relationships"].as_array().unwrap().len());
        manifest["observations"] = observed;
        manifest["readiness"] = json!({"explicitly_unready":real["readiness"]["explicitly_unready"],"translated":real["readiness"]["translated"],"real_revoked_denied":real["readiness"]["revoked_denied"],"fake_revoked_denied":emulated["readiness"]["revoked_denied"]});
        manifest["configured_homes"] = real["configured"].clone();
        manifest["overlay_links"] = overlay_links;
        manifest["steering_fixture"] = emulated["steering"].clone();
        manifest["fake_operations"]["version"] = json!(
            "diagnostic response only; content is not compared and release equality is not an adapter gate"
        );
        manifest["fake_operations"]["auth session issue/revoke"] = json!(
            "compared read-scoped acquisition and revoked credential denial on both boundaries; fake signature is fixture-only"
        );
        manifest["fake_operations"]["thread list/show"] = json!(
            "compared fixture listing membership, selected decoded metadata and every bounded transcript field, including text and run relationships"
        );
        manifest["fake_operations"]["thread send"] = json!(
            "compared queued acceptance, transcript visibility and consumed persisted input/run fields"
        );
        manifest["fake_operations"]["readiness translation"] = json!(
            "compared normalized profiles and checked explicit native unready facts against translated unavailability; cause wording is not compared"
        );
        manifest["outcome"] = json!("success");
        manifest["operation"] = json!("complete");
    }));
    if let Some(destination) = std::env::var_os("T3_CONTRACT_REPORT") {
        fs::write(destination, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    }
    if let Err(failure) = checked {
        std::panic::resume_unwind(failure);
    }
}
