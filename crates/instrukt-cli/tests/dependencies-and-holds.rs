//! `project/instrukt/spec/feature/work/dependencies-and-holds`:
//! prerequisites recorded for a delivery, and dispatch holds with an
//! inspectable reason.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod support;
#[path = "support/workflow.rs"]
mod workflow;

use std::collections::BTreeMap;
use std::path::Path;

use support::{Project, redact};
use workflow::Crew;

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_dependent_todo_retains_every_prerequisite_of_its_delivery() {
    let project = Project::new();
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.create("renderer-depth", "change", "Render lists at their depth.");
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );

    let recorded = project.ok([
        "update",
        "nested-lists",
        "--after",
        "parser-depth,renderer-depth",
    ]);
    let shown = project.ok(["show", "nested-lists"]);
    let listed = project.ok(["list"]);

    insta::assert_snapshot!(
        "prerequisites",
        redact(&format!("{recorded}---\n{shown}---\n{listed}"))
    );
}

/// Runs a change that must be refused; returns what it said, and the
/// todo's record before and after, which must be equal.
fn refused_unchanged(project: &Project, slug: &str, change: &[&str]) -> String {
    let before = project.ok(["show", slug]);
    let mut args = vec!["update", slug];
    args.extend_from_slice(change);
    let refusal = project.todo(&args).failure();
    let after = project.ok(["show", slug]);
    assert_eq!(
        after, before,
        "the refused change left the record as it was"
    );
    // What the developer is told; diagnostics carry the real time.
    String::from_utf8_lossy(&refusal.get_output().stderr)
        .lines()
        .filter(|line| line.starts_with("instrukt: "))
        .collect::<Vec<_>>()
        .join("\n")
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_prerequisite_that_already_waits_on_the_todo_is_refused_naming_the_cycle() {
    let project = Project::new();
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok(["update", "nested-lists", "--after", "parser-depth"]);

    let refused = refused_unchanged(&project, "parser-depth", &["--after", "nested-lists"]);

    insta::assert_snapshot!("direct-cycle", refused);
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_prerequisite_that_waits_on_the_todo_through_others_is_refused_naming_the_cycle() {
    let project = Project::new();
    project.create("lexer-depth", "change", "Count list depth in the lexer.");
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok(["update", "nested-lists", "--after", "parser-depth"]);
    project.ok(["update", "parser-depth", "--after", "lexer-depth"]);

    let refused = refused_unchanged(&project, "lexer-depth", &["--after", "nested-lists"]);

    insta::assert_snapshot!("indirect-cycle", refused);
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn grouping_a_todo_under_its_own_prerequisite_is_refused_naming_the_cycle() {
    // Workflow judges the parent's preparation through the crew flavor.
    let crew = Crew::new();
    let project = &crew.project;
    project.create(
        "list-depth",
        "feature",
        "Lists keep their depth everywhere.",
    );
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.ok(["update", "nested-lists", "--after", "list-depth"]);

    let refused = refused_unchanged(project, "nested-lists", &["--group", "list-depth"]);

    insta::assert_snapshot!("grouping-cycle", refused);
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_prerequisite_that_waits_through_a_group_on_its_own_child_is_refused_naming_the_cycle() {
    let crew = Crew::new();
    let project = &crew.project;
    project.create(
        "list-depth",
        "feature",
        "Lists keep their depth everywhere.",
    );
    project.create(
        "nested-lists",
        "change",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project.create("renderer", "change", "Render lists at their depth.");
    project.ok(["update", "nested-lists", "--group", "list-depth"]);
    project.ok(["update", "nested-lists", "--after", "renderer"]);

    // The group waits on its child, which waits on the renderer.
    let refused = refused_unchanged(project, "renderer", &["--after", "list-depth"]);

    insta::assert_snapshot!("cycle-through-a-group", refused);
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH1
#[test]
fn a_cycle_through_a_prerequisite_changed_on_another_computer_is_refused() {
    let project = Project::new();
    project.create("parser-depth", "change", "Track list depth in the parser.");
    project.create(
        "nested-lists",
        "feature",
        "Keep the depth of nested lists when parsing Markdown.",
    );
    project
        .other_computer()
        .ok(["update", "parser-depth", "--after", "nested-lists"]);

    let refused = refused_unchanged(&project, "nested-lists", &["--after", "parser-depth"]);

    insta::assert_snapshot!("cycle-through-a-remote-change", refused);
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH2
#[test]
fn a_held_todo_shows_why_it_waits_and_keeps_its_recorded_work() {
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

    let held = project.ok([
        "update",
        "nested-lists",
        "--hold",
        "--reason",
        "waiting for the renderer rewrite to land",
    ]);
    let shown = project.ok(["show", "nested-lists"]);

    insta::assert_snapshot!("held", redact(&format!("{held}---\n{shown}")));
}

/// Planned documentation of `list-depth` in the project's working tree: a
/// new snippet it delivers, a planned block in a live snippet, and that
/// snippet's next version as a companion; beside it, a snippet another
/// todo delivers.
const LIST_DEPTH_DOCS: [(&str, &str); 4] = [
    (
        "docs/project/spec/feature/depth.md",
        "---\ndescription: List depth.\ndelivered_by: [list-depth]\n---\n\n# Depth — Spec\n\nLists keep their depth.\n",
    ),
    (
        "docs/project/spec/feature/lists.md",
        "---\ndescription: Lists.\n---\n\n# Lists — Spec\n\nThe current lists.\n\n<!-- planned:list-depth -->\nDepth everywhere.\n<!-- /planned:list-depth -->\n",
    ),
    (
        "docs/project/spec/feature/lists.list-depth.md",
        "---\ndescription: Lists.\n---\n\n# Lists — Spec\n\nThe lists as list-depth delivers them.\n",
    ),
    (
        "docs/project/spec/feature/renderer.md",
        "---\ndescription: Rendering.\ndelivered_by: [renderer]\n---\n\n# Renderer — Spec\n\nRendered at depth.\n",
    ),
];

/// Writes `list-depth`'s planned documentation into `project`'s working
/// tree, commits it, and files `list-depth`.
fn plan_list_depth(project: &Project) {
    for (path, text) in LIST_DEPTH_DOCS {
        project.repo.write(path, text);
    }
    project.repo.commit("plan list-depth's documentation");
    project.create(
        "list-depth",
        "feature",
        "Lists keep their depth everywhere.",
    );
}

/// A crew project whose todo `list-depth` has planned documentation not
/// yet delivered.
fn todo_with_planned_documentation() -> Crew {
    let crew = Crew::new();
    plan_list_depth(&crew.project);
    crew
}

/// Every file below `dir`, by path relative to it, with its text.
fn files_below(dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path.strip_prefix(root).unwrap().display().to_string();
                out.insert(relative, std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Documentation files as one text.
fn rendered(files: &BTreeMap<String, String>) -> String {
    files
        .iter()
        .map(|(path, text)| format!("== {path}\n{text}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a failed command told the developer, its temporary paths named
/// `[TMP]` and an atomic write's random scratch name `.tmp<name>`.
fn told(project: &Project, assert: &assert_cmd::assert::Assert) -> String {
    let base = project.env.home().parent().unwrap().display().to_string();
    let text = String::from_utf8_lossy(&assert.get_output().stderr)
        .lines()
        .filter(|line| line.starts_with("instrukt: "))
        .collect::<Vec<_>>()
        .join("\n")
        .replace(&base, "[TMP]");
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("/.tmp") {
        out.push_str(&rest[..at]);
        out.push_str("/.tmp<name>");
        rest = rest[at + "/.tmp".len()..].trim_start_matches(|c: char| c.is_ascii_alphanumeric());
    }
    out.push_str(rest);
    out
}

/// `instrukt todo create nested-lists … --group list-depth`.
fn file_nested_lists(project: &Project) -> assert_cmd::assert::Assert {
    project.todo([
        "create",
        "nested-lists",
        "Keep the depth of nested lists when parsing Markdown.",
        "--kind",
        "change",
        "--approved",
        "--group",
        "list-depth",
    ])
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH3
#[test]
fn filing_the_first_work_under_a_todo_moves_its_planned_documentation_to_that_work() {
    let crew = todo_with_planned_documentation();
    let project = &crew.project;

    let filed = support::stdout(&file_nested_lists(project).success());
    let docs = files_below(&project.repo.path().join("docs"));

    insta::assert_snapshot!(
        "documentation-moved-to-the-first-child",
        redact(&format!("{filed}---\n{}", rendered(&docs)))
    );
}

/// Files `nested-lists` under `list-depth`, which must be refused; returns
/// what the developer was told, after checking that the documentation
/// below `docs` and `list-depth`'s record are as they were and that no
/// `nested-lists` was filed.
fn refused_filing(project: &Project, docs: &Path) -> String {
    let documentation = files_below(docs);
    let record = project.ok(["show", "list-depth"]);

    let refusal = file_nested_lists(project).failure();

    assert_eq!(
        files_below(docs),
        documentation,
        "the documentation still belongs to list-depth"
    );
    assert_eq!(
        project.ok(["show", "list-depth"]),
        record,
        "list-depth's record is as it was"
    );
    project.todo(["show", "nested-lists"]).failure();
    told(project, &refusal)
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH3
#[test]
fn a_filing_refused_for_a_cycle_leaves_the_planned_documentation_with_the_original_todo() {
    let crew = todo_with_planned_documentation();
    let project = &crew.project;
    // The new work waits on the renderer, which waits on list-depth: as
    // its group, list-depth would wait on itself.
    project.create("renderer", "change", "Render lists at their depth.");
    project.ok(["update", "renderer", "--after", "list-depth"]);
    let documentation = files_below(&project.repo.path().join("docs"));
    let record = project.ok(["show", "list-depth"]);

    let refusal = project
        .todo([
            "create",
            "nested-lists",
            "Keep the depth of nested lists when parsing Markdown.",
            "--kind",
            "change",
            "--approved",
            "--after",
            "renderer",
            "--group",
            "list-depth",
        ])
        .failure();

    assert_eq!(
        files_below(&project.repo.path().join("docs")),
        documentation,
        "the documentation still belongs to list-depth"
    );
    assert_eq!(project.ok(["show", "list-depth"]), record);
    project.todo(["show", "nested-lists"]).failure();
    insta::assert_snapshot!("refused-filing-cycle", told(project, &refusal));
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH3
#[test]
fn a_filing_under_a_todo_whose_preparation_workflow_cannot_judge_is_refused_naming_why() {
    // No flavor is selected: workflow cannot say whether list-depth is
    // still before prepared.
    let project = Project::new();
    plan_list_depth(&project);

    let refusal = refused_filing(&project, &project.repo.path().join("docs"));

    insta::assert_snapshot!("refused-filing-undetermined-preparation", refusal);
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH3
#[test]
fn a_documentation_move_that_fails_part_way_leaves_everything_with_the_original_todo() {
    let crew = todo_with_planned_documentation();
    let project = &crew.project;
    // A later planned snippet sits in a directory nothing can be written
    // to: the move fails after the earlier files have moved.
    project.repo.write(
        "docs/project/spec/locked/late.md",
        "---\ndescription: Late.\ndelivered_by: [list-depth]\n---\n\n# Late — Spec\n\nLast.\n",
    );
    project.repo.commit("plan a late snippet");
    let locked = project.repo.path().join("docs/project/spec/locked");
    set_mode(&locked, 0o555);

    let refusal = refused_filing(project, &project.repo.path().join("docs"));

    set_mode(&locked, 0o755);
    insta::assert_snapshot!("failed-filing-part-way", refusal);
}

// spec: project/instrukt/spec/feature/work/dependencies-and-holds UC-DH3
#[test]
fn documentation_reached_through_a_link_is_not_moved() {
    let crew = todo_with_planned_documentation();
    let project = &crew.project;
    // `docs/` is a link to documentation outside the repository.
    let outside = project.env.home().parent().unwrap().join("elsewhere");
    std::fs::rename(project.repo.path().join("docs"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, project.repo.path().join("docs")).unwrap();

    let refusal = refused_filing(project, &outside);

    insta::assert_snapshot!("refused-filing-linked-documentation", refusal);
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}
