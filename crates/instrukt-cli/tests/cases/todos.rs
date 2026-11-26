//! `project/instrukt/spec/feature/work/todos`: project work recorded
//! as todos with explicit kinds, and withdrawn work recovered under its
//! identity.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use crate::fixtures::work as support;
use crate::fixtures::workflow::Crew;
use instrukt_todos::evidence::artifact_names;
use support::{Project, redact, stdout};

/// The identity a `create` reported (its last word).
fn identity(created: &str) -> String {
    created
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_owned()
}

// spec: project/instrukt/spec/feature/work/todos UC-TD4
#[test]
fn a_reused_name_gives_the_new_work_and_earlier_work_is_reached_by_its_address() {
    let project = Project::new();
    let head = project.repo.head();
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.ok(["deliver", "parser-depth", "--commit", &head]);
    project.create("parser-depth", "change", "Track table depth in the parser.");
    let lexer = identity(&project.create("lexer-depth", "change", "Count depth in the lexer."));
    project.ok(["deliver", "lexer-depth", "--commit", &head]);

    let reused = project.ok(["show", "parser-depth"]);
    let refused = project.todo(["show", "lexer-depth"]).failure();
    let address = format!("lexer-depth@{}", &lexer[..8]);
    let earlier = project.ok(["show", &address]);

    insta::assert_snapshot!(
        "reused-name",
        redact(&format!(
            "{reused}---\n{}---\n{earlier}",
            String::from_utf8_lossy(&refused.get_output().stderr)
        ))
        .replace(&lexer[..8], "<prefix>")
    );
}

// spec: project/instrukt/spec/feature/work/todos UC-TD5
#[test]
fn withdrawal_releases_the_working_copy_and_branch_and_refuses_while_work_would_be_lost() {
    let project = Project::new();
    let id = identity(&project.create("nested-lists", "change", "Keep nested list depth."));
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "implementation-plan",
        "1. Track depth in the parser.",
    ]);
    let copy = project.env.path("demo-nested-lists");
    project.repo.git([
        "worktree",
        "add",
        "--quiet",
        "-b",
        "nested-lists",
        copy.to_str().unwrap(),
    ]);
    std::fs::write(copy.join("parser.rs"), "fn parse() {}\n").unwrap();
    project.env.git(&copy, ["add", "--all"]);
    project
        .env
        .git(&copy, ["commit", "--quiet", "-m", "Track list depth"]);
    project
        .env
        .instrukt(&copy)
        .args(["todo", "stamp", "nested-lists", "--build-started"])
        .assert()
        .success();

    std::fs::write(copy.join("draft.rs"), "// unfinished\n").unwrap();
    let uncommitted = project.todo(["remove", "nested-lists"]).failure();
    std::fs::remove_file(copy.join("draft.rs")).unwrap();
    let workspace = project.repo.path().join("workspace/nested-lists");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("report.md"), "results\n").unwrap();
    let busy = project.todo(["remove", "nested-lists"]).failure();
    std::fs::remove_file(workspace.join("report.md")).unwrap();

    let withdrawn = project.ok(["remove", "nested-lists"]);
    let shown = project.ok(["show", &format!("nested-lists@{}", &id[..8])]);

    let base = project.env.path("").display().to_string();
    insta::assert_snapshot!(
        "withdrawn-without-loss",
        redact(&format!(
            "{}---\n{}---\n{withdrawn}---\n{shown}",
            String::from_utf8_lossy(&uncommitted.get_output().stderr),
            String::from_utf8_lossy(&busy.get_output().stderr),
        ))
        .replace(base.trim_end_matches('/'), "[TMP]")
    );
    assert!(!copy.exists(), "the working copy is released");
    assert!(!workspace.exists(), "the empty workspace is removed");
    assert!(
        project
            .repo
            .git(["branch", "--list", "nested-lists"])
            .is_empty(),
        "the branch is released"
    );
}

// spec: project/instrukt/spec/feature/work/todos UC-TD1
#[test]
fn a_request_filed_with_its_kind_is_retained_as_a_todo_with_a_durable_identity() {
    let project = Project::new();

    let filed = project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!("recorded", redact(&format!("{filed}---\n{shown}")));
}

