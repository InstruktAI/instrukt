//! Scoped source publication and recovery through instrukt tick and actual Redis.
#![allow(clippy::unwrap_used, reason = "test: failed steps fail the test")]

use crate::common;
use crate::fixtures::event_transport as transport_support;
use crate::fixtures::workflow;
use common::redis::RedisServer;
use instrukt_runner::transport::EVENT_STREAM;
use transport_support::Computer;

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET1
#[test]
fn a_local_runner_occurrence_is_published_only_to_local_redis() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.tick();
    computer.change();
    computer.tick();
    let local_len: u64 = redis::cmd("XLEN")
        .arg(EVENT_STREAM)
        .query(&mut local.connection())
        .unwrap();
    let cluster_len: u64 = redis::cmd("XLEN")
        .arg(EVENT_STREAM)
        .query(&mut cluster.connection())
        .unwrap();
    assert_eq!((local_len, cluster_len), (1, 0));
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET3
#[test]
fn absent_local_configuration_reports_cluster_fallback() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    computer.config(&cluster, None);
    computer.env.run(&computer.env.path(""), ["tick"]).success();
    let log = std::fs::read_to_string(computer.env.log_dir().join("tick.log")).unwrap();
    let messages = log
        .lines()
        .filter(|line| line.contains(" logger=instrukt_runner::transport "))
        .map(|line| line.split_once(' ').unwrap().1)
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(messages, @r#"level=WARNING logger=instrukt_runner::transport msg="local Redis is unconfigured; local events use cluster Redis" event=local_redis_fallback"#);
    computer.change();
    computer.tick();
    let len: u64 = redis::cmd("XLEN")
        .arg(EVENT_STREAM)
        .query(&mut cluster.connection())
        .unwrap();
    assert_eq!(len, 1);
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET5
#[test]
fn a_failed_publication_retries_the_committed_occurrence_with_the_same_id() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    computer.config(&cluster, None);
    computer.tick();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("-xadd")
        .query::<()>(&mut cluster.connection())
        .unwrap();
    computer.change();
    computer.tick();
    let db = computer.db();
    let (id, pending, failure): (String, bool, bool) = db
        .query_row(
            "SELECT id, publication_state = 'pending', publication_failure IS NOT NULL FROM events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(pending && failure);
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("+xadd")
        .query::<()>(&mut cluster.connection())
        .unwrap();
    computer.tick();
    let entries: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
        .arg(EVENT_STREAM)
        .arg("-")
        .arg("+")
        .query(&mut cluster.connection())
        .unwrap();
    assert_eq!(entries.ids.len(), 1);
    let wire: String =
        redis::from_redis_value_ref(entries.ids[0].map.get("event").unwrap()).unwrap();
    let envelope = instrukt_runner::events::EventEnvelope::decode(&wire).unwrap();
    assert_eq!(envelope.id, id);
    let confirmed: bool = db
        .query_row(
            "SELECT publication_state = 'confirmed' FROM events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(confirmed);
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET3
#[test]
fn a_configured_local_outage_does_not_reroute_to_cluster() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let mut local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.tick();
    local.stop();
    computer.change();
    computer.tick();
    let cluster_len: u64 = redis::cmd("XLEN")
        .arg(EVENT_STREAM)
        .query(&mut cluster.connection())
        .unwrap();
    assert_eq!(cluster_len, 0);
    let pending: bool = computer
        .db()
        .query_row(
            "SELECT publication_state = 'pending' FROM events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(pending);
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET4
#[test]
fn identical_scope_endpoints_are_read_once_per_consumer() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, Some(&redis));
    computer.tick();
    computer.change();
    computer.tick();
    let sources: u64 = computer
        .db()
        .query_row("SELECT count(*) FROM consumer_sources", [], |row| {
            row.get(0)
        })
        .unwrap();
    let outcomes: u64 = computer
        .db()
        .query_row("SELECT count(*) FROM consumer_outcomes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!((sources, outcomes, computer.handled()), (1, 1, 1));
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET6
#[test]
fn cluster_outage_leaves_local_source_acceptance_responsive() {
    let computer = Computer::new();
    let mut cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.subscribe("runner.upstream_changed");
    computer.tick();
    // Use an opaque received occurrence so S3's acceptance is the boundary;
    // subsequent runtime execution is the separate S6 contract.
    let envelope = serde_json::json!({"id":"local-during-outage", "type":"runner.upstream_changed", "source":computer.computer(), "subject":"commit:fork/current", "time":"2026-11-05T00:00:00Z", "data":{}, "home":"project:fork", "home_owner":"unresolved"});
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("malformed")
        .query::<String>(&mut cluster.connection())
        .unwrap();
    computer.tick();
    redis::cmd("SAVE")
        .query::<()>(&mut cluster.connection())
        .unwrap();
    cluster.stop();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg(envelope.to_string())
        .query::<String>(&mut local.connection())
        .unwrap();
    computer.tick();
    assert_eq!(computer.accepted(), 1);
    cluster.restart();
    computer.tick();
    computer.tick();
    let pending: redis::streams::StreamPendingReply = redis::cmd("XPENDING")
        .arg(EVENT_STREAM)
        .arg(computer.group())
        .query(&mut cluster.connection())
        .unwrap();
    assert_eq!(
        pending.count(),
        0,
        "the restored source resumed its original pending group"
    );
    assert_eq!(computer.accepted(), 1);
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET7
#[test]
fn independent_source_positions_are_never_compared_across_scopes() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.subscribe("erp.invoice.*");
    computer.tick();
    let envelope = |id: &str| serde_json::json!({"id":id, "type":"erp.invoice.paid", "source":computer.computer(), "subject":"todo:sample", "time":"2026-11-05T00:00:00Z", "data":{}, "home":"project:fork", "home_owner":"unresolved"});
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("9000000000000-0")
        .arg("event")
        .arg(envelope("local").to_string())
        .query::<String>(&mut local.connection())
        .unwrap();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("1-0")
        .arg("event")
        .arg(envelope("cluster-first").to_string())
        .query::<String>(&mut cluster.connection())
        .unwrap();
    computer.tick();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("2-0")
        .arg("event")
        .arg(envelope("cluster-next").to_string())
        .query::<String>(&mut cluster.connection())
        .unwrap();
    computer.tick();
    assert_eq!(computer.accepted(), 3);
    let positions: u64 = computer.db().query_row("SELECT count(DISTINCT source) FROM consumer_cursors WHERE consumer = 'procedure-activation'", [], |row| row.get(0)).unwrap();
    assert_eq!(positions, 2);
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET7
#[test]
fn a_source_left_out_of_the_configuration_is_abandoned_and_drained_when_it_returns() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.subscribe("erp.invoice.*");
    computer.tick();
    let envelope = |id: &str| {
        serde_json::json!({"id":id, "type":"erp.invoice.paid", "source":computer.computer(), "subject":"todo:sample", "time":"2026-11-05T00:00:00Z", "data":{}, "home":"project:fork", "home_owner":"unresolved"}).to_string()
    };
    let append = |server: &RedisServer, id: &str| {
        redis::cmd("XADD")
            .arg(EVENT_STREAM)
            .arg("*")
            .arg("event")
            .arg(envelope(id))
            .query::<String>(&mut server.connection())
            .unwrap();
    };
    // An event waits in local Redis when local Redis leaves the configuration.
    append(&local, "left-in-local");
    computer.config(&cluster, None);
    append(&cluster, "after-the-change");
    computer.tick();
    // The new source runs at once; the former one is abandoned and named.
    let accepted = || -> Vec<String> {
        computer
            .db()
            .prepare("SELECT event_id FROM procedure_activations ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    insta::assert_debug_snapshot!(accepted(), @r#"
        [
            "after-the-change",
        ]
        "#);
    let mut abandoned = computer.consumers_status()["abandoned_sources"].clone();
    for source in abandoned.as_array_mut().unwrap() {
        assert!(!source["source"].as_str().unwrap().is_empty());
        source["source"] = "[SOURCE]".into();
    }
    insta::assert_json_snapshot!(abandoned, @r#"
        [
          {
            "consumer": "procedure-activation",
            "source": "[SOURCE]",
            "pending": "its pending events stay unread in that Redis until it is configured again"
          }
        ]
        "#);
    // Configured again, it is drained from its own group.
    computer.config(&cluster, Some(&local));
    computer.tick();
    insta::assert_debug_snapshot!(accepted(), @r#"
        [
            "after-the-change",
            "left-in-local",
        ]
        "#);
    insta::assert_json_snapshot!(computer.consumers_status()["abandoned_sources"], @"[]");
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET7
#[test]
fn the_old_runtime_cannot_read_a_source_a_new_runtime_abandoned() {
    use instrukt_core::config::{Configuration, Env};
    use instrukt_core::home::Home;
    use instrukt_runner::consumers::{Consumer, ConsumerRuntime, Interest};
    use instrukt_runner::events::TypePattern;
    use instrukt_runner::state::RunnerState;
    use instrukt_runner::transport::{EventScope, ScopedTransport};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    let home = Home::at_user_home(computer.env.home().to_path_buf());
    let state = RunnerState::new(&home);
    instrukt_core::machine::machine_id(&home).unwrap();
    let handled = Arc::new(AtomicUsize::new(0));
    let consumer = Consumer {
        name: "handover-test".into(),
        interest: Interest {
            patterns: vec![TypePattern::parse("erp.invoice.paid").unwrap()],
            own_computer_only: false,
            external_on_reacting: false,
            subject_kinds: Vec::new(),
            home_kinds: Vec::new(),
        },
        scopes: vec![EventScope::Local, EventScope::Cluster],
        reaction: Arc::new({
            let handled = Arc::clone(&handled);
            move |_, _| {
                handled.fetch_add(1, Ordering::Relaxed);
                Ok(serde_json::Value::Null)
            }
        }),
    };
    let runtime = || {
        let config = Configuration::load_global(&home).unwrap();
        let transport = ScopedTransport::from_config(&config, &Env::default(), &state).unwrap();
        ConsumerRuntime::new(state.clone(), transport, "handover", vec![consumer.clone()]).unwrap()
    };
    let now = jiff::Timestamp::now();
    computer.config(&cluster, Some(&local));
    let old = runtime();
    old.drain(now);
    let event = serde_json::json!({
        "id": "left-behind", "type": "erp.invoice.paid", "source": "handover",
        "subject": "todo:sample", "time": now.to_string(), "data": {},
        "home": "project:fork", "home_owner": "unresolved"
    });
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg(event.to_string())
        .query::<String>(&mut local.connection())
        .unwrap();

    computer.config(&cluster, None);
    let replacement = runtime();
    old.drain(now);
    replacement.drain(now);
    assert_eq!(handled.load(Ordering::Relaxed), 0);
    assert_eq!(
        computer.consumers_status()["abandoned_sources"][0]["consumer"],
        "handover-test"
    );

    computer.config(&cluster, Some(&local));
    runtime().drain(now);
    assert_eq!(handled.load(Ordering::Relaxed), 1);
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET5
#[test]
fn finalize_ready_keeps_its_stamp_and_occurrence_through_a_publication_failure() {
    let crew = workflow::Crew::new();
    let project = &crew.project;
    let redis_server = RedisServer::new(&project.env);
    let global_path = project.env.instrukt_home().join("instrukt.yaml");
    let mut global = std::fs::read_to_string(&global_path).unwrap();
    global.push_str(&format!(
        "modules: {{events: true}}\nredis:\n  cluster:\n    url: '{}'\n",
        redis_server.url
    ));
    std::fs::write(global_path, global).unwrap();
    project.env.run(crew.root(), ["sync"]).success();
    let mut redis = redis_server.connection();
    project.env.run(crew.root(), ["tick"]).success();
    project.create("handoff", "change", "Hand committed work to integration.");
    crew.write(
        "handoff",
        "implementation-plan",
        "Prepare the final candidate.",
    );
    crew.write("handoff", "plan-review-findings", "Approved plan.");
    crew.stamp(
        "handoff",
        &["--verdict", "approve", "--phase", "plan_review"],
    );
    crew.next("work", "handoff");
    let start = |procedure: Option<&str>| {
        let role = match procedure {
            None => "setup",
            Some("crew/procedure/finalize") => "finalizer",
            _ => "build-reviewer",
        };
        crew.worker("handoff", role, procedure)
    };
    let stamp = |worker: &serde_json::Value, flags: &[&str]| {
        project
            .env
            .instrukt(std::path::Path::new(worker["worktree"].as_str().unwrap()))
            .env(
                "T3CODE_THREAD_ID",
                worker["session"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("t3:")
                    .unwrap(),
            )
            .args(["todo", "stamp", "handoff"])
            .args(flags)
            .assert()
            .success();
    };
    let setup = start(None);
    stamp(&setup, &["--role-complete"]);
    let finalizer = start(Some("crew/procedure/finalize"));
    let path = std::path::Path::new(finalizer["worktree"].as_str().unwrap());
    std::fs::write(path.join("handoff.rs"), "// ready for integration\n").unwrap();
    project.env.git(path, ["add", "handoff.rs"]);
    project.env.git(
        path,
        ["commit", "--quiet", "-m", "feat: prepare the handoff"],
    );
    project
        .env
        .git(path, ["push", "--quiet", "origin", "HEAD:handoff/work"]);
    project.repo.git(["fetch", "--quiet", "origin"]);
    stamp(&finalizer, &["--role-complete"]);
    let reviewer = start(Some("crew/procedure/review-build"));
    crew.write(
        "handoff",
        "review-findings",
        "Approve the finalized candidate.",
    );
    stamp(
        &reviewer,
        &["--verdict", "approve", "--phase", "build_review"],
    );
    stamp(&finalizer, &["--finalize-ready"]);
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("-xadd")
        .query::<()>(&mut redis)
        .unwrap();
    project.env.run(crew.root(), ["tick"]).success();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        project.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let (id, pending, stamp_count): (String, bool, u64) = db.query_row("SELECT id, publication_state = 'pending', (SELECT count(*) FROM stamps WHERE fact = 'finalize_ready') FROM events WHERE name = 'workflow.finalized'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert!(pending);
    assert_eq!((stamp_count, crew.runtime.started_count()), (1, 3));
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("+xadd")
        .query::<()>(&mut redis)
        .unwrap();
    project.env.run(crew.root(), ["tick"]).success();
    let (confirmed, stamp_count): (bool, u64) = db.query_row("SELECT publication_state = 'confirmed', (SELECT count(*) FROM stamps WHERE fact = 'finalize_ready') FROM events WHERE id = ?1", [&id], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert!(confirmed);
    assert_eq!((stamp_count, crew.runtime.started_count()), (1, 3));
    let stream: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
        .arg(EVENT_STREAM)
        .arg("-")
        .arg("+")
        .query(&mut redis)
        .unwrap();
    assert_eq!(stream.ids.len(), 1);
    let encoded: String =
        redis::from_redis_value_ref(stream.ids[0].map.get("event").unwrap()).unwrap();
    assert_eq!(
        instrukt_runner::events::EventEnvelope::decode(&encoded)
            .unwrap()
            .id,
        id
    );
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET7
#[test]
fn a_blocked_source_reader_defers_handover_until_a_later_attempt() {
    use instrukt_core::config::{Configuration, Env};
    use instrukt_core::home::Home;
    use instrukt_runner::consumers::{Consumer, ConsumerRuntime, Interest};
    use instrukt_runner::events::TypePattern;
    use instrukt_runner::state::RunnerState;
    use instrukt_runner::transport::{EventScope, ScopedTransport};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    let gate = common::redis_proxy::DeliveryGate::new(&local);
    let home = Home::at_user_home(computer.env.home().to_path_buf());
    let state = RunnerState::new(&home);
    let consumer = Consumer {
        name: "blocked-handover".into(),
        interest: Interest {
            patterns: vec![TypePattern::parse("erp.invoice.paid").unwrap()],
            own_computer_only: false,
            external_on_reacting: false,
            subject_kinds: Vec::new(),
            home_kinds: Vec::new(),
        },
        scopes: vec![EventScope::Local, EventScope::Cluster],
        reaction: Arc::new(|_, _| Ok(serde_json::Value::Null)),
    };
    let runtime = || {
        let config = Configuration::load_global(&home).unwrap();
        let transport = ScopedTransport::from_config(&config, &Env::default(), &state).unwrap();
        ConsumerRuntime::new(state.clone(), transport, "handover", vec![consumer.clone()])
    };
    let now = jiff::Timestamp::now();
    computer.config_urls(&cluster.url, Some(&gate.url));
    let old = runtime().unwrap();
    old.drain(now);
    gate.arm();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("held by Redis")
        .query::<String>(&mut local.connection())
        .unwrap();

    std::thread::scope(|threads| {
        threads.spawn(|| old.drain(now));
        transport_support::until(|| gate.intercepted());
        computer.config(&cluster, None);
        let (send, result) = mpsc::channel();
        let runtime = &runtime;
        let binder =
            threads.spawn(move || send.send(runtime().err().map(|error| error.to_string())));
        let deferred = result.recv_timeout(Duration::from_secs(4));
        let abandoned: usize = computer
            .db()
            .query_row(
                "SELECT count(*) FROM consumer_sources WHERE state = 'abandoned'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        gate.release();
        binder.join().unwrap().unwrap();
        assert_eq!(abandoned, 0);
        assert!(
            matches!(deferred, Ok(Some(error)) if error.contains("lock")),
            "handover should have deferred"
        );
    });
    assert!(runtime().is_ok());
    let abandoned: usize = computer
        .db()
        .query_row(
            "SELECT count(*) FROM consumer_sources WHERE state = 'abandoned'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(abandoned, 1);
}

// spec: project/instrukt/spec/feature/runner/event-transport UC-ET7
#[test]
fn a_hosted_runner_reports_a_stalled_handover_and_retries_after_the_reader_stops() {
    let mut computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    let gate = common::redis_proxy::DeliveryGate::new(&local);
    let clock = computer.env.path("handover-clock");
    let set_time = |time: &str| {
        let staged = clock.with_extension("new");
        std::fs::write(&staged, time).unwrap();
        std::fs::rename(staged, &clock).unwrap();
    };
    set_time("2030-01-01T00:00:00Z");
    computer
        .env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    computer.config_urls(&cluster.url, Some(&gate.url));
    computer.subscribe("erp.invoice.*");
    computer.tick();

    gate.arm();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("blocked")
        .query::<String>(&mut local.connection())
        .unwrap();
    let _running = transport_support::Running::start(&computer);
    transport_support::until(|| gate.intercepted());
    computer.config(&cluster, None);
    set_time("2030-01-01T00:01:01Z");
    common::wait_until(
        "a pending handover with an age while Redis withholds the read reply",
        std::time::Duration::from_secs(12),
        || {
            let item = &computer.consumers_status()["pending_source_transitions"][0];
            item["transition"] == "runner"
                && item["age_seconds"].as_i64().is_some_and(|age| age >= 1)
        },
    );
    gate.release();
    set_time("2030-01-01T00:02:02Z");
    transport_support::until(|| {
        let status = computer.consumers_status();
        status["pending_source_transitions"]
            .as_array()
            .is_some_and(Vec::is_empty)
            && status["abandoned_sources"]
                .as_array()
                .is_some_and(|sources| sources.len() == 1)
    });
}
