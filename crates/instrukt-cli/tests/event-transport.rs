//! Scoped source publication and recovery through instrukt tick and actual Redis.
#![allow(clippy::unwrap_used, reason = "test: failed steps fail the test")]
mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod support;
#[path = "support/event_transport.rs"]
mod transport_support;
#[path = "support/workflow.rs"]
mod workflow;

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
    let log = std::fs::read_to_string(
        computer
            .env
            .home()
            .join(".local/state/instrukt-ai/instrukt/tick.log"),
    )
    .unwrap();
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