// spec: project/instrukt/spec/feature/work/todos UC-TD2
#[test]
fn work_withdrawn_on_another_computer_is_restored_there_under_its_identity() {
    let project = Project::new();
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth through parse and render.",
    ]);
    project.ok(["remove", "nested-lists"]);

    let elsewhere = project.other_computer();
    let restored = elsewhere.ok(["reactivate", "nested-lists"]);
    let shown = elsewhere.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "restored-elsewhere",
        redact(&format!("{restored}---\n{shown}"))
    );
}

// spec: project/instrukt/spec/feature/work/todos UC-TD2
#[test]
fn a_name_two_withdrawn_todos_carried_restores_neither_until_one_is_addressed() {
    let project = Project::new();
    let here = identity(&project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    ));
    project.ok(["remove", "nested-lists"]);
    // Another computer files and withdraws different work under the name.
    let elsewhere = project.other_computer();
    let there = identity(&elsewhere.ok([
        "create",
        "nested-lists",
        "Keep the depth of nested tables.",
        "--kind",
        "feature",
        "--approved",
    ]));
    elsewhere.ok(["remove", "nested-lists"]);

    let ambiguous = project.todo(["reactivate", "nested-lists"]).failure();
    let address = format!("nested-lists@{}", &there[..8]);
    let restored = project.ok(["reactivate", &address]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "restore-names-each-candidate",
        redact(&format!(
            "{}---\n{restored}---\n{shown}",
            String::from_utf8_lossy(&ambiguous.get_output().stderr)
        ))
        .replace(&here[..8], "<here>")
        .replace(&there[..8], "<there>")
    );
}

// spec: project/instrukt/spec/feature/work/todos UC-TD2
#[test]
fn withdrawn_work_is_restored_with_its_intent_and_evidence_under_its_identity() {
    let project = Project::new();
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok([
        "artifacts",
        "write",
        "nested-lists",
        "prd",
        "Lists keep their depth through parse and render.",
    ]);
    let withdrawn = project.ok(["remove", "nested-lists"]);

    let restored = project.ok(["reactivate", "nested-lists"]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!(
        "restored",
        redact(&format!("{withdrawn}---\n{restored}---\n{shown}"))
    );
}

// spec: project/instrukt/spec/feature/work/todos UC-TD2
#[test]
fn withdrawn_work_past_its_recovery_window_is_gone_and_cannot_be_restored() {
    let mut project = Project::new();
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok(["remove", "nested-lists"]);

    // Eight days later: past the seven-day recovery window.
    let later =
        jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() + 8 * 86_400).unwrap();
    project.env.set_env("INSTRUKT_NOW", &later.to_string());
    let restore = project.todo(["reactivate", "nested-lists"]).failure();
    let withdrawn = project.todo(["list", "--removed"]).success();

    insta::assert_snapshot!(
        "gone-after-the-window",
        redact(&format!(
            "{}---\n{}{}",
            String::from_utf8_lossy(&restore.get_output().stderr),
            String::from_utf8_lossy(&withdrawn.get_output().stderr),
            stdout(&withdrawn)
        ))
    );
}

// spec: project/instrukt/spec/feature/work/todos UC-TD3
#[test]
fn filing_changing_preparing_and_working_on_a_todo_name_the_switch_while_tracking_is_off() {
    let project = Project::new();
    project.repo.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: demo\nmodules:\n  todos: false\n",
    );

    let refusals: Vec<String> = [
        vec![
            "create",
            "nested-lists",
            "Keep list depth.",
            "--kind",
            "feature",
            "--approved",
        ],
        vec!["update", "nested-lists", "--hold", "--reason", "waiting"],
        vec!["work", "nested-lists"],
    ]
    .into_iter()
    .map(|args| {
        let refused = project.todo(&args).failure();
        format!(
            "todo {}\n{}",
            args[0],
            String::from_utf8_lossy(&refused.get_output().stderr)
        )
    })
    .collect();

    insta::assert_snapshot!("tracking-off", refusals.join("---\n"));
    assert!(
        !project
            .env
            .instrukt_home()
            .join("state/instrukt.db")
            .exists(),
        "nothing was recorded"
    );
}

/// `instrukt bugs <args>` run in `cwd` of the project's computer.
fn bugs(project: &Project, cwd: &std::path::Path, args: &[&str]) -> assert_cmd::assert::Assert {
    project.env.instrukt(cwd).arg("bugs").args(args).assert()
}

