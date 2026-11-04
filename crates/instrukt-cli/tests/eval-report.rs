//! `local/projects/instrukt/spec/feature/work/eval-report`: an HTML
//! evaluation report derived from the work record of every project the home
//! publishes, by project and evaluation window, with the model a separate
//! dimension and human acts in a row of their own.
//!
//! The eval fingerprint comes from real publication: the `flows` project
//! publishes the `crew` flavor with an `eval: true` policy through
//! `instrukt sync`, and republishes the policy changed. The model comes from
//! the agent runtime's `model` template. Two projects selecting that flavor,
//! `demo` and `lists`, record work reflected to the fake GitHub.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod support;
#[path = "support/workflow.rs"]
mod workflow;

use std::path::{Path, PathBuf};

use instrukt_core::config::CURRENT_SCHEMA_VERSION;
use support::{Project, redact, stdout};

/// The review policy the flows project publishes into the eval set.
fn review_policy(rule: &str) -> String {
    documented::snippet(
        "policy",
        "Review",
        "How work is reviewed.",
        "eval: true\n",
        &[],
        &format!("{rule}\n"),
    )
}

/// The developer's project `demo`, whose agent runtime reports the model
/// named in a file the test controls; the `flows` project publishing the
/// eval set; and a second project, `lists`, recording work of its own.
struct Evaluated {
    project: Project,
    model: PathBuf,
    flavor: workflow::Flavor,
    lists: common::Repo,
}

/// A project layer for `slug` that tracks work on GitHub `repository`
/// under the `crew` flavor.
fn tracked(slug: &str, repository: &str) -> String {
    format!(
        "schema_version: {CURRENT_SCHEMA_VERSION}\nproject: {slug}\nmodules:\n  todos: true\n\
         github:\n  repository: {repository}\nworkflow:\n  domain: crew\n"
    )
}

impl Evaluated {
    fn new() -> Self {
        let mut project = Project::with_runtime(Some("AGENT_SESSION"));
        let model = project.env.path("model");
        let layer = project.env.instrukt_home().join("instrukt.yaml");
        let global = std::fs::read_to_string(&layer).unwrap();
        std::fs::write(
            &layer,
            format!("{global}    model: cat '{}'\n", model.display()),
        )
        .unwrap();
        project.env.set_env("AGENT_SESSION", "s1");
        project.env.set_env("INSTRUKT_NOW", "2030-01-05T08:00:00Z");
        let flavor = workflow::Flavor::publish(&project.env);
        // The eval set: a review policy, published an hour later.
        project.env.set_env("INSTRUKT_NOW", "2030-01-05T09:00:00Z");
        flavor.repo.write(
            "docs/global/crew/policy/review.md",
            &review_policy("Review every change against its requirements."),
        );
        flavor.republish(&project.env, "put the review policy in the eval set");
        project
            .repo
            .write("instrukt.yaml", &tracked("demo", support::REPOSITORY));
        project.repo.commit("select the crew workflow");
        project.github.add_repo("acme/lists");
        let lists = project.env.git_repo("lists");
        lists
            .write("instrukt.yaml", &tracked("lists", "acme/lists"))
            .write("README.md", "# lists\n");
        lists.commit("the lists project");
        Self {
            project,
            model,
            flavor,
            lists,
        }
    }

    /// The runtime reports `model` from now on.
    fn uses(&self, model: &str) {
        std::fs::write(&self.model, format!("{model}\n")).unwrap();
    }

    /// The clock reads `at` from now on.
    fn at(&mut self, at: &str) {
        self.project.env.set_env("INSTRUKT_NOW", at);
    }

    /// The eval set changes: the review policy is rewritten and published.
    fn change_process(&self, rule: &str) {
        self.flavor
            .repo
            .write("docs/global/crew/policy/review.md", &review_policy(rule));
        self.flavor
            .republish(&self.project.env, "tighten the review policy");
    }

    /// `instrukt todo <args>` in the project checked out at `repo`.
    fn todo(&self, repo: &Path, args: &[&str]) {
        self.project
            .env
            .instrukt(repo)
            .arg("todo")
            .args(args)
            .assert()
            .success();
    }

