//! A developer's project working through a workflow flavor: a second
//! project publishes the `crew` domain (`workflow/workflow.yaml` and the
//! procedures declaring its roles) with a real `instrukt sync`; the
//! developer's project selects it with `workflow.domain`, records its work
//! on the fake GitHub, and runs agent sessions through a fake runtime
//! whose caller session variable is `T3CODE_THREAD_ID`.
#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use instrukt_core::config::CURRENT_SCHEMA_VERSION;

use crate::common::{Repo, TestEnv};
use crate::fixtures::runtime::FakeRuntime;
use crate::fixtures::work::{Project, REPOSITORY, stdout};

/// The flavor's procedures: path below `procedure/`, and the role each
/// declares.
const PROCEDURES: [(&str, &str); 11] = [
    ("todo-lead", "todo-lead"),
    ("review-commits", "commit-reviewer"),
    ("intake", "intaker"),
    ("review-prd", "prd-reviewer"),
    ("plan", "planner"),
    ("review-plan", "plan-reviewer"),
    ("build", "builder"),
    ("review-build", "build-reviewer"),
    ("deferrals", "deferrer"),
    ("finalize", "finalizer"),
    ("drift-analyze", "drift-analyzer"),
];

const WORKFLOW: &str = "\
orchestrator: todo-lead
requires: []
roles:
  - todo-lead
  - commit-reviewer
  - integrator
  - intaker
  - prd-reviewer
  - planner
  - plan-reviewer
  - builder
  - fixer
  - build-reviewer
  - deferrer
  - finalizer
  - drift-analyzer
";

/// A procedure snippet declaring `role`, taking the todo's slug.
fn procedure(title: &str, role: &str) -> String {
    format!(
        "---\ndescription: The {title} procedure.\nrole: {role}\nactivation_trigger: When {title} is owed.\n\
         parameters:\n  - name: slug\n    required: true\n---\n\n\
         # {title} — Procedure\n\n## Goal\n\nDo {title}.\n\n## Preconditions\n\nNone.\n\n\
         ## Steps\n\n1. Do it.\n\n## Outputs\n\nDone.\n\n## Recovery\n\nRetry.\n"
    )
}

/// The flavor publisher's repository.
pub struct Flavor {
    pub repo: Repo,
}

impl Flavor {
    /// Writes the `crew` flavor into a new project `flows`, commits it and
    /// publishes it.
    pub fn publish(env: &TestEnv) -> Self {
        let repo = env.git_repo("flows");
        repo.write(
            "instrukt.yaml",
            &format!("schema_version: {CURRENT_SCHEMA_VERSION}\nproject: flows\n"),
        )
        .write("docs/domain/crew/workflow/workflow.yaml", WORKFLOW);
        for (name, role) in PROCEDURES {
            repo.write(
                &format!("docs/domain/crew/procedure/{name}.md"),
                &procedure(name, role),
            );
        }
        let flavor = Self { repo };
        flavor.republish(env, "publish the crew flavor");
        flavor
    }

    /// Commits the flavor's changes and publishes them.
    pub fn republish(&self, env: &TestEnv, message: &str) {
        self.repo.commit(message);
        env.run(self.repo.path(), ["sync"]).success();
    }
}

/// A developer's project selecting the `crew` flavor, with an `origin`
/// remote holding `main`, running in agent session `lead-1`.
pub struct Crew {
    pub project: Project,
    pub runtime: FakeRuntime,
    pub flavor: Flavor,
    pub origin: PathBuf,
}

impl Crew {
    pub fn new() -> Self {
        let mut project = Project::new();
        let runtime = FakeRuntime::new(&project.env, "agent", false);
        let global = format!(
            "schema_version: {CURRENT_SCHEMA_VERSION}\ngithub:\n  api_url: {}\nrunner: {{runtime: t3, notify: 'off'}}\n",
            project.github.url()
        );
        std::fs::write(project.env.instrukt_home().join("instrukt.yaml"), global).unwrap();
        project.github.configure(&project.env);
        let flavor = Flavor::publish(&project.env);
        project.repo.write(
            "instrukt.yaml",
            &format!(
                "schema_version: {CURRENT_SCHEMA_VERSION}\nproject: demo\nboard: acme/1\nmodules:\n  todos: true\ngithub:\n  repository: {REPOSITORY}\nworkflow:\n  domain: crew\n"
            ),
        );
        project.repo.commit("select the crew workflow");
        let origin = project.env.bare_remote("demo-origin");
        project
            .repo
            .git(["remote", "add", "origin", origin.to_str().unwrap()]);
        project
            .repo
            .git(["push", "--quiet", "-u", "origin", "main"]);
        project.env.set_env("T3CODE_THREAD_ID", "lead-1");
        Self {
            project,
            runtime,
            flavor,
            origin,
        }
    }

