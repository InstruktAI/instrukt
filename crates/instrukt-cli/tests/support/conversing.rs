//! A company whose people address its agent through Slack: a company home
//! with members and a declared Slack connector, the fake T3 host and its
//! durable source, the Slack stand-in at the network line, and the real
//! long-lived `instrukt run`.
#![allow(dead_code, clippy::unwrap_used, reason = "shared test setup")]
use std::time::Duration;

use instrukt_company::conversations::Status;
use instrukt_company::conversations::envelopes::{self, Span};
use instrukt_core::ids::ProjectSlug;

use crate::common::{self, TestEnv};
use crate::fixtures::documented;
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
    /// How many member envelopes it carries.
    pub members: usize,
    /// How many messages accompany it.
    pub accompanying: usize,
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

/// Waits for `condition`, longer than the connector's own bound on a Web API
/// call (30 seconds), so a post slow under load is never taken for one that
/// did not come.
pub fn wait(what: &str, condition: impl FnMut() -> bool) {
    common::wait_until(what, Duration::from_secs(45), condition);
}

/// Waits until the runner has recorded `session` as its conversation's:
/// its start is done and the turn under way. The assignment itself answers
/// from the binding made before the start.
pub fn wait_for_binding(env: &TestEnv, session: &str) {
    let db_path = env.instrukt_home().join("state/instrukt.db");
    let session_ref = format!("t3:{session}");
    wait("the recorded binding", || {
        instrukt_core::state_db::rusqlite::Connection::open(&db_path)
            .ok()
            .and_then(|db| {
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM connected_conversations WHERE session=?1)",
                    [&session_ref],
                    |row| row.get::<_, bool>(0),
                )
                .ok()
            })
            .unwrap_or(false)
    });
}