    /// Files `slug` in the project at `repo` and reviews its prd: a first
    /// review returning it for more when `returned`, then an approving one.
    fn reviewed(&self, repo: &Path, slug: &str, returned: bool) {
        let todo = |args: &[&str]| self.todo(repo, args);
        todo(&[
            "create",
            slug,
            "Keep list depth.",
            "--kind",
            "feature",
            "--approved",
        ]);
        todo(&["artifacts", "write", slug, "prd", "Lists keep their depth."]);
        if returned {
            todo(&[
                "artifacts",
                "write",
                slug,
                "prd-review-findings",
                "Mixed lists are undefined.",
            ]);
            todo(&[
                "stamp",
                slug,
                "--verdict",
                "needs_work",
                "--phase",
                "prd_review",
            ]);
            todo(&[
                "artifacts",
                "write",
                slug,
                "prd",
                "Lists keep their depth, mixed lists too.",
            ]);
        }
        todo(&[
            "artifacts",
            "write",
            slug,
            "prd-review-findings",
            "No findings.",
        ]);
        todo(&[
            "stamp",
            slug,
            "--verdict",
            "approve",
            "--phase",
            "prd_review",
        ]);
    }

    /// A human approves `slug`'s prd, stating it in a conversation.
    fn approved_by_human(&self, repo: &Path, slug: &str) {
        self.todo(
            repo,
            &[
                "stamp",
                slug,
                "--prd-approved",
                "--by",
                "human",
                "--session",
                "thread-host:thread-7",
            ],
        );
    }

    /// `instrukt eval report` with `args`: what it printed, and the HTML
    /// it wrote at `html`, temporary paths named `[TMP]`.
    fn report(&self, args: &[&str], html: &Path) -> (String, String) {
        let env = &self.project.env;
        let printed = stdout(
            &env.instrukt(self.project.repo.path())
                .args(["eval", "report"])
                .args(args)
                .assert()
                .success(),
        );
        let base = env.home().parent().unwrap().display().to_string();
        let page = std::fs::read_to_string(html).unwrap();
        (printed.replace(&base, "[TMP]"), redact(&page))
    }
}

// spec: local/projects/instrukt/spec/feature/work/eval-report UC-ER1
#[test]
fn a_report_compares_outcomes_across_process_changes_and_models_in_every_project() {
    let mut eval = Evaluated::new();
    let (demo, lists) = (
        eval.project.repo.path().to_path_buf(),
        eval.lists.path().to_path_buf(),
    );
    // Both work projects publish too, the eval set unchanged: no process
    // change.
    eval.at("2030-01-05T12:00:00Z");
    documented::sync(&eval.project.env, &eval.project.repo);
    eval.at("2030-01-05T12:30:00Z");
    documented::sync(&eval.project.env, &eval.lists);
    // Under the first eval set, with one model.
    eval.uses("claude-opus 4.1");
    eval.at("2030-01-06T09:00:00Z");
    eval.reviewed(&demo, "nested-lists", true);
    eval.approved_by_human(&demo, "nested-lists");
    // The process changes; the same model works on, then another, in
    // both projects.
    eval.at("2030-01-12T09:00:00Z");
    eval.change_process("Review every change against its requirements and its tests.");
    eval.at("2030-01-13T09:00:00Z");
    eval.reviewed(&demo, "parser-depth", false);
    eval.uses("claude-sonnet 4.5");
    eval.at("2030-01-14T09:00:00Z");
    eval.reviewed(&demo, "renderer", true);
    eval.reviewed(&lists, "list-markers", false);
    eval.approved_by_human(&lists, "list-markers");
    eval.at("2030-01-15T09:00:00Z");

    let html = demo.join("scratchpad/eval/index.html");
    let (printed, page) = eval.report(&[], &html);

    insta::assert_snapshot!("printed", printed);
    insta::assert_snapshot!("report", page);
}

// spec: local/projects/instrukt/spec/feature/work/eval-report UC-ER1
#[test]
fn with_no_recorded_work_the_report_says_there_is_nothing_to_compare() {
    let mut eval = Evaluated::new();
    eval.at("2030-01-15T09:00:00Z");
    let html = eval.project.env.path("elsewhere/report.html");

    let (printed, page) = eval.report(&["--out", html.to_str().unwrap()], &html);

    insta::assert_snapshot!("nothing-printed", printed);
    insta::assert_snapshot!("nothing-to-compare", page);
}
