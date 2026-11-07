//! Pure producer, received codec and declaration conventions. CLI discovery is
//! bound separately at the real binary; no transport or runtime seam is faked.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test: failed construction fails the test"
)]

use std::collections::BTreeMap;

use instrukt_core::ids::{CommitSha, ProjectSlug, TodoSlug};
use serde_json::{Value, json};

use super::{
    CiResult, DataField, DataValueKind, DeadLettered, EmissionContext, EnvelopeError, Event,
    EventCatalog, EventDeclaration, EventEnvelope, EventHome, EventHomeOwner, EventRef, HomeKind,
    LifecycleState, ProducerEvent, ReferenceKind, StateDeclaration, StateStanding, occurrence_id,
};

fn context() -> EmissionContext {
    EmissionContext {
        source: "computer-one".into(),
        time: "2026-11-05T12:00:00Z".parse().unwrap(),
        project_owner: EventHomeOwner::Unresolved,
        caller_session: None,
    }
}

fn finalized() -> Event {
    Event::WorkflowFinalized {
        project: ProjectSlug::parse("demo").unwrap(),
        todo: TodoSlug::parse("ready").unwrap(),
        revision: CommitSha::parse(&"a".repeat(40)).unwrap(),
    }
}

fn received() -> Value {
    json!({
        "id": "external-occurrence", "type": "erp.invoice_paid", "source": "ingress",
        "subject": "inbound_source:native:project:demo", "time": "2026-11-05T12:00:00Z",
        "data": { "invoice": "opaque", "nested": { "value": [1, true, null] } },
        "home": "project:demo", "home_owner": "unresolved", "related": []
    })
}

