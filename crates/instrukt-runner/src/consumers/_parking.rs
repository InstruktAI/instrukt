//! Restart-recognizable Redis parking stages followed by ACK and local settlement.

use jiff::Timestamp;
use serde_json::json;

use super::_reader::{RawFields, Reader};
use super::_store::{self, Attempt, Settlement};
use super::{ConsumerError, SettledOutcome};
use crate::events::{
    DeadLettered, EmissionContext, EventEnvelope, EventHomeOwner, EventRef, ReferenceKind,
};
use crate::state;
use crate::transport::{
    DEAD_LETTER_STREAM, EVENT_STREAM, EventScope, STREAM_MAXLEN, TransportCoordinate,
};

impl Reader<'_> {
    pub fn park(
        &self,
        coordinate: &TransportCoordinate,
        seq: Option<u64>,
        raw: &RawFields,
        event: Option<&EventEnvelope>,
        attempt: &Attempt,
        now: Timestamp,
    ) -> Result<(), ConsumerError> {
        let receipt = format!(
            "instrukt:events:parking:{}",
            crate::events::occurrence_id(&[
                &self.group,
                &coordinate.source,
                &coordinate.stream,
                &coordinate.entry_id
            ])
        );
        let original = event
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| state::invalid(&std::path::PathBuf::new(), e))?
            .unwrap_or_else(|| json!({"raw_fields": raw}));
        let record = json!({
            "consumer": self.consumer.name, "consumer_id": self.identity, "source": coordinate.source,
            "original_stream": coordinate.stream, "original_entry_id": coordinate.entry_id,
            "original_envelope": original, "attempt_count": attempt.attempts,
            "first_attempt_at": attempt.first_attempt_at, "last_attempt_at": attempt.last_attempt_at,
            "failure": attempt.failure,
        }).to_string();
        // GET validates the receipt key before append. Recording only after XADD
        // avoids the source's claim-before-append false-success failure.
        let persist = redis::Script::new(
            r#"
            local prior = redis.call('GET', KEYS[2])
            if prior then return prior end
            local id = redis.call('XADD', KEYS[1], '*', 'record', ARGV[1])
            redis.call('SET', KEYS[2], id)
            return id
        "#,
        );
        let dead_letter_id: String = self.endpoint.run(|conn| {
            persist
                .key(DEAD_LETTER_STREAM)
                .key(&receipt)
                .arg(&record)
                .invoke(conn)
        })?;
        if !self.runtime.prepare_scope(EventScope::Local) {
            return Err(ConsumerError::NotPrepared);
        }
        let facts = DeadLettered {
            attempt_count: attempt.attempts.to_string(),
            consumer: self.consumer.name.clone(),
            consumer_id: self.group.clone(),
            dlq_destination: DEAD_LETTER_STREAM.into(),
            failing_stage: attempt
                .failure
                .split(':')
                .next()
                .unwrap_or("handling")
                .into(),
            original_entry_id: coordinate.entry_id.clone(),
            original_stream: coordinate.stream.clone(),
        };
        let mut announcement = facts
            .envelope(&EmissionContext {
                source: self.runtime.computer.clone(),
                time: now,
                project_owner: EventHomeOwner::None,
                caller_session: None,
            })
            .map_err(crate::transport::TransportError::from)?;
        // Independent Redis streams may use the same entry ID.
        announcement.subject = EventRef::new(
            ReferenceKind::StreamEntry,
            &format!(
                "{}/{}/{}",
                coordinate.source, coordinate.stream, coordinate.entry_id
            ),
        )
        .map_err(crate::transport::TransportError::from)?;
        announcement.id = crate::events::occurrence_id(&[
            "instrukt.system.event.dead_lettered",
            &coordinate.source,
            &self.group,
            &coordinate.stream,
            &coordinate.entry_id,
        ]);
        announcement.causationid = event.map(|e| e.id.clone());
        let encoded = announcement
            .encode()
            .map_err(crate::transport::TransportError::from)?;
        let announce_receipt = format!("{receipt}:announced");
        let announce = redis::Script::new(
            r#"
            local prior = redis.call('GET', KEYS[2])
            if prior then return prior end
            local id = redis.call('XADD', KEYS[1], 'MAXLEN', '~', ARGV[1], '*', 'event', ARGV[2])
            redis.call('SET', KEYS[2], id)
            return id
        "#,
        );
        // This declared platform fact is local even when its failing source is cluster.
        if !event.is_some_and(|event| event.event_type == DeadLettered::declaration().name) {
            self.runtime
                .transport
                .endpoint(EventScope::Local)
                .run::<String>(|conn| {
                    announce
                        .key(EVENT_STREAM)
                        .key(announce_receipt)
                        .arg(STREAM_MAXLEN)
                        .arg(encoded)
                        .invoke(conn)
                })?;
        } else {
            tracing::warn!(event = "dead_letter_announcement_parked", consumer = %self.consumer.name, "parking a failed announcement without recursively announcing it");
        }
        let outcome = SettledOutcome::Parked {
            destination: DEAD_LETTER_STREAM.into(),
            entry_id: dead_letter_id,
        };
        let settlement = Settlement {
            consumer: &self.consumer.name,
            coordinate,
            seq,
            event_id: None,
            outcome: &outcome,
            now,
        };
        _store::parking_ready(&self.runtime.state, &settlement)?;
        self.ack(coordinate)?;
        _store::finish_parking(&self.runtime.state, &settlement)?;
        Ok(())
    }
}