    pub fn root(&self) -> &Path {
        self.project.repo.path()
    }

    /// Runs `instrukt todo work <slug>` from `cwd`: its output and exit status.
    pub fn next_in(&self, cwd: &Path, _flow: &str, slug: &str) -> (String, bool) {
        let branch = self.project.env.git(cwd, ["branch", "--show-current"]);
        let in_workspace = branch == format!("{slug}/work");
        let mut command = self.project.env.instrukt(cwd);
        command.args(["todo", "work", slug]);
        if in_workspace {
            let proof = crate::fixtures::runtime::placement_evidence(
                &self.project.env,
                &crate::fixtures::runtime::NativePlacement {
                    cwd,
                    slug,
                    role: "todo-lead",
                    session: self
                        .project
                        .env
                        .injected("T3CODE_THREAD_ID")
                        .unwrap_or("outside-context"),
                },
            );
            command
                .args(["--placement-evidence", "-"])
                .write_stdin(proof);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.code().is_some(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        (
            String::from_utf8(output.stdout).unwrap().replace(
                self.project
                    .env
                    .path("")
                    .to_str()
                    .unwrap()
                    .trim_end_matches('/'),
                "[TMP]",
            ),
            output.status.success(),
        )
    }

    /// Ensures the single `<slug>/work` worktree exists and returns its path.
    pub fn workspace(&self, slug: &str) -> PathBuf {
        let folder = self.project.env.path(&format!("team-{slug}-work"));
        if !folder.exists() {
            self.project.repo.git([
                "worktree",
                "add",
                "--quiet",
                "-b",
                &format!("{slug}/work"),
                folder.to_str().unwrap(),
                "main",
            ]);
        }
        folder
    }

    pub fn phase_workspace(&self, slug: &str, _prepare: bool) -> PathBuf {
        self.workspace(slug)
    }

    pub fn next(&self, _flow: &str, slug: &str) -> String {
        let path = self.workspace(slug);
        let (out, ok) = self.next_in(&path, "work", slug);
        assert!(ok, "{out}");
        out
    }

    /// Supply a native child result to record-only attempt/bind commands.
    pub fn worker(&self, slug: &str, role: &str, procedure: Option<&str>) -> serde_json::Value {
        self.next("work", slug);
        let mut cmd = self.project.env.instrukt(self.root());
        cmd.args([
            "todo", "attempt", slug, "--role", role, "--brief", "-", "--json",
        ]);
        if let Some(procedure) = procedure {
            cmd.args(["--procedure", procedure, "--args", slug]);
        }
        let out = cmd.write_stdin("Do the assigned role.").assert().success();
        let mut value: serde_json::Value =
            serde_json::from_slice(&out.get_output().stdout).unwrap();
        let id = format!("{}-{}", role, value["attempt"].as_u64().unwrap());
        self.runtime.open_session(&id);
        let path = std::path::Path::new(value["worktree"].as_str().unwrap());
        self.project
            .env
            .instrukt(self.root())
            .args([
                "todo",
                "bind",
                slug,
                "--role",
                role,
                "--attempt",
                &value["attempt"].to_string(),
                "--session",
                &format!("t3:{id}"),
                "--worktree",
                path.to_str().unwrap(),
                "--branch",
                value["branch"].as_str().unwrap(),
                "--base",
                value["base"].as_str().unwrap(),
            ])
            .assert()
            .success();
        value["session"] = format!("t3:{id}").into();
        value
    }

    /// Runs `instrukt todo <args>` in `cwd`, which must succeed.
    pub fn todo_in<I, S>(&self, cwd: &Path, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        stdout(
            &self
                .project
                .env
                .instrukt(cwd)
                .arg("todo")
                .args(args)
                .assert()
                .success(),
        )
        .replace(
            self.project
                .env
                .path("")
                .to_str()
                .unwrap()
                .trim_end_matches('/'),
            "[TMP]",
        )
    }

    /// Runs `instrukt todo <args>` in the project, which must succeed.
    pub fn todo<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.todo_in(self.root(), args)
    }

    /// Writes a version of artifact `name` of `slug`.
    pub fn write(&self, slug: &str, name: &str, content: &str) {
        self.todo(["artifacts", "write", slug, name, content]);
    }

    /// `todo stamp <slug> <args>` in the project.
    pub fn stamp(&self, slug: &str, args: &[&str]) {
        let mut all = vec!["stamp", slug];
        all.extend_from_slice(args);
        self.todo(all);
    }
}
