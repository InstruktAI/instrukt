//! Source acquisition through the real binary with isolated scoped Redis instances.
#![allow(dead_code, clippy::unwrap_used, reason = "shared integration setup")]

use crate::common::redis::RedisServer;
use crate::common::{Repo, TestEnv};
use crate::documented::{project, sync};
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
        let project = project(&env, "fork", &[("instrukt.yaml", "schema_version: 1\nproject: fork\nevents:\n  watch:\n    - {remote: upstream, ref: refs/heads/main}\n".into())]);
        project.git(["remote", "add", "upstream", bare.to_str().unwrap()]);
        sync(&env, &project);
        Self {
            env,
            project,
            upstream,
        }
    }

    pub fn config(&self, cluster: &RedisServer, local: Option<&RedisServer>) {
        let local = local.map_or(String::new(), |server| {
            format!("  local:\n    url: '{}'\n", server.url)
        });
        std::fs::write(self.env.instrukt_home().join("instrukt.yaml"), format!("schema_version: 1\nmodules: {{events: true}}\nrunner: {{runtime: t3}}\nredis:\n  cluster:\n    url: '{}'\n{local}", cluster.url)).unwrap();
    }

    pub fn tick(&self) {
        self.env.run(&self.env.path(""), ["tick"]).success();
    }

    pub fn change(&self) {
        self.upstream.write("new.md", "changed\n");
        self.upstream.commit("feat: next release");
        self.upstream.git(["push", "--quiet", "origin", "main"]);
    }

    pub fn db(&self) -> Connection {
        Connection::open(self.env.instrukt_home().join("state/instrukt.db")).unwrap()
    }
}
