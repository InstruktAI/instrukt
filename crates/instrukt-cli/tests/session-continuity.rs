//! Runtime-owned threads and native child records, exercised through instrukt's CLI.
#![allow(clippy::unwrap_used, reason = "isolated CLI setup")]
mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/flavor.rs"]
mod flavor;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod support;
#[path = "support/workflow.rs"]
mod workflow;
use flavor::Tracked;
use runtime::FakeT3;
fn change(project: &Tracked, slug: &str) {
    project.todo([
        "create",
        slug,
        "Improve the parser.",
        "--kind",
        "change",
        "--approved",
    ]);
}
fn shown(project: &Tracked, bytes: &[u8]) -> String {
    support::redact(&String::from_utf8_lossy(bytes).replace(
        project.env.path("").to_str().unwrap().trim_end_matches('/'),
        "[TMP]",
    ))
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn runner_places_only_the_orchestrator_and_workers_use_native_bindings() {
    let p = Tracked::new(true);
    change(&p, "parser");
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    let fake = FakeT3::create(&p.env);
    insta::assert_snapshot!(p.env.git(&fake.worktree("s1"),["branch","--show-current"]),@"parser/prepare");
    assert!(fake.worktree("s1").is_dir());
    for command in ["run", "send", "alive", "conversation", "link"] {
        p.env.run(p.repo.path(), ["sessions", command]).failure();
    }
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS13
#[test]
fn lost_start_result_recovers_the_same_orchestrator_and_its_phase_worktree() {
    let p = Tracked::new(true);
    change(&p, "parser");
    p.rt.kill_runner_on_next_start();
    p.env.run(p.env.home(), ["tick"]).failure();
    p.tick();
    p.tick();
    assert_eq!(p.rt.started_count(), 1);
    let fake = FakeT3::create(&p.env);
    assert!(fake.worktree("s1").is_dir());
    assert_eq!(fake.message_count("s1"), 1);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS13
#[test]
fn empty_discovery_and_outages_never_resolve_an_uncertain_start() {
    let p = Tracked::new(true);
    change(&p, "parser");
    let fake = FakeT3::create(&p.env);
    fake.flag("empty-start", true);
    p.tick();
    let record = p.todo(["show", "parser"]);
    let key = record
        .lines()
        .find_map(|line| line.trim().strip_prefix("key: "))
        .unwrap()
        .to_owned();
    fake.flag("empty-start", false);
    p.tick();
    assert_eq!(fake.threads().len(), 0);
    p.env
        .run(p.repo.path(), ["sessions", "resolve-start", &key, "--none"])
        .success();
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    fake.flag("list-failure", true);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS12
#[test]
fn conflicting_keyed_threads_never_admit_a_replacement() {
    let p = Tracked::new(true);
    change(&p, "parser");
    let fake = FakeT3::create(&p.env);
    fake.flag("silent-start", true);
    p.tick();
    fake.flag("duplicate", true);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    let key = fake.title("s1");
    let out = p
        .env
        .run(
            p.repo.path(),
            ["sessions", "resolve-start", &key, "--session", "t3:s1"],
        )
        .failure();
    insta::assert_snapshot!("duplicate-key", shown(&p, &out.get_output().stderr));
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS1
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS5
#[test]
fn native_discussion_reports_the_link_gap_without_rebuilding_it() {
    let p = Tracked::new(false);
    let out = p
        .env
        .run(p.repo.path(), ["config", "get", "runtime.discuss"])
        .success();
    insta::assert_snapshot!(
        "native-discussion-gap",
        String::from_utf8_lossy(&out.get_output().stdout)
    );
    assert_eq!(p.rt.calls().len(), 0);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS6
#[test]
fn installed_t3_does_not_select_itself() {
    let p = Tracked::new(false);
    std::fs::write(
        p.env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 1\n",
    )
    .unwrap();
    let out = p
        .env
        .run(p.repo.path(), ["sessions", "list", "--json"])
        .failure();
    insta::assert_snapshot!("no-selected-runtime", shown(&p, &out.get_output().stderr));
    assert_eq!(p.rt.calls().len(), 0);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS19
#[test]
fn operational_listing_filters_only_known_settled_activity_and_keeps_unknown_times() {
    let p = Tracked::new(false);
    let fake = FakeT3::create(&p.env);
    for id in ["active", "recent", "old", "unknown"] {
        fake.seed(
            id,
            p.repo.path(),
            &format!("instrukt:run:{id}"),
            "Native assignment",
        );
    }
    for id in ["recent", "old", "unknown"] {
        fake.set_status(id, "completed");
        fake.change(id, "settled", true.into());
    }
    fake.change("old", "updatedAt", "2020-01-01T00:00:00Z".into());
    fake.change("unknown", "updatedAt", serde_json::Value::Null);
    let list = p
        .env
        .instrukt(p.repo.path())
        .env("INSTRUKT_NOW", "2026-11-07T10:00:00Z")
        .args(["sessions", "list", "--json"])
        .assert()
        .success();
    let items: Vec<serde_json::Value> = serde_json::from_slice(&list.get_output().stdout).unwrap();
    assert_eq!(items.len(), 3);
    insta::assert_snapshot!(
        "recent-active-and-settled-sessions",
        shown(&p, &list.get_output().stdout)
    );
    let all = p
        .env
        .run(p.repo.path(), ["sessions", "list", "--all", "--json"])
        .success();
    assert_eq!(
        serde_json::from_slice::<Vec<serde_json::Value>>(&all.get_output().stdout)
            .unwrap()
            .len(),
        4
    );
    p.env
        .run(
            p.repo.path(),
            ["sessions", "list", "--all", "--since", "3d"],
        )
        .failure();
    p.env
        .run(p.repo.path(), ["sessions", "list", "--since", "bad"])
        .failure();
}
fn pending_builder() -> (workflow::Crew, serde_json::Value) {
    let crew = workflow::Crew::new();
    crew.project.create("parser", "change", "Improve parser.");
    crew.write("parser", "implementation-plan", "Improve depth.");
    crew.write("parser", "plan-review-findings", "Approved.");
    crew.stamp(
        "parser",
        &["--verdict", "approve", "--phase", "plan_review"],
    );
    crew.next("work", "parser");
    let child = crew.worker("parser", "setup", None);
    (crew, child)
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS14
#[test]
fn an_idle_parents_native_child_result_has_no_instrukt_ending_delivery() {
    let (crew, child) = pending_builder();
    crew.runtime.open_session("lead-1");
    crew.runtime.finish(
        child["session"]
            .as_str()
            .unwrap()
            .strip_prefix("t3:")
            .unwrap(),
        "completed",
        "Native completion result.",
    );
    crew.project.env.run(crew.root(), ["tick"]).success();
    assert_eq!(FakeT3::create(&crew.project.env).message_count("lead-1"), 0);
    let result = crew.todo(["show", "parser"]);
    insta::assert_snapshot!(
        "native-ending-record",
        support::redact(
            &result.replace(
                crew.project
                    .env
                    .path("")
                    .to_str()
                    .unwrap()
                    .trim_end_matches('/'),
                "[TMP]"
            )
        )
    );
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS15
#[test]
fn a_busy_parents_native_result_uses_no_instrukt_waiting_slot() {
    let (crew, child) = pending_builder();
    crew.runtime.open_session("lead-1");
    crew.runtime.set_status("lead-1", "running");
    crew.runtime.finish(
        child["session"]
            .as_str()
            .unwrap()
            .strip_prefix("t3:")
            .unwrap(),
        "completed",
        "Native result while parent runs.",
    );
    crew.project.env.run(crew.root(), ["tick"]).success();
    assert_eq!(FakeT3::create(&crew.project.env).message_count("lead-1"), 0);
    let active = crew
        .project
        .env
        .instrukt(crew.root())
        .args([
            "todo", "attempt", "parser", "--role", "setup", "--brief", "-",
        ])
        .write_stdin("Changed inputs are refused.")
        .assert();
    active.failure();
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS16
#[test]
fn native_failure_or_input_wait_never_becomes_instrukt_completion() {
    for state in ["failed", "waiting"] {
        let (crew, child) = pending_builder();
        crew.runtime.open_session("lead-1");
        crew.runtime.set_status(
            child["session"]
                .as_str()
                .unwrap()
                .strip_prefix("t3:")
                .unwrap(),
            state,
        );
        crew.project.env.run(crew.root(), ["tick"]).success();
        assert_eq!(FakeT3::create(&crew.project.env).message_count("lead-1"), 0);
        crew.project
            .env
            .instrukt(crew.root())
            .args([
                "todo", "attempt", "parser", "--role", "setup", "--brief", "-",
            ])
            .write_stdin("Changed inputs stay blocked.")
            .assert()
            .failure();
    }
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS17
#[test]
fn renaming_a_todo_keeps_its_reserved_semantic_identity() {
    let p = Tracked::new(true);
    change(&p, "parser");
    p.tick();
    let fake = FakeT3::create(&p.env);
    let key = fake.title("s1");
    p.todo(["update", "parser", "--slug", "parser-depth"]);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.title("s1"), key);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS18
#[test]
fn approval_wakes_a_settled_orchestrator_in_its_retained_conversation() {
    let p = Tracked::new(true);
    change(&p, "parser");
    p.tick();
    let fake = FakeT3::create(&p.env);
    fake.set_status("s1", "completed");
    fake.change("s1", "settled", true.into());
    p.todo(["artifacts", "write", "parser", "prd", "Keep parser depth."]);
    p.todo([
        "artifacts",
        "write",
        "parser",
        "prd-review-findings",
        "Approve current requirements.",
    ]);
    p.todo([
        "stamp",
        "parser",
        "--verdict",
        "approve",
        "--phase",
        "prd_review",
    ]);
    p.todo(["stamp", "parser", "--prd-approved", "--by", "human"]);
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count("s1"), 2);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS5
// spec: project/instrukt/spec/feature/workflow/workflow-definition UC-WD2
#[test]
fn unattended_requirements_admit_creation_and_shared_children_but_refuse_attachment() {
    let crew = workflow::Crew::new();
    let path = "docs/domain/crew/workflow/work.yaml";
    let supported = crew.flavor.repo.read(path).replace(
        "requires: []",
        "requires: [worktree_create, shared_delegation]",
    );
    crew.flavor.repo.write(path, &supported);
    crew.flavor.republish(
        &crew.project.env,
        "declare supported workspace capabilities",
    );
    let revision = crew
        .project
        .env
        .run(crew.root(), ["config", "get", "--revision"])
        .success();
    let revision = String::from_utf8_lossy(&revision.get_output().stdout);
    crew.project
        .env
        .run(
            crew.root(),
            [
                "config",
                "patch",
                "--revision",
                revision.trim(),
                "--yaml",
                "modules: {todos: true}",
            ],
        )
        .success();
    let content = crew.flavor.repo.read(path).replace(
        "requires: [worktree_create, shared_delegation]",
        "requires: [worktree_attach]",
    );
    crew.flavor.repo.write(path, &content);
    crew.flavor
        .republish(&crew.project.env, "declare required native linking");
    let before = std::fs::read(crew.root().join("instrukt.yaml")).unwrap();
    let revision = crew
        .project
        .env
        .run(crew.root(), ["config", "get", "--revision"])
        .success();
    let revision = String::from_utf8_lossy(&revision.get_output().stdout);
    let refusal = crew
        .project
        .env
        .run(
            crew.root(),
            [
                "config",
                "patch",
                "--revision",
                revision.trim(),
                "--yaml",
                "modules: {todos: true}",
            ],
        )
        .failure();
    insta::assert_snapshot!(
        "required-attachment-refused",
        String::from_utf8_lossy(&refusal.get_output().stderr).replace(
            crew.project
                .env
                .path("")
                .to_str()
                .unwrap()
                .trim_end_matches('/'),
            "[TMP]"
        )
    );
    assert_eq!(
        std::fs::read(crew.root().join("instrukt.yaml")).unwrap(),
        before
    );
    assert_eq!(crew.runtime.calls().len(), 0);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS13
#[test]
fn disappearing_optional_native_project_metadata_still_recovers_the_same_reserved_thread() {
    let p = Tracked::new(true);
    change(&p, "parser");
    let fake = FakeT3::create(&p.env);
    fake.flag("silent-start", true);
    p.tick();
    let key = fake.title("s1");
    p.env
        .run(
            p.repo.path(),
            ["sessions", "resolve-start", &key, "--session", "t3:s1"],
        )
        .success();
    fake.flag("hide-project-id", true);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count("s1"), 1);
    insta::assert_snapshot!(
        "recovered-without-native-project-id",
        support::redact(&p.todo(["show", "parser"]))
    );
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn deliberate_entry_requires_native_recorded_placement_and_never_adopts_incidental_cwd() {
    let crew = workflow::Crew::new();
    let slug = "deliberate-placement";
    crew.project
        .create(slug, "change", "Bind the recorded native workspace.");
    crew.write(
        slug,
        "implementation-plan",
        "Use the same verified phase workspace.",
    );
    crew.write(slug, "plan-review-findings", "Approved.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);
    let runtime_calls = crew.runtime.calls().len();
    let path = crew.workspace(slug);
    let before = crew.todo(["show", slug]);
    let denied = crew
        .project
        .env
        .instrukt(&path)
        .args(["todo", "work", slug])
        .assert()
        .failure();
    insta::assert_snapshot!(
        "deliberate-native-placement-required",
        String::from_utf8_lossy(&denied.get_output().stderr)
    );
    assert_eq!(before, crew.todo(["show", slug]));
    let proof = runtime::placement_evidence(
        &crew.project.env,
        &runtime::NativePlacement {
            cwd: &path,
            slug,
            role: "work-lead",
            session: "lead-1",
        },
    );
    let mut wrong: serde_json::Value = serde_json::from_str(&proof).unwrap();
    wrong["worktree"] = crew.root().to_string_lossy().into_owned().into();
    crew.project
        .env
        .instrukt(&path)
        .args(["todo", "work", slug, "--placement-evidence", "-"])
        .write_stdin(wrong.to_string())
        .assert()
        .failure();
    assert_eq!(before, crew.todo(["show", slug]));
    crew.project
        .env
        .instrukt(&path)
        .args(["todo", "work", slug, "--placement-evidence", "-"])
        .write_stdin(proof)
        .assert()
        .success();
    assert_eq!(crew.runtime.calls().len(), runtime_calls);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS17
#[test]
fn renaming_during_a_lost_start_preserves_the_original_key_branch_and_opening_input() {
    let p = Tracked::new(true);
    change(&p, "parser");
    let fake = FakeT3::create(&p.env);
    fake.flag("silent-start", true);
    p.tick();
    let key = fake.title("s1");
    let opening = fake.message("s1", 0);
    p.todo(["update", "parser", "--slug", "parser-depth"]);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.title("s1"), key);
    assert_eq!(fake.message("s1", 0), opening);
    assert_eq!(fake.message_count("s1"), 1);
    insta::assert_snapshot!(
        "renamed-start-recovered",
        support::redact(&p.todo(["show", "parser-depth"]))
    );
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn deliberate_phase_placement_fetches_remote_main_while_local_main_lags() {
    let p = Tracked::new(false);
    change(&p, "fresh-base");
    let old = p.repo.head();
    let other = p
        .env
        .clone_repo(&p.env.path("demo-origin.git"), "integrator");
    other.write("integrated.txt", "reviewed green delivery");
    let delivered = other.commit("integrator: promote verified candidate");
    other.git(["push", "--quiet", "origin", "main"]);
    assert_eq!(p.repo.git(["rev-parse", "origin/main"]), old);
    let out = p
        .env
        .run(p.repo.path(), ["todo", "prepare", "fresh-base"])
        .success();
    let value: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(value["placement"]["from"], "origin/main");
    assert_eq!(value["placement"]["base"], delivered);
    assert_eq!(p.repo.head(), old);
    assert_eq!(p.rt.started_count(), 0);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn a_failed_phase_fetch_never_uses_the_stale_local_base() {
    let p = Tracked::new(false);
    change(&p, "no-fetch");
    p.repo.git([
        "remote",
        "set-url",
        "origin",
        p.env.path("missing-origin.git").to_str().unwrap(),
    ]);
    p.env
        .run(p.repo.path(), ["todo", "prepare", "no-fetch"])
        .failure();
    assert_eq!(p.rt.started_count(), 0);
    let branches = p.repo.git([
        "for-each-ref",
        "--format=%(refname)",
        "refs/heads/no-fetch/prepare",
    ]);
    assert_eq!(branches.lines().count(), 0);
}
