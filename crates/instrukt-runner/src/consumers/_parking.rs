//! Restart-recognizable Redis parking stages followed by ACK and local settlement.

use std::time::Duration;

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
        let outcome = SettledOutcome::Parked {
            destination: DEAD_LETTER_STREAM.into(),
            entry_id: dead_letter_id,
            announced: false,
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
        if event.is_some_and(|event| event.event_type == DeadLettered::declaration().name) {
            tracing::warn!(event = "dead_letter_announcement_parked", consumer = %self.consumer.name, "parking a failed announcement without recursively announcing it");
        } else {
            let result = self
                .announcement(coordinate, event, attempt, now)
                .and_then(|encoded| {
                    self.runtime
                        .transport
                        .endpoint(EventScope::Local)
                        .run_best_effort(ANNOUNCEMENT_TIMEOUT, |connection| {
                            redis::cmd("XADD")
                                .arg(EVENT_STREAM)
                                .arg("MAXLEN")
                                .arg("~")
                                .arg(STREAM_MAXLEN)
                                .arg("*")
                                .arg("event")
                                .arg(&encoded)
                                .query::<String>(connection)
                                .map(drop)
                        })
                        .map_err(|error| error.to_string())?;
                    _store::mark_announced(&self.runtime.state, &self.consumer.name, coordinate)
                        .map_err(|error| error.to_string())
                });
            if let Err(error) = result {
                tracing::warn!(event = "dead_letter_unannounced", consumer = %self.consumer.name, entry = %coordinate.entry_id, %error);
            }
        }
        Ok(())
    }

    fn announcement(
        &self,
        coordinate: &TransportCoordinate,
        event: Option<&EventEnvelope>,
        attempt: &Attempt,
        now: Timestamp,
    ) -> Result<String, String> {
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
            .map_err(|error| error.to_string())?;
        // Independent Redis streams may use the same entry ID.
        announcement.subject = EventRef::new(
            ReferenceKind::StreamEntry,
            &format!(
                "{}/{}/{}",
                coordinate.source, coordinate.stream, coordinate.entry_id
            ),
        )
        .map_err(|error| error.to_string())?;
        announcement.id = crate::events::occurrence_id(&[
            "instrukt.system.event.dead_lettered",
            &coordinate.source,
            &self.group,
            &coordinate.stream,
            &coordinate.entry_id,
        ]);
        announcement.causationid = event.map(|e| e.id.clone());
        announcement.encode().map_err(|error| error.to_string())
    }
}

const ANNOUNCEMENT_TIMEOUT: Duration = Duration::from_millis(250);
