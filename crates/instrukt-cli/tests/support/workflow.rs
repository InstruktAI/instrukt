//! A developer's project working through a workflow flavor: a second
//! project publishes the `crew` domain (its workflow files and the
//! procedures declaring its roles) with a real `instrukt sync`; the
//! developer's project selects it with `workflow.domain`, records its work
//! on the fake GitHub, and runs agent sessions through a fake runtime
//! whose session variable is `AGENT_SESSION`.
#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use instrukt_core::config::CURRENT_SCHEMA_VERSION;

use crate::common::{Repo, TestEnv};
use crate::runtime::FakeRuntime;
use crate::support::{Project, REPOSITORY, stdout};

/// The flavor's procedures: path below `procedure/`, and the role each
/// declares.
const PROCEDURES: [(&str, &str); 11] = [
    ("prepare-lead", "prepare-lead"),
    ("work-lead", "work-lead"),
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

const ACTIONS: &str = "\
roles: [prepare-lead, work-lead, intaker, prd-reviewer, planner, plan-reviewer, builder, build-reviewer, deferrer, finalizer, drift-analyzer]
draft-prd:
  command: instrukt docs get crew/procedure/intake
  template: templates/draft-prd.md
review-prd:
  command: instrukt sessions run --procedure crew/procedure/review-prd --args {slug} --todo {slug} --brief -
  template: templates/review-prd.md
revise-prd:
  command: instrukt docs get crew/procedure/intake
  template: templates/revise-prd.md
await-prd-approval:
  command: instrukt todo show {slug}
  template: templates/await-prd-approval.md
draft-plan:
  command: instrukt docs get crew/procedure/plan
  template: templates/draft-plan.md
review-plan:
  command: instrukt sessions run --procedure crew/procedure/review-plan --args {slug} --todo {slug} --brief -
  template: templates/review-plan.md
revise-plan:
  command: instrukt docs get crew/procedure/plan
  template: templates/revise-plan.md
analyze-drift:
  command: instrukt sessions run --procedure crew/procedure/drift-analyze --args {slug} --todo {slug} --brief -
  template: templates/analyze-drift.md
release-prepared:
  command: instrukt todo update {slug} --unclaim
  template: templates/release-prepared.md
await-prepare:
  command: instrukt docs get crew/procedure/prepare-lead
  template: templates/await-prepare.md
dispatch-build:
  command: instrukt sessions run --procedure crew/procedure/build --args {slug} --worktree {slug}/work --todo {slug} --brief -
  template: templates/dispatch-build.md
review-build:
  command: instrukt sessions run --procedure crew/procedure/review-build --args {slug} --todo {slug} --brief -
  template: templates/review-build.md
relay-findings:
  command: instrukt todo show {slug}
  template: templates/relay-findings.md
process-deferrals:
  command: instrukt sessions run --procedure crew/procedure/deferrals --args {slug} --todo {slug} --brief -
  template: templates/process-deferrals.md
dispatch-finalize:
  command: instrukt sessions run --procedure crew/procedure/finalize --args {slug} --todo {slug} --brief -
  template: templates/dispatch-finalize.md
restamp-finalize:
  command: instrukt sessions run --procedure crew/procedure/finalize --args {slug} --todo {slug} --brief -
  template: templates/restamp-finalize.md
hand-off-integration:
  command: instrukt todo show {slug}
  template: templates/hand-off-integration.md
effects:
  branch.publish:
    command: git -C {worktree.path} push origin HEAD:{slug}
    done_when:
      same: [worktree.head, remote.head]
";

/// The approved and current verdict of review `r` on `target`.
fn current(r: &str, target: &str, verdict: &str, findings: &str) -> String {
    format!(
        "    - equals: [{r}.verdict, {verdict}]\n    - same: [{r}.target_digest, {target}]\n    - same: [{r}.report_digest, {findings}]\n"
    )
}

fn prepare_flow() -> String {
    let prd = |verdict| {
        current(
            "prd_review",
            "prd.digest",
            verdict,
            "prd_review_findings.digest",
        )
    };
    let plan = |verdict| {
        current(
            "plan_review",
            "implementation_plan.digest",
            verdict,
            "plan_review_findings.digest",
        )
    };
    format!(
        "flow: prepare
orchestrator: prepare-lead
initial: start
states: [start, prd_drafting, prd_review, prd_revision, prd_approval, plan_drafting, plan_review, plan_revision, drift_analysis, prepared]
transitions:
- from: start
  to: prd_drafting
  when: {{equals: [kind, feature]}}
  action: draft-prd
- from: start
  to: plan_drafting
  when: {{equals: [kind, change]}}
  action: draft-plan
- from: prd_drafting
  to: prd_review
  when: {{exists: prd.digest}}
  action: review-prd
- from: prd_review
  to: prd_revision
  when:
    all:
{needs_work_prd}  action: revise-prd
- from: prd_review
  to: prd_approval
  when:
    all:
{approved_prd}  action: await-prd-approval
- from: prd_approval
  to: plan_drafting
  when: {{same: [prd_approval.digest, prd.digest]}}
  action: draft-plan
- from: plan_drafting
  to: plan_review
  when: {{exists: implementation_plan.digest}}
  action: review-plan
- from: plan_review
  to: plan_revision
  when:
    all:
{needs_work_plan}  action: revise-plan
- from: plan_review
  to: drift_analysis
  when:
    all:
{approved_plan}    - any:
      - equals: [drift.prd, pending]
      - equals: [drift.implementation_plan, pending]
  action: analyze-drift
- from: plan_review
  to: prepared
  when:
    all:
{approved_plan}  action: release-prepared
",
        needs_work_prd = prd("needs_work"),
        approved_prd = prd("approve"),
        needs_work_plan = plan("needs_work"),
        approved_plan = plan("approve"),
    )
}

fn work_flow() -> String {
    let plan = current(
        "plan_review",
        "implementation_plan.digest",
        "approve",
        "plan_review_findings.digest",
    );
    let not_plan = plan.replace("    - ", "        - ");
    let bound = "    - any:\n      - equals: [build_review.binding, exact_target]\n      - equals: [build_review.binding, clean_upstream_merges]\n";
    format!(
        "flow: work
orchestrator: work-lead
initial: start
states: [start, not_prepared, building, review, fixing, approved, deferrals_owed, handoff_pending, finalize_mismatch]
transitions:
- from: start
  to: not_prepared
  when:
    not:
      all:
{not_plan}  action: await-prepare
- from: start
  to: building
  when:
    all:
{plan}  action: dispatch-build
- from: building
  to: approved
  when:
    all:
    - equals: [build_review.verdict, approve]
{bound}  action: dispatch-finalize
  effects: [branch.publish]
- from: building
  to: review
  when: {{equals: [build.complete, true]}}
  action: review-build
- from: review
  to: fixing
  when:
    all:
    - equals: [build_review.verdict, changes_requested]
{bound}  action: relay-findings
- from: review
  to: approved
  when:
    all:
    - equals: [build_review.verdict, approve]
{bound}  action: dispatch-finalize
  effects: [branch.publish]
- from: approved
  to: deferrals_owed
  when: {{equals: [deferrals.pending, true]}}
  action: process-deferrals
- from: approved
  to: handoff_pending
  when:
    all:
    - equals: [finalize.status, ready]
    - equals: [worktree.clean, true]
    - same: [worktree.head, branch.head]
    - same: [branch.head, remote.head]
    - same: [finalize.sha, worktree.head]
  action: hand-off-integration
- from: approved
  to: finalize_mismatch
  when: {{equals: [finalize.status, ready]}}
  action: restamp-finalize
"
    )
}

/// Each action's instructions: what is owed, naming the evidence it binds.
const TEMPLATES: [(&str, &str); 17] = [
    ("draft-prd", "Draft the PRD of {slug} from its input."),
    (
        "review-prd",
        "Review the PRD of {slug} (version {prd.digest}).",
    ),
    (
        "revise-prd",
        "Revise the PRD of {slug} against findings {prd_review_findings.digest}.",
    ),
    (
        "await-prd-approval",
        "Ask the developer to approve PRD {prd.digest} of {slug}.",
    ),
    ("draft-plan", "Draft the implementation plan of {slug}."),
    (
        "review-plan",
        "Review plan {implementation_plan.digest} of {slug}.",
    ),
    (
        "revise-plan",
        "Revise plan {implementation_plan.digest} of {slug} against its findings.",
    ),
    ("release-prepared", "{slug} is prepared: release the claim."),
    (
        "analyze-drift",
        "Judge the drift of {slug}: PRD {drift.prd}, plan {drift.implementation_plan}.",
    ),
    (
        "await-prepare",
        "{slug} is not prepared; return it to preparation.",
    ),
    (
        "dispatch-build",
        "Build {slug} against plan {implementation_plan.digest}.",
    ),
    (
        "review-build",
        "Review the build of {slug} at {build_review.round} earlier rounds.",
    ),
    (
        "relay-findings",
        "Relay findings {build_review.verdict} on {build_review.target_digest} to the builder of {slug}.",
    ),
    ("process-deferrals", "Decide each deferral of {slug}."),
    (
        "dispatch-finalize",
        "Finalize {slug} from its approved build ({build_review.binding}).",
    ),
    (
        "restamp-finalize",
        "Finalize readiness of {slug} names {finalize.sha}; reconcile and restamp it.",
    ),
    (
        "hand-off-integration",
        "Hand {finalize.sha} of {slug} to the integrator.",
    ),
];

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
            &format!(
                "schema_version: {CURRENT_SCHEMA_VERSION}\nproject: flows\npublishes:\n  - slug: crew\n    trigger: When the crew works.\n"
            ),
        )
        .write("docs/global/crew/workflow/actions.yaml", ACTIONS)
        .write("docs/global/crew/workflow/prepare.yaml", &prepare_flow())
        .write("docs/global/crew/workflow/work.yaml", &work_flow());
        for (action, text) in TEMPLATES {
            repo.write(
                &format!("docs/global/crew/workflow/templates/{action}.md"),
                &format!("{text}\n"),
            );
        }
        for (name, role) in PROCEDURES {
            repo.write(
                &format!("docs/global/crew/procedure/{name}.md"),
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
            "schema_version: {CURRENT_SCHEMA_VERSION}\ngithub:\n  api_url: {}\nrunner: {{notify: 'off'}}\nruntimes:\n{}",
            project.github.url(),
            runtime.entry()
        );
        std::fs::write(project.env.instrukt_home().join("instrukt.yaml"), global).unwrap();
        let flavor = Flavor::publish(&project.env);
        project.repo.write(
            "instrukt.yaml",
            &format!(
                "schema_version: {CURRENT_SCHEMA_VERSION}\nproject: demo\nmodules:\n  todos: true\ngithub:\n  repository: {REPOSITORY}\nworkflow:\n  domain: crew\n"
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
        project.env.set_env("AGENT_SESSION", "lead-1");
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

    /// Runs `instrukt todo <flow> <slug>` from `cwd`: its output,
    /// and whether everything owed rendered.
    pub fn next_in(&self, cwd: &Path, flow: &str, slug: &str) -> (String, bool) {
        let output = self
            .project
            .env
            .instrukt(cwd)
            .args(["todo", flow, slug])
            .output()
            .unwrap();
        assert!(
            output.status.code().is_some(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        (
            String::from_utf8(output.stdout).unwrap(),
            output.status.success(),
        )
    }

    /// `instrukt todo <flow> <slug>` from the project, which must render
    /// everything it owes; its output.
    pub fn next(&self, flow: &str, slug: &str) -> String {
        let (out, ok) = self.next_in(self.root(), flow, slug);
        assert!(ok, "{out}");
        out
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
