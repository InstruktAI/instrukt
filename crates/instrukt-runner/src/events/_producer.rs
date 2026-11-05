//! Construction from typed producers, separated from received opaque facts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{
    DataField, EmissionContext, EnvelopeError, Event, EventDeclaration, EventEnvelope, EventHome,
    EventHomeOwner, EventRef, ReferenceKind, occurrence_id,
};

/// Producer-owned facts. The boundary adds computer, time, owner and caller;
/// callers may retain an explicitly supplied occurrence ID and causation.
#[derive(Debug, Clone)]
pub struct ProducerEvent {
    declaration: EventDeclaration,
    pub subject: EventRef,
    pub home: EventHome,
    pub data: Map<String, Value>,
    pub related: Vec<EventRef>,
    pub causationid: Option<String>,
    pub id: Option<String>,
}

impl ProducerEvent {
    pub fn new(
        declaration: EventDeclaration,
        subject: EventRef,
        home: EventHome,
        data: Map<String, Value>,
    ) -> Self {
        Self {
            declaration,
            subject,
            home,
            data,
            related: Vec::new(),
            causationid: None,
            id: None,
        }
    }

    /// `generated_id` is boundary-provided randomness or an occurrence key; it
    /// is consulted only when the producer supplied no ID.
    pub fn emit(
        self,
        context: &EmissionContext,
        generated_id: impl FnOnce() -> String,
    ) -> Result<EventEnvelope, EnvelopeError> {
        let mut envelope = EventEnvelope {
            id: self.id.unwrap_or_else(generated_id),
            event_type: self.declaration.name.clone(),
            source: context.source.clone(),
            subject: self.subject,
            time: context.time.try_into()?,
            data: self.data,
            home: self.home,
            home_owner: EventHomeOwner::None,
            related: self.related,
            causationid: self.causationid,
            extensions: BTreeMap::new(),
        };
        envelope.attach_context(context)?;
        self.declaration.validate_producer(&envelope)?;
        Ok(envelope)
    }
}

impl Event {
    /// Turn acquisition facts into a declaration-valid canonical envelope.
    /// The project slug qualifies local repository commits; an upstream commit
    /// additionally carries its remote identity.
    pub fn envelope(&self, context: &EmissionContext) -> Result<EventEnvelope, EnvelopeError> {
        let subject = match self {
            Self::UpstreamChanged {
                project,
                remote,
                revision,
                ..
            } => EventRef::new(
                ReferenceKind::Commit,
                &format!("{project}/{remote}/{revision}"),
            )?,
            Self::MainAhead {
                project,
                local_main,
                ..
            } => EventRef::new(ReferenceKind::Commit, &format!("{project}/{local_main}"))?,
            Self::CiCompleted {
                project, candidate, ..
            } => EventRef::new(ReferenceKind::Commit, &format!("{project}/{candidate}"))?,
            Self::MainFailing { project, head, .. } => {
                EventRef::new(ReferenceKind::Commit, &format!("{project}/{head}"))?
            }
            Self::WorkflowFinalized { todo, .. } => {
                EventRef::new(ReferenceKind::Todo, todo.as_str())?
            }
        };
        let Value::Object(mut data) = serde_json::to_value(self)? else {
            return Err(EnvelopeError::Declaration(
                "acquisition facts must be an object".into(),
            ));
        };
        data.remove("event");
        let mut producer = ProducerEvent::new(
            self.declaration(),
            subject,
            EventHome::Project(self.project().clone()),
            data,
        );
        producer.id = Some(self.occurrence_id());
        producer.emit(context, || self.occurrence_id())
    }
}

// The data fields and their declaration are generated from the same typed
// producer, including the intentionally string-valued attempt count.
macro_rules! dead_letter {
    ($($field:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct DeadLettered { $(pub $field: String),+ }

        impl DeadLettered {
            pub fn declaration() -> EventDeclaration {
                EventDeclaration {
                    name: "instrukt.system.event.dead_lettered".into(), version: 1,
                    description: "A persistently failing stream entry was parked in a dead letter destination.".into(),
                    subject_kind: Some(ReferenceKind::StreamEntry), home_kinds: Vec::new(),
                    data_fields: vec![$(DataField::of::<String>(stringify!($field))),+],
                    related_kinds: vec![ReferenceKind::Session], state: None,
                }
            }
        }
    };
}
dead_letter! { attempt_count, consumer, consumer_id, dlq_destination, failing_stage, original_entry_id, original_stream }

impl DeadLettered {
    pub fn envelope(&self, context: &EmissionContext) -> Result<EventEnvelope, EnvelopeError> {
        let Value::Object(data) = serde_json::to_value(self)? else {
            return Err(EnvelopeError::Declaration(
                "dead letter facts must be an object".into(),
            ));
        };
        let subject = EventRef::new(
            ReferenceKind::StreamEntry,
            &format!("{}/{}", self.original_stream, self.original_entry_id),
        )?;
        let mut producer = ProducerEvent::new(Self::declaration(), subject, EventHome::None, data);
        producer.id = Some(occurrence_id(&[
            &producer.declaration.name,
            &self.consumer,
            &self.consumer_id,
            &self.original_stream,
            &self.original_entry_id,
        ]));
        producer.emit(context, String::new)
    }
}

impl super::CiResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::NoCi => "no_ci",
        }
    }
}
