//! A developer's project on a fake GitHub whose pull requests are reviewed
//! automatically: events on, the `review-commits` procedure published and
//! subscribed to `runner.review_needed`, the project and reviewer Apps
//! installed, and `main` pushed to a bare `origin`. Sessions start on the
//! fake runtime of `support/runtime.rs`.
#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::path::{Path, PathBuf};

use assert_cmd::assert::Assert;
use serde_json::json;

use crate::common::github::{FakeGitHub, PullSeed};
use crate::common::{Repo, TestEnv};
use crate::fixtures::documented::{snippet, sync};
use crate::fixtures::runtime::{FakeRuntime, global_layer_with};

/// The GitHub repository of the project.
pub const REPOSITORY: &str = "acme/demo";

/// The reviewer App's id on the fake.
pub const REVIEWER_APP: u64 = 3;

/// The project App's id on the fake.
pub const PROJECT_APP: u64 = 1;

/// The project, its fake GitHub and the machine's fake runtime.
pub struct Reviewed {
    pub env: TestEnv,
    pub github: FakeGitHub,
    pub repo: Repo,
    pub rt: FakeRuntime,
}

impl Reviewed {
    /// A project with events on and the review procedure published, not yet
    /// opted in; both Apps installed with what automatic review needs.
    pub fn new() -> Self {
        let mut env = TestEnv::new();
        let github = FakeGitHub::start();
        github.add_repo(REPOSITORY);
        let rt = FakeRuntime::new(&env, "rt", false);
        global_layer_with(&env, "events: true", &[&rt], "ticks: off\n");
        github.configure(&env);
        env.set_env("GITHUB_TOKEN", "test-token");
        let repo = env.git_repo("demo");
        repo.write(
            "instrukt.yaml",
            &format!(
                "schema_version: {}\nproject: demo\nmodules: {{events: true}}\ngithub: {{repository: {REPOSITORY}}}\n",
                instrukt_core::config::PROJECT_SCHEMA_VERSION
            ),
        );
        repo.write(
            "docs/domain/sdlc/procedure/lifecycle/work/review-commits.md",
            &snippet(
                "procedure",
                "Review Commits",
                "Review a committed head independently and record the verdict.",
                "role: commit-reviewer\nsubscriptions:\n  - runner.review_needed\nsubscription_home: project\nsubscription_identity: subject\nsubscription_attendance: unattended\nparameters:\n  - name: slug\n    required: false\n",
                &[],
                "Review the head.\n",
            ),
        );
        repo.commit("docs: the review procedure");
        sync(&env, &repo);
        if !repo.git(["status", "--porcelain"]).is_empty() {
            repo.commit("fixture: include generated publication files");
        }
        let origin = env.bare_remote("demo-origin");
        repo.git(["remote", "add", "origin", origin.to_str().unwrap()]);
        repo.git(["push", "--quiet", "-u", "origin", "main"]);
        github.set_branch(REPOSITORY, "main", &repo.head());
        let reviewed = Self {
            env,
            github,
            repo,
            rt,
        };
        reviewed.grant_reviewer(true);
        reviewed
    }

    /// As [`Reviewed::new`], opted in through a reviewed commit of the
    /// project layer.
    pub fn opted_in() -> Self {
        let reviewed = Self::new();
        let layer = reviewed.repo.read("instrukt.yaml");
        reviewed.repo.write(
            "instrukt.yaml",
            &format!("{layer}review: {{automatic: true}}\n"),
        );
        reviewed.repo.commit("config: opt in to automatic review");
        reviewed.repo.git(["push", "--quiet", "origin", "main"]);
        reviewed
            .github
            .set_branch(REPOSITORY, "main", &reviewed.repo.head());
        reviewed
    }

    /// Installs the reviewer App with commit-status write access, or
    /// without it.
    pub fn grant_reviewer(&self, statuses: bool) {
        let statuses = if statuses { "write" } else { "read" };
        self.github.set_installation_permissions(
            REVIEWER_APP,
            json!({"contents":"read","pull_requests":"write","statuses":statuses,"issues":"write","checks":"read"}),
        );
    }

    /// Commits `file` on branch `branch` from `main` (or from the branch's
    /// head when it exists), pushes it and moves the branch on the fake;
    /// returns the new head.
    pub fn push(&self, branch: &str, file: &str) -> String {
        let exists = !self
            .repo
            .git(["branch", "--list", branch])
            .trim()
            .is_empty();
        if exists {
            self.repo.git(["checkout", "--quiet", branch]);
        } else {
            self.repo.git(["checkout", "--quiet", "-b", branch, "main"]);
        }
        self.repo.write(file, &format!("{branch} {file}\n"));
        let head = self.repo.commit(&format!("feat: {file}"));
        self.repo.git(["push", "--quiet", "origin", branch]);
        self.repo.git(["checkout", "--quiet", "main"]);
        self.github.set_branch(REPOSITORY, branch, &head);
        head
    }

