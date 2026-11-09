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
    let title = FakeT3::create(&p.env).title("s1");
    insta::assert_snapshot!("readable-orchestrator-title", support::redact(&title));
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
    let key = fake.title("s1").rsplit(" · ").next().unwrap().to_owned();
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
    let key = fake.title("s1").rsplit(" · ").next().unwrap().to_owned();
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
        .env("T3CODE_THREAD_ID", "s1")
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
    let prepare = "docs/domain/acme/workflow/prepare.yaml";
    p.repo.write(
        prepare,
        &p.repo.read(prepare).replace(
            "when: {not: {exists: implementation_plan.digest}}",
            "when: {not: {equals: [plan_review.verdict, approve]}}",
        ),
    );
    let work = "docs/domain/acme/workflow/work.yaml";
    p.repo.write(
        work,
        &p.repo.read(work).replace(
            "when: {exists: implementation_plan.digest}",
            "when: {equals: [plan_review.verdict, approve]}",
        ),
    );
    p.repo.commit("publish kickback-aware fixture lanes");
    p.repo.git(["push", "--quiet", "origin", "main"]);
    p.env.run(p.repo.path(), ["sync"]).success();
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
        .env("T3CODE_THREAD_ID", "s1")
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
        .env("T3CODE_THREAD_ID", "s1")
        .args(["todo", "update", slug, "--unclaim"])
        .assert()
        .success();
    p.tick();
    assert_eq!(fake.threads().len(), 2, "{}", p.todo(["show", slug]));
    let preparation = fake.worktree("s2");
    p.env
        .instrukt(&preparation)
        .env("T3CODE_THREAD_ID", "s2")
        .args([
            "todo",
            "artifacts",
            "write",
            slug,
            "implementation-plan",
            "The corrected plan.",
        ])
        .assert()
        .success();
    p.env
        .instrukt(&preparation)
        .env("T3CODE_THREAD_ID", "s2")
        .args([
            "todo",
            "artifacts",
            "write",
            slug,
            "plan-review-findings",
            "Approved corrected plan.",
        ])
        .assert()
        .success();
    p.env
        .instrukt(&preparation)
        .env("T3CODE_THREAD_ID", "s2")
        .args([
            "todo",
            "stamp",
            slug,
            "--verdict",
            "approve",
            "--phase",
            "plan_review",
        ])
        .assert()
        .success();
    p.env
        .instrukt(&preparation)
        .env("T3CODE_THREAD_ID", "s2")
        .args(["todo", "update", slug, "--unclaim"])
        .assert()
        .success();
    // This preparation changed only work-record artifacts; its Git head is already on main.
    p.repo
        .git(["worktree", "remove", preparation.to_str().unwrap()]);
    p.repo.git(["branch", "-d", "round-trip/prepare"]);
    p.tick();
    assert_eq!(fake.threads().len(), 2);
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
    let labelled = fake.title("s1");
    let key = labelled.rsplit(" · ").next().unwrap().to_owned();
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

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS12
#[test]
fn mixed_legacy_and_readable_titles_for_one_key_conflict() {
    let p = Tracked::new(true);
    change(&p, "duplicate-parser");
    let fake = FakeT3::create(&p.env);
    fake.flag("silent-start", true);
    p.tick();
    let title = fake.title("s1");
    let key = title.rsplit(" · ").next().unwrap();
    fake.seed("legacy", p.repo.path(), key, &fake.message("s1", 0));
    let out = p
        .env
        .run(
            p.repo.path(),
            ["sessions", "resolve-start", key, "--session", "t3:s1"],
        )
        .failure();
    insta::assert_snapshot!("mixed-title-conflict", shown(&p, &out.get_output().stderr));
    assert_eq!(fake.threads().len(), 2);
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS13
#[test]
fn a_titleless_persisted_request_retries_with_its_original_key_only_title() {
    let p = Tracked::new(true);
    change(&p, "old-start");
    let fake = FakeT3::create(&p.env);
    fake.flag("empty-start", true);
    p.tick();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let encoded: String = db
        .query_row("SELECT request FROM runtime_starts", [], |r| r.get(0))
        .unwrap();
    let current: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    let mut old: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/runtime-start-v14.json")).unwrap();
    for (field, value) in old.as_object_mut().unwrap() {
        *value = current[field].clone();
    }
    let key = old["key"].as_str().unwrap().to_owned();
    db.execute("UPDATE runtime_starts SET request=?1", [old.to_string()])
        .unwrap();
    db.pragma_update(None, "user_version", 14).unwrap();
    p.env
        .run(p.repo.path(), ["sessions", "resolve-start", &key, "--none"])
        .success();
    fake.flag("empty-start", false);
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
        .run(p.repo.path(), ["todo", "prepare", "native-parser"])
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
            placement["title"].as_str().unwrap(),
            "--worktree",
            placement["from"].as_str().unwrap(),
            "--branch",
            placement["branch"].as_str().unwrap(),
            "--",
            p.repo.path().to_str().unwrap(),
            "Native preparation opening.",
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
            role: "prepare-orchestrator",
            session: &id,
        },
    );
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args([
            "todo",
            "prepare",
            "native-parser",
            "--placement-evidence",
            "-",
        ])
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
fn deliberately_bound_driver_refuses_an_actual_native_relabel() {
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
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 1);
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
            role: "prepare-orchestrator",
            session: &id,
        },
    ))
    .unwrap();
    proof.as_object_mut().unwrap().remove("title");
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", &id)
        .args([
            "todo",
            "prepare",
            "native-parser",
            "--placement-evidence",
            "-",
        ])
        .write_stdin(proof.to_string())
        .assert()
        .success();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count(&id), 1);
}