/// The project as this computer's publication state records it, as a sync
/// of it leaves it.
fn recorded(project: &Project) {
    let home = instrukt_core::home::Home::at_user_home(project.env.home().to_path_buf());
    let record = instrukt_core::publication::PublishedRecord {
        project: instrukt_core::ids::ProjectSlug::parse("demo").unwrap(),
        commit: instrukt_core::ids::CommitSha::parse(&project.repo.head()).unwrap(),
        checkout: project.repo.path().to_path_buf(),
        remote: None,
    };
    std::fs::create_dir_all(home.publication_state_dir()).unwrap();
    std::fs::write(
        home.publication_state_dir().join("demo.yaml"),
        serde_yaml_ng::to_string(&record).unwrap(),
    )
    .unwrap();
}

/// What a command printed on stderr, the test's directory shown as `[TMP]`.
fn stderr(project: &Project, assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).replace(
        project.env.path("").to_str().unwrap().trim_end_matches('/'),
        "[TMP]",
    )
}

// spec: project/instrukt/spec/feature/work/todos UC-TD9
#[test]
fn a_report_without_approval_shows_the_questions_and_files_nothing() {
    let project = Project::new();
    let shown = bugs(
        &project,
        project.repo.path(),
        &["report", "Sync drops the ops domain"],
    )
    .success();
    assert!(stdout(&shown).is_empty());
    insta::assert_snapshot!("bug-report-questions", stderr(&project, &shown));
    insta::assert_snapshot!(project.ok(["list"]), @"no todos");
    // Showing the questions reads and writes nothing: the project's layer
    // as an older instrukt wrote it stays as it is.
    let layer = project.repo.path().join("instrukt.yaml");
    let older = std::fs::read_to_string(&layer).unwrap().replacen(
        &format!(
            "schema_version: {}",
            instrukt_core::config::PROJECT_SCHEMA_VERSION
        ),
        "schema_version: 5",
        1,
    );
    assert_ne!(older, std::fs::read_to_string(&layer).unwrap());
    std::fs::write(&layer, &older).unwrap();
    let shown = bugs(
        &project,
        project.repo.path(),
        &["report", "Sync drops the ops domain"],
    )
    .success();
    let shown = stderr(&project, &shown);
    let notice = shown.split("Nothing was filed.").next().unwrap_or_default();
    insta::assert_snapshot!(notice, @"instrukt: [TMP]/demo/instrukt.yaml: schema_version 5 inspected as 6; file migrates on the next writing command");
    assert_eq!(std::fs::read_to_string(&layer).unwrap(), older);
}

// spec: project/instrukt/spec/feature/work/todos UC-TD10
#[test]
fn a_faced_report_is_filed_as_a_bug_in_the_project_it_names() {
    let project = Project::new();
    // The project is one this computer records: it has published.
    recorded(&project);
    let elsewhere = project.env.home();
    let filed = bugs(
        &project,
        elsewhere,
        &[
            "report",
            "--approved",
            "Sync drops the ops domain",
            "--project",
            "demo",
            "--body",
            "`instrukt sync` in ops printed `published`; `instrukt docs index --domain ops` listed nothing.",
        ],
    )
    .success();
    let shown = project.ok(["show", "fix-sync-drops-the-ops-domain"]);
    insta::assert_snapshot!(
        "bug-report-filed",
        redact(&format!("{}---\n{shown}", stdout(&filed)))
    );

    // A project this computer does not hold: nothing is filed.
    let refused = bugs(
        &project,
        elsewhere,
        &[
            "report",
            "--approved",
            "Another defect",
            "--project",
            "elsewhere",
        ],
    )
    .failure()
    .code(1);
    insta::assert_snapshot!(stderr(&project, &refused), @"instrukt: this computer records no project `elsewhere`; name the owning project with `--project <slug>`: this computer records demo");
    insta::assert_snapshot!(
        redact(&project.ok(["list"])),
        @"fix-sync-drops-the-ops-domain · bug · active · unclaimed · last known <time> (less than a minute old)"
    );
}