/// The paths of the files process `pid` holds open, as the operating system
/// names them. A listing that cannot be made fails the test; it is never
/// taken as an empty one.
#[cfg(target_os = "macos")]
fn open_paths(pid: u32) -> Vec<std::path::PathBuf> {
    let output = std::process::Command::new(common::tool_executable("lsof"))
        .args(["-n", "-P", "-p", &pid.to_string(), "-F", "pn"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "lsof -p {pid} failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let listing = String::from_utf8(output.stdout).unwrap();
    // Each process record opens with its `p` field, and its files follow.
    let processes = listing
        .lines()
        .filter_map(|line| line.strip_prefix('p'))
        .collect::<Vec<_>>();
    assert_eq!(processes, [pid.to_string()], "lsof listed other processes");
    listing
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(std::path::PathBuf::from)
        .collect()
}

/// The paths of the files process `pid` holds open, as the operating system
/// names them. A listing that cannot be made fails the test; it is never
/// taken as an empty one.
#[cfg(target_os = "linux")]
fn open_paths(pid: u32) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|entry| {
            match std::fs::read_link(entry.unwrap().path()) {
                Ok(target) => Ok(Some(target)),
                // A descriptor closed between listing and reading is no
                // longer held.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
            .unwrap()
        })
        .collect()
}

/// Creates the company home `name` with `settings` applied through the
/// configuration verbs and its Slack tokens in its own secrets file.
pub fn company_home(env: &TestEnv, name: &str, app: &str, bot: &str, settings: &str) {
    let here = env.path("");
    env.run(&here, ["config", "init", "--company", name])
        .success();
    // An onboarded company's home has its runtime project.
    env.place_company_project(name);
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

/// Publishes the procedure conversations start from: one declaring the
/// role `company-agent`, in a domain of a docs project of its own. The
/// docs project keeps no history, so it adds no installation demand of
/// its own to the computer.
pub fn publish_company_procedure(env: &TestEnv) {
    let docs = documented::project(
        env,
        "assistance-docs",
        &[
            (
                "instrukt.yaml",
                format!(
                    "schema_version: {}\nproject: assistance-docs\nmodules: {{history: false, todos: false}}\n",
                    instrukt_core::config::PROJECT_SCHEMA_VERSION
                ),
            ),
            (
                "docs/domain/assistance/procedure/company-assistance.md",
                documented::snippet(
                    "procedure",
                    "Company Assistance",
                    "Assist a company's people in their conversations.",
                    "role: company-agent\n",
                    &[],
                    "Carry out the message that addresses you.\n",
                ),
            ),
        ],
    );
    documented::sync(env, &docs);
}

impl Company {
    /// A company whose Slack connector declares `extra` fields, on a
    /// computer with conversations switched on.
    pub fn new(extra: &str) -> Self {
        Self::on(TestEnv::new(), "", "", extra)
    }

    /// The company on a computer whose global layer also sets `modules`
    /// (entries each led by `, `; history keeps its default unless one sets
    /// it) and holds `machine` settings, with the procedure its
    /// conversations start from published.
    pub fn on(env: TestEnv, modules: &str, machine: &str, extra: &str) -> Self {
        let company = Self::without_procedure(env, modules, machine, extra);
        company.publish_procedure();
        company
    }

    pub fn publish_procedure(&self) {
        publish_company_procedure(&self.env);
    }

    /// The company as [`Self::on`] makes it, on a computer where no
    /// procedure declares the role its conversations start under yet.
    pub fn without_procedure(env: TestEnv, modules: &str, machine: &str, extra: &str) -> Self {
        let company = Self::with_fakes(env);
        std::fs::write(
            company.env.instrukt_home().join("instrukt.yaml"),
            format!(
                "schema_version: {}\nrunner: {{runtime: t3}}\nmodules: {{{}}}\n{machine}",
                instrukt_core::config::GLOBAL_SCHEMA_VERSION,
                modules.trim_start_matches(", ")
            ),
        )
        .unwrap();
        company_home(
            &company.env,
            COMPANY,
            APP_TOKEN,
            BOT_TOKEN,
            &slack_settings(MEMBERS, extra),
        );
        company
    }

    /// Start independent stand-ins in the test home before applying settings.
    pub fn with_fakes(mut env: TestEnv) -> Self {
        let slack = FakeSlack::start();
        env.set_env("INSTRUKT_SLACK_API_URL", &slack.api_url);
        let store = T3Store::new(&env);
        let runtime = FakeT3::create(&env);
        Self {
            env,
            store,
            runtime,
            slack,
            runner: None,
            turns: 0,
        }
    }

    /// A company with its configuration and published procedure already in
    /// place. The recovery scenarios start after configuration and publication.
    pub fn prepared_for_recovery(env: TestEnv) -> Self {
        use instrukt_core::config::{
            COMPANY_SCHEMA_VERSION, GLOBAL_SCHEMA_VERSION, SecretSource, set_secret_in,
        };
        use instrukt_core::home::Home;
        use instrukt_core::index::GlobalIndex;

        let company = Self::with_fakes(env);
        let home = Home::at_user_home(company.env.home().into());
        let slug = ProjectSlug::parse(COMPANY).unwrap();
        std::fs::write(
            home.root().join("instrukt.yaml"),
            format!("schema_version: {GLOBAL_SCHEMA_VERSION}\nrunner: {{runtime: t3}}\nmodules: {{history: false}}\n"),
        ).unwrap();
        let company_path = home.company_dir(&slug);
        std::fs::create_dir_all(&company_path).unwrap();
        company.env.place_company_project(COMPANY);
        let config = company_path.join("company.yaml");
        let identity = format!("schema_version: {COMPANY_SCHEMA_VERSION}\ncompany: {COMPANY}\n");
        std::fs::write(&config, &identity).unwrap();
        let source = SecretSource::Company(slug);
        set_secret_in(&home, &source, "SLACK_APP_TOKEN", APP_TOKEN).unwrap();
        set_secret_in(&home, &source, "SLACK_BOT_TOKEN", BOT_TOKEN).unwrap();
        std::fs::write(
            config,
            format!(
                "{identity}members: {MEMBERS}\nconnectors: {{slack: {{app_token: '${{SLACK_APP_TOKEN}}', bot_token: '${{SLACK_BOT_TOKEN}}'}}}}\n",
            ),
        ).unwrap();

        let procedure = "assistance/procedure/company-assistance.md";
        let description = "Assist a company's people in their conversations.";
        let snippet = home.root().join("docs").join(procedure);
        std::fs::create_dir_all(snippet.parent().unwrap()).unwrap();
        std::fs::write(
            snippet,
            documented::snippet(
                "procedure",
                "Company Assistance",
                description,
                "role: company-agent\n",
                &[],
                "Carry out the message that addresses you.\n",
            ),
        )
        .unwrap();
        let mut index = GlobalIndex::default();
        index.snippets.push(
            serde_yaml_ng::from_str(concat!(
                "id: assistance/procedure/company-assistance\n",
                "description: Assist a company's people in their conversations.\n",
                "type: procedure\n",
                "scope: global\n",
                "domain: assistance\n",
                "path: assistance/procedure/company-assistance.md\n",
                "source_project: assistance-docs\n",
                "role: company-agent\n",
            ))
            .unwrap(),
        );
        let state = home.root().join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("index.yaml"),
            serde_yaml_ng::to_string(&index).unwrap(),
        )
        .unwrap();
        company
    }

    /// Starts the long-lived runner and waits for its outward connection.
    pub fn run(&mut self) {
        let before = self.slack.connections();
        self.start_runner();
        wait("the connector's outward connection", || {
            self.slack.connections() > before
        });
    }

    pub fn start_runner(&mut self) {
        self.runner = Some(Running::spawn(
            self.env
                .instrukt_process(self.env.home())
                .arg("run")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
        ));
        std::fs::write(
            self.runtime.state_dir().join("runner-owner"),
            self.runner.as_ref().unwrap().child.id().to_string(),
        )
        .unwrap();
    }

    /// Whether the runner process is still running.
    pub fn runner_running(&mut self) -> bool {
        self.runner
            .as_mut()
            .is_some_and(|runner| runner.exited().is_none())
    }

    /// The runner process dies where it stands.
    pub fn runner_dies(&mut self) {
        self.runner = None;
    }

    /// Stops the runner as a service manager does (SIGTERM) and waits until
    /// its process has ended.
    pub fn stop_runner(&mut self) {
        let runner = self.runner.as_mut().unwrap();
        runner.signal(rustix::process::Signal::TERM);
        wait("the runner's end", || runner.exited().is_some());
    }

    /// Waits until the runner process has ended on its own.
    pub fn runner_ended(&mut self) {
        let runner = self.runner.as_mut().unwrap();
        wait("the runner's end", || runner.exited().is_some());
    }

    /// How many descriptors the running runner holds open on the folder
    /// `dir` itself. A kqueue watch of the folder holds one; reading files
    /// inside it holds none.
    pub fn runner_files_on(&self, dir: &std::path::Path) -> usize {
        let runner = self.runner.as_ref().unwrap();
        let dir = dir.canonicalize().unwrap();
        open_paths(runner.child.id())
            .iter()
            .filter(|path| **path == dir)
            .count()
    }

    /// The agent sessions started so far.
    pub fn sessions(&self) -> Vec<String> {
        self.runtime.threads()
    }

    /// A member asks in a conversation of their own: the agent's session for
    /// it starts and its answer is posted. The session's id.
    pub fn asked_and_answered(&mut self, question: &str, answer: &str) -> String {
        let posts = self.slack.posts().len();
        let session = self.ask_and_wait_for_session(question);
        self.finish_turn(&session, 0, answer);
        wait("the answer", || self.slack.posts().len() == posts + 1);
        session
    }

    /// Sends a new question and waits for its native start.
    pub fn ask_and_wait_for_session(&self, question: &str) -> String {
        let before = self.sessions();
        self.slack
            .mention("C1", "USTEVEN", &format!("<@UAGENT> {question}"), None);
        self.wait_for_new_session(&before)
    }

    pub fn wait_for_new_session(&self, before: &[String]) -> String {
        let mut started = None;
        wait("the question's session", || {
            let after = self.sessions();
            if after.len() == before.len() + 1 {
                started = after.into_iter().find(|session| !before.contains(session));
            }
            started.is_some()
        });
        started.unwrap()
    }

    /// Whether `instrukt events status` names a failing readiness refresh.
    pub fn readiness_failing(&self) -> bool {
        self.status()["workers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["worker"] == "readiness")
    }

    /// How many times `t3 auth session issue` was launched.
    pub fn issue_launches(&self) -> usize {
        self.runtime
            .calls()
            .iter()
            .filter(|call| call.starts_with("auth session issue "))
            .count()
    }

    /// The one session, once a message started it.
    pub fn session(&self) -> String {
        wait("the conversation's session", || {
            self.runtime.threads().len() == 1
        });
        self.runtime.threads().remove(0)
    }

    /// Sends an explicit member-visible reply via `instrukt company reply`
    /// while acting as `session`.
    pub fn reply(&self, session: &str, conversation: Option<&str>, text: &str) {
        let native = self.runtime.native_id(session);
        let here = self.env.path("");
        let mut args = vec!["company", "reply"];
        if let Some(conv) = conversation {
            args.extend(["--conversation", conv]);
        }
        args.extend(["--text", "-"]);
        self.env
            .instrukt(&here)
            .env("T3CODE_THREAD_ID", &native)
            .args(args)
            .write_stdin(text)
            .assert()
            .success();
    }

    /// Sets the active turn's curated member-visible status line via
    /// `instrukt company status` while acting as `session`.
    pub fn set_status_line(&self, session: &str, conversation: Option<&str>, text: &str) {
        let native = self.runtime.native_id(session);
        let here = self.env.path("");
        let mut args = vec!["company", "status"];
        if let Some(conv) = conversation {
            args.extend(["--conversation", conv]);
        }
        args.extend(["--text", text]);
        self.env
            .instrukt(&here)
            .env("T3CODE_THREAD_ID", &native)
            .args(args)
            .assert()
            .success();
    }

    /// Completes the active turn idle without invoking `instrukt company reply`,
    /// optionally recording raw assistant output in the fake thread to verify
    /// that raw output is never posted to members.
    pub fn finish_idle_without_reply(&mut self, session: &str, raw_output: Option<&str>) {
        let native = self.runtime.native_id(session);
        if let Some(raw) = raw_output {
            self.runtime.finish(&native, "idle", raw);
        } else {
            self.runtime.set_status(&native, "idle");
        }
    }

    /// Marks the active turn failed on the runtime projection.
    pub fn fail_turn(&mut self, session: &str) {
        let native = self.runtime.native_id(session);
        self.runtime.set_status(&native, "failed");
    }

    /// The runtime finishes the turn started by the session's `n`th input,
    /// answering with `reply` via explicit `instrukt company reply` (when
    /// `reply` is non-empty and not `"NOOP"`), and the session becomes idle.
    pub fn finish_turn(&mut self, session: &str, n: usize, reply: &str) {
        let native = self.runtime.native_id(session);
        let session = native.as_str();
        self.finish_projected_turn(session, n, reply);
    }

    /// Complete a turn in an already projected thread: calls
    /// `instrukt company reply` when `reply` is a real answer, then marks the
    /// runtime thread idle (`TurnState::Completed`).
    pub fn finish_projected_turn(&mut self, session: &str, n: usize, reply: &str) {
        let native = self.runtime.native_id(session);
        let session = native.as_str();
        self.turns += 1;
        if !reply.is_empty() && reply != "NOOP" {
            wait_for_binding(&self.env, session);
            let input = self.runtime.message(session, n);
            let explicit_conv = input.lines().find_map(|line| {
                line.strip_prefix("Reply command: instrukt company reply --conversation ")
                    .and_then(|rest| rest.split_whitespace().next())
                    .map(str::to_owned)
            });
            self.reply(session, explicit_conv.as_deref(), reply);
            self.runtime.set_status(session, "idle");
        } else {
            self.runtime.set_status(session, "idle");
        }
    }

    /// What the session's `n`th accepted input carries, as the registered
    /// vocabulary reads it: the member who addressed the agent, and who said
    /// each accompanying message and whether it was the agent's own post.
    pub fn delivered(&self, session: &str, n: usize) -> Delivered {
        let input = self.runtime.message(session, n);
        let spans = envelopes::segment(&input);
        let body = spans
            .iter()
            .find_map(|span| match span {
                Span::Member { body, .. } => Some(*body),
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
        Delivered {
            left_out: context.left_out,
            empty: usize::from(body.is_empty())
                + context
                    .entries
                    .iter()
                    .filter(|entry| entry.chars == 0)
                    .count(),
            accompanying: context.entries.len(),
            members: spans
                .iter()
                .filter(|span| matches!(span, Span::Member { .. }))
                .count(),
        }
    }

    /// The session's first input whole, with the delivery's boundary token
    /// as a placeholder: what the session opens with.
    pub fn opening(&self, session: &str) -> String {
        let input = self.runtime.message(session, 0);
        let boundary = envelopes::segment(&input)
            .iter()
            .find_map(|span| match span {
                Span::Member { boundary, .. } => Some((*boundary).to_owned()),
                _ => None,
            })
            .unwrap();
        input.replace(&boundary, "[BOUNDARY]")
    }

    /// The session's `n`th input as the company's agent reads it, from its
    /// conversation context on, with the delivery's boundary token as a
    /// placeholder: what a snapshot of a delivered conversation records.
    pub fn input(&self, session: &str, n: usize) -> String {
        let input = self.runtime.message(session, n);
        let boundary = envelopes::segment(&input)
            .iter()
            .find_map(|span| match span {
                Span::Member { boundary, .. } => Some((*boundary).to_owned()),
                _ => None,
            })
            .unwrap();
        let start = input.find("[Instrukt Conversation Context").unwrap();
        input[start..].replace(&boundary, "[BOUNDARY]")
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

    /// Ends the runner the way a signal does, with no chance to record it.
    pub fn kill_runner(&mut self) {
        drop(self.runner.take());
    }
}
