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
            Home::from_env().unwrap().machine_user_home(),
            root.join("home"),
            "contract worker requires its disposable home"
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
        let result = if stage == "real" {
            release::real(root)
        } else {
            (|| -> anyhow::Result<()> {
                let readiness = release::readiness()?;
                release::cli(
                    &["thread", "send", "--", "contract-thread", "-"],
                    Some("contract queued input"),
                )?;
                let result = release::observe("contract-thread")?;
                fs::write(
                    root.join("fake.json"),
                    json!({"adapter":result,"readiness":readiness,"steering":release::fake_steering(root)?}).to_string(),
                )?;
                Ok(())
            })()
        };
        assert!(result.is_ok(), "{stage} boundary failed: {result:?}");
        return;
    }
    let version = release::VERSION.trim();
    let mut manifest = json!({"version":version,"instrukt_revision":option_env!("GITHUB_SHA"),"upstream_sha":std::env::var("T3_CONTRACT_UPSTREAM_SHA").ok(),"outcome":"failure","operation":"fixture prerequisites","exclusions":["provider execution and successful CLI start","ready-thread consumption and completed-turn relay","provider-generated assistant/reasoning/tool/diff/subagent payloads","authenticated provider quota/account behavior","worktree launch with an opening model turn"],"fake_operations":{"readiness translation":"not verified","version":"not verified","auth session issue/revoke":"not verified","thread list/show":"not verified","thread send":"not verified","thread start":"provider-dependent success path unverified","consume/finish":"provider-entry/completion fixture control, not live verified","fault flags":"fault injection, not vendor behavior"}});
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut env = TestEnv::new();
        manifest["operation"] = json!("release acquisition/checksum");
        let binary = release::download(&env, version).expect("release acquisition and checksum");
        manifest["release"] =
            serde_json::from_slice(&fs::read(env.path("release.json")).unwrap()).unwrap();
        fs::remove_file(env.path("runtime-bin/t3")).unwrap();
        std::os::unix::fs::symlink(binary, env.path("runtime-bin/t3")).unwrap();
        let t3_base = env.home().join(".t3");
        env.set_env("T3CODE_HOME", t3_base.to_str().unwrap());
        env.set_env("T3CODE_NO_BROWSER", "true");
        env.set_env("T3CODE_TELEMETRY_ENABLED", "false");
        let project = env.git_repo("project");
        project.write("instrukt.yaml", "schema_version: 5\nproject: contract\n");
        project.commit("feat: disposable contract project");
        fs::create_dir_all(env.instrukt_home()).unwrap();
        fs::write(
            env.instrukt_home().join("instrukt.yaml"),
            "schema_version: 5\nrunner: {runtime: t3}\nmodules: {history: true}\n",
        )
        .unwrap();
        manifest["operation"] = json!("real CLI, readiness, projection and input boundaries");
        release::worker(&env, "real", &env.path("")).unwrap();
        let real: Value =
            serde_json::from_slice(&fs::read(env.path("real.json")).unwrap()).unwrap();
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
