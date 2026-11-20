//! Retained group progress, selection and parking through the real CLI and Redis.
#![allow(clippy::unwrap_used, reason = "test: failed steps fail the test")]

use crate::common;
use crate::fixtures::documented;
use crate::fixtures::event_transport as support;
use common::redis::RedisServer;
use instrukt_runner::consumers::{MAX_HANDLING_ATTEMPTS, SettledOutcome};
use instrukt_runner::events::EventEnvelope;
use instrukt_runner::transport::{DEAD_LETTER_STREAM, EVENT_STREAM};
use serde_json::json;
use support::Computer;

fn event(computer: &Computer, id: &str, name: &str) -> EventEnvelope {
    serde_json::from_value(json!({
        "id": id, "type": name, "source": computer.computer(), "subject": "todo:sample",
        "time": "2026-11-05T00:00:00Z", "data": {}, "home": "project:fork", "home_owner": "unresolved"
    })).unwrap()
}

fn append(server: &RedisServer, envelope: &EventEnvelope) -> String {
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg(envelope.encode().unwrap())
        .query(&mut server.connection())
        .unwrap()
}

fn pending(server: &RedisServer, computer: &Computer) -> usize {
    let reply: redis::streams::StreamPendingReply = redis::cmd("XPENDING")
        .arg(EVENT_STREAM)
        .arg(computer.group())
        .query(&mut server.connection())
        .unwrap();
    reply.count()
}

