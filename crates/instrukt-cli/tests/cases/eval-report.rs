//! `project/instrukt/spec/feature/work/eval-report`: an HTML
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

use crate::common;
use crate::fixtures::documented;
use crate::fixtures::history as fleet;
use crate::fixtures::runtime;
use crate::fixtures::work as support;
use crate::fixtures::workflow;
use std::path::{Path, PathBuf};

use instrukt_core::config::PROJECT_SCHEMA_VERSION;
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
        "schema_version: {PROJECT_SCHEMA_VERSION}\nproject: {slug}\nmodules:\n  todos: true\n\
         github:\n  repository: {repository}\nworkflow:\n  domain: crew\n"
    )
}

impl Evaluated {
    fn new() -> Self {
        let mut project = Project::with_runtime(Some("T3CODE_THREAD_ID"));
        let runtime = runtime::FakeRuntime::new(&project.env, "t3", false);
        runtime.open_session("s1");
        let model = runtime.state_dir().join("s1.json");
        project.env.set_env("T3CODE_THREAD_ID", "s1");
        project.env.set_env("INSTRUKT_NOW", "2030-01-05T08:00:00Z");
        let flavor = workflow::Flavor::publish(&project.env);
        // The eval set: a review policy, published an hour later.
        project.env.set_env("INSTRUKT_NOW", "2030-01-05T09:00:00Z");
        flavor.repo.write(
            "docs/domain/crew/policy/review.md",
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
        let mut thread: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&self.model).unwrap()).unwrap();
        let mut words = model.split_whitespace();
        thread["providerInstanceId"] = words.next().unwrap().into();
        thread["model"] = words.next().unwrap().into();
        std::fs::write(&self.model, serde_json::to_vec(&thread).unwrap()).unwrap();
    }

    /// The clock reads `at` from now on.
    fn at(&mut self, at: &str) {
        self.project.env.set_env("INSTRUKT_NOW", at);
    }

    /// The eval set changes: the review policy is rewritten and published.
    fn change_process(&self, rule: &str) {
        self.flavor
            .repo
            .write("docs/domain/crew/policy/review.md", &review_policy(rule));
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
        let page = std::fs::read_to_string(html).unwrap();
        (self.without_tmp(&printed), redact(&self.without_tmp(&page)))
    }

    /// `text` with the test's temporary directory, as written or resolved,
    /// named `[TMP]`.
    fn without_tmp(&self, text: &str) -> String {
        let base = self.project.env.home().parent().unwrap().to_path_buf();
        let resolved = base.canonicalize().unwrap();
        text.replace(&resolved.display().to_string(), "[TMP]")
            .replace(&base.display().to_string(), "[TMP]")
    }
}

// spec: project/instrukt/spec/feature/work/eval-report UC-ER1
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

// spec: project/instrukt/spec/feature/work/eval-report UC-ER1
#[test]
fn with_no_recorded_work_the_report_says_there_is_nothing_to_compare() {
    let mut eval = Evaluated::new();
    eval.at("2030-01-15T09:00:00Z");
    let html = eval.project.env.path("elsewhere/report.html");

    let (printed, page) = eval.report(&["--out", html.to_str().unwrap()], &html);

    insta::assert_snapshot!("nothing-printed", printed);
    insta::assert_snapshot!("nothing-to-compare", page);
}

// spec: project/instrukt/spec/feature/work/eval-report UC-ER1
#[test]
fn work_recorded_in_a_project_whose_checkout_is_gone_stays_in_the_report() {
    let mut eval = Evaluated::new();
    let lists = eval.lists.path().to_path_buf();
    eval.at("2030-01-05T12:30:00Z");
    documented::sync(&eval.project.env, &eval.lists);
    eval.uses("claude-opus 4.1");
    eval.at("2030-01-06T09:00:00Z");
    eval.reviewed(&lists, "list-markers", true);
    // The project's checkout is moved away after its work was recorded.
    std::fs::rename(&lists, eval.project.env.path("lists-moved")).unwrap();
    eval.at("2030-01-15T09:00:00Z");

    let html = eval.project.env.path("elsewhere/report.html");
    let (printed, page) = eval.report(&["--out", html.to_str().unwrap()], &html);

    insta::assert_snapshot!("checkout-gone-printed", printed);
    insta::assert_snapshot!("checkout-gone-report", page);
}

