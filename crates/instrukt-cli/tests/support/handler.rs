//! A published and activated responsibility, through the real CLI and native process boundary.
#![allow(
    dead_code,
    clippy::unwrap_used,
    reason = "shared isolated handler scenarios"
)]
use crate::{
    common::redis::RedisServer,
    fixtures::{documented, event_transport::Computer, runtime::FakeT3},
};
use instrukt_runner::transport::EVENT_STREAM;

pub struct Handler {
    pub computer: Computer,
    pub runtime: FakeT3,
    pub redis: RedisServer,
    pub agents_before: Vec<u8>,
}
impl Handler {
    pub fn activate(role: Option<&str>, configure: impl FnOnce(&mut serde_yaml_ng::Value)) -> Self {
        let computer = Computer::new();
        let runtime = FakeT3::create(&computer.env);
        let redis = RedisServer::new(&computer.env);
        computer.config(&redis, None);
        let mut agents: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(instrukt_core::config::SHIPPED_AGENTS).unwrap();
        configure(&mut agents);
        let agents_before = serde_yaml_ng::to_string(&agents).unwrap().into_bytes();
        std::fs::write(
            computer.env.instrukt_home().join("agents.yaml"),
            &agents_before,
        )
        .unwrap();
        let role = role
            .map(|role| format!("role: {role}\n"))
            .unwrap_or_default();
        computer.project.write(
            "docs/domain/reactions/procedure/respond.md",
            &documented::snippet(
                "procedure",
                "Respond",
                "Answer a fact.",
                &format!("{role}subscription_home: project\nsubscriptions: [example.changed]\n"),
                &[],
                "Inspect the current fact.\n",
            ),
        );
        computer
            .project
            .commit("docs: publish an independent responsibility");
        documented::sync(&computer.env, &computer.project);
        computer.tick();
        let event = serde_json::json!({"id":"handler-activation","type":"example.changed","source":computer.computer(),"subject":"todo:sample","home":"project:fork","home_owner":"unresolved","time":"2026-11-05T00:00:00Z","data":{}});
        redis::cmd("XADD")
            .arg(EVENT_STREAM)
            .arg("*")
            .arg("event")
            .arg(event.to_string())
            .query::<String>(&mut redis.connection())
            .unwrap();
        computer.tick();
        assert_eq!(runtime.threads().len(), 1);
        Self {
            computer,
            runtime,
            redis,
            agents_before,
        }
    }
    pub fn call(&self, args: &[&str]) -> assert_cmd::Command {
        let mut command = self.computer.env.instrukt(self.computer.project.path());
        command.env("T3CODE_THREAD_ID", "s1").args(args);
        command
    }
    pub fn help(&self) -> String {
        let result = self.call(&["--help"]).assert().success();
        String::from_utf8_lossy(&result.get_output().stdout).into_owned()
    }
}
