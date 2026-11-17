//! A company whose people address its agent through Slack: a company home
//! with members and a declared Slack connector, the fake T3 host and its
//! durable source, the Slack stand-in at the network line, and the real
//! long-lived `instrukt run`.
#![allow(dead_code, clippy::unwrap_used, reason = "shared test setup")]
use std::time::Duration;

use instrukt_core::ids::ProjectSlug;
use instrukt_runner::conversations::Status;
use instrukt_runner::envelopes::{self, Span};

use crate::common::{self, TestEnv};
use crate::fixtures::history::T3Store;
use crate::fixtures::slack::{APP_TOKEN, BOT_TOKEN, FakeSlack};
use crate::fixtures::t3::FakeT3;

/// The company the scenarios address.
pub const COMPANY: &str = "acme";

/// The members the company lists, by key and Slack identity.
pub const MEMBERS: &str = "{steven: {name: Steven Sonneveld, slack: USTEVEN}, anna: {name: Anna, slack: UANNA}, bram: {slack: UBRAM}}";

/// What one accepted runtime input carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    /// The member whose addressed message it is.
    pub member: ProjectSlug,
    /// How many member envelopes it carries.
    pub members: usize,
    /// Each accompanying message's speaker, as the company's member that
    /// speaker identity resolves to, and whether it is the agent's own post.
    pub context: Vec<(Option<ProjectSlug>, bool)>,
    /// How many of the addressed message and its accompanying messages
    /// arrived with no text.
    pub empty: usize,
    /// Whether the context says earlier messages were left out.
    pub left_out: bool,
}