/// Records `computer`'s work in project `demo` under both eval sets: one
/// todo under the first with `first_model`, the process changed, then one
/// under the second with `second_model` (returned once when `returned`).
fn records_work(eval: &mut Evaluated, todo: &str, first_model: &str, second_model: &str) {
    let demo = eval.project.repo.path().to_path_buf();
    eval.at("2030-01-05T12:00:00Z");
    documented::sync(&eval.project.env, &eval.project.repo);
    eval.uses(first_model);
    eval.at("2030-01-06T09:00:00Z");
    eval.reviewed(&demo, &format!("{todo}-first"), true);
    eval.at("2030-01-12T09:00:00Z");
    eval.change_process("Review every change against its requirements and its tests.");
    eval.uses(second_model);
    eval.at("2030-01-13T09:00:00Z");
    eval.reviewed(&demo, &format!("{todo}-second"), false);
}

/// `page` with each `flows` commit named by the computer whose flavor
/// repository made it, and each commits list in name order: the commits'
/// own identities, and so their sorted order, differ from run to run.
fn by_computer(page: &str, computers: &[(&str, &Evaluated)]) -> String {
    let mut page = page.to_owned();
    for (name, eval) in computers {
        let log = eval
            .project
            .env
            .git(eval.flavor.repo.path(), ["log", "--format=%H", "--reverse"]);
        for (number, commit) in log.lines().enumerate() {
            page = page.replace(commit, &format!("{name}'s commit {}", number + 1));
        }
    }
    let mut out = Vec::new();
    let mut list: Vec<&str> = Vec::new();
    let mut in_list = false;
    for line in page.lines() {
        if in_list && line.starts_with("<li>") {
            list.push(line);
            continue;
        }
        list.sort_unstable();
        out.append(&mut list);
        in_list = line == "<ul class=\"commits\">";
        out.push(line);
    }
    out.join("\n") + "\n"
}

// spec: project/instrukt/spec/feature/work/eval-report UC-ER2
#[test]
fn a_fleet_report_pools_every_computers_outcomes_by_eval_set_and_names_an_offline_one() {
    let (a_id, b_id, c_id, d_id) = (
        "aaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbb",
        "cccccccccccccccc",
        "dddddddddddddddd",
    );
    // A asks; B runs `instrukt run` with history off; C only ticks; D
    // ticked once and is offline since.
    let mut a = Evaluated::new();
    let mut b = Evaluated::new();
    let mut c = Evaluated::new();
    let mut d = Evaluated::new();
    let redis = common::redis::RedisServer::new(&a.project.env);
    records_work(&mut a, "a", "claude-opus 4.1", "claude-sonnet 4.5");
    records_work(&mut b, "b", "claude-opus 4.1", "claude-opus 4.1");
    records_work(&mut c, "c", "claude-sonnet 4.5", "claude-sonnet 4.5");
    // B also records work in a project only B knows.
    let lists = b.lists.path().to_path_buf();
    b.at("2030-01-13T12:00:00Z");
    documented::sync(&b.project.env, &b.lists);
    b.reviewed(&lists, "list-markers", false);
    for (env, id) in [
        (&a.project.env, a_id),
        (&b.project.env, b_id),
        (&c.project.env, c_id),
        (&d.project.env, d_id),
    ] {
        fleet::join_cluster(env, &redis.url, id);
    }
    let global = b.project.env.instrukt_home().join("instrukt.yaml");
    let mut layer = std::fs::read_to_string(&global).unwrap();
    layer.push_str("modules:\n  history: false\n");
    std::fs::write(&global, layer).unwrap();
    d.at("2030-01-14T08:00:00Z");
    documented::sync(&d.project.env, &d.project.repo);
    d.project.env.run(d.project.env.home(), ["tick"]).success();
    c.at("2030-01-15T08:59:00Z");
    c.project.env.run(c.project.env.home(), ["tick"]).success();
    b.at("2030-01-15T08:59:00Z");
    let _b_runner = fleet::evaluation_runner(&b.project.env);

    // A reports on the whole fleet; C answers on its next tick.
    a.at("2030-01-15T09:00:00Z");
    let html = a.project.env.path("elsewhere/report.html");
    let asking = a
        .project
        .env
        .instrukt_process(a.project.repo.path())
        .args(["eval", "report", "--out", html.to_str().unwrap()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut conn = fleet::cluster_connection(&a.project.env);
    common::wait_until(
        "the report's request on C's queue",
        std::time::Duration::from_secs(60),
        || {
            redis::cmd("LLEN")
                .arg(format!("instrukt:eval:requests:{c_id}"))
                .query::<usize>(&mut conn)
                .unwrap()
                >= 1
        },
    );
    c.project.env.run(c.project.env.home(), ["tick"]).success();
    let output = asking.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed = a.without_tmp(&String::from_utf8(output.stdout).unwrap());
    let page = std::fs::read_to_string(&html).unwrap();
    let page = redact(&a.without_tmp(&by_computer(
        &page,
        &[("a", &a), ("b", &b), ("c", &c), ("d", &d)],
    )));
    insta::assert_snapshot!("fleet-printed", printed);
    insta::assert_snapshot!("fleet-report", page);
}
