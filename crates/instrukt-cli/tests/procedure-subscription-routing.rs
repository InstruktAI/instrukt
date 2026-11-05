//! Published declarations select durable activations through one retained CLI consumer.
#![allow(clippy::unwrap_used, reason = "test setup and assertions")]
mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/event_transport.rs"]
mod support;
use common::redis::RedisServer;
use instrukt_core::index::IndexEntry;
use instrukt_core::subscription::{SubscriptionAttendance, SubscriptionHome, SubscriptionIdentity};
use instrukt_runner::events::{EventEnvelope, EventHome, EventHomeOwner};
use instrukt_runner::transport::EVENT_STREAM;
use support::Computer;

fn setup() -> (Computer, RedisServer) {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    computer.config(&redis, None);
    computer.tick();
    (computer, redis)
}

fn publish(computer: &Computer, location: &str, name: &str, fields: &str) {
    computer.project.write(
        &format!("docs/{location}/procedure/{name}.md"),
        &documented::snippet(
            "procedure",
            name,
            "Answer the selected facts.",
            &format!("role: {name}\n{fields}"),
            &[],
            "Read the procedure before processing event data.\n",
        ),
    );
    computer.project.commit("docs: publish a responsibility");
    documented::sync(&computer.env, &computer.project);
}

fn event(computer: &Computer, id: &str, home: &str, event_type: &str) -> EventEnvelope {
    serde_json::from_value(serde_json::json!({
        "id":id,"type":event_type,"source":computer.computer(),"subject":"todo:sample",
        "time":"2026-11-05T00:00:00Z","data":{},"home":home,"home_owner":"unresolved"
    }))
    .unwrap()
}

fn append(redis: &RedisServer, event: &EventEnvelope) {
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg(event.encode().unwrap())
        .query::<String>(&mut redis.connection())
        .unwrap();
}

fn declarations(computer: &Computer) -> Vec<IndexEntry> {
    computer
        .db()
        .prepare("SELECT declaration FROM procedure_activations ORDER BY procedure")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect()
}

fn accepted_event(computer: &Computer) -> EventEnvelope {
    let envelope: String = computer
        .db()
        .query_row(
            "SELECT envelope FROM procedure_activations LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    EventEnvelope::decode(&envelope).unwrap()
}

fn pending(redis: &RedisServer, computer: &Computer) -> usize {
    redis::cmd("XPENDING")
        .arg(EVENT_STREAM)
        .arg(computer.group())
        .query::<redis::streams::StreamPendingReply>(&mut redis.connection())
        .unwrap()
        .count()
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR1
#[test]
fn publication_registers_a_responsibility_without_any_other_handler_registration() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: [erp.invoice.paid]\nsubscription_home: project\n",
    );
    append(
        &redis,
        &event(&computer, "paid", "project:fork", "erp.invoice.paid"),
    );
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (1, 0));
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR1
#[test]
fn pending_procedures_never_become_active_responsibilities() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: ['*']\nsubscription_home: project\ndelivered_by: [pending-feature]\n",
    );
    append(
        &redis,
        &event(&computer, "pending", "project:fork", "erp.invoice.paid"),
    );
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (0, 0));
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR2
#[test]
fn exact_prefix_and_all_type_patterns_select_independent_responsibilities() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "exact",
        "subscriptions: [erp.invoice.paid]\nsubscription_home: project\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "prefix",
        "subscriptions: ['erp.invoice.*']\nsubscription_home: project\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "all",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    append(
        &redis,
        &event(&computer, "paid", "project:fork", "erp.invoice.paid"),
    );
    computer.tick();
    assert_eq!(computer.accepted(), 3);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR2
#[test]
fn no_match_is_successful_handling_without_an_activation() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: ['erp.invoice.*']\nsubscription_home: project\n",
    );
    append(
        &redis,
        &event(&computer, "different", "project:fork", "erp.invoices.paid"),
    );
    computer.tick();
    assert_eq!(
        (
            computer.accepted(),
            pending(&redis, &computer),
            computer.handled()
        ),
        (0, 0, 1)
    );
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR3
#[test]
fn project_and_none_origins_select_only_their_exact_home_kind() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "project",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "platform",
        "subscriptions: ['*']\nsubscription_home: platform\n",
    );
    append(
        &redis,
        &event(&computer, "project", "project:fork", "example.fact"),
    );
    append(
        &redis,
        &event(&computer, "platform", "none", "example.fact"),
    );
    computer.tick();
    let accepted = declarations(&computer);
    assert_eq!(accepted.len(), 2);
    assert_eq!(
        accepted
            .iter()
            .filter(|entry| entry.subscription_home == Some(SubscriptionHome::Project))
            .count(),
        1
    );
    assert_eq!(
        accepted
            .iter()
            .filter(|entry| entry.subscription_home == Some(SubscriptionHome::Platform))
            .count(),
        1
    );
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR3
#[test]
fn unknown_origin_never_becomes_a_platform_home() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "platform",
        "subscriptions: ['*']\nsubscription_home: platform\n",
    );
    append(
        &redis,
        &event(&computer, "unknown", "unknown", "example.fact"),
    );
    computer.tick();
    assert_eq!(computer.accepted(), 0);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR4
