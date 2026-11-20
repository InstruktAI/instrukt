//! Restart-recognizable Redis parking stages followed by ACK and local settlement.

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde_json::json;

use super::_reader::{RawFields, Reader};
use super::_store::{self, ANNOUNCEMENT_BATCH, AnnouncementResult, Attempt, Settlement};
use super::{ConsumerError, RECOVERY_SWEEP, SettledOutcome};
use crate::events::{
    DeadLettered, EmissionContext, EventEnvelope, EventHomeOwner, EventRef, ReferenceKind,
};
use crate::state;
use crate::transport::{
    DEAD_LETTER_STREAM, EVENT_STREAM, Endpoint, EventScope, STREAM_MAXLEN, TransportCoordinate,
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
                .announcement(coordinate, event, attempt, &receipt, now)
                .and_then(|job| {
                    self.announcements
                        .ok_or("announcement worker is unavailable".to_owned())?
                        .try_send(job)
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
        receipt: &str,
        now: Timestamp,
    ) -> Result<Announcement, String> {
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
        let encoded = announcement.encode().map_err(|error| error.to_string())?;
        Ok(Announcement {
            coordinate: coordinate.clone(),
            consumer: self.consumer.name.clone(),
            receipt: format!("{receipt}:announced"),
            encoded,
            queued: Instant::now(),
        })
    }
}

pub(super) const ANNOUNCEMENT_QUEUE: usize = 32;
const ANNOUNCEMENT_RECEIPT_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
const ANNOUNCEMENT_TIMEOUT: Duration = Duration::from_millis(250);
const ANNOUNCEMENT_LIFETIME: Duration = Duration::from_secs(1);

pub(super) struct Announcement {
    coordinate: TransportCoordinate,
    consumer: String,
    receipt: String,
    encoded: String,
    queued: Instant,
}

pub(super) type Announcements = SyncSender<Announcement>;

pub(super) fn announce(runtime: &super::ConsumerRuntime, jobs: Receiver<Announcement>) {
    let endpoint = runtime.transport.endpoint(EventScope::Local);
    let append = redis::Script::new(
        r#"
        local prior = redis.call('GET', KEYS[2])
        if prior then
            redis.call('SET', KEYS[2], prior, 'EX', ARGV[3])
            return prior
        end
        if ARGV[2] == '' then return false end
        if #ARGV > 3 then
            local prepared = {}
            for _, group in ipairs(redis.call('XINFO', 'GROUPS', KEYS[1])) do
                for i = 1, #group, 2 do
                    if group[i] == 'name' then prepared[group[i+1]] = true end
                end
            end
            for i = 4, #ARGV do
                if not prepared[ARGV[i]] then return redis.error_reply('local consumer position not prepared') end
            end
        end
        if not redis.acl_check_cmd('SET', KEYS[2], '0-0', 'EX', ARGV[3]) then
            return redis.error_reply('announcement receipt write not permitted')
        end
        local id = redis.call('XADD', KEYS[1], 'MAXLEN', '~', ARGV[1], '*', 'event', ARGV[2])
        redis.call('SET', KEYS[2], id, 'EX', ARGV[3])
        return id
        "#,
    );
    let mut retry_at = Some(Instant::now());

    loop {
        if retry_at.is_some_and(|when| when <= Instant::now()) {
            retry_at = match recover(runtime, &append) {
                Ok(false) => None,
                Ok(true) => Some(Instant::now() + RECOVERY_SWEEP),
                Err(error) => {
                    tracing::warn!(event = "dead_letter_announcement_recovery_deferred", %error);
                    Some(Instant::now() + RECOVERY_SWEEP)
                }
            };
        }
        let received = match retry_at {
            None => jobs.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(when) => jobs.recv_timeout(when.saturating_duration_since(Instant::now())),
        };
        let job = match received {
            Ok(job) => job,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if job.queued.elapsed() >= ANNOUNCEMENT_LIFETIME {
            tracing::warn!(event = "dead_letter_unannounced", consumer = %job.consumer, entry = %job.coordinate.entry_id, "announcement expired in the queue");
            continue;
        }
        match _store::record_announcement_intent(
            &runtime.state,
            &job.consumer,
            &job.coordinate,
            &endpoint.identity,
            &job.receipt,
            &job.encoded,
        ) {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(event = "dead_letter_unannounced", consumer = %job.consumer, entry = %job.coordinate.entry_id, "announcement intent limit reached");
                continue;
            }
            Err(error) => {
                tracing::warn!(event = "dead_letter_announcement_intent_failed", consumer = %job.consumer, entry = %job.coordinate.entry_id, %error);
                continue;
            }
        }
        let delivery = publish(runtime, endpoint, &append, &job.receipt, Some(&job.encoded))
            .and_then(|result| settle(&runtime.state, endpoint, &job, result));
        if let Err(error) = delivery {
            retry_at.get_or_insert_with(|| Instant::now() + RECOVERY_SWEEP);
            tracing::warn!(event = "dead_letter_announcement_deferred", consumer = %job.consumer, entry = %job.coordinate.entry_id, %error);
        }
    }
}

fn publish(
    runtime: &super::ConsumerRuntime,
    endpoint: &Endpoint,
    append: &redis::Script,
    receipt: &str,
    payload: Option<&str>,
) -> Result<AnnouncementResult, ConsumerError> {
    let groups: Vec<_> = runtime
        .consumers
        .iter()
        .filter(|consumer| {
            consumer
                .scopes
                .iter()
                .any(|scope| runtime.transport.endpoint(*scope).identity == endpoint.identity)
        })
        .map(|consumer| format!("consumer-{}-{}", consumer.name, runtime.computer))
        .collect();
    let result: Option<String> = endpoint.run_best_effort(ANNOUNCEMENT_TIMEOUT, |conn| {
        append
            .key(EVENT_STREAM)
            .key(receipt)
            .arg(STREAM_MAXLEN)
            .arg(payload.unwrap_or_default())
            .arg(ANNOUNCEMENT_RECEIPT_TTL_SECONDS)
            .arg(&groups)
            .invoke(conn)
    })?;
    Ok(if result.is_some() {
        AnnouncementResult::Published
    } else {
        tracing::warn!(event = "dead_letter_announcement_payload_unavailable", source = %endpoint.identity, receipt, "legacy intent has neither a receipt nor a saved payload");
        AnnouncementResult::MissingLegacyPayload
    })
}

fn settle(
    state: &state::RunnerState,
    endpoint: &Endpoint,
    job: &Announcement,
    result: AnnouncementResult,
) -> Result<(), ConsumerError> {
    _store::finish_announcement(state, &job.consumer, &job.coordinate, result)?;
    if matches!(result, AnnouncementResult::Published) {
        clean_receipt(endpoint, &job.receipt);
    }
    Ok(())
}

fn clean_receipt(endpoint: &Endpoint, receipt: &str) {
    let deleted = endpoint.run_best_effort(ANNOUNCEMENT_TIMEOUT, |conn| {
        redis::cmd("DEL").arg(receipt).query::<u64>(conn)
    });
    if let Err(error) = deleted {
        tracing::warn!(event = "dead_letter_announcement_receipt_cleanup_failed", source = %endpoint.identity, receipt, %error);
    }
}

// This is derived scheduling state, with one writer under the runner lock.
// Repeating a page is safe after a crash. Keeping the position between ticks
// prevents a blocked first page from monopolizing each scheduled wake.
fn recovery_position(path: &std::path::Path) -> Result<i64, state::StateError> {
    state::refuse_symlink(path)?;
    match std::fs::read_to_string(path) {
        Ok(text) => match text.trim().parse::<i64>() {
            Ok(position) if position >= 0 => Ok(position),
            _ => {
                tracing::warn!(event = "dead_letter_announcement_cursor_invalid", path = %path.display(), "restarting recovery from the first page");
                Ok(0)
            }
        },
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(state::StateError::Read {
            path: path.into(),
            source,
        }),
    }
}

/// Replay one bounded page per sweep and keep its position across runner ticks.
fn recover(
    runtime: &super::ConsumerRuntime,
    append: &redis::Script,
) -> Result<bool, ConsumerError> {
    let state = &runtime.state;
    let path = state
        .home()
        .state_dir()
        .join("announcement-recovery-cursor");
    let after = recovery_position(&path)?;
    let intents = _store::announcement_intents(state, after)?;
    let more = !intents.is_empty() || after != 0;
    let next = if intents.len() == ANNOUNCEMENT_BATCH {
        intents.last().map_or(0, |intent| intent.rowid)
    } else {
        0
    };
    let endpoints = runtime
        .transport
        .distinct(&[EventScope::Local, EventScope::Cluster]);
    for intent in intents {
        let endpoint = endpoints
            .iter()
            .find(|endpoint| endpoint.identity == intent.source);
        let deliver = match endpoint {
            Some(endpoint) => deliver_intent(state, runtime, endpoint, append, &intent),
            None if intent.confirmed => _store::finish_announcement(
                state,
                &intent.consumer,
                &intent.coordinate,
                AnnouncementResult::Published,
            )
            .map_err(ConsumerError::from),
            None => {
                tracing::warn!(event = "dead_letter_announcement_abandoned", source = %intent.source, receipt = %intent.receipt);
                _store::finish_announcement(
                    state,
                    &intent.consumer,
                    &intent.coordinate,
                    AnnouncementResult::Abandoned,
                )
                .map_err(ConsumerError::from)
            }
        };
        if let Err(error) = deliver {
            tracing::warn!(event = "dead_letter_announcement_deferred", source = %intent.source, receipt = %intent.receipt, %error);
        }
    }
    if next != after {
        state::write_bytes(&path, next.to_string().as_bytes())?;
    }
    Ok(more)
}

fn deliver_intent(
    state: &state::RunnerState,
    runtime: &super::ConsumerRuntime,
    endpoint: &Endpoint,
    append: &redis::Script,
    intent: &_store::AnnouncementIntent,
) -> Result<(), ConsumerError> {
    let result = if intent.confirmed {
        AnnouncementResult::Published
    } else {
        publish(
            runtime,
            endpoint,
            append,
            &intent.receipt,
            intent.payload.as_deref(),
        )?
    };
    _store::finish_announcement(state, &intent.consumer, &intent.coordinate, result)?;
    if matches!(result, AnnouncementResult::Published) {
        clean_receipt(endpoint, &intent.receipt);
    }
    Ok(())
}