fn read(value: Value) -> EventEnvelope {
    serde_json::from_value(value).unwrap()
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE1
#[test]
fn a_typed_occurrence_encodes_its_core_facts() {
    let event = finalized();
    let ctx = context();
    let encoded = event.envelope(&ctx).unwrap().encode().unwrap();
    let wire: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        wire.get("id").and_then(Value::as_str),
        Some(event.occurrence_id().as_str())
    );
    assert_eq!(wire.get("type").and_then(Value::as_str), Some(event.name()));
    assert_eq!(
        wire.get("source").and_then(Value::as_str),
        Some(ctx.source.as_str())
    );
    assert_eq!(read(wire).time.timestamp(), ctx.time);
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE2
#[test]
fn producer_names_keep_their_vocabulary_owner() {
    let event = finalized();
    assert_eq!(
        event.envelope(&context()).unwrap().event_type,
        "workflow.finalized"
    );
    assert_eq!(
        DeadLettered::declaration().name,
        "instrukt.system.event.dead_lettered"
    );
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE3
#[test]
fn a_todo_subject_is_distinct_from_its_project_origin() {
    let envelope = finalized().envelope(&context()).unwrap();
    assert_eq!(
        envelope.subject,
        EventRef::new(ReferenceKind::Todo, "ready").unwrap()
    );
    assert_eq!(
        envelope.home,
        EventHome::Project(ProjectSlug::parse("demo").unwrap())
    );
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE4
#[test]
fn declared_project_ownership_survives_the_wire() {
    let mut ctx = context();
    ctx.project_owner = EventHomeOwner::Company(ProjectSlug::parse("acme").unwrap());
    let event = finalized().envelope(&ctx).unwrap();
    assert_eq!(
        EventEnvelope::decode(&event.encode().unwrap())
            .unwrap()
            .home_owner,
        ctx.project_owner
    );
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE4
#[test]
fn ownerless_is_not_an_unresolved_lookup() {
    let mut ctx = context();
    ctx.project_owner = EventHomeOwner::Ownerless;
    let event = finalized().envelope(&ctx).unwrap();
    assert_eq!(
        EventEnvelope::decode(&event.encode().unwrap())
            .unwrap()
            .home_owner,
        EventHomeOwner::Ownerless
    );
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE4
#[test]
fn missing_project_owner_metadata_never_fabricates_ownerlessness() {
    let mut ctx = context();
    ctx.project_owner = EventHomeOwner::None;
    assert_eq!(
        finalized().envelope(&ctx).unwrap().home_owner,
        EventHomeOwner::Unresolved
    );
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE5
#[test]
fn unknown_origin_remains_distinct_from_none() {
    let mut input = received();
    input["home"] = json!("unknown");
    let envelope = read(input);
    assert_eq!(envelope.home, EventHome::Unknown);
    assert_ne!(envelope.home, EventHome::None);
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE6
#[test]
fn emitting_a_fact_adds_no_action_or_destination() {
    let envelope = finalized().envelope(&context()).unwrap();
    let wire = serde_json::to_value(envelope).unwrap();
    let known: BTreeMap<_, _> = wire
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.as_str(), value))
        .collect();
    assert!(!known.contains_key("action"));
    assert!(!known.contains_key("audience"));
    assert!(!known.contains_key("notification"));
    assert!(!known.contains_key("consumer_home"));
    assert!(!known.contains_key("scope"));
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE7
#[test]
fn context_attaches_the_actual_session_and_preserves_causation() {
    let event = finalized();
    let envelope = event.envelope(&context()).unwrap();
    let mut producer = ProducerEvent::new(
        event.declaration(),
        envelope.subject,
        envelope.home,
        envelope.data,
    );
    producer.causationid = Some("causing-occurrence".into());
    let cause = producer.causationid.clone();
    let mut ctx = context();
    ctx.caller_session = Some("t3:actual-session".into());
    let emitted = producer.emit(&ctx, || event.occurrence_id()).unwrap();
    assert_eq!(
        emitted.related,
        vec![
            EventRef::new(
                ReferenceKind::Session,
                ctx.caller_session.as_deref().unwrap()
            )
            .unwrap()
        ]
    );
    assert_eq!(emitted.causationid, cause);
    assert_ne!(Some(emitted.id), cause);
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE8
#[test]
fn emission_needs_no_consumer_or_home_resolver() {
    let mut input = received();
    input["home"] = json!("company:remote-company");
    let event = read(input);
    let mut declaration = finalized().declaration();
    declaration.name.clone_from(&event.event_type);
    declaration.subject_kind = None;
    declaration.home_kinds = vec![HomeKind::Company];
    declaration.data_fields = vec![
        DataField::of::<String>("invoice"),
        DataField::of::<serde_json::Map<String, Value>>("nested"),
    ];
    let producer = ProducerEvent::new(declaration, event.subject, event.home.clone(), event.data);
    assert_eq!(
        producer
            .emit(&context(), || "new-occurrence".into())
            .unwrap()
            .home,
        event.home
    );
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE9
#[test]
fn unfamiliar_homes_and_reference_kinds_round_trip_unchanged() {
    let mut input = received();
    input["home"] = json!("person:remote-person");
    input["home_owner"] = json!("none");
    input["subject"] = json!("future_kind:opaque:multi:colon");
    input["related"] = json!(["other_kind:other:identity"]);
    let event = read(input.clone());
    let output: Value = serde_json::from_str(&event.encode().unwrap()).unwrap();
    assert_eq!(output, input);
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE10
#[test]
fn catalog_and_typed_emission_share_the_same_declaration() {
    let event = finalized();
    let catalog = EventCatalog::builtin().unwrap();
    let declaration = catalog.get(event.name()).unwrap();
    assert_eq!(declaration, &event.declaration());
    declaration
        .validate(&event.envelope(&context()).unwrap())
        .unwrap();
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE11
#[test]
fn an_additive_data_field_survives_an_older_read() {
    let event = finalized();
    let mut envelope = event.envelope(&context()).unwrap();
    envelope
        .data
        .insert("new_optional_fact".into(), json!({"opaque": true}));
    let decoded = EventEnvelope::decode(&envelope.encode().unwrap()).unwrap();
    event.declaration().validate(&decoded).unwrap();
    assert_eq!(decoded.data, envelope.data);
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE12
#[test]
fn an_unfamiliar_optional_attribute_and_occurrence_id_survive_the_codec() {
    let mut input = received();
    input["future_attribute"] = json!({ "opaque": ["a", 3] });
    let event = read(input.clone());
    let output: Value = serde_json::from_str(&event.encode().unwrap()).unwrap();
    assert_eq!(output, input);
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE13
#[test]
fn occurrence_identity_is_independent_of_boundary_enrichment() {
    let event = finalized();
    let first = event.envelope(&context()).unwrap();
    let mut ctx = context();
    ctx.source = "computer-two".into();
    ctx.time = "2026-11-06T13:00:00Z".parse().unwrap();
    ctx.caller_session = Some("t3:another".into());
    assert_eq!(first.id, event.envelope(&ctx).unwrap().id);
    let mut other = event.clone();
    if let Event::WorkflowFinalized { revision, .. } = &mut other {
        *revision = CommitSha::parse(&"b".repeat(40)).unwrap();
    }
    assert_ne!(first.id, other.occurrence_id());
}

fn lifecycle(state: LifecycleState) -> EventDeclaration {
    let mut declaration = finalized().declaration();
    let state = StateDeclaration::new("instrukt.example.work", state);
    declaration.name = format!("{}.{}", state.family, state.state.as_str());
    declaration.state = Some(state);
    declaration
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE14
#[test]
fn platform_lifecycle_standing_is_carried_by_the_type() {
    let failing = lifecycle(LifecycleState::Failed);
    let recovered = lifecycle(LifecycleState::Recovered);
    let catalog = EventCatalog::new(vec![failing.clone(), recovered]).unwrap();
    let state = catalog.get(&failing.name).unwrap().state.as_ref().unwrap();
    assert_eq!(state.standing, StateStanding::Failing);
    assert_eq!(state.state, LifecycleState::Failed);
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE15
#[test]
fn a_failing_family_requires_an_ok_state() {
    let result = EventCatalog::new(vec![lifecycle(LifecycleState::Failed)]);
    assert!(result.is_err());
}

// spec: project/instrukt/spec/feature/runner/event-envelope UC-EE15
#[test]
fn failure_and_recovery_for_one_thing_keep_the_subject_and_origin() {
    let fact = finalized().envelope(&context()).unwrap();
    let failed = ProducerEvent::new(
        lifecycle(LifecycleState::Failed),
        fact.subject.clone(),
        fact.home.clone(),
        fact.data.clone(),
    )
    .emit(&context(), || "failure".into())
    .unwrap();
    let ok = ProducerEvent::new(
        lifecycle(LifecycleState::Recovered),
        fact.subject,
        fact.home,
        fact.data,
    )
    .emit(&context(), || "recovery".into())
    .unwrap();
    assert_eq!(failed.subject, ok.subject);
    assert_eq!(failed.home, ok.home);
}

#[test]
fn duplicate_declarations_are_refused() {
    let declaration = finalized().declaration();
    assert!(EventCatalog::new(vec![declaration.clone(), declaration]).is_err());
}

#[test]
fn a_stateless_platform_type_cannot_use_a_reserved_suffix() {
    let mut declaration = lifecycle(LifecycleState::Failed);
    declaration.state = None;
    assert!(EventCatalog::new(vec![declaration]).is_err());
}

#[test]
fn a_lifecycle_family_cannot_change_subject_kind() {
    let failing = lifecycle(LifecycleState::Failed);
    let mut ok = lifecycle(LifecycleState::Recovered);
    ok.subject_kind = Some(ReferenceKind::Commit);
    assert!(EventCatalog::new(vec![failing, ok]).is_err());
}

#[test]
fn a_state_cannot_disagree_with_its_type_name() {
    let mut failing = lifecycle(LifecycleState::Failed);
    failing.name = "instrukt.example.work.unrecognized".into();
    assert!(EventCatalog::new(vec![failing, lifecycle(LifecycleState::Recovered)]).is_err());
}

#[test]
fn a_domain_type_cannot_claim_a_platform_lifecycle_state() {
    let mut failing = lifecycle(LifecycleState::Failed);
    failing.name = "runner.example.failed".into();
    assert!(EventCatalog::new(vec![failing]).is_err());
}

#[test]
fn references_split_only_at_the_first_colon() {
    let reference = EventRef::parse("inbound_source:native:project:demo").unwrap();
    assert_eq!(reference.identity(), "native:project:demo");
    assert_eq!(reference.to_string(), "inbound_source:native:project:demo");
}

#[test]
fn empty_reference_identities_are_refused() {
    assert!(matches!(
        EventRef::parse("todo:"),
        Err(EnvelopeError::Reference(_))
    ));
}

#[test]
fn occurrence_key_components_cannot_alias_through_separators() {
    assert_ne!(occurrence_id(&["a", "b:c"]), occurrence_id(&["a:b", "c"]));
}

#[test]
fn producer_supplied_ids_are_preserved_without_generating_another() {
    let event = finalized();
    let envelope = event.envelope(&context()).unwrap();
    let mut producer = ProducerEvent::new(
        event.declaration(),
        envelope.subject,
        envelope.home,
        envelope.data,
    );
    producer.id = Some(envelope.id.clone());
    assert_eq!(
        producer
            .emit(&context(), || panic!("an ID already exists"))
            .unwrap()
            .id,
        envelope.id
    );
}

#[test]
fn a_producer_cannot_emit_a_subject_its_declaration_disallows() {
    let event = finalized();
    let mut envelope = event.envelope(&context()).unwrap();
    envelope.subject = EventRef::new(ReferenceKind::Commit, "demo/sha").unwrap();
    assert!(event.declaration().validate(&envelope).is_err());
}

#[test]
fn a_producer_cannot_emit_data_with_the_wrong_json_kind() {
    let event = finalized();
    let mut envelope = event.envelope(&context()).unwrap();
    envelope.data.insert("revision".into(), json!(42));
    assert!(event.declaration().validate(&envelope).is_err());
}

#[test]
fn an_optional_field_can_be_absent() {
    let event = finalized();
    let envelope = event.envelope(&context()).unwrap();
    let mut declaration = event.declaration();
    declaration
        .data_fields
        .push(DataField::of::<Option<String>>("optional"));
    declaration.validate(&envelope).unwrap();
}

#[test]
fn an_optional_field_can_be_null() {
    let event = finalized();
    let mut envelope = event.envelope(&context()).unwrap();
    let mut declaration = event.declaration();
    declaration
        .data_fields
        .push(DataField::of::<Option<String>>("optional"));
    envelope.data.insert("optional".into(), Value::Null);
    declaration.validate(&envelope).unwrap();
}

#[test]
fn a_received_event_cannot_have_an_empty_occurrence_id() {
    let mut input = received();
    input["id"] = json!(" ");
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
}

#[test]
fn a_received_event_requires_a_typed_subject() {
    let mut input = received();
    input.as_object_mut().unwrap().remove("subject");
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
}

#[test]
fn a_received_event_requires_an_rfc3339_timestamp() {
    let mut input = received();
    input["time"] = json!("yesterday");
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
}

#[test]
fn opaque_event_data_must_still_be_a_json_object() {
    let mut input = received();
    input["data"] = json!([1, 2]);
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
}

#[test]
fn the_no_ci_observation_is_a_typed_string_fact() {
    let event = Event::CiCompleted {
        project: ProjectSlug::parse("demo").unwrap(),
        candidate: CommitSha::parse(&"a".repeat(40)).unwrap(),
        result: CiResult::NoCi,
    };
    let envelope = event.envelope(&context()).unwrap();
    assert_eq!(
        serde_json::from_value::<CiResult>(envelope.data["result"].clone()).unwrap(),
        CiResult::NoCi
    );
    assert_eq!(
        event.declaration().data_fields.last().unwrap().kind,
        DataValueKind::String
    );
}

#[test]
fn receiving_an_offset_timestamp_preserves_its_offset_and_precision() {
    let mut input = received();
    input["time"] = json!("2026-11-05T14:00:00.000+02:00");
    let envelope = read(input.clone());
    assert_eq!(envelope.time.timestamp(), context().time);
    let output: Value = serde_json::from_str(&envelope.encode().unwrap()).unwrap();
    assert_eq!(output, input);
}

#[test]
fn upstream_state_subjects_name_the_watched_ref() {
    let project = ProjectSlug::parse("demo").unwrap();
    let revision = CommitSha::parse(&"c".repeat(40)).unwrap();
    let event = Event::UpstreamChanged {
        project: project.clone(),
        remote: "upstream".into(),
        r#ref: "refs/heads/main".into(),
        revision: revision.clone(),
    };
    let envelope = event.envelope(&context()).unwrap();
    assert_eq!(
        envelope.subject,
        EventRef::new(
            ReferenceKind::InboundSource,
            &format!("{project}/upstream/refs/heads/main")
        )
        .unwrap()
    );
    assert_eq!(envelope.id, event.occurrence_id());
}

#[test]
fn main_failing_state_subjects_name_the_canonical_ref() {
    let project = ProjectSlug::parse("demo").unwrap();
    let head = CommitSha::parse(&"c".repeat(40)).unwrap();
    let event = Event::MainFailing {
        project: project.clone(),
        head: head.clone(),
    };
    assert_eq!(
        event.envelope(&context()).unwrap().subject,
        EventRef::new(ReferenceKind::InboundSource, &format!("{project}/main")).unwrap()
    );
}

#[test]
fn ci_results_are_distinct_source_occurrences() {
    let project = ProjectSlug::parse("demo").unwrap();
    let candidate = CommitSha::parse(&"c".repeat(40)).unwrap();
    let event = Event::CiCompleted {
        project: project.clone(),
        candidate: candidate.clone(),
        result: CiResult::Success,
    };
    let other = Event::CiCompleted {
        project,
        candidate,
        result: CiResult::Failure,
    };
    assert_ne!(event.occurrence_id(), other.occurrence_id());
}

#[test]
fn a_dead_letter_names_its_stream_entry_without_a_registered_home() {
    let data = DeadLettered {
        attempt_count: "3".into(),
        consumer: "responsibility".into(),
        consumer_id: "computer".into(),
        dlq_destination: "dlq".into(),
        failing_stage: "handler".into(),
        original_entry_id: "123-0".into(),
        original_stream: "events".into(),
    };
    let envelope = data.envelope(&context()).unwrap();
    assert_eq!(envelope.home, EventHome::None);
    assert_eq!(
        envelope.subject,
        EventRef::new(ReferenceKind::StreamEntry, "events/123-0").unwrap()
    );
    DeadLettered::declaration().validate(&envelope).unwrap();
}

#[test]
fn a_producer_cannot_invent_undeclared_data_fields() {
    let event = finalized();
    let mut envelope = event.envelope(&context()).unwrap();
    envelope.data.insert("undeclared".into(), json!(true));
    assert!(event.declaration().validate_producer(&envelope).is_err());
}

#[test]
fn a_producer_cannot_use_an_undeclared_origin_kind() {
    let event = finalized();
    let mut envelope = event.envelope(&context()).unwrap();
    envelope.home = EventHome::Person(ProjectSlug::parse("person").unwrap());
    assert!(event.declaration().validate_producer(&envelope).is_err());
}

#[test]
fn a_producer_cannot_relate_an_undeclared_kind() {
    let event = finalized();
    let mut envelope = event.envelope(&context()).unwrap();
    envelope
        .related
        .push(EventRef::new(ReferenceKind::Operation, "an-operation").unwrap());
    assert!(event.declaration().validate_producer(&envelope).is_err());
}

#[test]
fn automation_does_not_invent_a_caller_session() {
    assert!(finalized().envelope(&context()).unwrap().related.is_empty());
}

#[test]
fn a_received_timestamp_cannot_use_temporal_zone_annotations() {
    let mut input = received();
    input["time"] = json!("2026-11-05T12:00:00Z[UTC]");
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
}

#[test]
fn encoding_refuses_extensions_that_duplicate_canonical_attributes() {
    let mut envelope = finalized().envelope(&context()).unwrap();
    envelope
        .extensions
        .insert("id".into(), json!("another-occurrence"));
    assert!(envelope.encode().is_err());
}

#[test]
fn a_producer_without_an_id_uses_the_boundary_generator() {
    let event = finalized();
    let envelope = event.envelope(&context()).unwrap();
    let producer = ProducerEvent::new(
        event.declaration(),
        envelope.subject,
        envelope.home,
        envelope.data,
    );
    let generated = occurrence_id(&["a fresh boundary occurrence"]);
    let emitted = producer.emit(&context(), || generated.clone()).unwrap();
    assert_eq!(emitted.id, generated);
}

#[test]
fn ci_subjects_name_the_candidate_commit() {
    let project = ProjectSlug::parse("demo").unwrap();
    let candidate = CommitSha::parse(&"c".repeat(40)).unwrap();
    let event = Event::CiCompleted {
        project: project.clone(),
        candidate: candidate.clone(),
        result: CiResult::Success,
    };
    assert_eq!(
        event.envelope(&context()).unwrap().subject,
        EventRef::new(ReferenceKind::Commit, &format!("{project}/{candidate}")).unwrap()
    );
}