    /// Opens a pull request from `head` into `main` by a member of the
    /// repository; returns its number.
    pub fn open(&self, head: &str, draft: bool) -> u64 {
        self.github.seed_pull(
            REPOSITORY,
            PullSeed {
                title: format!("Change {head}"),
                head: head.to_owned(),
                base: "main".into(),
                draft,
                login: Some("dev".into()),
                association: Some("MEMBER".into()),
                fork: None,
            },
        )
    }

    /// A wake-up, as the OS scheduler gives it, from the user's home.
    pub fn tick(&self) -> Assert {
        let run = self.env.run(self.env.home(), ["tick"]);
        let output = run.get_output();
        assert!(
            output.status.success(),
            "tick failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        run
    }

    /// A wake-up whose clock reads `at` (`INSTRUKT_NOW`).
    pub fn tick_at(&mut self, at: jiff::Timestamp) -> Assert {
        self.env.set_env("INSTRUKT_NOW", &at.to_string());
        let run = self.tick();
        self.env.remove_env("INSTRUKT_NOW");
        run
    }

    /// `instrukt` in the project's checkout as session `session`.
    pub fn as_session(&self, session: &str) -> assert_cmd::Command {
        let mut command = self.env.instrukt(self.repo.path());
        command.env("T3CODE_THREAD_ID", session);
        command
    }

    /// Records `session`'s verdict of `head` from the merge base with main.
    pub fn record(&self, session: &str, head: &str, verdict: &str) -> Assert {
        self.record_at(session, head, verdict, None)
    }

    /// As [`Reviewed::record`], with the command's clock reading `at`.
    pub fn record_at(&self, session: &str, head: &str, verdict: &str, at: Option<&str>) -> Assert {
        let base = self.repo.git(["merge-base", head, "main"]);
        let mut command = self.as_session(session);
        if let Some(at) = at {
            command.env("INSTRUKT_NOW", at);
        }
        command
            .args([
                "review",
                "record",
                "--head",
                head,
                "--base",
                &base,
                "--verdict",
                verdict,
                "--report",
                "-",
            ])
            .write_stdin(format!("# Review\n\nVerdict: {verdict}\n"))
            .assert()
    }

    pub fn root(&self) -> &Path {
        self.repo.path()
    }

    /// A linked worktree of the project at `name`, on a new branch `branch`
    /// from `main`, with `file` committed on it; returns its path and head.
    pub fn worktree(&self, name: &str, branch: &str, file: &str) -> (PathBuf, String) {
        let path = self.env.path(name);
        self.repo.git([
            "worktree",
            "add",
            "--quiet",
            "-b",
            branch,
            path.to_str().unwrap(),
            "main",
        ]);
        let head = self.commit_in(&path, file);
        (path, head)
    }

    /// Commits `file` in the worktree at `path`; returns the new head.
    pub fn commit_in(&self, path: &Path, file: &str) -> String {
        std::fs::write(path.join(file), format!("{file}\n")).unwrap();
        self.env.git(path, ["add", "--all"]);
        self.env
            .git(path, ["commit", "--quiet", "-m", &format!("feat: {file}")]);
        self.env.git(path, ["rev-parse", "HEAD"])
    }

    /// A file outside every worktree with `contents`, or only its path
    /// when `contents` is `None`.
    pub fn report(&self, name: &str, contents: Option<&str>) -> PathBuf {
        let path = self.env.path("reports").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if let Some(contents) = contents {
            std::fs::write(&path, contents).unwrap();
        }
        path
    }

    /// `instrukt review start` from `worktree` as the developer's agent
    /// session, naming the head's merge base with main as the base, the
    /// report destination `report` and the `extra` options.
    pub fn ask(
        &self,
        worktree: &Path,
        branch: &str,
        head: &str,
        report: &Path,
        extra: &[&str],
    ) -> Assert {
        let base = self.repo.git(["merge-base", head, "main"]);
        let mut command = self.env.instrukt(worktree);
        command.env("T3CODE_THREAD_ID", "developer-agent");
        command
            .args(["review", "start", "--worktree"])
            .arg(worktree)
            .args([
                "--branch", branch, "--head", head, "--base", &base, "--report",
            ])
            .arg(report)
            .args(extra)
            .assert()
    }

    /// The run's temporary paths, `commits` and the fake runtime's
    /// `sessions` in `text`, replaced by stable placeholders.
    pub fn redact(&self, text: &str, commits: &[(&str, &str)], sessions: &[&str]) -> String {
        let base = self.env.path("");
        let base = base.to_str().unwrap().trim_end_matches('/');
        let canonical = std::fs::canonicalize(base).unwrap();
        let mut text = text
            .replace(canonical.to_str().unwrap(), "[TMP]")
            .replace(base, "[TMP]");
        for (sha, name) in commits {
            text = text.replace(sha, &format!("[{name}]"));
        }
        for (index, session) in sessions.iter().enumerate() {
            text = text.replace(
                &self.rt.native_id(session),
                &format!("[SESSION-{}]", index + 1),
            );
        }
        let machine = self.env.instrukt_home().join("state/machine-id");
        if let Ok(machine) = std::fs::read_to_string(machine) {
            text = text.replace(machine.trim(), "[MACHINE]");
        }
        text
    }
}
