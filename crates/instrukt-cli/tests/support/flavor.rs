//! A developer's project that tracks its work with the todos module,
//! reflects it to a fake GitHub and selects a small published workflow
//! flavor, `acme`, whose lane orchestrators and task runner are procedures
//! declaring their roles. Sessions start on the fake runtime of
//! `support/runtime.rs`.
//!
//! The flavor: a change without an implementation plan owes the prepare
//! flow's planner, so its lane is preparation; a task owes its own
//! preparation until its task plan is approved, after which it runs.
#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::ffi::OsStr;

use assert_cmd::assert::Assert;
use instrukt_todos::evidence::artifact_names;

use crate::common::github::FakeGitHub;
use crate::common::{Repo, TestEnv};
use crate::documented::{snippet, sync};
use crate::runtime::{FakeRuntime, global_layer};

/// The GitHub repository the project's work is reflected to.
pub const REPOSITORY: &str = "acme/demo";

const ACTIONS: &str = "\
roles:
  - prepare-orchestrator
  - work-orchestrator
  - planner
  - task-preparer
  - task-runner
draft-plan:
  command: instrukt sessions run --procedure acme/procedure/plan --args {slug} --todo {slug} --brief -
  template: templates/draft-plan.md
prepare-task:
  command: instrukt sessions run --procedure acme/procedure/prepare-task --args {slug} --todo {slug} --brief -
  template: templates/prepare-task.md
release-prepared:
  command: instrukt todo update {slug} --unclaim
  template: templates/release-prepared.md
build:
  command: instrukt todo show {slug}
  template: templates/build.md
await-task-runner:
  command: instrukt todo show {slug}
  template: templates/await-task-runner.md
";

const PREPARE: &str = "\
flow: prepare
orchestrator: prepare-orchestrator
initial: start
states: [start, task_preparation, task_released, planning]
transitions:
- from: start
  to: task_preparation
  when: {equals: [kind, task]}
  action: prepare-task
- from: start
  to: planning
  when: {not: {exists: implementation_plan.digest}}
  action: draft-plan
- from: task_preparation
  to: task_released
  when:
    all:
    - same: [task_plan.digest, task_approved.target_digest]
    - any:
      - equals: [task_approved.by, human]
      - equals: [task_approved.by, peer]
  action: release-prepared
";

const WORK: &str = "\
flow: work
orchestrator: work-orchestrator
initial: start
states: [start, task_runner_route, building]
transitions:
- from: start
  to: task_runner_route
  when: {equals: [kind, task]}
  action: await-task-runner
- from: start
  to: building
  when: {exists: implementation_plan.digest}
  action: build
";

/// The project, its fake GitHub and the machine's fake runtime.
pub struct Tracked {
    pub env: TestEnv,
    pub github: FakeGitHub,
    pub repo: Repo,
    pub rt: FakeRuntime,
}

fn procedure(name: &str, role: &str, parameters: bool) -> (String, String) {
    let extra = if parameters {
        format!("role: {role}\nparameters:\n  - name: slug\n    required: true\n")
    } else {
        format!("role: {role}\n")
    };
    (
        format!("docs/global/acme/procedure/{name}.md"),
        snippet(
            "procedure",
            name,
            &format!("The {role}'s procedure."),
            &extra,
            &[],
            "Do the work.\n",
        ),
    )
}

impl Tracked {
    /// A published project whose `runner.autostart` is `autostart`.
    pub fn new(autostart: bool) -> Self {
        Self::with_global_modules(autostart, "todos: true")
    }

    /// As [`Tracked::new`], with the machine's global `modules` switches.
    /// The project's own layer always switches todos on.
    pub fn with_global_modules(autostart: bool, modules: &str) -> Self {
        let mut env = TestEnv::new();
        let github = FakeGitHub::start();
        github.add_repo(REPOSITORY);
        let rt = FakeRuntime::new(&env, "rt", false);
        global_layer(&env, modules, &[&rt]);
        github.configure(&env);
        env.set_env("GITHUB_TOKEN", "test-token");
        let repo = env.git_repo("demo");
        repo.write(
            "instrukt.yaml",
            &format!(
                "schema_version: 1\nproject: demo\nmodules: {{todos: true}}\n\
                 runner: {{autostart: {autostart}}}\nworkflow: {{domain: acme}}\n\
                 github: {{repository: {REPOSITORY}}}\n\
                 publishes:\n  - slug: acme\n    trigger: When working on acme.\n"
            ),
        );
        repo.write("docs/global/acme/workflow/actions.yaml", ACTIONS);
        repo.write("docs/global/acme/workflow/prepare.yaml", PREPARE);
        repo.write("docs/global/acme/workflow/work.yaml", WORK);
        for template in [
            "draft-plan",
            "prepare-task",
            "release-prepared",
            "build",
            "await-task-runner",
        ] {
            repo.write(
                &format!("docs/global/acme/workflow/templates/{template}.md"),
                &format!("{template} for {{slug}}\n"),
            );
        }
        for (name, role, parameters) in [
            ("prepare-overview", "prepare-orchestrator", true),
            ("work-overview", "work-orchestrator", true),
            ("plan", "planner", true),
            ("prepare-task", "task-preparer", true),
            ("run-task", "task-runner", true),
        ] {
            let (path, text) = procedure(name, role, parameters);
            repo.write(&path, &text);
        }
        repo.commit("docs: the acme flavor");
        sync(&env, &repo);
        Self {
            env,
            github,
            repo,
            rt,
        }
    }

    /// Runs `instrukt todo <args>` in the project; it must succeed.
    pub fn todo<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let run = self
            .env
            .instrukt(self.repo.path())
            .arg("todo")
            .args(args)
            .assert();
        let output = run.get_output();
        assert!(
            output.status.success(),
            "todo failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout.clone()).unwrap()
    }

    /// Runs `instrukt todo <args>` inside the fake runtime's session `id`,
    /// as that session's agent; it must succeed.
    pub fn todo_in_session<I, S>(&mut self, id: &str, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.env.set_env("RT_SESSION", id);
        let out = self.todo(args);
        self.env.remove_env("RT_SESSION");
        out
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

    /// A wake-up whose clock reads `at` (`INSTRUKT_NOW`), as the OS
    /// scheduler would give it at that time.
    pub fn tick_at(&mut self, at: &str) -> Assert {
        self.env.set_env("INSTRUKT_NOW", at);
        let run = self.tick();
        self.env.remove_env("INSTRUKT_NOW");
        run
    }

    /// An authorized task with `schedule` and `extra` creation flags.
    pub fn authorized_task(&self, slug: &str, schedule: &str, extra: &[&str]) {
        let mut args = vec![
            "create",
            slug,
            "Post the weekly status summary.",
            "--kind",
            "task",
            "--approved",
            "--schedule",
            schedule,
        ];
        args.extend_from_slice(extra);
        self.todo(args);
        self.todo([
            "artifacts",
            "write",
            slug,
            artifact_names::TASK_PLAN,
            "Collect the week's merged work and post a summary.",
        ]);
        self.todo(["stamp", slug, "--task-approved", "--by", "human"]);
    }
}
