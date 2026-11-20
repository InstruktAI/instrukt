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
    insta::assert_snapshot!(p.env.git(&fake.worktree("s1"),["branch","--show-current"]),@"parser/prepare");
    assert!(fake.worktree("s1").is_dir());
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let branch: String = db
        .query_row(
            "SELECT branch FROM runtime_bindings WHERE session='t3:s1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(branch, "parser/prepare");
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
    let mut p = Tracked::new(true);
    change(&p, "parser");
    let fake = FakeT3::create(&p.env);
    fake.flag("empty-start", true);
    p.tick_at("2030-01-01T00:00:00Z");
    let record = p.todo(["show", "parser"]);
    let key = record
        .lines()
        .find_map(|line| line.trim().strip_prefix("key: "))
        .unwrap()
        .to_owned();
    fake.flag("empty-start", false);
    p.tick_at("2030-01-01T00:01:00Z");
    assert_eq!(fake.threads().len(), 0);
    let status = || {
        let out = p
            .env
            .run(p.env.home(), ["events", "status", "--json"])
            .success();
        serde_json::from_slice::<serde_json::Value>(&out.get_output().stdout).unwrap()
            ["start_anomalies"].clone()
    };
    let finding = status()[0].clone();
    assert_eq!(finding["key"], key);
    assert!(
        finding["failure"]
            .as_str()
            .unwrap()
            .contains("instrukt sessions resolve-start")
    );
    assert!(p.rt.notifications().is_empty());
    p.tick_at("2030-01-01T00:06:00Z");
    assert_eq!(p.rt.notifications(), std::slice::from_ref(&key));
    p.env
        .run(p.repo.path(), ["sessions", "resolve-start", &key, "--none"])
        .success();
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    fake.flag("list-failure", true);
    p.tick();
    assert_eq!(fake.threads().len(), 1);
}
const RECOVERY_FOLLOWUP: &str = "Later work for the committed successor.";

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS25
#[test]
fn keyed_recovery_doors_follow_binding_state() {
    if let Ok(key) = std::env::var("INSTRUKT_TEST_KEYED_RECOVERY") {
        let home = instrukt_core::home::Home::from_env().unwrap();
        let invocation = instrukt_runner::keyed::RuntimeInvocation::acquire(
            &home,
            std::time::Duration::from_secs(30),
        )
        .unwrap();
        let adapter = instrukt_runner::runtime::T3::default();
        let now = "2030-01-06T09:00:21Z".parse().unwrap();
        if let Ok(target) = std::env::var("INSTRUKT_TEST_RECOVERY_TARGET") {
            let session = instrukt_core::ids::SessionRef::parse(&target).unwrap();
            let mut request = instrukt_runner::keyed::request(&home, &adapter, &key).unwrap();
            request.brief = RECOVERY_FOLLOWUP.into();
            let thread = instrukt_runner::keyed::followup(
                &adapter,
                &instrukt_runner::runtime::FollowupAssignment {
                    invocation: &invocation,
                    request: &request,
                    session: &session,
                    worktree_path: None,
                    project: Some("demo"),
                    now,
                },
            )
            .unwrap();
            assert_eq!(thread.session, session);
        } else {
            instrukt_runner::keyed::retry(&invocation, &adapter, &key, now).unwrap();
        }
        return;
    }
    std::thread::scope(|scope| {
        for phase in ["prepare", "work", "task"] {
            scope.spawn(move || check_todo_recovery_doors(phase));
        }
        scope.spawn(check_run_recovery_doors);
        #[cfg(debug_assertions)]
        scope.spawn(concurrent_resolutions_bind_the_native_thread_to_only_one_key);
        crate::procedure_event_delivery::check_standing_recovery_doors();
    });
}

