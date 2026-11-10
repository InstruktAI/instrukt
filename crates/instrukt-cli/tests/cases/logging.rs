//! A publication remains readable together with its command's log record.
#![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

use crate::common;
use crate::fixtures::documented;
use std::path::Path;
use std::time::Duration;

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
    let home = Home::at_user_home(env.home().to_path_buf());
    let project = ProjectSlug::parse("demo").unwrap();
    crate::common::wait_until(
        "hook publication to finish",
        Duration::from_secs(30),
        || {
            if let Some(record) = repository::last_run(&home, &project, Hook::PostCommit).unwrap()
                && let Some(SyncOutcome::Finished { exit_code, .. }) = record.sync
            {
                assert_eq!(exit_code, Some(0));
                return true;
            }
            false
        },
    );

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
