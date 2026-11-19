//! `project/instrukt/spec/feature/work/todos`: project work recorded
//! as todos with explicit kinds, and withdrawn work recovered under its
//! identity.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::fixtures::work as support;
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
        vec!["prepare", "nested-lists"],
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
    let listed = bugs(&project, project.env.home(), &["list", "--project", "demo"]).success();
    insta::assert_snapshot!(stdout(&listed), @r"
    fix-sync-drops-a-domain | active | not started
    fix-render-loses-a-heading | active | not started
    ");
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