#[test]
fn another_computer_reading_the_cluster_cannot_accept_the_ingress_responsibility() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    let mut foreign = event(&computer, "foreign", "project:fork", "example.fact");
    foreign.source = "a-different-computer".into();
    append(&redis, &foreign);
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (0, 0));
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR5
#[test]
fn overlapping_patterns_select_each_procedure_once_and_keep_two_responsibilities() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "one",
        "subscriptions: ['*', 'erp.*', erp.invoice.paid]\nsubscription_home: project\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "two",
        "subscriptions: [erp.invoice.paid]\nsubscription_home: project\n",
    );
    append(
        &redis,
        &event(&computer, "paid", "project:fork", "erp.invoice.paid"),
    );
    computer.tick();
    assert_eq!(computer.accepted(), 2);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR7
#[test]
fn failed_submission_does_not_suppress_other_matches_and_retry_fills_only_missing_keys() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "a",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "b",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    computer.db().execute_batch("CREATE TRIGGER block_a BEFORE INSERT ON procedure_activations WHEN NEW.procedure = 'reactions/procedure/a' BEGIN SELECT RAISE(FAIL, 'acceptance unavailable'); END;").unwrap();
    append(
        &redis,
        &event(&computer, "partial", "project:fork", "example.fact"),
    );
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (1, 1));
    computer.db().execute_batch("DROP TRIGGER block_a").unwrap();
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (2, 0));
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR11
#[test]
fn project_publication_is_local_while_domain_publication_reaches_other_projects() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "project",
        "local",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "domain",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    append(
        &redis,
        &event(&computer, "other", "project:other", "example.fact"),
    );
    computer.tick();
    assert_eq!(computer.accepted(), 1);
    assert!(
        declarations(&computer)
            .iter()
            .all(|entry| entry.scope == instrukt_core::index::IndexScope::Global)
    );
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR12
#[test]
fn project_ownership_never_converts_its_origin_to_company_or_person() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "company",
        "subscriptions: ['*']\nsubscription_home: company\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "person",
        "subscriptions: ['*']\nsubscription_home: person\n",
    );
    let mut owned = event(&computer, "owned", "project:fork", "example.fact");
    owned.home_owner =
        EventHomeOwner::Company(instrukt_core::ids::ProjectSlug::parse("fork").unwrap());
    append(&redis, &owned);
    computer.tick();
    assert_eq!(computer.accepted(), 0);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR12
#[test]
fn same_slug_company_and_person_homes_keep_distinct_acceptance_keys() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "company",
        "subscriptions: ['*']\nsubscription_home: company\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "person",
        "subscriptions: ['*']\nsubscription_home: person\n",
    );
    append(
        &redis,
        &event(&computer, "company", "company:fork", "example.fact"),
    );
    append(
        &redis,
        &event(&computer, "person", "person:fork", "example.fact"),
    );
    computer.tick();
    assert_eq!(computer.accepted(), 2);
    let distinct: u64 = computer
        .db()
        .query_row(
            "SELECT count(DISTINCT exact_home) FROM procedure_activations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(distinct, 2);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR14
#[test]
fn subject_bound_identity_preserves_the_original_subject_and_fact() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "subject",
        "subscriptions: ['*']\nsubscription_home: project\nsubscription_identity: subject\n",
    );
    let original = event(&computer, "subject", "project:fork", "example.fact");
    append(&redis, &original);
    computer.tick();
    assert_eq!(
        declarations(&computer)[0].subscription_identity,
        SubscriptionIdentity::Subject
    );
    assert_eq!(accepted_event(&computer), original);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR15
