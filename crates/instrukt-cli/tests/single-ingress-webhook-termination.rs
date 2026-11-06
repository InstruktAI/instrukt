//! Ingress provenance, cluster reach and source-scoped replay claims.
#![allow(clippy::unwrap_used, reason = "test setup and assertions")]
mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/webhooks.rs"]
mod hooks;
#[path = "support/event_transport.rs"]
mod support;
use common::redis::RedisServer;
use hooks::{BODY, GITHUB, NATIVE, Receiver, SECRET, declare, events};
use instrukt_runner::events::{EventHome, EventRef, ReferenceKind};
use sha2::{Digest, Sha256};
use support::Computer;

fn setup() -> (Computer, RedisServer, Receiver) {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    declare(
        &computer,
        "{erp: {secret: '${HOOK_KEY}'}, github: {verify: github, secret: '${HOOK_KEY}'}}",
    );
    let receiver = Receiver::start(&computer, &redis);
    (computer, redis, receiver)
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-1
// spec: project/instrukt/spec/feature/runner/event-transport UC-ET2
#[test]
fn a_second_computer_consumes_the_relay_from_cluster_without_a_listener() {
    let (ingress, redis, receiver) = setup();
    let local = RedisServer::new(&ingress.env);
    let global = ingress.env.instrukt_home().join("instrukt.yaml");
    let config = std::fs::read_to_string(&global).unwrap().replace(
        "redis:\n",
        &format!("redis:\n  local:\n    url: '{}'\n", local.url),
    );
    std::fs::write(global, config).unwrap();
    ingress.tick();
    let other = Computer::new();
    other.config(&redis, None);
    other.tick();
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 202);
    ingress.tick();
    other.tick();
    let received = |computer: &Computer| {
        computer
            .db()
            .query_row(
                "SELECT count(*) FROM events WHERE name = 'erp.invoice.paid'",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap()
    };
    assert_eq!((received(&ingress), received(&other)), (1, 1));
    assert_eq!(other.accepted(), 0);
    let local_entries: u64 = redis::cmd("XLEN")
        .arg(instrukt_runner::transport::EVENT_STREAM)
        .query(&mut local.connection())
        .unwrap();
    assert_eq!(
        local_entries, 0,
        "intake must bypass the configured local Redis"
    );
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-2
#[test]
fn intake_accepts_without_a_matching_procedure_or_runtime_execution() {
    let (computer, redis, receiver) = setup();
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 202);
    assert_eq!(events(&redis).len(), 1);
    assert_eq!(computer.accepted(), 0);
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-3
#[test]
fn invalid_signed_deliveries_never_reach_cluster() {
    let (_computer, redis, receiver) = setup();
    assert_eq!(receiver.signed(NATIVE, BODY, "wrong-key", &[]).0, 401);
    assert!(events(&redis).is_empty());
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-4
#[test]
fn a_non_ingress_computer_refuses_to_start_a_listener() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.tick();
    let global = computer.env.instrukt_home().join("instrukt.yaml");
    let config = std::fs::read_to_string(&global).unwrap()
        + "ingress: {computer: 'abcdef0123456789', port: 19099}\n";
    std::fs::write(global, config).unwrap();
    let result = computer
        .env
        .run(&computer.env.path(""), ["webhooks", "serve"])
        .failure();
    insta::assert_snapshot!(
        "wrong-ingress",
        String::from_utf8_lossy(&result.get_output().stderr)
    );
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-4
#[test]
fn changing_ingress_placement_withdraws_a_running_receiver() {
    let (computer, _redis, mut receiver) = setup();
    let global = computer.env.instrukt_home().join("instrukt.yaml");
    let config = std::fs::read_to_string(&global)
        .unwrap()
        .replace(&computer.computer(), "abcdef0123456789");
    std::fs::write(global, config).unwrap();
    support::until(|| receiver.exited());
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-5
#[test]
fn one_receiver_verifies_multiple_project_protocols_with_isolated_claims() {
    let (computer, redis, receiver) = setup();
    let other = documented::project(&computer.env, "other", &[]);
    documented::sync(&computer.env, &other);
    other.write("instrukt.yaml", "schema_version: 1\nproject: other\nwebhooks: {erp: {secret: '${HOOK_KEY}'}, github: {verify: github, secret: '${HOOK_KEY}'}}\n");
    other.write(".env", &format!("HOOK_KEY={SECRET}\n"));
    for route in [NATIVE, "/hooks/inbound/project/other/erp"] {
        assert_eq!(receiver.signed(route, BODY, SECRET, &[]).0, 202);
        assert_eq!(receiver.signed(route, BODY, SECRET, &[]).0, 202);
    }
    for route in [GITHUB, "/hooks/inbound/project/other/github"] {
        assert_eq!(
            receiver
                .signed(route, b"{}", SECRET, &[("X-GitHub-Delivery", "one-id")])
                .0,
            202
        );
        assert_eq!(
            receiver
                .signed(route, b"{}", SECRET, &[("X-GitHub-Delivery", "one-id")])
                .0,
            202
        );
    }
    assert_eq!(events(&redis).len(), 4);
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-6
#[test]
fn relayed_envelope_retains_ingress_origin_subject_time_and_data() {
    let (computer, redis, receiver) = setup();
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 202);
    let event = events(&redis).remove(0);
    assert_eq!(event.source, computer.computer());
    assert_eq!(
        event.home,
        EventHome::Project(instrukt_core::ids::ProjectSlug::parse("fork").unwrap())
    );
    assert_eq!(
        event.subject,
        EventRef::new(ReferenceKind::InboundSource, "erp:project:fork").unwrap()
    );
    assert!(!event.id.is_empty());
    assert!(event.time.timestamp() <= jiff::Timestamp::now());
    let wrapper: serde_json::Value = serde_json::from_slice(BODY).unwrap();
    assert_eq!(event.data, wrapper["payload"].as_object().unwrap().clone());
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-7
#[test]
fn an_unregistered_external_type_activates_through_the_retained_consumer_only() {
    let (computer, _redis, receiver) = setup();
    computer.subscribe("erp.invoice.*");
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 202);
    assert_eq!(computer.accepted(), 0);
    computer.tick();
    assert_eq!(computer.accepted(), 1);
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-8
#[test]
fn session_headers_supply_no_attribution_to_an_external_fact() {
    let (_computer, redis, receiver) = setup();
    assert_eq!(
        receiver
            .signed(
                NATIVE,
                BODY,
                SECRET,
                &[("X-Session-Id", "spoof"), ("X-Caller", "session:spoof")]
            )
            .0,
        202
    );
    let event = events(&redis).remove(0);
    assert!(event.related.is_empty());
    assert!(event.causationid.is_none());
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-10
#[test]
fn provider_looking_native_names_never_get_github_treatment() {
    let (computer, redis, receiver) = setup();
    declare(&computer, "{github: {secret: '${HOOK_KEY}'}}");
    assert_eq!(
        receiver
            .request(
                "POST",
                GITHUB,
                br#"{"name":"push","payload":{}}"#,
                &[(
                    "X-Hook-Signature",
                    &hooks::signature(br#"{"name":"push","payload":{}}"#, SECRET)
                )]
            )
            .0,
        202
    );
    let event = events(&redis).remove(0);
    computer.subscribe("github.push");
    computer.tick();
    assert_eq!(computer.accepted(), 1);
    assert!(instrukt_runner::webhooks::github_view(&event).is_none());
}

// spec: project/instrukt/spec/feature/runner/single-ingress-webhook-termination UC-SIT-11
#[test]
fn verification_schemes_keep_identical_facts_and_their_claims_distinct() {
    let (computer, redis, receiver) = setup();
    let body = br#"{"name":"push","payload":{}}"#;
    declare(&computer, "{github: {secret: '${HOOK_KEY}'}}");
    assert_eq!(
        receiver
            .request(
                "POST",
                GITHUB,
                body,
                &[("X-Hook-Signature", &hooks::signature(body, SECRET))]
            )
            .0,
        202
    );
    declare(
        &computer,
        "{github: {verify: github, secret: '${HOOK_KEY}'}}",
    );
    // Match the native digest identity so only the scheme separates the claims.
    let delivery = format!("{:x}", Sha256::digest(body));
    assert_eq!(
        receiver
            .signed(
                GITHUB,
                b"{}",
                SECRET,
                &[("X-GitHub-Event", "push"), ("X-GitHub-Delivery", &delivery)]
            )
            .0,
        202
    );
    let events = events(&redis);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].data, events[1].data);
    assert_eq!(events[0].event_type, events[1].event_type);
    assert_eq!(events[0].subject, events[1].subject);
    assert_ne!(events[0].extensions, events[1].extensions);
    assert_ne!(events[0].id, events[1].id);
}
