//! `local/projects/instrukt/spec/feature/work/eval-report`: an HTML
//! evaluation report derived from the work record, by project and
//! evaluation window, with the model a separate dimension.
//!
//! The eval fingerprint comes from real publication: a second project
//! publishes an `eval: true` policy with `instrukt sync`, and republishes
//! it changed. The model comes from the agent runtime's `model` template.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/work.rs"]
mod support;

use std::path::Path;

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

/// A project whose agent runtime reports the model named in a file the
/// test controls, and a flows project publishing the eval set.
struct Evaluated {
    project: Project,
    model: std::path::PathBuf,
    flows: common::Repo,
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
        let flows = documented::project(
            &project.env,
            "flows",
            &["crew"],
            &[(
                "docs/global/crew/policy/review.md",
                review_policy("Review every change against its requirements."),
            )],
        );
        documented::sync(&project.env, &flows);
        Self {
            project,
            model,
            flows,
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
        self.flows
            .write("docs/global/crew/policy/review.md", &review_policy(rule));
        self.flows.commit("tighten the review policy");
        documented::sync(&self.project.env, &self.flows);
    }

    /// Files `slug` and reviews its prd: a first review returning it for
    /// more when `returned`, then an approving one.
    fn reviewed(&self, slug: &str, returned: bool) {
        let p = &self.project;
        p.create(slug, "feature", "Keep list depth through parse and render.");
        p.ok(["artifacts", "write", slug, "prd", "Lists keep their depth."]);
        if returned {
            p.ok([
                "artifacts",
                "write",
                slug,
                "prd-review-findings",
                "Depth is undefined for mixed lists.",
            ]);
            p.ok([
                "stamp",
                slug,
                "--verdict",
                "needs_work",
                "--phase",
                "prd_review",
            ]);
            p.ok([
                "artifacts",
                "write",
                slug,
                "prd",
                "Lists keep their depth, mixed lists too.",
            ]);
        }
        p.ok([
            "artifacts",
            "write",
            slug,
            "prd-review-findings",
            "No findings.",
        ]);
        p.ok([
            "stamp",
            slug,
            "--verdict",
            "approve",
            "--phase",
            "prd_review",
        ]);
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
fn a_report_compares_outcomes_across_process_changes_and_models() {
    let mut eval = Evaluated::new();
    // Under the first eval set, with one model.
    eval.uses("claude-opus 4.1");
    eval.at("2030-01-06T09:00:00Z");
    eval.reviewed("nested-lists", true);
    eval.project.ok([
        "stamp",
        "nested-lists",
        "--prd-approved",
        "--by",
        "human",
        "--session",
        "thread-host:thread-7",
    ]);
    // The process changes; the same model works on, then another.
    eval.change_process("Review every change against its requirements and its tests.");
    eval.at("2030-01-13T09:00:00Z");
    eval.reviewed("parser-depth", false);
    eval.uses("claude-sonnet 4.5");
    eval.at("2030-01-14T09:00:00Z");
    eval.reviewed("renderer", true);
    eval.at("2030-01-15T09:00:00Z");

    let html = eval.project.repo.path().join("scratchpad/eval/index.html");
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
