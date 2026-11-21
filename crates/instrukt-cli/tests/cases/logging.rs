//! A publication remains readable together with its command's log record.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::common;
use crate::fixtures::documented;
use std::path::Path;

use common::TestEnv;
use instrukt_core::repository::{Hook, SyncOutcome};

// spec: project/instrukt/spec/feature/configuration/logging UC-LG1
#[test]
fn a_hook_publication_is_retrievable_and_recorded_in_the_sync_log() {
    let mut env = TestEnv::new();
    let bin = Path::new(env!("CARGO_BIN_EXE_instrukt")).parent().unwrap();
    env.set_env(
        "PATH",
        &format!("{}:{}", bin.display(), env.injected("PATH").unwrap()),
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
    let record = env.finished_hook_run(repo.path(), Hook::PostCommit);
    assert!(
        matches!(
            record.sync,
            Some(SyncOutcome::Finished {
                exit_code: Some(0),
                ..
            })
        ),
        "{record:?}"
    );

    // This is the public log location the InstruktAI reader discovers.
    let log = std::fs::read_to_string(env.log_dir().join("sync.log")).unwrap();
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

// spec: project/instrukt/spec/feature/configuration/logging UC-LG2
#[test]
fn an_unwritable_log_directory_keeps_publication_output_and_logs_to_stderr() {
    let mut env = TestEnv::new();
    let repo = documented::project(
        &env,
        "demo",
        &[(
            "docs/domain/acme/policy/rules.md",
            documented::snippet(
                "policy",
                "Rules",
                "The rules.",
                "",
                &[],
                "Follow the rules.\n",
            ),
        )],
    );
    let credential = "fixture-credential-never-printed";
    let blocked = env.path(&format!("Bearer {credential}"));
    std::fs::write(&blocked, "not a directory").unwrap();
    env.set_env("INSTRUKT_LOG_DIR", blocked.to_str().unwrap());
    let output = env.run(repo.path(), ["sync"]).success();
    let path = env.path("");
    let commit = repo.git(["rev-parse", "HEAD"]);
    let redact = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .replace(path.to_str().unwrap().trim_end_matches('/'), "[TMP]")
            .replace(commit.trim(), "[SHA]")
    };
    insta::assert_snapshot!(
        "publication-output-with-logging-fallback",
        redact(&output.get_output().stdout)
    );
    let diagnostics = redact(&output.get_output().stderr);
    assert!(
        !diagnostics.contains(credential),
        "credential leaked in a fallback record"
    );
    let records = diagnostics
        .lines()
        .map(|line| format!("[TIME] {}", line.split_once(' ').unwrap().1))
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("publication-diagnostics-on-stderr", records);
    check_redacted_write_failure();
}

fn check_redacted_write_failure() {
    use crate::fixtures::{conversing::Running, flavor::Tracked};
    use std::process::Stdio;
    use std::time::Duration;

    let mut project = Tracked::new(false);
    let credential = "fixture-write-failure-secret";
    let state = project.env.path(&format!("Bearer {credential}"));
    project
        .env
        .set_env("INSTRUKT_LOG_DIR", state.to_str().unwrap());
    let clock = project.env.path("log-rotation-clock");
    std::fs::write(&clock, "2030-01-06T00:00:00Z").unwrap();
    let stderr = project.env.path("rotation.stderr");
    let mut runner = Running::spawn(
        project
            .env
            .instrukt_process(project.env.home())
            .arg("run")
            .env("INSTRUKT_NOW", format!("@{}", clock.display()))
            .env("INSTRUKT_LOG_LEVEL", "DEBUG")
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&stderr).unwrap()),
    );
    let log = state.join("run.log");
    common::wait_until(
        "the runner writes to its active log",
        Duration::from_secs(30),
        || std::fs::read_to_string(&log).is_ok_and(|text| text.contains("runner_wakeup_done")),
    );
    std::fs::rename(&log, log.with_extension("old")).unwrap();
    std::fs::create_dir(&log).unwrap();

    let next_clock = clock.with_extension("next");
    std::fs::write(&next_clock, "2030-01-06T00:02:00Z").unwrap();
    std::fs::rename(next_clock, &clock).unwrap();
    common::wait_until(
        "the runner reports the runtime log write failure",
        Duration::from_secs(30),
        || std::fs::read_to_string(&stderr).is_ok_and(|text| text.contains("dropped a log line")),
    );
    let diagnostics = std::fs::read_to_string(&stderr).unwrap();
    let report = diagnostics
        .lines()
        .find(|line| line.starts_with("instrukt-ai-logging:"))
        .unwrap();
    assert!(
        !report.contains(credential),
        "credential leaked in the write-failure diagnostic"
    );
    assert!(
        report.contains("Bearer <REDACTED>: IsADirectory"),
        "{report}"
    );
    assert!(runner.exited().is_none());
}
