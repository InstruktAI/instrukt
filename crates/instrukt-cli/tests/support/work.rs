//! A project with the todos module on, reflecting its work to a fake
//! GitHub, for the work feature tests.
#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::collections::BTreeMap;
use std::ffi::OsStr;

use assert_cmd::assert::Assert;
use instrukt_core::config::CURRENT_SCHEMA_VERSION;

use crate::common::github::FakeGitHub;
use crate::common::{Repo, TestEnv};

/// The GitHub repository the project's work is reflected to.
pub const REPOSITORY: &str = "acme/demo";

/// A developer's project: a repository with one commit on `main`, its
/// project layer switching todos on, and a fake GitHub holding
/// [`REPOSITORY`] with a token in the environment.
pub struct Project {
    pub env: TestEnv,
    pub github: FakeGitHub,
    pub repo: Repo,
}

impl Project {
    pub fn new() -> Self {
        Self::with_runtime(None)
    }

    /// A project on a machine whose runtime mapping hosts agent sessions
    /// identified by `session_env`.
    pub fn with_runtime(session_env: Option<&str>) -> Self {
        let github = FakeGitHub::start();
        github.add_repo(REPOSITORY).add_project("acme", 1);
        let (env, repo) = machine(&github.url(), session_env);
        Self { env, github, repo }
    }

    /// Another computer of the same developer team: its own home and
    /// checkout of the project, reflecting to the same GitHub.
    pub fn other_computer(&self) -> Computer {
        let (env, repo) = machine(&self.github.url(), None);
        Computer { env, repo }
    }

    /// Runs `instrukt todo <args>` in the repository.
    pub fn todo<I, S>(&self, args: I) -> Assert
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        todo(&self.env, &self.repo, args)
    }

    /// Runs a todo command that must succeed; returns its stdout.
    pub fn ok<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        stdout(&self.todo(args).success()).replace(
            self.env.path("").to_str().unwrap().trim_end_matches('/'),
            "[TMP]",
        )
    }

    /// Files `slug` of `kind` with `intent`.
    pub fn create(&self, slug: &str, kind: &str, intent: &str) -> String {
        self.ok(["create", slug, intent, "--kind", kind, "--approved"])
    }
}

/// A computer of the developer team reflecting the project to `github`:
/// its own home and checkout.
pub fn computer(github: &FakeGitHub) -> Computer {
    let (env, repo) = machine(&github.url(), None);
    Computer { env, repo }
}

/// A second computer working on the same project.
pub struct Computer {
    pub env: TestEnv,
    pub repo: Repo,
}

impl Computer {
    pub fn todo<I, S>(&self, args: I) -> Assert
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        todo(&self.env, &self.repo, args)
    }

    pub fn ok<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        stdout(&self.todo(args).success())
    }
}

fn todo<I, S>(env: &TestEnv, repo: &Repo, args: I) -> Assert
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    env.instrukt(repo.path()).arg("todo").args(args).assert()
}

/// A computer: a temporary home whose global layer points at the GitHub
/// API `api_url` (with `session_env` naming an agent runtime's session
/// variable, if given), a token, and the project's checkout.
fn machine(api_url: &str, session_env: Option<&str>) -> (TestEnv, Repo) {
    let mut env = TestEnv::new();
    let mut global =
        format!("schema_version: {CURRENT_SCHEMA_VERSION}\ngithub:\n  api_url: {api_url}\n");
    let _ = session_env;
    global.push_str("runner: {runtime: t3}\n");
    let path = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, global).unwrap();
    crate::common::github::configure_apps(&env, api_url);
    env.set_env("GITHUB_TOKEN", "test-token");
    let repo = env.git_repo("demo");
    repo.write(
            "instrukt.yaml",
            &format!(
                "schema_version: {CURRENT_SCHEMA_VERSION}\nproject: demo\nboard: acme/1\nmodules:\n  todos: true\ngithub:\n  repository: {REPOSITORY}\n"
            ),
        );
    repo.write("README.md", "# demo\n");
    repo.commit("the developer's project");
    (env, repo)
}

pub fn stdout(assert: &Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

/// `text` with what changes from run to run replaced: every time by
/// `<time>` (the real clock decides which commands share a second), and
/// content digests, commits and identities by `<digest N>`, `<commit N>`
/// and `<id N>` in order of first appearance. The same value always gets
/// the same name, so a snapshot still shows which values are equal.
pub fn redact(text: &str) -> String {
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut times: Vec<String> = timestamps(text);
    times.sort();
    times.dedup();
    for time in &times {
        names.insert(time.clone(), "<time>".to_owned());
    }
    let mut out = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(time) = times.iter().find(|t| rest.starts_with(t.as_str())) {
            out.push_str(&names[time]);
            rest = &rest[time.len()..];
            continue;
        }
        let run = rest
            .find(|c: char| !(c.is_ascii_digit() || ('a'..='f').contains(&c)))
            .unwrap_or(rest.len());
        let kind = match run {
            64 if out.ends_with("sha256:") => Some("digest"),
            64 => Some("event"),
            40 => Some("commit"),
            32 => Some("id"),
            _ => None,
        };
        let preceded_by_word = out
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric());
        match kind {
            Some(kind) if !preceded_by_word || kind == "digest" => {
                let value = &rest[..run];
                let name = names.entry(value.to_owned()).or_insert_with(|| {
                    let count = counts.entry(kind).or_default();
                    *count += 1;
                    format!("<{kind} {count}>")
                });
                if kind == "digest" {
                    out.truncate(out.len() - "sha256:".len());
                }
                out.push_str(name);
                rest = &rest[run..];
            }
            _ => {
                let step = if run == 0 {
                    rest.chars().next().map_or(1, char::len_utf8)
                } else {
                    run
                };
                out.push_str(&rest[..step]);
                rest = &rest[step..];
            }
        }
    }
    out
}

/// The `YYYY-MM-DDTHH:MM:SSZ` times in `text`.
fn timestamps(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    for at in 0..bytes.len().saturating_sub(19) {
        if !bytes[at].is_ascii_digit()
            || !text.is_char_boundary(at)
            || bytes.get(at + 4) != Some(&b'-')
            || bytes.get(at + 10) != Some(&b'T')
        {
            continue;
        }
        if let Some(end) = text[at..].find('Z').filter(|n| *n < 40) {
            let candidate = &text[at..=at + end];
            if candidate.parse::<jiff::Timestamp>().is_ok() {
                found.push(candidate.into());
            }
        }
    }
    found
}
