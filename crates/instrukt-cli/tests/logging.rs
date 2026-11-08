//! A publication remains readable together with its command's log record.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

mod common;
#[path = "support/documented.rs"]
mod documented;

use std::path::Path;
use std::time::{Duration, Instant};

use common::TestEnv;
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::repository::{self, Hook, SyncOutcome};

// spec: project/instrukt/spec/feature/configuration/logging UC-LG1
#[test]
fn a_hook_publication_is_retrievable_and_recorded_in_the_sync_log() {
    let mut env = TestEnv::new();
    let bin = Path::new(env!("CARGO_BIN_EXE_instrukt")).parent().unwrap();
    env.set_env(
        "PATH",
        &format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
    );
    let repo = documented::project(&env, "demo", &[]);
    env.run(repo.path(), ["hooks", "install"]).success();
    repo.write(
        "docs/domain/acme/policy/rules.md",
        &documented::snippet(
            "policy",
            "Rules",
            "The rules.",
            "",
            &[],
            "Follow the rules.\n",
        ),
    );
    repo.write(
        "docs/domain/acme/index.yaml",
        "description: Acme knowledge.\nexpanded: []\nlisted: []\n",
    );
    let commit = repo.commit("docs: publish the rules");

    // The detached hook worker records completion after its sync exits.
    let home = Home::at_user_home(env.home().to_path_buf());
    let project = ProjectSlug::parse("demo").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(record) = repository::last_run(&home, &project, Hook::PostCommit).unwrap()
            && let Some(SyncOutcome::Finished { exit_code, .. }) = record.sync
        {
            assert_eq!(exit_code, Some(0));
            break;
        }
        assert!(Instant::now() < deadline, "hook publication did not finish");
        std::thread::sleep(Duration::from_millis(20));
    }

    // This is the public log location the InstruktAI reader discovers.
    let log = std::fs::read_to_string(
        env.home()
            .join(".local/state/instrukt-ai/instrukt/sync.log"),
    )
    .unwrap();
    let read = env
        .run(repo.path(), ["docs", "get", "acme/policy/rules"])
        .success();
    let output = String::from_utf8_lossy(&read.get_output().stdout).replace(
        env.path("").display().to_string().trim_end_matches('/'),
        "[TMP]",
    );
    insta::assert_snapshot!("published-rules", output);
    let records = log
        .lines()
        .map(|line| format!("[TIME] {}", line.split_once(' ').unwrap().1))
        .collect::<Vec<_>>()
        .join("\n")
        .replace(&commit, "[SHA]");
    insta::assert_snapshot!("publication-log", records);
    insta::assert_snapshot!(String::from_utf8_lossy(&read.get_output().stderr), @"");
}

/// Only instrukt's declared write records; timestamps and logger metadata are external.
fn write_events(env: &TestEnv, source: &str) -> Vec<String> {
    let file = env
        .home()
        .join(format!(".local/state/instrukt-ai/instrukt/{source}.log"));
    std::fs::read_to_string(file)
        .unwrap()
        .lines()
        .filter_map(|line| {
            let (_, tail) = line.split_once(" event=")?;
            if tail.starts_with("secret_written") || tail.starts_with("agents_defaults_written") {
                Some(format!("event={tail}"))
            } else {
                None
            }
        })
        .collect()
}

// spec: project/instrukt/spec/feature/configuration/logging UC-LG1
#[test]
fn secret_mutations_emit_one_secret_safe_completion_each() {
    let env = TestEnv::new();
    env.instrukt(env.home())
        .args(["secrets", "set", "SIGNING"])
        .write_stdin("fixture-value-stays-in-the-protected-file")
        .assert()
        .success()
        .stdout("");
    env.run(env.home(), ["secrets", "generate", "GENERATED"])
        .success()
        .stdout("");
    let events = write_events(&env, "secrets");
    assert_eq!(events.len(), 2);
    insta::assert_json_snapshot!("secret-write-completions", events);
}

// spec: project/instrukt/spec/feature/configuration/logging UC-LG1
#[test]
fn defaults_install_and_replace_are_recorded_without_unchanged_read_noise() {
    use sha2::{Digest, Sha256};
    let env = TestEnv::new();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    env.run(env.home(), ["config", "show", "agents"]).success();
    let prior = instrukt_core::config::SHIPPED_AGENTS.replace("gpt-6.1-sol", "prior-model");
    std::fs::write(env.instrukt_home().join("agents.yaml"), &prior).unwrap();
    std::fs::write(env.instrukt_home().join("state/agents-deployment.json"),serde_json::json!({"schema_version":1,"digest":hex::encode(Sha256::digest(prior.as_bytes()))}).to_string()).unwrap();
    env.run(env.home(), ["config", "init", "--global"])
        .success();
    env.run(env.home(), ["config", "show", "agents"]).success();
    let events = write_events(&env, "config");
    assert_eq!(events.len(), 2);
    insta::assert_json_snapshot!("defaults-write-completions", events);
}
