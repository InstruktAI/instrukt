//! Retained group progress, selection and parking through the real CLI and Redis.
#![allow(clippy::unwrap_used, reason = "test: failed steps fail the test")]
mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/event_transport.rs"]
mod support;

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
fn a_failed_announcement_resumes_without_persisting_a_second_dead_letter() {
    let computer = Computer::new();
    let cluster = RedisServer::new(&computer.env);
    let local = RedisServer::new(&computer.env);
    computer.config(&cluster, Some(&local));
    computer.tick();
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg("malformed")
        .query::<String>(&mut cluster.connection())
        .unwrap();
    computer.tick();
    computer.tick();
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
    assert_eq!((letters, pending(&cluster, &computer)), (1, 1));
    computer.tick();
    redis::cmd("DEL")
        .arg(EVENT_STREAM)
        .query::<u64>(&mut local.connection())
        .unwrap();
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
        (1, 2, 0, 0)
    );
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