fn last_outcome(computer: &Computer) -> SettledOutcome {
    let text: String = computer.db().query_row("SELECT outcome FROM consumer_outcomes WHERE consumer = 'procedure-activation' ORDER BY rowid DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    serde_json::from_str(&text).unwrap()
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC2
#[test]
fn two_entries_for_one_occurrence_accept_only_one_activation() {
    let mut computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer
        .env
        .set_env("INSTRUKT_NOW", "2026-11-05T00:00:00.9Z");
    computer.tick();
    let envelope = event(&computer, "one-occurrence", "erp.invoice.paid");
    append(&redis, &envelope);
    computer.tick();
    computer
        .env
        .set_env("INSTRUKT_NOW", "2026-11-12T00:00:00.1Z");
    append(&redis, &envelope);
    computer.tick();
    assert_eq!(
        (
            computer.accepted(),
            computer.handled(),
            pending(&redis, &computer)
        ),
        (1, 1, 0)
    );
    assert!(matches!(last_outcome(&computer), SettledOutcome::Duplicate));
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC2
#[test]
fn an_expired_handled_mark_allows_the_occurrence_to_be_handled_again() {
    let mut computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.env.set_env("INSTRUKT_NOW", "2026-11-05T00:00:00Z");
    computer.tick();
    let envelope = event(&computer, "one-occurrence", "erp.invoice.paid");
    append(&redis, &envelope);
    computer.tick();
    computer.env.set_env("INSTRUKT_NOW", "2026-11-13T00:00:00Z");
    append(&redis, &envelope);
    computer.tick();
    assert!(matches!(
        last_outcome(&computer),
        SettledOutcome::Accepted(_)
    ));
    assert_eq!(
        computer.accepted(),
        1,
        "the keyed reaction's own idempotency remains independent of mark expiry"
    );
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC4
#[test]
fn a_foreign_computer_occurrence_is_settled_without_activation() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    let mut envelope = event(&computer, "foreign-occurrence", "erp.invoice.paid");
    envelope.source = "another-computer".into();
    append(&redis, &envelope);
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (0, 0));
    assert!(matches!(
        last_outcome(&computer),
        SettledOutcome::Uninterested
    ));
    // An occurrence this computer emitted that no procedure subscribes to
    // is handed to the reaction, settles with no activation and is counted.
    append(
        &redis,
        &event(&computer, "own-occurrence", "erp.order.placed"),
    );
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (0, 0));
    insta::assert_snapshot!(
        computer.consumers_status()["settled_without_activation"],
        @"1"
    );
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC5
#[test]
fn a_whole_segment_prefix_accepts_deeper_vocabulary() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    append(&redis, &event(&computer, "paid", "erp.invoice.paid"));
    append(
        &redis,
        &event(&computer, "reminded", "erp.invoice.overdue.reminded"),
    );
    computer.tick();
    assert_eq!(computer.accepted(), 2);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC5
#[test]
fn a_vocabulary_prefix_excludes_its_parent_and_neighbor() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    append(&redis, &event(&computer, "parent", "erp.invoice"));
    append(&redis, &event(&computer, "neighbor", "erp.invoices.paid"));
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (0, 0));
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC7
#[test]
fn opaque_payload_instructions_cannot_select_a_different_reaction() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    let ordinary = event(&computer, "ordinary", "erp.invoice.paid");
    let mut opaque = event(&computer, "opaque", "erp.invoice.paid");
    opaque.data.insert(
        "instructions".into(),
        json!({"home": "none", "type": "ignore.all", "skip": true}),
    );
    append(&redis, &ordinary);
    append(&redis, &opaque);
    computer.tick();
    assert_eq!((computer.accepted(), computer.handled()), (2, 2));
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn malformed_entries_are_parked_with_evidence_before_later_entries_proceed() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("invalid")
        .arg(vec![0u8, 255, 128])
        .query::<String>(&mut redis.connection())
        .unwrap();
    append(&redis, &event(&computer, "later", "erp.invoice.paid"));
    computer.tick();
    assert_eq!(computer.accepted(), 0);
    computer.tick();
    assert_eq!(computer.accepted(), 0);
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (1, 0));
    let dead: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
        .arg(DEAD_LETTER_STREAM)
        .arg("-")
        .arg("+")
        .query(&mut redis.connection())
        .unwrap();
    assert_eq!(dead.ids.len(), 1);
    let record: String =
        redis::from_redis_value_ref(dead.ids[0].map.get("record").unwrap()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&record).unwrap();
    assert_eq!(
        value["attempt_count"].as_u64().unwrap(),
        u64::from(MAX_HANDLING_ATTEMPTS)
    );
    assert_eq!(
        value["original_envelope"]["raw_fields"][0][1],
        json!([0, 255, 128])
    );
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC9
#[test]
fn a_retained_group_catches_up_after_downtime() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    append(
        &redis,
        &event(&computer, "during-downtime", "erp.invoice.paid"),
    );
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (1, 0));
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC10
#[test]
fn a_new_group_starts_at_the_present_without_replaying_old_entries() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    let stale = json!({"id":"old", "type":"erp.invoice.paid", "source":"old-source", "subject":"todo:sample", "time":"2026-11-05T00:00:00Z", "data":{}, "home":"project:fork", "home_owner":"unresolved"});
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg(stale.to_string())
        .query::<String>(&mut redis.connection())
        .unwrap();
    computer.tick();
    let outcomes: u64 = computer
        .db()
        .query_row("SELECT count(*) FROM consumer_outcomes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(outcomes, 0);
    append(&redis, &event(&computer, "new", "erp.invoice.paid"));
    computer.tick();
    assert_eq!(computer.accepted(), 1);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC3
#[test]
fn a_binary_killed_after_handled_commit_recovers_ack_without_repeating_acceptance() {
    let mut computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.env.set_env("INSTRUKT_NOW", "2026-11-05T00:00:00Z");
    computer.tick();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("-xack")
        .query::<()>(&mut redis.connection())
        .unwrap();
    append(
        &redis,
        &event(&computer, "interrupted-ack", "erp.invoice.paid"),
    );
    let mut running = support::Running::start(&computer);
    support::until(|| computer.handled() == 1 && pending(&redis, &computer) == 1);
    running.kill();
    let before: String = computer
        .db()
        .query_row(
            "SELECT handled_at FROM handled_events WHERE consumer = 'procedure-activation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("+xack")
        .query::<()>(&mut redis.connection())
        .unwrap();
    computer.env.set_env("INSTRUKT_NOW", "2026-11-06T00:00:00Z");
    computer.tick();
    let after: String = computer
        .db()
        .query_row(
            "SELECT handled_at FROM handled_events WHERE consumer = 'procedure-activation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        after, before,
        "duplicate recovery did not repeat handling or renew the mark"
    );
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (1, 0));
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC3
#[test]
fn a_binary_killed_before_the_local_outcome_reoffers_its_pending_entry() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    let gate = common::redis_proxy::DeliveryGate::new(&redis);
    computer.config_urls(&gate.url, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    gate.arm();
    append(
        &redis,
        &event(&computer, "interrupted-outcome", "erp.invoice.paid"),
    );
    let mut running = support::Running::start(&computer);
    support::until(|| gate.intercepted());
    running.kill();
    assert_eq!((computer.handled(), pending(&redis, &computer)), (0, 1));
    gate.release();
    computer.tick();
    assert_eq!(
        (
            computer.accepted(),
            computer.handled(),
            pending(&redis, &computer)
        ),
        (1, 1, 0)
    );
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn a_failed_dead_letter_persist_keeps_later_work_unsettled_until_repaired() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    redis::cmd("SET")
        .arg(DEAD_LETTER_STREAM)
        .arg("wrong-type")
        .query::<()>(&mut redis.connection())
        .unwrap();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("malformed")
        .query::<String>(&mut redis.connection())
        .unwrap();
    append(&redis, &event(&computer, "waiting", "erp.invoice.paid"));
    computer.tick();
    computer.tick();
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (0, 1));
    redis::cmd("DEL")
        .arg(DEAD_LETTER_STREAM)
        .query::<u64>(&mut redis.connection())
        .unwrap();
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (1, 0));
    let letters: u64 = redis::cmd("XLEN")
        .arg(DEAD_LETTER_STREAM)
        .query(&mut redis.connection())
        .unwrap();
    assert_eq!(letters, 1);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn a_dead_letter_that_cannot_be_announced_is_counted_and_later_work_proceeds() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.subscribe("erp.invoice.*");
    computer.tick();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("malformed")
        .query::<String>(&mut cluster.connection())
        .unwrap();
    append(&cluster, &event(&computer, "later", "erp.invoice.paid"));
    computer.tick();
    computer.tick();
    // Local Redis cannot take the announcement when the entry is parked.
    redis::cmd("SET")
        .arg(EVENT_STREAM)
        .arg("wrong-type")
        .query::<()>(&mut local.connection())
        .unwrap();
    computer.tick();
    let letters: u64 = redis::cmd("XLEN")
        .arg(DEAD_LETTER_STREAM)
        .query(&mut cluster.connection())
        .unwrap();
    assert_eq!(
        (letters, pending(&cluster, &computer), computer.accepted()),
        (1, 0, 1)
    );
    assert_eq!(computer.consumers_status()["unannounced_dead_letters"], 1);
    assert_eq!(
        redis::cmd("TYPE")
            .arg(EVENT_STREAM)
            .query::<String>(&mut local.connection())
            .unwrap(),
        "string"
    );

    // Recovery resends the durable intent after local Redis can append again.
    redis::cmd("DEL")
        .arg(EVENT_STREAM)
        .query::<u64>(&mut local.connection())
        .unwrap();
    computer.tick();
    computer.tick();
    let letters: u64 = redis::cmd("XLEN")
        .arg(DEAD_LETTER_STREAM)
        .query(&mut cluster.connection())
        .unwrap();
    let announcements: u64 = redis::cmd("XLEN")
        .arg(EVENT_STREAM)
        .query(&mut local.connection())
        .unwrap();
    assert_eq!(
        (letters, announcements, pending(&cluster, &computer)),
        (1, 1, 0)
    );
    assert_eq!(computer.consumers_status()["unannounced_dead_letters"], 0);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn a_binary_killed_after_parking_recovers_only_the_unfinished_ack_and_settlement() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.tick();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("malformed")
        .query::<String>(&mut redis.connection())
        .unwrap();
    computer.tick();
    computer.tick();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("-xack")
        .query::<()>(&mut redis.connection())
        .unwrap();
    let mut running = support::Running::start(&computer);
    support::until(|| {
        computer
            .db()
            .query_row("SELECT count(*) FROM event_parking", [], |r| {
                r.get::<_, u64>(0)
            })
            .unwrap()
            == 1
    });
    running.kill();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("+xack")
        .query::<()>(&mut redis.connection())
        .unwrap();
    computer.tick();
    let letters: u64 = redis::cmd("XLEN")
        .arg(DEAD_LETTER_STREAM)
        .query(&mut redis.connection())
        .unwrap();
    let entries: u64 = redis::cmd("XLEN")
        .arg(EVENT_STREAM)
        .query(&mut redis.connection())
        .unwrap();
    let unfinished: u64 = computer
        .db()
        .query_row("SELECT count(*) FROM event_parking", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        (letters, entries, pending(&redis, &computer), unfinished),
        (1, 1, 0, 0)
    );
    assert_eq!(computer.consumers_status()["unannounced_dead_letters"], 1);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC9
#[test]
fn a_trimmed_pending_payload_is_recorded_as_loss_instead_of_success() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.tick();
    let id = append(&redis, &event(&computer, "trimmed", "erp.invoice.paid"));
    let _: redis::streams::StreamReadReply = redis::cmd("XREADGROUP")
        .arg("GROUP")
        .arg(computer.group())
        .arg(format!("reader-{}", computer.computer()))
        .arg("COUNT")
        .arg(1)
        .arg("STREAMS")
        .arg(EVENT_STREAM)
        .arg(">")
        .query(&mut redis.connection())
        .unwrap();
    redis::cmd("XDEL")
        .arg(EVENT_STREAM)
        .arg(id)
        .query::<u64>(&mut redis.connection())
        .unwrap();
    computer.tick();
    assert!(matches!(
        last_outcome(&computer),
        SettledOutcome::PayloadLost
    ));
    assert_eq!(computer.handled(), 0);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC1
#[test]
fn a_failing_computer_group_does_not_stall_another_computers_group() {
    let failing = Computer::new();
    let responsive = Computer::new();
    let redis = RedisServer::new(&failing.env);
    failing.config(&redis, None);
    responsive.config(&redis, None);
    failing.subscribe("erp.invoice.*");
    responsive.subscribe("erp.invoice.*");
    failing.tick();
    responsive.tick();
    std::fs::write(
        failing.env.instrukt_home().join("state/index.yaml"),
        "invalid: [",
    )
    .unwrap();
    append(
        &redis,
        &event(&failing, "failing-occurrence", "erp.invoice.paid"),
    );
    append(
        &redis,
        &event(&responsive, "responsive-occurrence", "erp.invoice.paid"),
    );
    failing.tick();
    responsive.tick();
    assert_eq!((failing.accepted(), pending(&redis, &failing)), (0, 1));
    assert_eq!(
        (responsive.accepted(), pending(&redis, &responsive)),
        (1, 0)
    );
    let groups: redis::streams::StreamInfoGroupsReply = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg(EVENT_STREAM)
        .query(&mut redis.connection())
        .unwrap();
    assert_eq!(groups.groups.len(), 2);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC6
#[test]
fn an_embedded_wildcard_cannot_be_published_as_a_subscription() {
    let computer = Computer::new();
    computer.project.write(
        "docs/domain/reactions/procedure/respond.md",
        &documented::snippet(
            "procedure",
            "Respond",
            "Select facts.",
            "role: responder\nsubscription_home: project\nsubscriptions: ['erp.*.paid']\n",
            &[],
            "Respond.\n",
        ),
    );
    computer
        .project
        .commit("docs: declare an invalid subscription");
    let rejected = computer
        .env
        .run(computer.project.path(), ["sync"])
        .failure();
    let validation = computer
        .env
        .run(computer.project.path(), ["docs", "validate"])
        .failure();
    let text = format!(
        "{}{}{}",
        String::from_utf8_lossy(&validation.get_output().stdout),
        String::from_utf8_lossy(&validation.get_output().stderr),
        String::from_utf8_lossy(&rejected.get_output().stderr)
    )
    .replace(&computer.project.path().display().to_string(), "[PROJECT]")
    .replace(&computer.project.head(), "[SHA]");
    insta::assert_snapshot!(text);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC6
#[test]
fn an_empty_type_segment_cannot_be_published_as_a_subscription() {
    let computer = Computer::new();
    computer.project.write(
        "docs/domain/reactions/procedure/respond.md",
        &documented::snippet(
            "procedure",
            "Respond",
            "Select facts.",
            "role: responder\nsubscription_home: project\nsubscriptions: ['erp..paid']\n",
            &[],
            "Respond.\n",
        ),
    );
    computer
        .project
        .commit("docs: declare an invalid subscription");
    let rejected = computer
        .env
        .run(computer.project.path(), ["sync"])
        .failure();
    let validation = computer
        .env
        .run(computer.project.path(), ["docs", "validate"])
        .failure();
    let text = format!(
        "{}{}{}",
        String::from_utf8_lossy(&validation.get_output().stdout),
        String::from_utf8_lossy(&validation.get_output().stderr),
        String::from_utf8_lossy(&rejected.get_output().stderr)
    )
    .replace(&computer.project.path().display().to_string(), "[PROJECT]")
    .replace(&computer.project.head(), "[SHA]");
    insta::assert_snapshot!(text);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn an_incomplete_index_retries_then_parks_the_event() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    computer.config(&redis, Some(&local));
    computer.subscribe("erp.invoice.*");
    computer.tick();
    std::fs::write(
        computer.env.instrukt_home().join("state/index.yaml"),
        "schema_version: 2\n",
    )
    .unwrap();
    append(&redis, &event(&computer, "paid", "erp.invoice.paid"));
    computer.tick();
    assert_eq!(pending(&redis, &computer), 1);
    assert_eq!(computer.accepted(), 0);
    for _ in 1..MAX_HANDLING_ATTEMPTS {
        computer.tick();
    }
    assert_eq!(pending(&redis, &computer), 0);
    let parked: String = redis::cmd("XRANGE")
        .arg(DEAD_LETTER_STREAM)
        .arg("-")
        .arg("+")
        .query::<redis::streams::StreamRangeReply>(&mut redis.connection())
        .unwrap()
        .ids[0]
        .get("record")
        .unwrap();
    assert!(parked.contains("missing field `snippets`"), "{parked}");
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn a_hung_announcement_does_not_delay_cluster_ack_or_later_work() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.subscribe("erp.invoice.*");
    computer.tick();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("malformed")
        .query::<String>(&mut cluster.connection())
        .unwrap();
    append(&cluster, &event(&computer, "later", "erp.invoice.paid"));
    for _ in 1..MAX_HANDLING_ATTEMPTS {
        computer.tick();
    }
    redis::cmd("CLIENT")
        .arg("PAUSE")
        .arg(30_000)
        .arg("ALL")
        .query::<()>(&mut local.connection())
        .unwrap();
    let _running = support::Running::start(&computer);
    common::wait_until(
        "the later cluster event is accepted while local Redis is paused",
        std::time::Duration::from_secs(12),
        || computer.accepted() == 1 && pending(&cluster, &computer) == 0,
    );
    let parked: bool = computer.db().query_row(
        "SELECT EXISTS(SELECT 1 FROM consumer_outcomes WHERE json_extract(outcome, '$.parked.announced') = 0)",
        [], |row| row.get(0)
    ).unwrap();
    assert!(parked);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn a_suppressed_recursive_announcement_is_counted_as_unannounced() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    std::fs::write(
        computer.env.instrukt_home().join("state/index.yaml"),
        "schema_version: 99\nsnippets: []\n",
    )
    .unwrap();
    append(
        &redis,
        &event(
            &computer,
            "failed-announcement",
            "instrukt.system.event.dead_lettered",
        ),
    );
    for _ in 0..MAX_HANDLING_ATTEMPTS {
        computer.tick();
    }
    assert!(matches!(
        last_outcome(&computer),
        SettledOutcome::Parked {
            announced: false,
            ..
        }
    ));
    assert_eq!(computer.consumers_status()["unannounced_dead_letters"], 1);
}

fn park_with_lost_announcement_reply() -> (
    Computer,
    RedisServer,
    RedisServer,
    common::redis_proxy::DeliveryGate,
    String,
) {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    let gate = common::redis_proxy::DeliveryGate::announcements(&local);
    computer.config_urls(&cluster.url, Some(&gate.url));
    computer.subscribe("erp.invoice.*");
    computer.tick();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("malformed")
        .query::<String>(&mut cluster.connection())
        .unwrap();
    for _ in 1..MAX_HANDLING_ATTEMPTS {
        computer.tick();
    }
    gate.arm();
    let mut running = support::Running::start(&computer);
    support::until(|| gate.intercepted());
    running.kill();
    assert_eq!(pending(&cluster, &computer), 0);
    assert_eq!(computer.consumers_status()["unannounced_dead_letters"], 1);
    let mut connection = local.connection();
    let receipts: Vec<String> = redis::cmd("KEYS")
        .arg("instrukt:events:parking:*:announced")
        .query(&mut connection)
        .unwrap();
    assert_eq!(receipts.len(), 1, "an open intent still needs its receipt");
    let receipt = receipts.into_iter().next().unwrap();
    gate.release();
    (computer, cluster, local, gate, receipt)
}

fn announcement_receipt_ttl(connection: &mut redis::Connection, receipt: &str) -> i64 {
    redis::cmd("TTL").arg(receipt).query(connection).unwrap()
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn announcement_receipts_expire_refresh_on_replay_and_survive_endpoint_removal() {
    let (computer, _cluster, local, _gate, receipt) = park_with_lost_announcement_reply();
    let mut connection = local.connection();
    let ttl = announcement_receipt_ttl(&mut connection, &receipt);
    assert!((604_790..=604_800).contains(&ttl), "receipt TTL: {ttl}");

    // Model operator-owned Redis. SET with expiry and early deletion are allowed.
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("-expire")
        .query::<()>(&mut connection)
        .unwrap();
    assert!(
        redis::cmd("EXPIRE")
            .arg(&receipt)
            .arg(60)
            .query::<bool>(&mut connection)
            .is_err()
    );
    computer.tick();
    assert_eq!(computer.consumers_status()["unannounced_dead_letters"], 0);
    assert_eq!(announcement_receipt_ttl(&mut connection, &receipt), -2);
    assert_eq!(
        redis::cmd("XLEN")
            .arg(EVENT_STREAM)
            .query::<u64>(&mut connection)
            .unwrap(),
        1
    );

    // A separate lost reply lets us inspect the refreshed TTL after confirmation.
    let (computer, _cluster, local, _gate, receipt) = park_with_lost_announcement_reply();
    let mut connection = local.connection();
    assert!(
        redis::cmd("EXPIRE")
            .arg(&receipt)
            .arg(60)
            .query::<bool>(&mut connection)
            .unwrap()
    );
    // Keep the receipt inspectable after confirmation. DEL is best effort.
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("default")
        .arg("-del")
        .query::<()>(&mut connection)
        .unwrap();
    computer.tick();
    assert_eq!(computer.consumers_status()["unannounced_dead_letters"], 0);
    let ttl = announcement_receipt_ttl(&mut connection, &receipt);
    assert!((604_790..=604_800).contains(&ttl), "refreshed TTL: {ttl}");
    let entries: u64 = redis::cmd("XLEN")
        .arg(EVENT_STREAM)
        .query(&mut connection)
        .unwrap();
    assert_eq!(entries, 1);
    let retained_address: bool = computer.db().query_row(
        "SELECT EXISTS(SELECT 1 FROM consumer_outcomes WHERE json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL)",
        [], |row| row.get(0),
    ).unwrap();
    assert!(!retained_address);

    // Removing another destination before confirmation leaves an expiring receipt.
    let (computer, cluster, local, _gate, receipt) = park_with_lost_announcement_reply();
    computer.config(&cluster, None);
    computer.tick();
    assert_eq!(computer.consumers_status()["abandoned_announcements"], 1);
    let ttl = announcement_receipt_ttl(&mut local.connection(), &receipt);
    assert!(
        (604_760..=604_800).contains(&ttl),
        "orphaned receipt TTL: {ttl}"
    );
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn a_null_snippet_list_leaves_a_matching_event_pending_for_retry() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    std::fs::write(
        computer.env.instrukt_home().join("state/index.yaml"),
        "schema_version: 2\nsnippets:\n",
    )
    .unwrap();
    append(&redis, &event(&computer, "paid", "erp.invoice.paid"));
    computer.tick();
    assert_eq!(pending(&redis, &computer), 1);
    assert!(
        computer.consumers_status()["subscriptions"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("snippets must be a YAML sequence"))
    );
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC13
#[test]
fn docs_get_reports_why_a_requested_published_entry_was_dropped() {
    let computer = Computer::new();
    computer.subscribe("erp.invoice.*");
    let path = computer.env.instrukt_home().join("state/index.yaml");
    let mut index: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let id = "reactions/procedure/respond";
    let entry = index["snippets"]
        .as_sequence_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["id"].as_str() == Some(id))
        .unwrap();
    entry.as_mapping_mut().unwrap().remove("description");
    std::fs::write(&path, serde_yaml_ng::to_string(&index).unwrap()).unwrap();

    let shown = computer
        .env
        .run(
            computer.project.path(),
            ["docs", "get", id, "reactions/procedure/unknown"],
        )
        .failure();
    let error = String::from_utf8_lossy(&shown.get_output().stderr);
    assert!(
        error.contains("unknown snippet id: reactions/procedure/unknown"),
        "{error}"
    );
    assert!(
        error.contains(id) && error.contains("missing field `description`"),
        "{error}"
    );
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn removed_endpoints_retire_open_announcements_in_oldest_first_batches() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.subscribe("erp.invoice.*");
    computer.tick();
    let connection = computer.db();
    let payload = event(
        &computer,
        "announcement",
        "instrukt.system.event.dead_lettered",
    )
    .encode()
    .unwrap();
    let outcome = json!({"parked":{
        "destination": DEAD_LETTER_STREAM, "entry_id": "1-0", "announced": false,
        "announcement_source": "removed-redis", "announcement_receipt": "instrukt:events:old",
        "announcement_payload": payload,
    }})
    .to_string();
    for entry in 1..=33 {
        connection.execute(
            "INSERT INTO consumer_outcomes VALUES ('procedure-activation', 'origin', ?1, ?2, ?3, '2026-11-05T00:00:00Z')",
            (EVENT_STREAM, format!("{entry}-0"), &outcome),
        ).unwrap();
    }
    computer.tick();
    let open: String = connection.query_row(
        "SELECT group_concat(entry_id) FROM consumer_outcomes WHERE json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL",
        [], |row| row.get(0),
    ).unwrap_or_default();
    assert_eq!(open, "33-0");
    assert_eq!(computer.consumers_status()["abandoned_announcements"], 32);
    computer.tick();
    let open: u32 = connection.query_row(
        "SELECT count(*) FROM consumer_outcomes WHERE json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(open, 0);
    assert_eq!(computer.consumers_status()["abandoned_announcements"], 33);
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn a_full_announcement_journal_leaves_new_parking_settled_without_an_intent() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.subscribe("erp.invoice.*");
    computer.tick();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("malformed")
        .query::<String>(&mut cluster.connection())
        .unwrap();
    for _ in 1..MAX_HANDLING_ATTEMPTS {
        computer.tick();
    }
    redis::cmd("SET")
        .arg(EVENT_STREAM)
        .arg("wrong-type")
        .query::<()>(&mut local.connection())
        .unwrap();
    let socket = std::path::Path::new(local.url.strip_prefix("unix://").unwrap());
    let socket = socket
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(socket.file_name().unwrap());
    let endpoint =
        instrukt_runner::events::occurrence_id(&[&format!("unix:{}", socket.display()), "0"]);
    let outcome = json!({"parked":{
        "destination": DEAD_LETTER_STREAM, "entry_id": "1-0", "announced": false,
        "announcement_source": endpoint, "announcement_receipt": "instrukt:events:pending",
        "announcement_payload": "{}",
    }})
    .to_string();
    let connection = computer.db();
    let plan: String = connection
        .query_row(
            "EXPLAIN QUERY PLAN SELECT count(*) FROM consumer_outcomes
         WHERE json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL
         AND json_extract(outcome, '$.parked.announced') = 0",
            [],
            |row| row.get(3),
        )
        .unwrap();
    assert!(
        plan.contains("USING INDEX consumer_open_announcements"),
        "{plan}"
    );

    connection.execute(
        "WITH RECURSIVE entries(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM entries WHERE n < 1024)
         INSERT INTO consumer_outcomes SELECT 'procedure-activation', 'older', ?1, n || '-0', ?2, '2026-11-05T00:00:00Z' FROM entries",
        (EVENT_STREAM, outcome),
    ).unwrap();
    computer.tick();
    assert_eq!(pending(&cluster, &computer), 0);
    let open: u32 = connection.query_row(
        "SELECT count(*) FROM consumer_outcomes WHERE json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(open, 1024);
    assert_eq!(
        computer.consumers_status()["unannounced_dead_letters"],
        1025
    );
}

// spec: project/instrukt/spec/feature/runner/event-consumers UC-EC8
#[test]
fn scheduled_ticks_resume_after_a_blocked_announcement_page_and_wrap() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let mut local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.subscribe("erp.invoice.*");
    computer.tick();
    let socket = std::path::Path::new(local.url.strip_prefix("unix://").unwrap());
    let socket = socket
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(socket.file_name().unwrap());
    let offline =
        instrukt_runner::events::occurrence_id(&[&format!("unix:{}", socket.display()), "0"]);
    local.stop();

    let db = computer.db();
    for entry in 1..=33 {
        let source = if entry == 33 {
            "removed-redis"
        } else {
            &offline
        };
        let payload = event(
            &computer,
            &format!("announcement-{entry}"),
            "instrukt.system.event.dead_lettered",
        )
        .encode()
        .unwrap();
        let outcome = json!({"parked":{
            "destination": DEAD_LETTER_STREAM, "entry_id": "1-0", "announced": false,
            "announcement_source": source, "announcement_receipt": format!("instrukt:events:pending:{entry}"),
            "announcement_payload": payload,
        }}).to_string();
        db.execute(
            "INSERT INTO consumer_outcomes VALUES ('procedure-activation', 'origin', ?1, ?2, ?3, '2026-11-05T00:00:00Z')",
            (EVENT_STREAM, format!("{entry}-0"), outcome),
        ).unwrap();
    }
    let counts = || {
        db.query_row(
            "SELECT count(*) FILTER (WHERE json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL),
                    count(*) FILTER (WHERE json_extract(outcome, '$.parked.announcement_abandoned') = 1)
             FROM consumer_outcomes",
            [], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)),
        ).unwrap()
    };
    computer.tick();
    assert_eq!(counts(), (33, 0), "only the blocked first page was tried");
    computer.tick();
    assert_eq!(
        counts(),
        (32, 1),
        "the next tick reaches the removed endpoint"
    );
    local.restart();
    computer.tick();
    assert_eq!(
        counts(),
        (0, 1),
        "recovery wraps to the now-reachable first page"
    );
}
