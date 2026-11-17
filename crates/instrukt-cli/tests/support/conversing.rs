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

/// The members the company lists, with their own tool logins.
pub const MEMBERS: &str = "{steven: {name: Steven Sonneveld, email: steven@example.com}}";

/// What one accepted runtime input carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    /// The name of the person whose addressed message it is.
    pub member: String,
    /// How many member envelopes it carries.
    pub members: usize,
    /// Each accompanying message's speaker, by name, and whether it is the
    /// agent's own post.
    pub context: Vec<(String, bool)>,
    /// How many Slack mentions in markup the addressed message and its
    /// accompanying messages still carry.
    pub markup: usize,
    /// How many of the addressed message and its accompanying messages
    /// arrived with no text.
    pub empty: usize,
    /// Whether the context says earlier messages were left out.
    pub left_out: bool,
}

/// A child process the test owns: ended and reaped when dropped, also when
/// the test fails.
pub struct Running {
    child: std::process::Child,
    /// How it ended, when a native wait already reaped it.
    reaped: Option<std::process::ExitStatus>,
}
impl Running {
    pub fn spawn(command: &mut std::process::Command) -> Self {
        Self {
            child: command.spawn().unwrap(),
            reaped: None,
        }
    }
    fn pid(&self) -> rustix::process::Pid {
        rustix::process::Pid::from_raw(i32::try_from(self.child.id()).unwrap()).unwrap()
    }
    /// How it ended, once it has.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        if self.reaped.is_none() {
            return self.child.try_wait().unwrap();
        }
        self.reaped
    }
    /// Sends it `signal`.
    pub fn signal(&self, signal: rustix::process::Signal) {
        rustix::process::kill_process(self.pid(), signal).unwrap();
    }
    /// Whether it has stopped since it was last asked, as a native wait
    /// reports without blocking. An end it reports is kept as its exit.
    pub fn stopped(&mut self) -> bool {
        if self.reaped.is_some() {
            return false;
        }
        let options = rustix::process::WaitOptions::UNTRACED | rustix::process::WaitOptions::NOHANG;
        match rustix::process::waitpid(Some(self.pid()), options).unwrap() {
            Some((_, status)) if status.stopped() => true,
            Some((_, status)) => {
                use std::os::unix::process::ExitStatusExt as _;
                self.reaped = Some(std::process::ExitStatus::from_raw(status.as_raw()));
                false
            }
            None => false,
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        if self.reaped.is_some() {
            return;
        }
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
        }
        self.child.wait().unwrap();
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
                "schema_version: {}\nrunner: {{runtime: t3}}\nmodules: {{history: true{modules}}}\n{machine}",
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
        self.runner = Some(Running::spawn(
            self.env
                .instrukt_process(self.env.home())
                .arg("run")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
        ));
        wait("the connector's outward connection", || {
            self.slack.connections() > before
        });
    }

    /// The runner process dies where it stands.
    pub fn runner_dies(&mut self) {
        self.runner = None;
    }

    /// Waits until the runner process has ended on its own.
    pub fn runner_ended(&mut self) {
        let runner = self.runner.as_mut().unwrap();
        wait("the runner's end", || runner.exited().is_some());
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
                Span::Member { member, body, .. } => Some(((*member).to_owned(), *body)),
                _ => None,
            })
            .unwrap();
        let context = spans
            .iter()
            .find_map(|span| match span {
                Span::Context { body, .. } => envelopes::context_body(body),
                _ => None,
            })
            .unwrap();
        let mentioned = instrukt_runner::connectors::mentioned;
        Delivered {
            member,
            left_out: context.left_out,
            empty: usize::from(body.is_empty())
                + context
                    .entries
                    .iter()
                    .filter(|entry| entry.chars == 0)
                    .count(),
            markup: mentioned(body).len()
                + context
                    .entries
                    .iter()
                    .map(|entry| mentioned(&entry.text).len())
                    .sum::<usize>(),
            context: context
                .entries
                .iter()
                .map(|entry| (entry.speaker.to_owned(), entry.own))
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

    /// The name Slack shows for the person `slack`.
    pub fn member(&self, slack: &str) -> String {
        self.slack.person(slack).shown()
    }

    /// Ends the runner the way a signal does, with no chance to record it.
    pub fn kill_runner(&mut self) {
        drop(self.runner.take());
    }
}
