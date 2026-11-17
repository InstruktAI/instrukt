//! Source acquisition through the real binary with isolated scoped Redis instances.
#![allow(dead_code, clippy::unwrap_used, reason = "shared integration setup")]

use crate::common::redis::RedisServer;
use crate::common::{Repo, TestEnv};
use crate::fixtures::documented::{project, sync};
use instrukt_core::state_db::rusqlite::Connection;

pub struct Computer {
    pub env: TestEnv,
    pub project: Repo,
    pub upstream: Repo,
}

impl Computer {
    pub fn new() -> Self {
        let env = TestEnv::new();
        let bare = env.bare_remote("upstream");
        let upstream = env.clone_repo(&bare, "upstream-work");
        upstream.write("README.md", "first\n");
        upstream.commit("feat: first release");
        upstream.git(["push", "--quiet", "origin", "main"]);
        let project = project(
            &env,
            "fork",
            &[(
                "instrukt.yaml",
                format!(
                    "schema_version: {}\nproject: fork\nevents:\n  watch:\n    - {{remote: upstream, ref: refs/heads/main}}\n",
                    instrukt_core::config::CURRENT_SCHEMA_VERSION
                ),
            )],
        );
        project.git(["remote", "add", "upstream", bare.to_str().unwrap()]);
        sync(&env, &project);
        Self {
            env,
            project,
            upstream,
        }
    }

    pub fn config(&self, cluster: &RedisServer, local: Option<&RedisServer>) {
        self.config_urls(&cluster.url, local.map(|server| server.url.as_str()));
    }

    pub fn config_urls(&self, cluster: &str, local: Option<&str>) {
        let local = local.map_or(String::new(), |url| format!("  local:\n    url: '{url}'\n"));
        std::fs::write(self.env.instrukt_home().join("instrukt.yaml"), format!("schema_version: 1\nmodules: {{events: true}}\nrunner: {{runtime: t3}}\nredis:\n  cluster:\n    url: '{cluster}'\n{local}")).unwrap();
    }

    pub fn tick(&self) {
        self.env.run(&self.env.path(""), ["tick"]).success();
    }

    pub fn change(&self) {
        self.upstream.write("new.md", "changed\n");
        self.upstream.commit("feat: next release");
        self.upstream.git(["push", "--quiet", "origin", "main"]);
    }

    pub fn subscribe(&self, pattern: &str) {
        self.project.write(
            "docs/domain/reactions/procedure/respond.md",
            &crate::fixtures::documented::snippet(
                "procedure",
                "Respond",
                "Accept the selected facts.",
                &format!(
                    "subscription_home: project\nsubscriptions: ['{pattern}']\nrole: responder\n"
                ),
                &[],
                "Accept the selected occurrence.\n",
            ),
        );
        self.project.commit("docs: publish a reaction");
        crate::fixtures::documented::sync(&self.env, &self.project);
    }

    pub fn computer(&self) -> String {
        std::fs::read_to_string(self.env.instrukt_home().join("state/machine-id"))
            .unwrap()
            .trim()
            .to_owned()
    }

    pub fn accepted(&self) -> u64 {
        self.db()
            .query_row("SELECT count(*) FROM procedure_activations", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    pub fn handled(&self) -> u64 {
        self.db()
            .query_row(
                "SELECT count(*) FROM handled_events WHERE consumer = 'procedure-activation'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn group(&self) -> String {
        format!("consumer-procedure-activation-{}", self.computer())
    }

    pub fn db(&self) -> Connection {
        Connection::open(self.env.instrukt_home().join("state/instrukt.db")).unwrap()
    }
}

pub struct Running(std::process::Child);

impl Running {
    pub fn start(computer: &Computer) -> Self {
        Self(
            computer
                .env
                .instrukt_process(&computer.env.path(""))
                .arg("run")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
    pub fn kill(&mut self) {
        self.0.kill().unwrap();
        assert!(!self.0.wait().unwrap().success());
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            self.0.kill().unwrap();
        }
        self.0.wait().unwrap();
    }
}

pub fn until(observed: impl FnMut() -> bool) {
    crate::common::wait_until(
        "the expected boundary",
        std::time::Duration::from_secs(30),
        observed,
    );
}