/// A child process the test owns: ended and reaped when dropped, also when
/// the test fails.
pub struct Running(std::process::Child);
impl Running {
    pub fn spawn(command: &mut std::process::Command) -> Self {
        Self(command.spawn().unwrap())
    }
    pub fn id(&self) -> u32 {
        self.0.id()
    }
    /// How it ended, once it has.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.0.try_wait().unwrap()
    }
    /// Sends it `signal` (`STOP`, `CONT`).
    pub fn signal(&self, signal: &str) {
        let sent = std::process::Command::new("kill")
            .args([format!("-{signal}"), self.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success());
    }
    /// Whether the operating system has it stopped.
    pub fn stopped(&self) -> bool {
        let state = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &self.id().to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&state.stdout)
            .trim()
            .starts_with('T')
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

pub struct Company {
    pub env: TestEnv,
    pub store: T3Store,
    pub runtime: FakeT3,
    pub slack: FakeSlack,
    runner: Option<Running>,
    turns: u64,
}

pub fn wait(what: &str, condition: impl FnMut() -> bool) {
    common::wait_until(what, Duration::from_secs(20), condition);
}

/// Creates the company home `name` with `settings` applied through the
/// configuration verbs and its Slack tokens in its own secrets file.
pub fn company_home(env: &TestEnv, name: &str, app: &str, bot: &str, settings: &str) {
    let here = env.path("");
    env.run(&here, ["config", "init", "--company", name])
        .success();
    for (reference, value) in [("SLACK_APP_TOKEN", app), ("SLACK_BOT_TOKEN", bot)] {
        env.instrukt(&here)
            .args(["secrets", "set", reference, "--company", name])
            .write_stdin(value)
            .assert()
            .success();
    }
    patch_company(env, name, settings);
}

/// Applies `settings` to the company home `name` through the configuration
/// verbs.
pub fn patch_company(env: &TestEnv, name: &str, settings: &str) {
    let here = env.path("");
    let revision = env
        .run(&here, ["config", "get", "--company", name, "--revision"])
        .success();
    let revision = String::from_utf8(revision.get_output().stdout.clone()).unwrap();
    env.run(
        &here,
        [
            "config",
            "patch",
            "--company",
            name,
            "--revision",
            revision.trim(),
            "--yaml",
            settings,
        ],
    )
    .success();
}

/// The declaration of a Slack connector reading both tokens from the home.
pub fn slack_settings(members: &str, extra: &str) -> String {
    format!(
        "{{members: {members}, connectors: {{slack: {{app_token: '${{SLACK_APP_TOKEN}}', bot_token: '${{SLACK_BOT_TOKEN}}'{extra}}}}}}}"
    )
}

impl Company {
    /// A company whose Slack connector declares `extra` fields, on a
    /// computer with conversations switched on.
    pub fn new(extra: &str) -> Self {
        Self::on(TestEnv::new(), "", "", extra)
    }

    /// The company on a computer whose global layer also switches on
    /// `modules` and holds `machine` settings.
    pub fn on(mut env: TestEnv, modules: &str, machine: &str, extra: &str) -> Self {
        let slack = FakeSlack::start();
        env.set_env("INSTRUKT_SLACK_API_URL", &slack.api_url);
        let store = T3Store::new(&env);
        let runtime = FakeT3::create(&env);
        std::fs::write(
            env.instrukt_home().join("instrukt.yaml"),
            format!(
                "schema_version: {}\nrunner: {{runtime: t3}}\nmodules: {{conversations: true{modules}}}\n{machine}",
                instrukt_core::config::CURRENT_SCHEMA_VERSION
            ),
        )
        .unwrap();
        company_home(
            &env,
            COMPANY,
            APP_TOKEN,
            BOT_TOKEN,
            &slack_settings(MEMBERS, extra),
        );
        Self {
            env,
            store,
            runtime,
            slack,
            runner: None,
            turns: 0,
        }
    }

    /// Starts the long-lived runner and waits for its outward connection.
    pub fn run(&mut self) {
        let before = self.slack.connections();
        self.runner = Some(Running(
            self.env
                .instrukt_process(self.env.home())
                .arg("run")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        ));
        wait("the connector's outward connection", || {
            self.slack.connections() > before
        });
    }

    /// Waits until the runner process has ended on its own.
    pub fn runner_ended(&mut self) {
        let runner = self.runner.as_mut().unwrap();
        wait("the runner's end", || {
            runner.0.try_wait().unwrap().is_some()
        });
    }

    /// The agent sessions started so far.
    pub fn sessions(&self) -> Vec<String> {
        self.runtime.threads()
    }

    /// The one session, once a message started it.
    pub fn session(&self) -> String {
        wait("the conversation's session", || {
            self.runtime.threads().len() == 1
        });
        self.runtime.threads().remove(0)
    }

    /// The runtime finishes the turn started by the session's `n`th input,
    /// answering with `reply`, and the session becomes idle.
    pub fn finish_turn(&mut self, session: &str, n: usize, reply: &str) {
        self.turns += 1;
        let turn = format!("turn-{}", self.turns);
        self.store
            .thread(session, None, None, "2026-11-08T10:00:00.000Z");
        let input = self.runtime.message(session, n);
        self.store.turn(session, &turn, self.turns, &input, reply);
        self.runtime.set_status(session, "idle");
    }

    /// What the session's `n`th accepted input carries, as the registered
    /// vocabulary reads it: the member who addressed the agent, and who said
    /// each accompanying message and whether it was the agent's own post.
    pub fn delivered(&self, session: &str, n: usize) -> Delivered {
        let input = self.runtime.message(session, n);
        let spans = envelopes::segment(&input);
        let (member, body) = spans
            .iter()
            .find_map(|span| match span {
                Span::Member { member, body, .. } => {
                    Some((ProjectSlug::parse(member).unwrap(), *body))
                }
                _ => None,
            })
            .unwrap();
        let members = self.members();
        let context = spans
            .iter()
            .find_map(|span| match span {
                Span::Context { body, .. } => envelopes::context_body(body),
                _ => None,
            })
            .unwrap();
        Delivered {
            member,
            left_out: context.left_out,
            empty: usize::from(body.is_empty())
                + context
                    .entries
                    .iter()
                    .filter(|entry| entry.chars == 0)
                    .count(),
            context: context
                .entries
                .iter()
                .map(|entry| {
                    (
                        members
                            .resolve(instrukt_runner::connectors::SLACK, entry.speaker)
                            .map(|member| member.key.clone()),
                        entry.own,
                    )
                })
                .collect(),
            members: spans
                .iter()
                .filter(|span| matches!(span, Span::Member { .. }))
                .count(),
        }
    }

    /// Whether the session's `n`th input arrived while a turn was running,
    /// as the runtime recorded it on arrival; finishing a turn leaves this
    /// record as it was.
    pub fn queued(&self, session: &str, n: usize) -> bool {
        let thread: serde_json::Value = serde_json::from_slice(
            &std::fs::read(self.runtime.state_dir().join(format!("{session}.json"))).unwrap(),
        )
        .unwrap();
        thread["messages"][n]["busy"].as_bool().unwrap_or(false)
    }

    /// The conversations `instrukt events status` reports with pending work
    /// or failures.
    pub fn conversations(&self) -> Status {
        serde_json::from_value(self.status()["conversations"].clone()).unwrap()
    }

    /// `instrukt events status --json`.
    pub fn status(&self) -> serde_json::Value {
        let out = self
            .env
            .run(self.env.home(), ["events", "status", "--json"])
            .success();
        serde_json::from_slice(&out.get_output().stdout).unwrap()
    }

    pub fn company(&self) -> ProjectSlug {
        ProjectSlug::parse(COMPANY).unwrap()
    }

    /// The key of the member the company lists for a Slack identity, as
    /// configuration resolves it.
    pub fn member(&self, slack: &str) -> ProjectSlug {
        self.members()
            .resolve(instrukt_runner::connectors::SLACK, slack)
            .unwrap()
            .key
            .clone()
    }

    /// The company's member list, as configuration reads it.
    pub fn members(&self) -> instrukt_core::config::Members {
        let home = instrukt_core::home::Home::at_user_home(self.env.home().to_path_buf());
        instrukt_core::config::Configuration::load_company(&home.for_inspection(), &self.company())
            .unwrap()
            .members()
            .unwrap()
    }
}
