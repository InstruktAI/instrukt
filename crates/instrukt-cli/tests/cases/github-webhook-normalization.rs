//! Provider normalization follows verified HTTP intake and typed relay provenance.
#![allow(clippy::unwrap_used, reason = "test setup and assertions")]
use crate::common;
use crate::fixtures::event_transport as support;
use crate::fixtures::webhooks as hooks;
use common::redis::RedisServer;
use hooks::{GITHUB, Receiver, SECRET, declare, events};
use instrukt_runner::webhooks::{GithubView, github_view};
use support::Computer;

fn receive(name: Option<&str>, payload: serde_json::Value, pattern: &str) -> GithubView {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    declare(
        &computer,
        "{github: {verify: github, secret: '${HOOK_KEY}'}}",
    );
    let receiver = Receiver::start(&computer, &redis);
    computer.subscribe(pattern);
    let headers = name
        .map(|name| vec![("x-GITHUB-event", name)])
        .unwrap_or_default();
    assert_eq!(
        receiver
            .signed(
                GITHUB,
                &serde_json::to_vec(&payload).unwrap(),
                SECRET,
                &headers
            )
            .0,
        202
    );
    let view = github_view(&events(&redis)[0]).unwrap();
    computer.tick();
    assert_eq!(
        computer.accepted(),
        1,
        "the normalized vocabulary must select the published responsibility"
    );
    assert_eq!(view.payload, payload.as_object().unwrap().clone());
    view
}

// spec: project/instrukt/spec/feature/runner/github-webhook-normalization UC-GH1
#[test]
fn push_extracts_repository_sender_and_ref_from_the_verified_document() {
    let payload = serde_json::json!({"repository":{"full_name":"owner/repo"},"sender":{"login":"sender"},"ref":"refs/heads/main"});
    let view = receive(Some("push"), payload.clone(), "github.push");
    assert_eq!(
        view.properties.get("repo"),
        payload["repository"].get("full_name")
    );
    assert_eq!(
        view.properties.get("sender"),
        payload["sender"].get("login")
    );
    assert_eq!(view.properties.get("ref"), payload.get("ref"));
    assert!(view.properties["action"].is_null());
}

// spec: project/instrukt/spec/feature/runner/github-webhook-normalization UC-GH2
#[test]
fn release_extracts_action_with_explicit_nullable_ref() {
    let payload = serde_json::json!({"repository":{"full_name":"owner/repo"},"sender":{"login":"sender"},"action":"published"});
    let view = receive(Some("release"), payload.clone(), "github.release");
    assert_eq!(view.properties.get("action"), payload.get("action"));
    assert!(view.properties["ref"].is_null());
}

// spec: project/instrukt/spec/feature/runner/github-webhook-normalization UC-GH3
#[test]
fn ping_preserves_registration_metadata_without_a_builtin_effect() {
    let payload = serde_json::json!({"zen":"Keep it logically awesome.","hook_id":123});
    let view = receive(Some("ping"), payload.clone(), "github.ping");
    assert_eq!(view.properties.get("zen"), payload.get("zen"));
    assert_eq!(view.properties.get("hook_id"), payload.get("hook_id"));
}

// spec: project/instrukt/spec/feature/runner/github-webhook-normalization UC-GH4
#[test]
fn missing_event_header_is_unknown_even_when_the_body_looks_like_a_push() {
    receive(
        None,
        serde_json::json!({"ref":"refs/heads/main"}),
        "github.unknown",
    );
}

// spec: project/instrukt/spec/feature/runner/github-webhook-normalization UC-GH4
#[test]
fn malformed_event_header_is_unknown_rather_than_guessed_from_the_body() {
    receive(
        Some("push invalid"),
        serde_json::json!({"ref":"refs/heads/main"}),
        "github.unknown",
    );
}

// spec: project/instrukt/spec/feature/runner/github-webhook-normalization UC-GH5
#[test]
fn mixed_header_and_event_casing_select_the_lowercase_vocabulary() {
    receive(Some("PuSh"), serde_json::json!({}), "github.push");
}

// spec: project/instrukt/spec/feature/runner/github-webhook-normalization UC-GH6
#[test]
fn malformed_or_missing_properties_stay_present_as_nulls() {
    let view = receive(
        Some("push"),
        serde_json::json!({"repository":[],"sender":{"login":42},"action":false,"ref":{}}),
        "github.push",
    );
    assert_eq!(view.properties.len(), 4);
    assert!(view.properties.values().all(serde_json::Value::is_null));
}

// spec: project/instrukt/spec/feature/runner/github-webhook-normalization UC-GH8
#[test]
fn the_provider_view_keeps_the_original_relay_timestamp_and_document() {
    let mut computer = Computer::new();
    computer.env.set_env("INSTRUKT_NOW", "2026-11-05T03:04:05Z");
    let redis = RedisServer::new(&computer.env);
    declare(
        &computer,
        "{github: {verify: github, secret: '${HOOK_KEY}'}}",
    );
    let receiver = Receiver::start(&computer, &redis);
    assert_eq!(
        receiver
            .signed(GITHUB, b"{}", SECRET, &[("X-GitHub-Event", "push")])
            .0,
        202
    );
    let event = events(&redis).remove(0);
    let view = github_view(&event).unwrap();
    assert_eq!(
        view.source,
        instrukt_core::config::WebhookProtocol::Github.as_str()
    );
    assert_eq!(format!("{}.{}", view.source, view.r#type), event.event_type);
    assert_eq!(view.timestamp, event.time);
    assert_eq!(view.payload, event.data);
    assert!(matches!(
        event.home,
        instrukt_runner::events::EventHome::Project(_)
    ));
    assert_eq!(event.source, computer.computer());
}

// spec: project/instrukt/spec/feature/runner/github-webhook-normalization UC-GH8
#[test]
fn github_verification_uses_the_declared_domain_instead_of_a_protocol_path() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    declare(&computer, "{code: {verify: github, secret: '${HOOK_KEY}'}}");
    let receiver = Receiver::start(&computer, &redis);
    computer.subscribe("code.push");
    let signature = hooks::signature(b"{}", SECRET);
    assert_eq!(
        receiver
            .request(
                "POST",
                "/hooks/inbound/project/fork/code",
                b"{}",
                &[
                    ("X-Hub-Signature-256", &signature),
                    ("X-GitHub-Event", "push")
                ]
            )
            .0,
        202
    );
    let event = events(&redis).remove(0);
    assert!(github_view(&event).is_some());
    assert_eq!(
        event.subject,
        instrukt_runner::events::EventRef::new(
            instrukt_runner::events::ReferenceKind::InboundSource,
            "code:project:fork"
        )
        .unwrap()
    );
    computer.tick();
    assert_eq!(computer.accepted(), 1);
}