fn check_todo_recovery_doors(phase: &str) {
    let mut p = Tracked::new(true);
    let fake = FakeT3::create(&p.env);
    let worktree = phase != "task";
    if worktree {
        change(&p, "recovery");
        if phase == "work" {
            p.todo([
                "artifacts",
                "write",
                "recovery",
                "implementation-plan",
                "Plan.",
            ]);
            p.todo([
                "artifacts",
                "write",
                "recovery",
                "plan-review-findings",
                "Approved.",
            ]);
            p.todo([
                "stamp",
                "recovery",
                "--verdict",
                "approve",
                "--phase",
                "plan_review",
            ]);
        }
    } else {
        p.authorized_task("recovery", "2030-01-06T09:00:00Z", &[]);
        p.env.set_env("INSTRUKT_NOW", "2030-01-06T09:00:20Z");
    }
    let db = recovery_db(&p.env);
    fail_recovery_receipt(&db, "s1");
    p.tick();
    check_recovery_doors(
        &p.env,
        p.repo.path(),
        &fake,
        &db,
        worktree.then_some(phase),
        || {
            p.tick();
        },
        |key, native| {
            let (bound, claimed): (String, String) = db.query_row(
                    "SELECT b.session, t.claim_session FROM session_bindings b
                     JOIN todos t ON t.id=b.todo_id WHERE b.semantic_key=?1 AND b.released_at IS NULL",
                    [key], |row| Ok((row.get(0)?,row.get(1)?)),
                ).unwrap();
            assert_eq!(bound, format!("t3:{native}"), "{phase}");
            assert_eq!(claimed, bound, "{phase}");
            fake.set_status(native, "idle");
            run_recovery_worker(&p.env, p.repo.path(), key, Some(&claimed));
            assert!(
                fake.input_log(native).contains(RECOVERY_FOLLOWUP),
                "{phase}"
            );
        },
    );
}