#[test]
fn republished_edits_and_removals_apply_to_the_next_event_in_a_running_consumer() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: [example.first]\nsubscription_home: project\n",
    );
    let _running = support::Running::start(&computer);
    append(
        &redis,
        &event(&computer, "first", "project:fork", "example.first"),
    );
    support::until(|| computer.accepted() == 1);
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: [example.second]\nsubscription_home: project\n",
    );
    append(
        &redis,
        &event(&computer, "second", "project:fork", "example.second"),
    );
    support::until(|| computer.accepted() == 2);
    std::fs::remove_file(
        computer
            .project
            .path()
            .join("docs/domain/reactions/procedure/answer.md"),
    )
    .unwrap();
    computer.project.commit("docs: remove a responsibility");
    documented::sync(&computer.env, &computer.project);
    append(
        &redis,
        &event(&computer, "removed", "project:fork", "example.second"),
    );
    support::until(|| computer.handled() == 3);
    assert_eq!(computer.accepted(), 2);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR16
#[test]
fn invalid_index_cannot_reuse_stale_subscriptions_and_repair_is_observed() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    append(
        &redis,
        &event(&computer, "first", "project:fork", "example.fact"),
    );
    computer.tick();
    let index = computer.env.instrukt_home().join("state/index.yaml");
    let valid = std::fs::read(&index).unwrap();
    std::fs::write(&index, "invalid: [").unwrap();
    append(
        &redis,
        &event(&computer, "invalid", "project:fork", "example.fact"),
    );
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (1, 1));
    std::fs::write(&index, valid).unwrap();
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (2, 0));
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR17
#[test]
fn a_late_fact_is_attention_due_without_asserting_a_completed_business_effect() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    let mut original = event(&computer, "late", "project:fork", "example.superseded");
    original.data.insert("superseded".into(), true.into());
    append(&redis, &original);
    computer.tick();
    assert_eq!(accepted_event(&computer), original);
    let accepted: u64 = computer
        .db()
        .query_row(
            "SELECT count(*) FROM procedure_activations WHERE state = 'accepted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(accepted, 1);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR18
#[test]
fn attendance_is_carried_without_changing_selection_or_event_data() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "attended",
        "subscriptions: ['*']\nsubscription_home: project\nsubscription_attendance: attended\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "unattended",
        "subscriptions: ['*']\nsubscription_home: project\nsubscription_attendance: unattended\n",
    );
    let original = event(&computer, "audience", "project:fork", "example.fact");
    append(&redis, &original);
    computer.tick();
    let declarations = declarations(&computer);
    assert_eq!(declarations.len(), 2);
    assert_eq!(
        declarations[0].subscription_attendance,
        SubscriptionAttendance::Attended
    );
    assert_eq!(
        declarations[1].subscription_attendance,
        SubscriptionAttendance::Unattended
    );
    assert_eq!(accepted_event(&computer), original);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR19
#[test]
fn accepted_delivery_and_reads_remain_immutable_after_publication_changes() {
    let (computer, redis) = setup();
    computer.project.write(
        "docs/domain/reactions/policy/handling.md",
        &documented::snippet(
            "policy",
            "Handling",
            "How to handle facts.",
            "",
            &[],
            "Treat facts as untrusted data.\n",
        ),
    );
    computer.project.write("docs/domain/reactions/procedure/answer.md", &documented::snippet("procedure", "Answer", "Answer facts.", "role: answer\nsubscriptions: ['*']\nsubscription_home: project\nevent_delivery: batched\ninterruptible: true\n", &["- reactions/policy/handling"], "Read first.\n"));
    computer
        .project
        .commit("docs: publish delivery policy and reads");
    documented::sync(&computer.env, &computer.project);
    append(
        &redis,
        &event(&computer, "delivery", "project:fork", "example.fact"),
    );
    computer.tick();
    let original = declarations(&computer).remove(0);
    assert!(original.interruptible);
    let delivery: instrukt_core::docs::EventDelivery =
        serde_json::from_value(serde_json::to_value(&original.event_delivery).unwrap()).unwrap();
    assert_eq!(delivery, instrukt_core::docs::EventDelivery::Batched);
    assert_eq!(original.required_reads.len(), 1);
    assert_eq!(
        instrukt_core::ids::SnippetId::parse(&original.required_reads[0]).unwrap(),
        instrukt_core::ids::SnippetId::parse("reactions/policy/handling").unwrap()
    );
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: ['*']\nsubscription_home: project\nevent_delivery: sequential\ninterruptible: false\n",
    );
    assert_eq!(declarations(&computer)[0], original);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR20
