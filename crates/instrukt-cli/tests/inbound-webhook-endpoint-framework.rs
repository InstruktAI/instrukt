//! Native and GitHub intake through the actual receiver and cluster Redis.
#![allow(clippy::unwrap_used, reason = "test setup and assertions")]
mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/webhooks.rs"]
mod hooks;
#[path = "support/event_transport.rs"]
mod support;

use common::redis::RedisServer;
use hooks::{BODY, GITHUB, NATIVE, OLD, Receiver, SECRET, declare, events};
use support::Computer;

fn setup() -> (Computer, RedisServer, Receiver) {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    declare(
        &computer,
        "{native: {secret: '${HOOK_KEY}'}, github: {secret: '${HOOK_KEY}'}}",
    );
    let receiver = Receiver::start(&computer, &redis);
    (computer, redis, receiver)
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF5
#[test]
fn neither_protocol_echoes_a_handshake() {
    let (_computer, redis, receiver) = setup();
    for route in [NATIVE, GITHUB] {
        assert_eq!(receiver.request("GET", route, b"", &[]).0, 405);
    }
    assert!(events(&redis).is_empty());
}

fn rejects_native(bytes: &[u8]) {
    let (_computer, redis, receiver) = setup();
    assert_eq!(receiver.signed(NATIVE, bytes, SECRET, &[]).0, 400);
    assert!(events(&redis).is_empty());
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF8
#[test]
fn verified_malformed_json_is_a_client_error() {
    rejects_native(b"{");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF8
#[test]
fn verified_nonobject_wrappers_are_client_errors() {
    rejects_native(b"[]");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF8
#[test]
fn native_wrappers_admit_no_extra_fields() {
    rejects_native(br#"{"domain":"erp","name":"paid","payload":{},"extra":true}"#);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF8
#[test]
fn native_wrapper_fields_cannot_be_duplicated() {
    rejects_native(br#"{"domain":"erp","domain":"x","name":"paid","payload":{}}"#);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF17
#[test]
fn registration_and_the_source_are_both_required_for_a_route() {
    let (computer, redis, receiver) = setup();
    assert_eq!(
        receiver
            .signed("/hooks/inbound/project/absent", BODY, SECRET, &[])
            .0,
        404
    );
    declare(&computer, "{}");
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 404);
    assert_eq!(
        receiver
            .signed(
                "/hooks/inbound/project/fork?secret=value",
                BODY,
                SECRET,
                &[]
            )
            .0,
        404
    );
    assert!(events(&redis).is_empty());
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF18
#[test]
fn authentication_precedes_even_malformed_body_parsing() {
    let (_computer, redis, receiver) = setup();
    assert_eq!(receiver.request("POST", NATIVE, b"{", &[]).0, 401);
    let signature = hooks::signature(BODY, SECRET);
    assert_eq!(
        receiver
            .request("POST", NATIVE, b"{", &[("X-Hook-Signature", &signature)])
            .0,
        401
    );
    assert!(events(&redis).is_empty());
}

fn invalid_declaration(declaration: &str, snapshot: &str) {
    let (computer, _redis, _receiver) = setup();
    declare(&computer, declaration);
    let result = computer
        .env
        .run(computer.project.path(), ["config", "validate"])
        .failure();
    let stdout = String::from_utf8_lossy(&result.get_output().stdout)
        .replace(computer.project.path().to_str().unwrap(), "[PROJECT]");
    insta::assert_snapshot!(snapshot, stdout);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF19
#[test]
fn literal_signing_material_is_refused_without_echoing_it() {
    invalid_declaration(
        "{native: {secret: literal-signing-material}}",
        "literal-secret",
    );
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF19
#[test]
fn a_missing_required_reference_is_refused() {
    invalid_declaration("{native: {}}", "missing-reference");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF19
#[test]
fn interpolated_literal_material_is_not_an_exact_reference() {
    invalid_declaration(
        "{native: {secret: 'prefix${HOOK_KEY}'}}",
        "nonexact-reference",
    );
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF19
#[test]
fn sources_cannot_be_nested_under_a_direction_or_sources_key() {
    invalid_declaration(
        "{sources: {native: {secret: '${HOOK_KEY}'}}}",
        "unsupported-source-nesting",
    );
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF19
#[test]
fn unsupported_home_declarations_are_capability_findings() {
    invalid_declaration("{person: {secret: '${HOOK_KEY}'}}", "unsupported-home");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF20
#[test]
fn declarations_are_added_changed_and_removed_without_a_restart() {
    let (computer, redis, receiver) = setup();
    declare(&computer, "{}");
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 404);
    declare(&computer, "{native: {secret: '${HOOK_KEY}'}}");
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 202);
    declare(&computer, "{native: {secret: '${OLD_KEY}'}}");
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 401);
    assert_eq!(receiver.signed(NATIVE, BODY, OLD, &[]).0, 202);
    declare(&computer, "{}");
    assert_eq!(receiver.signed(NATIVE, BODY, OLD, &[]).0, 404);
    assert_eq!(events(&redis).len(), 1);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF21
#[test]
fn unknown_external_vocabulary_carries_opaque_payload_unchanged() {
    let (_computer, redis, receiver) = setup();
    let wrapper = serde_json::json!({"domain":"exotic", "name":"fact.observed", "payload":{"instructions":["opaque"],"null":null,"n":[1,2]}});
    assert_eq!(
        receiver
            .signed(NATIVE, &serde_json::to_vec(&wrapper).unwrap(), SECRET, &[])
            .0,
        202
    );
    assert_eq!(
        events(&redis)[0].data,
        wrapper["payload"].as_object().unwrap().clone()
    );
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF22
#[test]
fn native_domains_must_be_single_segments() {
    rejects_native(br#"{"domain":"er.p","name":"paid","payload":{}}"#);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF22
#[test]
fn native_names_cannot_contain_empty_segments() {
    rejects_native(br#"{"domain":"erp","name":"invoice..paid","payload":{}}"#);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF22
#[test]
fn native_segments_must_start_with_a_lowercase_letter() {
    rejects_native(br#"{"domain":"ERP","name":"paid","payload":{}}"#);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF22
#[test]
fn native_segments_admit_only_lowercase_digits_and_underscore() {
    rejects_native(br#"{"domain":"erp","name":"paid-1","payload":{}}"#);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF22
#[test]
fn native_names_cannot_be_empty() {
    rejects_native(br#"{"domain":"erp","name":"","payload":{}}"#);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF23
#[test]
fn append_failure_never_leaves_a_completed_claim_that_swallows_retry() {
    let (_computer, redis, receiver) = setup();
    redis::cmd("SET")
        .arg(instrukt_runner::transport::EVENT_STREAM)
        .arg("wrong-type")
        .query::<()>(&mut redis.connection())
        .unwrap();
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 502);
    redis::cmd("DEL")
        .arg(instrukt_runner::transport::EVENT_STREAM)
        .query::<()>(&mut redis.connection())
        .unwrap();
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 202);
    assert_eq!(events(&redis).len(), 1);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF23
#[test]
fn disconnected_cluster_is_a_transient_failure_not_local_acceptance() {
    let (computer, mut redis, receiver) = setup();
    redis.stop();
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 502);
    assert_eq!(computer.accepted(), 0);
    redis.restart();
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 202);
    assert_eq!(events(&redis).len(), 1);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF24
#[test]
fn missing_project_configuration_is_a_server_error() {
    let (computer, redis, receiver) = setup();
    std::fs::remove_file(computer.project.path().join("instrukt.yaml")).unwrap();
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 500);
    assert!(events(&redis).is_empty());
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF24
#[test]
fn unresolved_previous_secret_blocks_the_whole_declaration_safely() {
    let (computer, redis, receiver) = setup();
    declare(
        &computer,
        "{native: {secret: '${HOOK_KEY}', previous_secret: '${MISSING}'}}",
    );
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 500);
    let result = computer
        .env
        .run(computer.project.path(), ["config", "validate"])
        .failure();
    insta::assert_snapshot!(
        "missing-secret",
        String::from_utf8_lossy(&result.get_output().stdout)
            .replace(computer.project.path().to_str().unwrap(), "[PROJECT]")
    );
    assert!(events(&redis).is_empty());
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF25
#[test]
fn github_json_and_parameterized_form_have_identical_meaning() {
    let (_computer, redis, receiver) = setup();
    let json = br#"{"repository":{"full_name":"owner/repo"},"ref":"refs/heads/main"}"#;
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("payload", std::str::from_utf8(json).unwrap())
        .finish();
    assert_eq!(
        receiver
            .signed(
                GITHUB,
                json,
                SECRET,
                &[
                    ("Content-Type", "application/json"),
                    ("X-GitHub-Event", "push")
                ]
            )
            .0,
        202
    );
    assert_eq!(
        receiver
            .signed(
                GITHUB,
                form.as_bytes(),
                SECRET,
                &[
                    (
                        "Content-Type",
                        "application/x-www-form-urlencoded; charset=utf-8"
                    ),
                    ("X-GitHub-Event", "push")
                ]
            )
            .0,
        202
    );
    let events = events(&redis);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].data, events[1].data);
    assert_eq!(events[0].event_type, events[1].event_type);
}

fn rejects_form(body: &str) {
    let (_computer, redis, receiver) = setup();
    assert_eq!(
        receiver
            .signed(
                GITHUB,
                body.as_bytes(),
                SECRET,
                &[("Content-Type", "application/x-www-form-urlencoded")]
            )
            .0,
        400
    );
    assert!(events(&redis).is_empty());
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF26
#[test]
fn duplicate_github_form_fields_are_refused() {
    rejects_form("payload=%7B%7D&payload=%7B%7D");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF26
#[test]
fn extra_github_form_fields_are_refused() {
    rejects_form("payload=%7B%7D&extra=x");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF26
#[test]
fn missing_github_payload_field_is_refused() {
    rejects_form("x=y");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF26
#[test]
fn nonobject_github_form_payload_is_refused() {
    rejects_form("payload=%5B%5D");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF26
#[test]
fn invalid_github_percent_escapes_are_refused() {
    rejects_form("payload=%ZZ");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF26
#[test]
fn invalid_github_form_utf8_is_refused() {
    rejects_form("payload=%FF");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF26
#[test]
fn trailing_github_form_separators_are_refused() {
    rejects_form("payload=%7B%7D&");
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF27
#[test]
fn current_and_previous_verify_until_previous_is_removed() {
    let (computer, redis, receiver) = setup();
    declare(
        &computer,
        "{native: {secret: '${HOOK_KEY}', previous_secret: '${OLD_KEY}'}}",
    );
    let new = receiver.signed(NATIVE, BODY, SECRET, &[]);
    let old = receiver.signed(NATIVE, BODY, OLD, &[]);
    assert_eq!(new.0, 202);
    assert_eq!(old, new);
    declare(&computer, "{native: {secret: '${HOOK_KEY}'}}");
    assert_eq!(receiver.signed(NATIVE, BODY, OLD, &[]).0, 401);
    assert_eq!(events(&redis).len(), 1);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF28
#[test]
fn signing_values_are_project_scoped_and_config_reads_preserve_references() {
    let (mut computer, redis, receiver) = setup();
    computer.env.set_env("HOOK_KEY", "process-override");
    drop(receiver);
    let receiver = Receiver::start(&computer, &redis);
    let other = documented::project(&computer.env, "other", &[]);
    documented::sync(&computer.env, &other);
    other.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: other\nwebhooks: {native: {secret: '${HOOK_KEY}'}}\n",
    );
    other.write(".env", "HOOK_KEY=other-project-key\n");
    assert_eq!(
        receiver
            .signed("/hooks/inbound/project/other", BODY, SECRET, &[])
            .0,
        401
    );
    assert_eq!(
        receiver
            .signed(
                "/hooks/inbound/project/other",
                BODY,
                "other-project-key",
                &[]
            )
            .0,
        202
    );
    assert_eq!(
        receiver.signed(NATIVE, BODY, "other-project-key", &[]).0,
        401
    );
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 202);
    let result = computer
        .env
        .run(
            computer.project.path(),
            ["config", "get", "webhooks", "--json"],
        )
        .success();
    insta::assert_snapshot!(
        "source-references",
        String::from_utf8_lossy(&result.get_output().stdout)
    );
    std::fs::remove_file(computer.project.path().join(".env")).unwrap();
    assert_eq!(
        receiver.signed(NATIVE, BODY, "process-override", &[]).0,
        500
    );
    assert_eq!(events(&redis).len(), 2);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF28
#[test]
fn literal_dollar_signs_in_dotenv_material_never_expand_process_variables() {
    let (mut computer, redis, receiver) = setup();
    computer.env.set_env("EXPAND_ME", "process-secret");
    drop(receiver);
    let receiver = Receiver::start(&computer, &redis);
    computer.project.write(
        ".env",
        "# 'ignored comment\nHOOK_KEY=\"literal$EXPAND_ME\"\n",
    );
    assert_eq!(
        receiver.signed(NATIVE, BODY, "literal$EXPAND_ME", &[]).0,
        202
    );
    assert_eq!(
        receiver
            .signed(NATIVE, BODY, "literalprocess-secret", &[])
            .0,
        401
    );
    assert_eq!(events(&redis).len(), 1);
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF19
#[test]
fn a_declared_previous_reference_cannot_be_null() {
    invalid_declaration(
        "{native: {secret: '${HOOK_KEY}', previous_secret: null}}",
        "null-previous-reference",
    );
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF19
#[test]
fn source_declarations_with_events_off_are_a_finding_and_create_no_endpoint() {
    let (computer, redis, receiver) = setup();
    computer.project.write("instrukt.yaml", "schema_version: 1\nproject: fork\nmodules: {events: false}\nwebhooks: {native: {secret: '${HOOK_KEY}'}}\n");
    let result = computer
        .env
        .run(computer.project.path(), ["config", "validate"])
        .failure();
    insta::assert_snapshot!(
        "events-off-source",
        String::from_utf8_lossy(&result.get_output().stdout)
            .replace(computer.project.path().to_str().unwrap(), "[PROJECT]")
    );
    assert_eq!(receiver.signed(NATIVE, BODY, SECRET, &[]).0, 404);
    assert!(events(&redis).is_empty());
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF19
#[test]
fn an_invalid_source_patch_is_atomic() {
    let (computer, _redis, _receiver) = setup();
    let revision = computer
        .env
        .run(computer.project.path(), ["config", "get", "--revision"])
        .success();
    let revision = String::from_utf8_lossy(&revision.get_output().stdout)
        .trim()
        .to_owned();
    let before = std::fs::read(computer.project.path().join("instrukt.yaml")).unwrap();
    computer
        .env
        .run(
            computer.project.path(),
            [
                "config",
                "patch",
                "--revision",
                &revision,
                "--yaml",
                "webhooks: {native: {secret: literal-material}}\n",
            ],
        )
        .failure();
    assert_eq!(
        std::fs::read(computer.project.path().join("instrukt.yaml")).unwrap(),
        before
    );
}

// spec: project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework UC-IF28
#[test]
fn non_ingress_computers_validate_references_without_project_signing_values() {
    let (computer, _redis, receiver) = setup();
    drop(receiver);
    let global = computer.env.instrukt_home().join("instrukt.yaml");
    std::fs::write(
        &global,
        std::fs::read_to_string(&global)
            .unwrap()
            .replace(&computer.computer(), "abcdef0123456789"),
    )
    .unwrap();
    std::fs::remove_file(computer.project.path().join(".env")).unwrap();
    computer
        .env
        .run(computer.project.path(), ["config", "validate"])
        .success();
}
