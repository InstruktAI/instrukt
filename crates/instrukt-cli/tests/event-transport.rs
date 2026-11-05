//! Scoped source publication and recovery through instrukt tick and actual Redis.
#![allow(clippy::unwrap_used, reason = "test: failed steps fail the test")]
mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/event_transport.rs"]
mod support;

use common::redis::RedisServer;
use instrukt_runner::transport::EVENT_STREAM;
use support::Computer;

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
    let run = computer.env.run(&computer.env.path(""), ["tick"]).success();
    let stderr = String::from_utf8_lossy(&run.get_output().stderr);
    let messages = stderr
        .lines()
        .map(|line| line.split_once(" WARN ").map_or(line, |(_, text)| text))
        .filter(|line| line.starts_with("instrukt_runner::transport:"))
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(messages, @"instrukt_runner::transport: local Redis is unconfigured; local events use cluster Redis event=\"local_redis_fallback\"");
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
    redis::cmd("SET")
        .arg(EVENT_STREAM)
        .arg("wrong-type")
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
    redis::cmd("DEL")
        .arg(EVENT_STREAM)
        .query::<u64>(&mut cluster.connection())
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
