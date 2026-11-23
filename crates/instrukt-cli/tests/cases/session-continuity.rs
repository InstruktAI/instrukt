//! Runtime-owned threads and native child records, exercised through instrukt's CLI.
#![allow(clippy::unwrap_used, reason = "isolated CLI setup")]
use crate::common;
use crate::fixtures::flavor;
use crate::fixtures::runtime;
use crate::fixtures::work as support;
use crate::fixtures::workflow;
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
    let title = FakeT3::create(&p.env).title("s1");
    insta::assert_snapshot!("readable-orchestrator-title", support::redact(&title));
    let fake = FakeT3::create(&p.env);
    insta::assert_snapshot!(p.env.git(&fake.worktree("s1"),["branch","--show-current"]),@"parser/work");
    assert!(fake.worktree("s1").is_dir());
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let branch: String = db
        .query_row(
            "SELECT branch FROM runtime_bindings WHERE session=?1",
            [format!("t3:{}", fake.native_id("s1"))],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(branch, "parser/work");
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

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS24
#[test]
fn replacing_a_deleted_task_updates_the_work_binding_and_claim_atomically() {
    let mut p = Tracked::new(true);
    p.authorized_task("weekly-status", "2030-01-06T09:00:00Z", &[]);
    p.tick_at("2030-01-06T09:00:20Z");
    let fake = FakeT3::create(&p.env);
    let old = fake.native_id("s1");
    let key = fake.key("s1");
    fake.delete("s1");
    p.tick_at("2030-01-06T09:01:20Z");
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let (binding,claim,runtime,accepted):(String,String,String,bool)=db.query_row(
        "SELECT b.session,t.claim_session,r.session,r.accepted FROM session_bindings b JOIN todos t ON t.id=b.todo_id JOIN runtime_bindings r ON r.semantic_key=b.semantic_key WHERE b.semantic_key=?1",[&key],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_ne!(runtime, format!("t3:{old}"));
    assert_eq!(binding, runtime);
    assert_eq!(claim, runtime);
    assert!(accepted);
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count("s2"), 1);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS1
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS5
#[test]
fn native_discussion_directs_agents_to_the_membership_commands() {
    let p = Tracked::new(false);
    let out = p
        .env
        .run(p.repo.path(), ["config", "get", "runtime.discuss"])
        .success();
    insta::assert_snapshot!(
        "native-discussion-instruction",
        String::from_utf8_lossy(&out.get_output().stdout)
    );
    assert_eq!(p.rt.calls().len(), 0);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS6
#[test]
fn an_unset_runtime_is_the_compiled_one() {
    let p = Tracked::new(false);
    std::fs::write(
        p.env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 1\n",
    )
    .unwrap();
    p.env
        .run(p.repo.path(), ["sessions", "list", "--json"])
        .success();
    assert!(!p.rt.calls().is_empty());
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
fn unattended_requirements_admit_workspace_capabilities_and_refuse_missing_operations() {
    let crew = workflow::Crew::new();
    let path = "docs/domain/crew/workflow/workflow.yaml";
    let supported = crew.flavor.repo.read(path).replace(
        "requires: []",
        "requires: [worktree_create, shared_delegation, worktree_attach]",
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
        "requires: [worktree_create, shared_delegation, worktree_attach]",
        "requires: [notify]",
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
    let _refused = crew
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
    let cause = String::from_utf8_lossy(&_refused.get_output().stderr);
    assert!(
        cause.contains("requires notify") && cause.contains("runtime `t3` lacks"),
        "{cause}"
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
    fake.flag("silent-start", false);
    p.tick();
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
            role: "todo-lead",
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
        .run(p.repo.path(), ["todo", "work", "fresh-base"])
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
        .run(p.repo.path(), ["todo", "work", "no-fetch"])
        .failure();
    assert_eq!(p.rt.started_count(), 0);
    let branches = p.repo.git([
        "for-each-ref",
        "--format=%(refname)",
        "refs/heads/no-fetch/work",
    ]);
    assert_eq!(branches.lines().count(), 0);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn releasing_a_driver_resumes_its_same_native_phase_and_tree_on_pickup() {
    let p = Tracked::new(true);
    change(&p, "resume-parser");
    p.tick();
    let fake = FakeT3::create(&p.env);
    let worktree = fake.worktree("s1");
    p.env
        .instrukt(&worktree)
        .env("T3CODE_THREAD_ID", fake.native_id("s1"))
        .args(["todo", "update", "resume-parser", "--unclaim"])
        .assert()
        .success();
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert!(worktree.is_dir());
    assert_eq!(fake.message_count("s1"), 2);
    insta::assert_snapshot!(
        "retained-driver-resumed",
        support::redact(&p.todo(["show", "resume-parser"]))
    );
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS17
// spec: project/instrukt/spec/feature/docs/pending-documentation UC-PD2
#[test]
fn renamed_target_retrieval_reads_the_retained_preparation_assignment() {
    let p = Tracked::new(true);
    change(&p, "old-parser");
    p.tick();
    let fake = FakeT3::create(&p.env);
    let worktree = fake.worktree("s1");
    let file = worktree.join("docs/project/spec/feature/parser.md");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file,"---\ndescription: The retained parser contract.\ndelivered_by: [old-parser]\n---\n# Parser — Spec\n\n## What it is\n\nThe parser.\n\n## Canonical fields\n\nKeep this contract in its retained tree.\n").unwrap();
    p.env
        .run(p.repo.path(), ["docs", "index", "--target", "old-parser"])
        .success();
    p.env
        .run(
            p.repo.path(),
            [
                "docs",
                "get",
                "project/demo/spec/feature/parser",
                "--target",
                "old-parser",
            ],
        )
        .success();
    p.todo(["update", "old-parser", "--slug", "new-parser"]);
    let file = worktree.join("docs/project/spec/feature/parser.md");
    let mut text = std::fs::read_to_string(&file).unwrap();
    text.push_str("\n<!-- planned:new-parser -->\nA refinement authored under the intermediate name.\n<!-- /planned:new-parser -->\n");
    std::fs::write(file, text).unwrap();
    p.todo(["update", "new-parser", "--slug", "final-parser"]);
    let index = p
        .env
        .run(p.repo.path(), ["docs", "index", "--target", "final-parser"])
        .success();
    let read = p
        .env
        .run(
            p.repo.path(),
            [
                "docs",
                "get",
                "project/demo/spec/feature/parser",
                "--target",
                "final-parser",
            ],
        )
        .success();
    insta::assert_snapshot!(
        "renamed-target-retained-source",
        shown(
            &p,
            &[
                index.get_output().stdout.clone(),
                read.get_output().stdout.clone()
            ]
            .concat()
        )
    );
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
// spec: project/instrukt/spec/feature/work/todos UC-TD7
#[test]
fn native_dispatch_facts_carry_the_resolved_family_model_effort_and_options() {
    let crew = workflow::Crew::new();
    let slug = "native-selection";
    common::agent_profiles(
        &crew.project.env,
        "default: {order: [claude], tier: standard}\nprofiles:\n  claude:\n    standard: [{model: claude-opus-5-5, effort: high, fastMode: false}]\n",
    );
    crew.project
        .create(slug, "change", "Carry resolved native inputs.");
    crew.next("prepare", slug);
    let tree = crew.phase_workspace(slug, true);
    let result = crew
        .project
        .env
        .instrukt(&tree)
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
        ])
        .write_stdin("Use the resolved native model inputs.")
        .assert()
        .success();
    insta::assert_snapshot!(
        "native-selection-facts",
        support::redact(
            &String::from_utf8_lossy(&result.get_output().stdout).replace(
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

// spec: project/instrukt/spec/feature/work/todos UC-TD8
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn proven_native_no_start_retries_with_the_next_available_model() {
    let crew = workflow::Crew::new();
    let slug = "native-fallback";
    common::agent_profiles(
        &crew.project.env,
        "default: {order: [codex, claude], tier: strong}\nprofiles:\n  codex:\n    strong: [{model: gpt-6-astra, effort: high}, {model: gpt-6.1-sol, effort: high}]\n",
    );
    crew.project
        .create(slug, "change", "Retry only after proof of no native start.");
    crew.next("prepare", slug);
    let tree = crew.phase_workspace(slug, true);
    let attempt = || {
        crew.project
            .env
            .instrukt(&tree)
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
            ])
            .write_stdin("Retain this original brief.")
            .assert()
            .success()
    };
    let first = attempt();
    let first: serde_json::Value = serde_json::from_slice(&first.get_output().stdout).unwrap();
    // This is explicit no-start evidence, never a timeout or empty listing.
    crew.project
        .env
        .instrukt(&tree)
        .args([
            "todo",
            "attempt",
            slug,
            "--role",
            "planner",
            "--attempt",
            "1",
            "--not-started",
            "--reason",
            "Native rejection confirmed before child creation.",
        ])
        .assert()
        .success();
    crew.project
        .env
        .run(
            &tree,
            [
                "agents",
                "status",
                "codex",
                "unavailable",
                "--model",
                "gpt-6-astra",
                "--until",
                "2099-01-01T00:00:00Z",
                "--reason",
                "quota",
            ],
        )
        .success();
    let retry = attempt();
    let second: serde_json::Value = serde_json::from_slice(&retry.get_output().stdout).unwrap();
    assert_eq!(first["attempt"].as_u64(), second["attempt"].as_u64());
    insta::assert_snapshot!(
        "native-model-chain-fallback",
        support::redact(
            &String::from_utf8_lossy(&retry.get_output().stdout).replace(
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

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
// spec: project/instrukt/spec/feature/workflow/delivery-cycle UC-DC1
#[test]
fn a_kickback_round_trip_resumes_the_retained_work_driver_without_resetting_its_tree() {
    let p = Tracked::new(true);
    let slug = "round-trip";
    change(&p, slug);
    p.todo([
        "artifacts",
        "write",
        slug,
        "implementation-plan",
        "The initial plan.",
    ]);
    p.todo([
        "artifacts",
        "write",
        slug,
        "plan-review-findings",
        "Approved initial plan.",
    ]);
    p.todo([
        "stamp",
        slug,
        "--verdict",
        "approve",
        "--phase",
        "plan_review",
    ]);
    p.tick();
    let fake = FakeT3::create(&p.env);
    let retained = fake.worktree("s1");
    std::fs::write(
        retained.join("retained.txt"),
        "Committed work from before kickback.",
    )
    .unwrap();
    p.env.git(&retained, ["add", "retained.txt"]);
    p.env.git(
        &retained,
        ["commit", "--quiet", "-m", "retain partial implementation"],
    );
    let head = p.env.git(&retained, ["rev-parse", "HEAD"]);
    p.github
        .set_branch(flavor::REPOSITORY, "round-trip/work", &head);
    p.env
        .instrukt(&retained)
        .env("T3CODE_THREAD_ID", fake.native_id("s1"))
        .args([
            "todo",
            "stamp",
            slug,
            "--kickback",
            "--reason",
            "Revise the plan while preserving implementation.",
        ])
        .assert()
        .success();
    p.env
        .instrukt(&retained)
        .env("T3CODE_THREAD_ID", fake.native_id("s1"))
        .args(["todo", "update", slug, "--unclaim"])
        .assert()
        .success();
    // Single unified todo-orchestrator resumes the retained driver s1 in place without spawning a second prepare thread.
    p.tick();
    assert_eq!(fake.threads().len(), 1, "{}", p.todo(["show", slug]));
    assert_eq!(fake.message_count("s1"), 2);
    assert!(retained.join("retained.txt").is_file());
    assert_eq!(p.env.git(&retained, ["rev-parse", "HEAD"]), head);
    insta::assert_snapshot!(
        "kickback-retained-driver",
        support::redact(&p.todo(["show", slug]))
    );
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS13
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS20
#[test]
fn legacy_key_only_thread_is_reused_after_a_lost_start() {
    let p = Tracked::new(true);
    change(&p, "legacy-parser");
    let fake = FakeT3::create(&p.env);
    fake.flag("silent-start", true);
    p.tick();
    let key = fake.key("s1");
    fake.change("s1", "title", key.clone().into());
    fake.flag("silent-start", false);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.title("s1"), key);
    assert_eq!(fake.message_count("s1"), 1);
    let out = p
        .env
        .run(p.repo.path(), ["sessions", "list", "--all", "--json"])
        .success();
    let sessions: Vec<serde_json::Value> =
        serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(sessions[0]["semantic_key"], key);
    assert_eq!(sessions[0]["title"], key);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS13
#[test]
fn a_titleless_persisted_request_retries_with_its_original_key_only_title() {
    let p = Tracked::new(true);
    change(&p, "old-start");
    let fake = FakeT3::create(&p.env);
    fake.flag("start-failure", true);
    p.tick();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let encoded: String = db
        .query_row("SELECT request FROM runtime_bindings", [], |r| r.get(0))
        .unwrap();
    let current: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    let mut old: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/runtime-start-v14.json")).unwrap();
    for (field, value) in old.as_object_mut().unwrap() {
        *value = current[field].clone();
    }
    let key = old["key"].as_str().unwrap().to_owned();
    db.execute("UPDATE runtime_bindings SET request=?1", [old.to_string()])
        .unwrap();
    db.execute_batch("CREATE TABLE runtime_starts (runtime TEXT NOT NULL,semantic_key TEXT NOT NULL,project TEXT,kind TEXT NOT NULL,subject TEXT NOT NULL,checkout TEXT NOT NULL,brief TEXT NOT NULL,request TEXT NOT NULL,begun_at TEXT NOT NULL,reported INTEGER NOT NULL,resolved_at TEXT,resolved_by TEXT,resolution TEXT,state TEXT NOT NULL,PRIMARY KEY(runtime,semantic_key)); ALTER TABLE runtime_bindings DROP COLUMN generation; ALTER TABLE runtime_bindings DROP COLUMN start_key; ALTER TABLE runtime_bindings DROP COLUMN replaced_session; ALTER TABLE runtime_bindings DROP COLUMN legacy_modes; ALTER TABLE connected_conversations DROP COLUMN display_title; DROP TABLE runtime_generations;").unwrap();
    db.pragma_update(None, "user_version", 14).unwrap();
    p.tick();
    let normalized: String = db
        .query_row("SELECT request FROM runtime_bindings", [], |r| r.get(0))
        .unwrap();
    let normalized: serde_json::Value = serde_json::from_str(&normalized).unwrap();
    assert_eq!(normalized["title"], key);
    fake.flag("start-failure", false);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.title("s1"), key);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count("s1"), 1);
}

fn deliberate_preparation() -> (Tracked, FakeT3, String, std::path::PathBuf, String) {
    let p = Tracked::new(true);
    change(&p, "native-parser");
    let plan = p
        .env
        .run(p.repo.path(), ["todo", "work", "native-parser"])
        .success();
    let value: serde_json::Value = serde_json::from_slice(&plan.get_output().stdout).unwrap();
    let placement = &value["placement"];
    let start = p
        .env
        .command(std::path::Path::new("t3"), p.repo.path())
        .args([
            "thread",
            "start",
            "--title",
            &format!(
                "todo-orchestrator · native-parser · {}",
                placement["key"].as_str().unwrap()
            ),
            "--worktree",
            placement["from"].as_str().unwrap(),
            "--branch",
            placement["branch"].as_str().unwrap(),
            "--",
            p.repo.path().to_str().unwrap(),
            "Native orchestrator opening.",
        ])
        .output()
        .unwrap();
    assert!(start.status.success());
    let id = String::from_utf8(start.stdout).unwrap().trim().to_owned();
    let fake = FakeT3::create(&p.env);
    let tree = fake.worktree(&id);
    let proof = runtime::placement_evidence(
        &p.env,
        &runtime::NativePlacement {
            cwd: &tree,
            slug: "native-parser",
            role: "todo-orchestrator",
            session: &id,
        },
    );
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args(["todo", "work", "native-parser", "--placement-evidence", "-"])
        .write_stdin(proof)
        .assert()
        .success();
    let title = fake.title(&id);
    (p, fake, id, tree, title)
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS17
#[test]
fn renamed_deliberate_driver_resumes_with_its_captured_title() {
    let (p, fake, id, tree, title) = deliberate_preparation();
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args([
            "todo",
            "update",
            "native-parser",
            "--slug",
            "native-parser-renamed",
            "--unclaim",
        ])
        .assert()
        .success();
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 2);
    assert_eq!(fake.title(&id), title);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn deliberately_bound_driver_keeps_its_native_identity_after_a_relabel() {
    let (p, fake, id, tree, title) = deliberate_preparation();
    let key = title.rsplit(" · ").next().unwrap();
    fake.change(&id, "title", format!("Changed label · {key}").into());
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args(["todo", "update", "native-parser", "--unclaim"])
        .assert()
        .success();
    p.tick();
    assert_eq!(
        fake.threads().len(),
        1,
        "{}",
        String::from_utf8_lossy(
            &p.env
                .run(p.env.home(), ["events", "status", "--json"])
                .get_output()
                .stdout
        )
    );
    assert_eq!(fake.message_count(&id), 2);

    // This legacy native admission has no retained opening request. It is
    // not recreated from a guessed brief; the phase worktree stays intact.
    fake.delete(&id);
    p.tick();
    assert!(fake.threads().is_empty());
    assert!(tree.is_dir());
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn an_older_public_placement_proof_is_normalized_before_binding() {
    let (p, fake, id, tree, title) = deliberate_preparation();
    let key = title.rsplit(" · ").next().unwrap();
    fake.change(&id, "title", key.into());
    let mut proof: serde_json::Value = serde_json::from_str(&runtime::placement_evidence(
        &p.env,
        &runtime::NativePlacement {
            cwd: &tree,
            slug: "native-parser",
            role: "todo-orchestrator",
            session: &id,
        },
    ))
    .unwrap();
    proof.as_object_mut().unwrap().remove("title");
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args(["todo", "work", "native-parser", "--placement-evidence", "-"])
        .write_stdin(proof.to_string())
        .assert()
        .success();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 1);
}

fn previous_driver_record(p: &Tracked) -> serde_json::Value {
    p.github
        .comments(flavor::REPOSITORY, 1)
        .iter()
        .rev()
        .find_map(|comment| {
            comment["body"].as_str()?.lines().find_map(|line| {
                line.strip_prefix("<!-- instrukt:work-record:v2:")
                    .and_then(|v| v.strip_suffix(" -->"))
                    .map(|v| {
                        serde_json::from_str::<serde_json::Value>(v).unwrap()["record"].clone()
                    })
            })
        })
        .unwrap()
}
fn previous_driver_binding(record: &serde_json::Value) -> serde_json::Value {
    let mut binding = record["bindings"][0].clone();
    let mut old: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/native-placement-v14.json")).unwrap();
    for (field, value) in old.as_object_mut().unwrap() {
        *value = binding["native_placement"][field].clone();
    }
    binding["native_placement"] = old;
    binding
}
fn retain_previous_driver_input(p: &Tracked, id: &str) {
    let mut record = previous_driver_record(p);
    let old = previous_driver_binding(&record);
    record["bindings"] = serde_json::json!([old]);
    record["schema"] = 2.into();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.execute(
        "UPDATE session_bindings SET evidence=?1,display_title=NULL WHERE session=?2",
        instrukt_core::state_db::rusqlite::params![old.to_string(), format!("t3:{id}")],
    )
    .unwrap();
    db.execute("INSERT OR REPLACE INTO listing_copies(project,issue_number,slug,kind,status,remote_revision,record,copied_at) VALUES('demo',1,?1,'change','active',?2,?3,'2026-11-09T12:00:00Z')",instrukt_core::state_db::rusqlite::params![record["slug"].as_str().unwrap(),record["revision"].as_u64().unwrap(),record.to_string()]).unwrap();
    db.execute_batch("CREATE TABLE runtime_starts (runtime TEXT NOT NULL,semantic_key TEXT NOT NULL,project TEXT,kind TEXT NOT NULL,subject TEXT NOT NULL,checkout TEXT NOT NULL,brief TEXT NOT NULL,request TEXT NOT NULL,begun_at TEXT NOT NULL,reported INTEGER NOT NULL,resolved_at TEXT,resolved_by TEXT,resolution TEXT,state TEXT NOT NULL,PRIMARY KEY(runtime,semantic_key)); ALTER TABLE runtime_bindings DROP COLUMN generation; ALTER TABLE runtime_bindings DROP COLUMN start_key; ALTER TABLE runtime_bindings DROP COLUMN replaced_session; ALTER TABLE runtime_bindings DROP COLUMN legacy_modes; ALTER TABLE connected_conversations DROP COLUMN display_title; DROP TABLE runtime_generations;").unwrap();
    db.pragma_update(None, "user_version", 14).unwrap();
}
fn retained_driver_titles(p: &Tracked, id: &str) -> (serde_json::Value, serde_json::Value) {
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let evidence: String = db
        .query_row(
            "SELECT evidence FROM session_bindings WHERE session=?1",
            [format!("t3:{id}")],
            |row| row.get(0),
        )
        .unwrap();
    let copied: String = db
        .query_row(
            "SELECT record FROM listing_copies WHERE project='demo' AND issue_number=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
    let copied: serde_json::Value = serde_json::from_str(&copied).unwrap();
    (
        evidence["native_placement"]["title"].clone(),
        copied["bindings"][0]["native_placement"]["title"].clone(),
    )
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn previous_readable_driver_resumes_after_local_and_cached_conversion() {
    let (p, fake, id, tree, title) = deliberate_preparation();
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args(["todo", "update", "native-parser", "--unclaim"])
        .assert()
        .success();
    retain_previous_driver_input(&p, &id);
    p.env
        .run(p.repo.path(), ["sessions", "list", "--all", "--json"])
        .success();
    let (local, copied) = retained_driver_titles(&p, &id);
    assert_eq!(local, title);
    assert_eq!(copied, title);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 2);
    assert_eq!(fake.title(&id), title);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS17
#[test]
fn renamed_previous_readable_driver_recovers_its_original_phase_title() {
    let (p, fake, id, tree, title) = deliberate_preparation();
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args([
            "todo",
            "update",
            "native-parser",
            "--slug",
            "renamed-native-parser",
            "--unclaim",
        ])
        .assert()
        .success();
    retain_previous_driver_input(&p, &id);
    p.env
        .run(p.repo.path(), ["sessions", "list", "--all", "--json"])
        .success();
    let (local, copied) = retained_driver_titles(&p, &id);
    assert_eq!(local, title);
    assert_eq!(copied, title);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 2);
    assert_eq!(fake.title(&id), title);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS17
#[test]
fn previous_driver_uses_retained_runtime_title_when_its_request_is_unknown() {
    let (p, fake, id, tree, title) = deliberate_preparation();
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args(["todo", "update", "native-parser", "--unclaim"])
        .assert()
        .success();
    p.tick();
    fake.consume(&id);
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let (unknown, observed): (bool, String) = db
        .query_row(
            "SELECT request IS NULL,display_title FROM runtime_bindings WHERE session=?1",
            [format!("t3:{id}")],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(unknown);
    assert_eq!(observed, title);
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args([
            "todo",
            "update",
            "native-parser",
            "--slug",
            "renamed-native-parser",
            "--unclaim",
        ])
        .assert()
        .success();
    retain_previous_driver_input(&p, &id);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 3);
    assert_eq!(fake.title(&id), title);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn previous_driver_conversion_preserves_native_identity_after_a_relabel() {
    let (p, fake, id, tree, title) = deliberate_preparation();
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args(["todo", "update", "native-parser", "--unclaim"])
        .assert()
        .success();
    retain_previous_driver_input(&p, &id);
    let key = title.rsplit(" · ").next().unwrap();
    let changed = format!("Unexpected label · {key}");
    fake.change(&id, "title", changed.clone().into());
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 2);
    assert_eq!(fake.title(&id), changed);
    fake.change(&id, "title", title.into());
    p.tick();
    assert_eq!(fake.message_count(&id), 2);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS17
#[test]
fn previous_whole_key_driver_still_resumes_after_a_rename() {
    let (p, fake, id, tree, title) = deliberate_preparation();
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args([
            "todo",
            "update",
            "native-parser",
            "--slug",
            "renamed-native-parser",
            "--unclaim",
        ])
        .assert()
        .success();
    retain_previous_driver_input(&p, &id);
    let key = title.rsplit(" · ").next().unwrap().to_owned();
    fake.change(&id, "title", key.clone().into());
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 2);
    assert_eq!(fake.title(&id), key);
}
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS17
#[test]
fn remote_previous_driver_recovers_its_readable_title_after_a_rename() {
    let (p, fake, id, tree, title) = deliberate_preparation();
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args([
            "todo",
            "update",
            "native-parser",
            "--slug",
            "renamed-native-parser",
        ])
        .assert()
        .success();
    let mut record = previous_driver_record(&p);
    let base = record["revision"].as_u64().unwrap();
    record["bindings"] = serde_json::json!([previous_driver_binding(&record)]);
    record["schema"] = 2.into();
    record["revision"] = (base + 1).into();
    let envelope = serde_json::json!({"base":base,"record":record});
    p.github.seed_comment(
        flavor::REPOSITORY,
        1,
        "instrukt-test",
        "COLLABORATOR",
        &format!("<!-- instrukt:work-record:v2:{envelope} -->"),
    );
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args(["todo", "update", "renamed-native-parser", "--unclaim"])
        .assert()
        .success();
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 2);
    assert_eq!(fake.title(&id), title);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS3
#[test]
fn a_previous_bound_runtime_request_is_normalized_before_retained_pickup() {
    let p = Tracked::new(true);
    change(&p, "old-bound-runtime");
    p.tick();
    let fake = FakeT3::create(&p.env);
    let tree = fake.worktree("s1");
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let encoded: String = db
        .query_row(
            "SELECT request FROM runtime_bindings WHERE session=?1",
            [format!("t3:{}", fake.native_id("s1"))],
            |row| row.get(0),
        )
        .unwrap();
    let current: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    let mut old: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/runtime-start-v14.json")).unwrap();
    for (field, value) in old.as_object_mut().unwrap() {
        *value = current[field].clone();
    }
    let key = old["key"].as_str().unwrap().to_owned();
    db.execute(
        "UPDATE runtime_bindings SET request=?1 WHERE session=?2",
        [old.to_string(), format!("t3:{}", fake.native_id("s1"))],
    )
    .unwrap();
    db.execute_batch("CREATE TABLE runtime_starts (runtime TEXT NOT NULL,semantic_key TEXT NOT NULL,project TEXT,kind TEXT NOT NULL,subject TEXT NOT NULL,checkout TEXT NOT NULL,brief TEXT NOT NULL,request TEXT NOT NULL,begun_at TEXT NOT NULL,reported INTEGER NOT NULL,resolved_at TEXT,resolved_by TEXT,resolution TEXT,state TEXT NOT NULL,PRIMARY KEY(runtime,semantic_key)); ALTER TABLE runtime_bindings DROP COLUMN generation; ALTER TABLE runtime_bindings DROP COLUMN start_key; ALTER TABLE runtime_bindings DROP COLUMN replaced_session; ALTER TABLE runtime_bindings DROP COLUMN legacy_modes; ALTER TABLE connected_conversations DROP COLUMN display_title; DROP TABLE runtime_generations;").unwrap();
    db.pragma_update(None, "user_version", 14).unwrap();
    fake.change("s1", "title", key.clone().into());
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", fake.native_id("s1"))
        .args(["todo", "update", "old-bound-runtime", "--unclaim"])
        .assert()
        .success();
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count("s1"), 2);
    assert_eq!(fake.title("s1"), key);
    let normalized: String = db
        .query_row(
            "SELECT request FROM runtime_bindings WHERE session=?1",
            [format!("t3:{}", fake.native_id("s1"))],
            |row| row.get(0),
        )
        .unwrap();
    let normalized: serde_json::Value = serde_json::from_str(&normalized).unwrap();
    assert_eq!(normalized["title"], key);
}