// spec: project/instrukt/spec/feature/work/todos UC-TD11
#[test]
fn the_open_bugs_of_a_project_are_listed_and_nothing_else() {
    let project = Project::new();
    recorded(&project);
    project.create("dark-mode", "feature", "Offer a dark mode.");
    for description in [
        "Sync drops a domain",
        "Render loses a heading",
        "Hook waits forever",
    ] {
        bugs(
            &project,
            project.repo.path(),
            &["report", "--approved", description],
        )
        .success();
    }
    project.ok(["remove", "fix-hook-waits-forever"]);
    // One bug delivered, one parked in the icebox.
    for description in ["Status hides a section", "Index skips a domain"] {
        bugs(
            &project,
            project.repo.path(),
            &["report", "--approved", description],
        )
        .success();
    }
    let head = project.repo.head();
    project.ok(["deliver", "fix-status-hides-a-section", "--commit", &head]);
    project.ok(["freeze", "fix-index-skips-a-domain"]);
    // The project's layer as an older instrukt wrote it: reading the work
    // converts it in memory and leaves the file as it is.
    let layer = project.repo.path().join("instrukt.yaml");
    let current = std::fs::read_to_string(&layer).unwrap();
    let older = current.replacen(
        &format!(
            "schema_version: {}",
            instrukt_core::config::PROJECT_SCHEMA_VERSION
        ),
        "schema_version: 5",
        1,
    );
    assert_ne!(older, current);
    std::fs::write(&layer, &older).unwrap();
    let listed = bugs(&project, project.env.home(), &["list", "--project", "demo"]).success();
    insta::assert_snapshot!(stdout(&listed), @r"
    fix-sync-drops-a-domain | active | not started
    fix-render-loses-a-heading | active | not started
    fix-index-skips-a-domain | iceboxed | not started
    ");
    // Each read reports the pending migration.
    let mut notices = vec![stderr(&project, &listed)];
    for (at, args) in [
        (project.repo.path(), &["list"][..]),
        (
            project.env.home(),
            &["suppressions", "--project", "demo"][..],
        ),
    ] {
        notices.push(stderr(&project, &bugs(&project, at, args).success()));
    }
    // `--project` combines with every listing filter of the work.
    let mut filtered = Vec::new();
    for filter in ["--iceboxed", "--removed", "--delivered"] {
        let listed = project
            .env
            .instrukt(project.env.home())
            .args(["todo", "list", "--project", "demo", filter])
            .assert()
            .success();
        filtered.push(format!("{filter}: {}", redact(&stdout(&listed))));
        notices.push(stderr(&project, &listed));
    }
    insta::assert_snapshot!(notices.join(""), @r"
    instrukt: [TMP]/demo/instrukt.yaml: schema_version 5 inspected as 6; file migrates on the next writing command
    instrukt: [TMP]/demo/instrukt.yaml: schema_version 5 inspected as 6; file migrates on the next writing command
    instrukt: [TMP]/demo/instrukt.yaml: schema_version 5 inspected as 6; file migrates on the next writing command
    instrukt: [TMP]/demo/instrukt.yaml: schema_version 5 inspected as 6; file migrates on the next writing command
    instrukt: [TMP]/demo/instrukt.yaml: schema_version 5 inspected as 6; file migrates on the next writing command
    instrukt: [TMP]/demo/instrukt.yaml: schema_version 5 inspected as 6; file migrates on the next writing command
    ");
    insta::assert_snapshot!(filtered.join(""), @r"
    --iceboxed: fix-index-skips-a-domain · bug · iceboxed · unclaimed · last known <time> (less than a minute old)
    --removed: fix-hook-waits-forever · bug · withdrawn · unclaimed · last known <time> (less than a minute old)
    --delivered: fix-status-hides-a-section · bug · delivered · unclaimed · last known <time> (less than a minute old)
    ");
    assert_eq!(std::fs::read_to_string(&layer).unwrap(), older);
}

// spec: project/instrukt/spec/feature/work/todos UC-TD12
#[test]
fn a_bug_filed_as_ordinary_work_is_sent_to_the_front_door() {
    let project = Project::new();
    for approved in [false, true] {
        let mut args = vec![
            "create",
            "fix-render",
            "Render loses a heading.",
            "--kind",
            "bug",
        ];
        if approved {
            args.push("--approved");
        }
        let refused = project.todo(args).failure().code(1);
        insta::allow_duplicates! {
            insta::assert_snapshot!(stderr(&project, &refused), @"instrukt: a bug is not created as ordinary work: report it through the bug front door, `instrukt bugs report '<description>'`, which puts the questions to face before anything is filed");
        }
    }
    insta::assert_snapshot!(project.ok(["list"]), @"no todos");
}

/// Runs `instrukt <args>` in `tree` as the worker session `session`.
fn as_worker(crew: &Crew, tree: &std::path::Path, session: &str, args: &[&str]) {
    crew.project
        .env
        .instrukt(tree)
        .env("T3CODE_THREAD_ID", session.strip_prefix("t3:").unwrap())
        .args(args)
        .assert()
        .success();
}

/// The lines `todo show` prints for one role attempt, from its header to the
/// next entry.
fn attempt_record(shown: &str, attempt: &str) -> String {
    let header = format!("  {attempt}: ");
    shown
        .lines()
        .skip_while(|line| !line.starts_with(&header))
        .enumerate()
        .take_while(|(at, line)| *at == 0 || line.starts_with("    "))
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

// spec: project/instrukt/spec/feature/work/todos UC-TD13
#[test]
fn a_further_round_keeps_the_worker_that_did_the_earlier_one() {
    let mut crew = Crew::new();
    let slug = "retained-planner";
    crew.project
        .env
        .set_env("INSTRUKT_NOW", "2030-01-05T08:00:00Z");
    common::agent_profiles(
        &crew.project.env,
        "default: {order: [claude], tier: standard}\nprofiles:\n  claude:\n    standard: [{model: claude-opus-5-5, effort: high}]\n",
    );
    crew.project
        .create(slug, "change", "Keep list depth inside block quotes.");
    crew.next("prepare", slug);

    // The planner's earlier round completes and its plan is sent back.
    let first = crew.worker(slug, "planner", Some("crew/procedure/plan"));
    let session = first["session"].as_str().unwrap().to_owned();
    let tree = std::path::PathBuf::from(first["worktree"].as_str().unwrap());
    as_worker(
        &crew,
        &tree,
        &session,
        &[
            "todo",
            "artifacts",
            "write",
            slug,
            artifact_names::IMPLEMENTATION_PLAN,
            "1. Parse quotes first.",
        ],
    );
    as_worker(
        &crew,
        &tree,
        &session,
        &["todo", "stamp", slug, "--role-complete"],
    );
    crew.write(
        slug,
        artifact_names::PLAN_REVIEW_FINDINGS,
        "Nested quotes are unplanned.",
    );
    crew.stamp(slug, &["--verdict", "needs_work", "--phase", "plan_review"]);
    let earlier = attempt_record(&crew.todo(["show", slug]), "planner/1");
    assert!(!earlier.is_empty());

    // An hour later the role is given a further round, which names the
    // earlier round's conversation and is recorded on it without the runtime.
    crew.project
        .env
        .set_env("INSTRUKT_NOW", "2030-01-05T09:00:00Z");
    crew.next("prepare", slug);
    let reserved = crew
        .project
        .env
        .instrukt(crew.root())
        .args([
            "todo",
            "attempt",
            slug,
            "--role",
            "planner",
            "--procedure",
            "crew/procedure/plan",
            "--args",
            slug,
            "--brief",
            "-",
            "--json",
        ])
        .write_stdin("Answer the plan review findings.")
        .assert()
        .success();
    let further: serde_json::Value = serde_json::from_slice(&reserved.get_output().stdout).unwrap();
    assert_eq!(further["prior_session"]["session"], first["session"]);
    assert_eq!(further["prior_session"]["attempt"], first["attempt"]);
    assert_eq!(further["prior_session"]["selection"], first["selection"]);
    let calls = crew.runtime.calls();
    crew.project
        .env
        .instrukt(crew.root())
        .args([
            "todo",
            "bind",
            slug,
            "--role",
            "planner",
            "--attempt",
            &further["attempt"].to_string(),
            "--session",
            &session,
            "--worktree",
            further["worktree"].as_str().unwrap(),
            "--branch",
            further["branch"].as_str().unwrap(),
            "--base",
            further["base"].as_str().unwrap(),
        ])
        .assert()
        .success();
    assert_eq!(crew.runtime.calls(), calls);

    // The further round's evidence lands on it; the earlier round's record,
    // its release time included, stays as it was.
    as_worker(
        &crew,
        &tree,
        &session,
        &[
            "todo",
            "artifacts",
            "write",
            slug,
            artifact_names::IMPLEMENTATION_PLAN,
            "1. Parse quotes first, nested quotes included.",
        ],
    );
    as_worker(
        &crew,
        &tree,
        &session,
        &["todo", "stamp", slug, "--role-complete"],
    );
    let shown = crew.todo(["show", slug]);
    assert_eq!(attempt_record(&shown, "planner/1"), earlier);
    insta::assert_snapshot!("continued-worker-record", redact(&shown));
}