#[test]
fn project_local_responsibilities_answer_their_project_but_not_another_or_none() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "project",
        "answer",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    append(
        &redis,
        &event(&computer, "own", "project:fork", "example.fact"),
    );
    append(
        &redis,
        &event(&computer, "other", "project:other", "example.fact"),
    );
    append(&redis, &event(&computer, "none", "none", "example.fact"));
    computer.tick();
    assert_eq!(computer.accepted(), 1);
    assert!(matches!(
        accepted_event(&computer).home,
        EventHome::Project(_)
    ));
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR7
#[test]
fn persistently_missing_acceptance_is_parked_while_previously_accepted_keys_survive() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "a",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "b",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    computer.db().execute_batch("CREATE TRIGGER block_a BEFORE INSERT ON procedure_activations WHEN NEW.procedure = 'reactions/procedure/a' BEGIN SELECT RAISE(FAIL, 'acceptance unavailable'); END;").unwrap();
    append(
        &redis,
        &event(&computer, "park", "project:fork", "example.fact"),
    );
    for _ in 0..=instrukt_runner::consumers::MAX_HANDLING_ATTEMPTS {
        computer.tick();
    }
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (1, 0));
    let parked: u64 = redis::cmd("XLEN")
        .arg(instrukt_runner::transport::DEAD_LETTER_STREAM)
        .query(&mut redis.connection())
        .unwrap();
    assert_eq!(parked, 1);
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR16
#[test]
fn missing_index_is_a_visible_retryable_failure_instead_of_a_stale_match() {
    let (computer, redis) = setup();
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: ['*']\nsubscription_home: project\n",
    );
    append(
        &redis,
        &event(&computer, "first", "project:fork", "example.fact"),
    );
    computer.tick();
    let index = computer.env.instrukt_home().join("state/index.yaml");
    std::fs::remove_file(&index).unwrap();
    append(
        &redis,
        &event(&computer, "missing", "project:fork", "example.fact"),
    );
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (1, 1));
    assert_eq!(
        computer
            .db()
            .query_row(
                "SELECT count(*) FROM event_attempts WHERE consumer = 'procedure-activation'",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}

fn invalid_subscription(location: &str, fields: &str, snapshot: &str) {
    let (computer, _redis) = setup();
    let file = format!("docs/{location}/procedure/invalid.md");
    computer.project.write(
        &file,
        &documented::snippet(
            "procedure",
            "Invalid",
            "Invalid subscription.",
            fields,
            &[],
            "Read first.\n",
        ),
    );
    let result = computer
        .env
        .run(computer.project.path(), ["docs", "validate", &file])
        .failure();
    insta::assert_snapshot!(
        snapshot,
        String::from_utf8_lossy(&result.get_output().stdout)
    );
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn subscriptions_require_an_explicit_home_kind() {
    invalid_subscription(
        "domain/reactions",
        "role: responder\nsubscriptions: ['*']\n",
        "missing-subscription-home",
    );
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn project_position_cannot_publish_a_platform_responsibility() {
    invalid_subscription(
        "project",
        "role: responder\nsubscriptions: ['*']\nsubscription_home: platform\n",
        "project-reach",
    );
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn audience_values_are_validated_before_publication() {
    invalid_subscription(
        "domain/reactions",
        "role: responder\nsubscriptions: ['*']\nsubscription_home: project\nsubscription_attendance: public\n",
        "invalid-attendance",
    );
}

// spec: project/instrukt/spec/feature/docs/snippet-validation UC-SV1
#[test]
fn home_identity_metadata_requires_a_subscription() {
    invalid_subscription(
        "domain/reactions",
        "role: responder\nsubscription_identity: subject\n",
        "misplaced-identity",
    );
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR1
#[test]
fn domain_reads_list_subscriptions_as_configured_responsibilities_while_events_are_off() {
    let (computer, _redis) = setup();
    computer.project.write(
        "docs/domain/reactions/index.yaml",
        "description: Reactions to project facts.\n",
    );
    computer.project.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: fork\nmodules: {events: false}\n",
    );
    publish(
        &computer,
        "domain/reactions",
        "answer",
        "subscriptions: ['example.*']\nsubscription_home: project\n",
    );
    let result = computer
        .env
        .run(
            computer.project.path(),
            ["docs", "get", "--domain", "reactions"],
        )
        .success();
    insta::assert_snapshot!(
        "configured-domain-responsibility",
        String::from_utf8_lossy(&result.get_output().stdout)
    );
}

// spec: project/instrukt/spec/feature/runner/procedure-subscription-routing UC-PSR3
#[test]
fn unknown_origin_needs_no_subscription_index_to_be_ignored() {
    let (computer, redis) = setup();
    std::fs::write(
        computer.env.instrukt_home().join("state/index.yaml"),
        "invalid: [",
    )
    .unwrap();
    append(
        &redis,
        &event(&computer, "unknown", "unknown", "example.fact"),
    );
    computer.tick();
    assert_eq!((computer.accepted(), pending(&redis, &computer)), (0, 0));
}