fn check_run_recovery_doors() {
    let p = Tracked::new(false);
    let fake = FakeT3::create(&p.env);
    let db = recovery_db(&p.env);
    let key = "instrukt:run:demo:recovery";
    fake.seed("s1", p.repo.path(), key, "Opening");
    let input = serde_json::json!({
        "checkout":p.repo.path(),"key":key,"title":key,"brief":"Opening","worktree":null,
        "selection":{"profile":"codex","kind":"codex","instance":"codex","model":"gpt-test","effort":null,"options":{}}
    });
    let path = p.repo.path().to_string_lossy();
    db.execute(
        "INSERT INTO runtime_starts(runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state)
         VALUES('t3',?1,'demo','run',?1,?2,'Opening',?3,'2030-01-06T09:00:20Z',0,'unresolved')",
        instrukt_core::state_db::rusqlite::params![key,path,input.to_string()],
    ).unwrap();
    let mut intent = input;
    intent["opening_acknowledged"] = true.into();
    db.execute(
        "INSERT INTO runtime_bindings(runtime,semantic_key,kind,project,subject,session,checkout,accepted,request)
         VALUES('t3',?1,'run','demo',?1,'t3:s1',?2,0,?3)",
        instrukt_core::state_db::rusqlite::params![key,path,intent.to_string()],
    ).unwrap();
    check_recovery_doors(
        &p.env,
        p.repo.path(),
        &fake,
        &db,
        None,
        || run_recovery_worker(&p.env, p.repo.path(), key, None),
        |key, native| {
            let target: String = db
                .query_row(
                    "SELECT session FROM runtime_bindings WHERE semantic_key=?1 AND accepted=1",
                    [key],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(target, format!("t3:{native}"));
            fake.set_status(native, "idle");
            run_recovery_worker(&p.env, p.repo.path(), key, Some(&target));
            assert!(fake.input_log(native).contains(RECOVERY_FOLLOWUP));
        },
    );

    let home = instrukt_core::home::Home::at_user_home(p.env.home().into());
    let broken_key = "instrukt:run:demo:broken-lock";
    use sha2::{Digest, Sha256};
    let name = Sha256::digest(format!("t3:{broken_key}").as_bytes());
    let broken_lock = home.locks_dir().join(format!("runtime-key-{name:x}.lock"));
    // An exact lock file path that is a directory fails even as root.
    std::fs::create_dir_all(&broken_lock).unwrap();
    let os_error = std::fs::OpenOptions::new()
        .write(true)
        .open(&broken_lock)
        .unwrap_err()
        .to_string();
    let broken = p
        .env
        .run(
            p.repo.path(),
            ["sessions", "resolve-start", broken_key, "--none"],
        )
        .failure();
    let stderr = String::from_utf8_lossy(&broken.get_output().stderr);
    let diagnostic = format!(
        "instrukt: cannot use the recovery lock in {} for semantic key `{broken_key}` ({os_error})",
        home.locks_dir().display()
    );
    let errors = stderr
        .lines()
        .filter(|line| !line.starts_with("instrukt: clock overridden"))
        .collect::<Vec<_>>();
    assert_eq!(errors, [diagnostic.as_str()]);

    let _dispatch = instrukt_core::fsx::LockGuard::acquire(
        &home.state_dir().join("runtime-dispatch.lock"),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    for (command, option, session) in [
        ("replace-deleted", "--stopped", "t3:s1"),
        ("resolve-start", "--session", "t3:s1"),
    ] {
        let result = p
            .env
            .instrukt(p.repo.path())
            .args(["sessions", command, "input:t3:s1", option, session])
            .timeout(std::time::Duration::from_secs(10))
            .assert()
            .failure();
        let stderr = String::from_utf8_lossy(&result.get_output().stderr);
        assert!(stderr.lines().any(|line| line ==
            "instrukt: `input:t3:s1` is an internal input-lock key. Use the assignment's `instrukt:…` semantic key from `instrukt events status`"
        ), "{stderr}");
    }
}

#[cfg(debug_assertions)]
fn concurrent_resolutions_bind_the_native_thread_to_only_one_key() {
    let p = Tracked::new(false);
    let fake = FakeT3::create(&p.env);
    let db = recovery_db(&p.env);
    let keys = ["instrukt:run:demo:first", "instrukt:run:demo:second"];
    fake.seed("s1", p.repo.path(), "An unbound thread", "Opening");
    let resolve = |key: &str, marker: &str| {
        let input = serde_json::json!({
            "checkout": p.repo.path(), "key": key, "title": key,
            "brief": "Retained opening", "worktree": null
        });
        db.execute(
            "INSERT INTO runtime_starts(runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state)
             VALUES('t3',?1,'demo','run',?1,?2,'Retained opening',?3,'2030-01-06T09:00:20Z',0,'unresolved')",
            instrukt_core::state_db::rusqlite::params![key,p.repo.path().to_string_lossy(),input.to_string()],
        ).unwrap();
        let hold = p.env.path(marker);
        std::fs::write(&hold, key).unwrap();
        let child = p
            .env
            .instrukt_process(p.repo.path())
            .env("INSTRUKT_TEST_RESOLVE_BINDING_HOLD", &hold)
            .args(["sessions", "resolve-start", key, "--session", "t3:s1"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        (hold, child)
    };
    let (first, first_child) = resolve(keys[0], "first-binding");
    let (second, second_child) = resolve(keys[1], "second-binding");
    common::wait_until(
        "both different keys reach the binding transaction",
        std::time::Duration::from_secs(15),
        || first.with_extension("observed").exists() && second.with_extension("observed").exists(),
    );
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
    let outputs = [
        first_child.wait_with_output().unwrap(),
        second_child.wait_with_output().unwrap(),
    ];
    assert_eq!(
        outputs.iter().filter(|o| o.status.success()).count(),
        1,
        "{outputs:?}"
    );
    let winner = usize::from(!outputs[0].status.success());
    let loser = 1 - winner;
    let error = String::from_utf8_lossy(&outputs[loser].stderr);
    assert!(
        error.contains("already belongs to another runtime key"),
        "{error}"
    );
    assert!(
        error.contains(&format!(
            "instrukt sessions resolve-start {} --session <runtime:id>",
            keys[loser]
        )),
        "{error}"
    );
    let bound: String = db
        .query_row(
            "SELECT semantic_key FROM runtime_bindings WHERE session='t3:s1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bound, keys[winner]);
    let state: String = db
        .query_row(
            "SELECT state FROM runtime_starts WHERE semantic_key=?1",
            [keys[loser]],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "unresolved");
}

fn run_recovery_worker(
    env: &common::TestEnv,
    root: &std::path::Path,
    key: &str,
    target: Option<&str>,
) {
    let mut worker = env.command(&std::env::current_exe().unwrap(), root);
    worker
        .args([
            "--exact",
            "session_continuity::keyed_recovery_doors_follow_binding_state",
        ])
        .env("INSTRUKT_TEST_KEYED_RECOVERY", key);
    if let Some(target) = target {
        worker.env("INSTRUKT_TEST_RECOVERY_TARGET", target);
    }
    let output = worker.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn recovery_db(env: &common::TestEnv) -> instrukt_core::state_db::rusqlite::Connection {
    let home = instrukt_core::home::Home::at_user_home(env.home().into());
    drop(instrukt_core::state_db::StateDb::open(&home).unwrap());
    instrukt_core::state_db::rusqlite::Connection::open(
        env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap()
}

pub(super) fn fail_recovery_receipt(
    db: &instrukt_core::state_db::rusqlite::Connection,
    native: &str,
) {
    db.execute_batch(&format!(
        "CREATE TRIGGER fail_recovery_receipt BEFORE INSERT ON runtime_inputs
         WHEN NEW.session='{native}' BEGIN SELECT RAISE(ABORT, 'recovery receipt failed'); END;"
    ))
    .unwrap();
}

pub(super) fn check_recovery_doors(
    env: &common::TestEnv,
    root: &std::path::Path,
    fake: &FakeT3,
    db: &instrukt_core::state_db::rusqlite::Connection,
    phase: Option<&str>,
    wake: impl FnOnce(),
    later: impl FnOnce(&str, &str),
) {
    let key = fake.key("s1");
    let none = ["sessions", "resolve-start", &key, "--none"];
    let exact = ["sessions", "resolve-start", &key, "--session", "t3:s1"];
    let first_replace = ["sessions", "replace-deleted", &key, "--stopped", "t3:s1"];
    let expect_door = |args: &[&str], door: &[&str]| {
        let refused = env.run(root, args.iter().copied()).failure();
        let guidance = String::from_utf8_lossy(&refused.get_output().stderr);
        let expected = format!("instrukt {}", door.join(" "));
        assert!(guidance.contains(&expected), "{key}: {guidance}");
    };
    let binding = || {
        db.query_row(
            "SELECT session,accepted FROM runtime_bindings WHERE semantic_key=?1",
            [&key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
        )
    };
    let delete = |native: &str| {
        let folder = phase.map(|_| fake.worktree(native));
        fake.set_status(native, "stopped");
        fake.delete(native);
        folder
    };
    let clear_phase = |folder: Option<std::path::PathBuf>| {
        if let (Some(folder), Some(phase)) = (folder, phase) {
            env.git(root, ["worktree", "remove", folder.to_str().unwrap()]);
            env.git(root, ["branch", "-d", &format!("recovery/{phase}")]);
        }
    };

    assert_eq!(binding().unwrap(), ("t3:s1".into(), false), "{key}");
    expect_door(&none, &exact);
    expect_door(&first_replace, &exact);
    env.run(root, exact).success();
    assert_eq!(binding().unwrap(), ("t3:s1".into(), false), "{key}");

    let first_phase = delete("s1");
    expect_door(&exact, &none);
    expect_door(&first_replace, &none);
    fake.flag("show-failure", true);
    env.run(root, none).failure();
    assert_eq!(binding().unwrap(), ("t3:s1".into(), false), "{key}");
    fake.flag("show-failure", false);
    env.run(root, none).success();
    assert!(
        matches!(
            binding(),
            Err(instrukt_core::state_db::rusqlite::Error::QueryReturnedNoRows)
        ),
        "{key}"
    );
    let (state, reason): (String, String) = db
        .query_row(
            "SELECT state,resolution FROM runtime_starts WHERE semantic_key=?1",
            [&key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (state.as_str(), reason.as_str()),
        ("retry_allowed", "none"),
        "{key}"
    );
    assert!(fake.threads().is_empty(), "{key}");
    clear_phase(first_phase);
    wake();
    assert_eq!(binding().unwrap(), ("t3:s2".into(), true), "{key}");

    let committed_phase = delete("s2");
    let replace = ["sessions", "replace-deleted", &key, "--stopped", "t3:s2"];
    expect_door(&none, &replace);
    let s2_exact = ["sessions", "resolve-start", &key, "--session", "t3:s2"];
    expect_door(&s2_exact, &replace);
    db.execute_batch("DROP TRIGGER IF EXISTS fail_recovery_receipt;")
        .unwrap();
    clear_phase(committed_phase);
    env.run(root, replace).success();
    assert_eq!(binding().unwrap(), ("t3:s3".into(), true), "{key}");
    let receipt: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_inputs WHERE session='s3' AND input_id=?1 AND state='accepted')",
        [format!("assignment:{key}")], |row| row.get(0),
    ).unwrap();
    assert!(receipt, "{key}");
    assert_eq!(fake.threads(), ["s3"], "{key}");
    later(&key, "s3");
    assert_eq!(fake.message_count("s3"), 2, "{key}");

    let next_phase = delete("s3");
    let next_replace = ["sessions", "replace-deleted", &key, "--stopped", "t3:s3"];
    expect_door(&none, &next_replace);
    fail_recovery_receipt(db, "s4");
    clear_phase(next_phase);
    let interrupted = env.run(root, next_replace).failure();
    let failure = String::from_utf8_lossy(&interrupted.get_output().stderr);
    assert!(
        failure.contains("recovery receipt failed"),
        "{key}: {failure}"
    );
    assert_eq!(binding().unwrap(), ("t3:s4".into(), false), "{key}");

    let s4_exact = ["sessions", "resolve-start", &key, "--session", "t3:s4"];
    expect_door(&none, &next_replace);
    expect_door(&s4_exact, &next_replace);
    db.execute_batch("DROP TRIGGER fail_recovery_receipt")
        .unwrap();
    let guidance = env.run(root, none).failure();
    let guidance = String::from_utf8_lossy(&guidance.get_output().stderr);
    let command = guidance
        .split_once("`instrukt sessions replace-deleted ")
        .unwrap()
        .1;
    let command = command.split_once('`').unwrap().0;
    let mut args = vec!["sessions", "replace-deleted"];
    args.extend(command.split_whitespace());
    env.run(root, args).success();
    assert_eq!(binding().unwrap(), ("t3:s4".into(), true), "{key}");
    assert_eq!(fake.threads(), ["s4"], "{key}");
    assert_eq!(fake.message_count("s4"), 1, "{key}");

    let retired_phase = delete("s4");
    let replace_accepted = ["sessions", "replace-deleted", &key, "--stopped", "t3:s4"];
    fail_recovery_receipt(db, "s5");
    clear_phase(retired_phase);
    env.run(root, replace_accepted).failure();
    assert_eq!(binding().unwrap(), ("t3:s5".into(), false), "{key}");

    let pending_phase = delete("s5");
    let replace_pending = ["sessions", "replace-deleted", &key, "--stopped", "t3:s5"];
    expect_door(&none, &replace_pending);
    let s5_exact = ["sessions", "resolve-start", &key, "--session", "t3:s5"];
    expect_door(&s5_exact, &replace_pending);
    clear_phase(pending_phase);
    env.run(root, replace_pending).success();
    assert_eq!(binding().unwrap(), ("t3:s6".into(), true), "{key}");
    assert_eq!(fake.threads(), ["s6"], "{key}");
    assert_eq!(fake.message_count("s6"), 1, "{key}");
}

// spec: project/instrukt/spec/feature/runner/session-continuity UC-SS24
#[test]
fn replacing_a_deleted_task_updates_the_work_binding_and_claim_atomically() {
    let mut p = Tracked::new(true);
    p.authorized_task("weekly-status", "2030-01-06T09:00:00Z", &[]);
    p.tick_at("2030-01-06T09:00:20Z");
    let fake = FakeT3::create(&p.env);
    let key = fake.key("s1");
    fake.delete("s1");
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let command = || {
        p.env.run(
            p.env.home(),
            ["sessions", "replace-deleted", &key, "--stopped", "t3:s1"],
        )
    };
    let (machine, claimed_at): (String, String) = db
        .query_row(
            "SELECT claim_machine,claim_at FROM todos WHERE slug='weekly-status'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    db.execute(
        "UPDATE todos SET claim_machine=NULL,claim_session=NULL,claim_at=NULL WHERE slug='weekly-status'",
        [],
    ).unwrap();
    let ineligible = command().failure();
    assert!(
        fake.threads().is_empty(),
        "an ineligible replacement must start nothing"
    );
    let error = String::from_utf8_lossy(&ineligible.get_output().stderr);
    assert!(error.contains("todo replacement is ineligible"), "{error}");
    db.execute(
        "UPDATE todos SET claim_machine=?1,claim_session='t3:s1',claim_at=?2 WHERE slug='weekly-status'",
        (&machine, &claimed_at),
    ).unwrap();

    db.execute(
        "INSERT OR REPLACE INTO executions(project,todo_id,role,session,workflow_state,recovery_sent,anomaly_raised)
         SELECT t.project,t.id,b.role,'t3:s1','task-executing',1,1
         FROM todos t JOIN session_bindings b ON t.id=b.todo_id WHERE b.semantic_key=?1",
        [&key],
    ).unwrap();
    let execution = || {
        db.query_row(
            "SELECT session,recovery_sent,anomaly_raised FROM executions",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .unwrap()
    };
    db.execute_batch("CREATE TRIGGER fail_work_replacement BEFORE UPDATE OF claim_session ON todos
        WHEN NEW.claim_session='t3:s2' BEGIN SELECT RAISE(ABORT, 'work binding write failed'); END;").unwrap();
    let failed = command().failure();
    let error = String::from_utf8_lossy(&failed.get_output().stderr);
    assert!(
        error.contains("was not confirmed in the todo record"),
        "{error}"
    );
    let sessions = || {
        db.query_row(
            "SELECT b.session,t.claim_session,r.session,r.accepted FROM session_bindings b
             JOIN todos t ON b.todo_id=t.id JOIN runtime_bindings r ON r.semantic_key=b.semantic_key
             WHERE b.semantic_key=?1",
            [&key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            },
        )
        .unwrap()
    };
    assert_eq!(
        sessions(),
        ("t3:s1".into(), "t3:s1".into(), "t3:s2".into(), false)
    );
    assert_eq!(execution(), ("t3:s1".into(), true, true));
    assert_eq!(fake.threads(), ["s2"]);
    db.execute_batch("DROP TRIGGER fail_work_replacement;")
        .unwrap();

    // An unrelated exact-title match must not displace the returned ID.
    fake.seed("lookalike", p.repo.path(), &key, "Unrelated opening.");
    fake.change("s2", "title", "Renamed replacement".into());
    let refused = command().failure();
    let guidance = String::from_utf8_lossy(&refused.get_output().stderr);
    let live_door = format!("instrukt sessions replace-deleted {key} --stopped t3:s1");
    assert!(guidance.contains(&live_door), "{guidance}");
    let printed = guidance
        .split_once("then run `")
        .unwrap()
        .1
        .split_once('`')
        .unwrap()
        .0;
    let live_retry = || p.env.run(p.repo.path(), printed.split_whitespace().skip(1));
    assert_eq!(
        sessions(),
        ("t3:s1".into(), "t3:s1".into(), "t3:s2".into(), false)
    );
    fake.change("s2", "title", format!("Renamed replacement · {key}").into());
    db.execute_batch(
        "CREATE TRIGGER fail_opening_receipt BEFORE INSERT ON runtime_inputs
        WHEN NEW.session='s2' BEGIN SELECT RAISE(ABORT, 'opening receipt write failed'); END;",
    )
    .unwrap();
    let failed = live_retry().failure();
    let error = String::from_utf8_lossy(&failed.get_output().stderr);
    assert!(error.contains("opening receipt write failed"), "{error}");
    assert_eq!(
        sessions(),
        ("t3:s1".into(), "t3:s1".into(), "t3:s2".into(), false)
    );
    assert_eq!(execution(), ("t3:s1".into(), true, true));
    db.execute_batch("DROP TRIGGER fail_opening_receipt;")
        .unwrap();

    assert_eq!(fake.message_count("lookalike"), 1);
    fake.delete("lookalike");
    fake.change("s2", "projectId", "another-native-project".into());

    let moved = command().failure();
    let guidance = String::from_utf8_lossy(&moved.get_output().stderr);
    assert!(guidance.contains("native project changed"), "{guidance}");
    assert!(guidance.contains(&live_door), "{guidance}");
    let resolve_pending = || {
        p.env.run(
            p.repo.path(),
            ["sessions", "resolve-start", &key, "--session", "t3:s2"],
        )
    };
    let refused = resolve_pending().failure();
    let guidance = String::from_utf8_lossy(&refused.get_output().stderr);
    assert!(guidance.contains(&live_door), "{guidance}");
    let replace_pending = || {
        p.env.run(
            p.env.home(),
            ["sessions", "replace-deleted", &key, "--stopped", "t3:s2"],
        )
    };
    let still_live = replace_pending().failure();
    let refusal = String::from_utf8_lossy(&still_live.get_output().stderr);
    assert!(
        refusal.contains("thread t3:s2 must be deleted"),
        "{refusal}"
    );
    assert_eq!(fake.threads(), ["s2"]);
    assert_eq!(
        sessions(),
        ("t3:s1".into(), "t3:s1".into(), "t3:s2".into(), false)
    );

    fake.set_status("s2", "stopped");
    fake.delete("s2");
    let door = format!("instrukt sessions replace-deleted {key} --stopped t3:s2");
    for refused in [
        resolve_pending().failure(),
        p.env
            .run(p.repo.path(), ["sessions", "resolve-start", &key, "--none"])
            .failure(),
        command().failure(),
    ] {
        let guidance = String::from_utf8_lossy(&refused.get_output().stderr);
        assert!(guidance.contains(&door), "{guidance}");
    }
    let status = p
        .env
        .run(p.repo.path(), ["events", "status", "--json"])
        .success();
    let status: serde_json::Value = serde_json::from_slice(&status.get_output().stdout).unwrap();
    assert!(
        status["start_anomalies"][0]["failure"]
            .as_str()
            .unwrap()
            .contains(&door)
    );
    assert!(fake.threads().is_empty());

    db.execute_batch(
        "CREATE TRIGGER fail_next_receipt BEFORE INSERT ON runtime_inputs
         WHEN NEW.session='s3' BEGIN SELECT RAISE(ABORT, 'next opening receipt write failed'); END;",
    ).unwrap();
    let failed = replace_pending().failure();
    let guidance = String::from_utf8_lossy(&failed.get_output().stderr);
    assert!(
        guidance.contains("next opening receipt write failed"),
        "{guidance}"
    );
    assert!(guidance.contains(&door), "{guidance}");
    assert_eq!(fake.threads(), ["s3"]);
    assert_eq!(
        sessions(),
        ("t3:s1".into(), "t3:s1".into(), "t3:s3".into(), false)
    );
    assert_eq!(execution(), ("t3:s1".into(), true, true));
    db.execute_batch("DROP TRIGGER fail_next_receipt;").unwrap();

    db.execute_batch(
        "CREATE TRIGGER fail_replacement_reflection BEFORE INSERT ON reflections
        WHEN NEW.pending=0 AND (SELECT claim_session FROM todos WHERE id=NEW.todo_id)='t3:s3'
        BEGIN SELECT RAISE(ABORT, 'reflection save failed'); END;",
    )
    .unwrap();
    let replaced = replace_pending().success();
    let text = String::from_utf8_lossy(&replaced.get_output().stdout);
    assert!(
        text.contains("warning:") && text.contains("reflection save failed"),
        "{text}"
    );
    let repeated = replace_pending().success();
    assert!(
        String::from_utf8_lossy(&repeated.get_output().stdout)
            .contains("replaced t3:s2 with t3:s3")
    );
    db.execute_batch("DROP TRIGGER fail_replacement_reflection;")
        .unwrap();
    replace_pending().success();
    let pending: bool = db
        .query_row("SELECT pending FROM reflections", [], |row| row.get(0))
        .unwrap();
    assert!(!pending);
    assert_eq!(
        sessions(),
        ("t3:s3".into(), "t3:s3".into(), "t3:s3".into(), true)
    );
    assert_eq!(execution(), ("t3:s3".into(), false, false));
    let opening_accepted: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_inputs WHERE session='s3' AND input_id=?1 AND state='accepted')",
        [format!("assignment:{key}")], |row| row.get(0),
    ).unwrap();
    assert!(opening_accepted);
    assert_eq!(fake.threads(), ["s3"]);
    assert_eq!(fake.message_count("s3"), 1);
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
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.execute(
        "INSERT INTO runtime_bindings(runtime,semantic_key,kind,subject,session,checkout,accepted)
         VALUES ('t3','other-key','run','other-key','t3:s1',?1,1)",
        [p.repo.path().to_string_lossy().as_ref()],
    )
    .unwrap();
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
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        p.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.execute(
        "INSERT INTO runtime_bindings(runtime,semantic_key,kind,subject,session,checkout,accepted)
         VALUES ('t3','other-key','run','other-key','t3:s1',?1,1)",
        [p.repo.path().to_string_lossy().as_ref()],
    )
    .unwrap();
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
        serde_json::from_str(include_str!("../fixtures/runtime-start-v14.json")).unwrap();
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
    let normalized: String = db
        .query_row("SELECT request FROM runtime_starts", [], |r| r.get(0))
        .unwrap();
    let normalized: serde_json::Value = serde_json::from_str(&normalized).unwrap();
    assert_eq!(normalized["title"], key);
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

    // Deleting the thread leaves its phase worktree in place. A replacement
    // must not hand the assignment to a thread in the project root.
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
fn previous_driver_conversion_does_not_accept_a_genuine_native_relabel() {
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
    assert_eq!(fake.message_count(&id), 1);
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
            "SELECT request FROM runtime_bindings WHERE session='t3:s1'",
            [],
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
        "UPDATE runtime_bindings SET request=?1 WHERE session='t3:s1'",
        [old.to_string()],
    )
    .unwrap();
    db.pragma_update(None, "user_version", 14).unwrap();
    fake.change("s1", "title", key.clone().into());
    p.env
        .instrukt(&tree)
        .env("T3CODE_THREAD_ID", "s1")
        .args(["todo", "update", "old-bound-runtime", "--unclaim"])
        .assert()
        .success();
    p.tick();
    assert_eq!(fake.threads().len(), 1);
    assert_eq!(fake.message_count("s1"), 2);
    assert_eq!(fake.title("s1"), key);
    let normalized: String = db
        .query_row(
            "SELECT request FROM runtime_bindings WHERE session='t3:s1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let normalized: serde_json::Value = serde_json::from_str(&normalized).unwrap();
    assert_eq!(normalized["title"], key);
}
