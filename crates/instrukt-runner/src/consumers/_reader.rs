//! One ordered source reader. Network operations never hold the reaction or writer gate.

use std::sync::{Arc, Mutex};

use jiff::Timestamp;

use super::_sources::{self, BoundSource};
use super::_store::{self, Settlement};
use super::{
    Consumer, ConsumerError, ConsumerRuntime, MAX_HANDLING_ATTEMPTS, STALE_PENDING_MS,
    SettledOutcome,
};
use crate::events::EventEnvelope;
use crate::transport::{EVENT_STREAM, Endpoint, TransportCoordinate};

pub(super) type RawFields = Vec<(Vec<u8>, Vec<u8>)>;
type RawEntry = (String, RawFields);
type RawRead = Vec<(Vec<u8>, Vec<RawEntry>)>;

pub(super) enum DrainState {
    BudgetUsed,
    Waiting,
    Retired,
}

pub(super) struct Reader<'a> {
    pub runtime: &'a ConsumerRuntime,
    pub consumer: &'a Consumer,
    pub endpoint: Arc<Endpoint>,
    pub group: String,
    pub identity: String,
    gate: &'a Mutex<()>,
    retiring: bool,
}

impl<'a> Reader<'a> {
    pub fn new(
        runtime: &'a ConsumerRuntime,
        consumer: &'a Consumer,
        gate: &'a Mutex<()>,
        source: BoundSource,
    ) -> Self {
        Self {
            runtime,
            consumer,
            endpoint: source.endpoint,
            retiring: source.retiring,
            group: format!("consumer-{}-{}", consumer.name, runtime.computer),
            identity: format!("reader-{}", runtime.computer),
            gate,
        }
    }

    pub fn prepare(&self) -> Result<(), ConsumerError> {
        let created = self.endpoint.run(|conn| {
            match redis::cmd("XGROUP")
                .arg("CREATE")
                .arg(EVENT_STREAM)
                .arg(&self.group)
                .arg("$")
                .arg("MKSTREAM")
                .query::<()>(conn)
            {
                Ok(()) => Ok(true),
                Err(error) if error.code() == Some("BUSYGROUP") => Ok(false),
                Err(error) => Err(error),
            }
        })?;
        if created {
            let known = _sources::prepared(
                &self.runtime.state,
                &self.consumer.name,
                &self.endpoint.identity,
            )?;
            if known {
                tracing::warn!(event = "consumer_group_lost", consumer = %self.consumer.name, source = %self.endpoint.identity, "the Redis group was lost; its new position starts at the current tail; retained SQLite rows do not restore replay");
            }
        }
        if !created {
            _sources::prepared(
                &self.runtime.state,
                &self.consumer.name,
                &self.endpoint.identity,
            )?;
        }
        Ok(())
    }

    pub fn drain(&self, now: Timestamp, budget: usize) -> Result<DrainState, ConsumerError> {
        if !self.retiring {
            self.prepare()?;
        }
        if !self.retiring && _sources::retiring(&self.runtime.state, &self.consumer.name)? {
            return Ok(DrainState::Waiting);
        }
        for (coordinate, seq, outcome) in _store::parked(
            &self.runtime.state,
            &self.consumer.name,
            &self.endpoint.identity,
        )? {
            self.ack(&coordinate)?;
            _store::finish_parking(
                &self.runtime.state,
                &Settlement {
                    consumer: &self.consumer.name,
                    coordinate: &coordinate,
                    seq,
                    event_id: None,
                    outcome: &outcome,
                    now,
                },
            )?;
        }
        if self.retiring && self.retire_if_idle()? {
            return Ok(DrainState::Retired);
        }
        let mut exhausted = true;
        for _ in 0..budget {
            let pending: Vec<(String, String, u64, u64)> = self.endpoint.command(
                redis::cmd("XPENDING")
                    .arg(EVENT_STREAM)
                    .arg(&self.group)
                    .arg("-")
                    .arg("+")
                    .arg(1),
            )?;
            if let Some((id, owner, idle, _deliveries)) = pending.into_iter().next() {
                // An earlier outstanding entry always precedes new work, even if
                // another reader has not yet become stale enough to reclaim it.
                if owner != self.identity && idle < STALE_PENDING_MS {
                    exhausted = false;
                    break;
                }
                let threshold = if owner == self.identity {
                    0
                } else {
                    STALE_PENDING_MS
                };
                let claimed: Vec<RawEntry> = self.endpoint.command(
                    redis::cmd("XCLAIM")
                        .arg(EVENT_STREAM)
                        .arg(&self.group)
                        .arg(&self.identity)
                        .arg(threshold)
                        .arg(&id),
                )?;
                if let Some(entry) = claimed.into_iter().next() {
                    if !self.process(entry, now)? {
                        exhausted = false;
                        break;
                    }
                } else {
                    // XCLAIM removes a trimmed payload from the PEL. Preserve a
                    // visible loss outcome; it is never successful handling.
                    tracing::warn!(event = "consumer_payload_trimmed", consumer = %self.consumer.name, entry_id = %id, source = %self.endpoint.identity);
                    let coordinate = self.coordinate(id);
                    _store::settle(
                        &self.runtime.state,
                        &Settlement {
                            consumer: &self.consumer.name,
                            coordinate: &coordinate,
                            seq: None,
                            event_id: None,
                            outcome: &SettledOutcome::PayloadLost,
                            now,
                        },
                    )?;
                }
                continue;
            }
            let fresh: RawRead = self.endpoint.command(
                redis::cmd("XREADGROUP")
                    .arg("GROUP")
                    .arg(&self.group)
                    .arg(&self.identity)
                    .arg("COUNT")
                    .arg(1)
                    .arg("STREAMS")
                    .arg(EVENT_STREAM)
                    .arg(">"),
            )?;
            let Some(entry) = fresh.into_iter().flat_map(|(_, entries)| entries).next() else {
                exhausted = false;
                break;
            };
            if !self.process(entry, now)? {
                exhausted = false;
                break;
            }
        }
        if self.retiring && self.retire_if_idle()? {
            return Ok(DrainState::Retired);
        }
        Ok(if exhausted {
            DrainState::BudgetUsed
        } else {
            DrainState::Waiting
        })
    }

    fn retire_if_idle(&self) -> Result<bool, ConsumerError> {
        let script = redis::Script::new(
            r#"
            if redis.call('EXISTS', KEYS[1]) == 0 then return 2 end
            local groups = redis.call('XINFO', 'GROUPS', KEYS[1])
            local position = nil
            for _, group in ipairs(groups) do
                local name, last = nil, nil
                for i = 1, #group, 2 do
                    if group[i] == 'name' then name = group[i+1] end
                    if group[i] == 'last-delivered-id' then last = group[i+1] end
                end
                if name == ARGV[1] then position = last end
            end
            if not position then return 2 end
            if redis.call('XPENDING', KEYS[1], ARGV[1])[1] > 0 then return 0 end
            if #redis.call('XRANGE', KEYS[1], '(' .. position, '+', 'COUNT', 1) > 0 then return 0 end
            redis.call('XGROUP', 'DESTROY', KEYS[1], ARGV[1])
            return 1
        "#,
        );
        let retired: u8 = self
            .endpoint
            .run(|conn| script.key(EVENT_STREAM).arg(&self.group).invoke(conn))?;
        if retired != 0 {
            if retired == 2 {
                tracing::warn!(event = "consumer_group_lost_during_handover", consumer = %self.consumer.name, source = %self.endpoint.identity);
            }
            _sources::retire(
                &self.runtime.state,
                &self.consumer.name,
                &self.endpoint.identity,
            )?;
        }
        Ok(retired != 0)
    }

    fn coordinate(&self, entry_id: String) -> TransportCoordinate {
        TransportCoordinate {
            source: self.endpoint.identity.clone(),
            stream: EVENT_STREAM.into(),
            entry_id,
        }
    }

    fn process(&self, entry: RawEntry, now: Timestamp) -> Result<bool, ConsumerError> {
        let (id, raw) = entry;
        let coordinate = self.coordinate(id);
        let envelopes: Vec<_> = raw.iter().filter(|(field, _)| field == b"event").collect();
        let decoded = match envelopes.as_slice() {
            [(_, value)] => std::str::from_utf8(value)
                .map_err(|error| error.to_string())
                .and_then(|value| EventEnvelope::decode(value).map_err(|error| error.to_string())),
            [] => Err("missing event field".into()),
            _ => Err("ambiguous duplicate event fields".into()),
        };
        let seq = decoded
            .as_ref()
            .ok()
            .map(|event| _store::materialize(&self.runtime.state, event, &coordinate, now))
            .transpose()?;
        let _gate = self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let outcome = match &decoded {
            Ok(event)
                if !self
                    .consumer
                    .interest
                    .matches(event, &self.runtime.computer) =>
            {
                Some(SettledOutcome::Uninterested)
            }
            Ok(event)
                if _store::handled(&self.runtime.state, &self.consumer.name, &event.id, now)? =>
            {
                Some(SettledOutcome::Duplicate)
            }
            _ => None,
        };
        if let Some(outcome) = outcome {
            _store::settle(
                &self.runtime.state,
                &Settlement {
                    consumer: &self.consumer.name,
                    coordinate: &coordinate,
                    seq,
                    event_id: decoded.as_ref().ok().map(|e| e.id.as_str()),
                    outcome: &outcome,
                    now,
                },
            )?;
            drop(_gate);
            self.ack(&coordinate)?;
            return Ok(true);
        }
        let prior = _store::attempt(&self.runtime.state, &self.consumer.name, &coordinate)?;
        let mut attempt = match prior {
            Some(attempt) if attempt.attempts >= MAX_HANDLING_ATTEMPTS => attempt,
            _ => {
                let mut attempt = _store::begin_attempt(
                    &self.runtime.state,
                    &self.consumer.name,
                    &coordinate,
                    now,
                )?;
                let result = match &decoded {
                    Ok(event) => {
                        (self.consumer.reaction)(event, now).map_err(|error| error.to_string())
                    }
                    Err(error) => Err(format!("decode: {error}")),
                };
                match result {
                    Ok(value) => {
                        _store::settle(
                            &self.runtime.state,
                            &Settlement {
                                consumer: &self.consumer.name,
                                coordinate: &coordinate,
                                seq,
                                event_id: decoded.as_ref().ok().map(|e| e.id.as_str()),
                                outcome: &SettledOutcome::Accepted(value),
                                now,
                            },
                        )?;
                        drop(_gate);
                        self.ack(&coordinate)?;
                        return Ok(true);
                    }
                    Err(failure) => {
                        attempt.failure = failure;
                        _store::save_attempt(
                            &self.runtime.state,
                            &self.consumer.name,
                            &coordinate,
                            &attempt,
                        )?;
                    }
                }
                attempt
            }
        };
        drop(_gate);
        if attempt.attempts < MAX_HANDLING_ATTEMPTS {
            tracing::warn!(event = "consumer_attempt_failed", consumer = %self.consumer.name, entry_id = %coordinate.entry_id, attempts = attempt.attempts, failure = %attempt.failure);
            return Ok(false);
        }
        // Keep the last failure rather than substituting a stage error on retry.
        attempt.attempts = MAX_HANDLING_ATTEMPTS;
        self.park(&coordinate, seq, &raw, decoded.as_ref().ok(), &attempt, now)?;
        Ok(true)
    }

    pub fn ack(&self, coordinate: &TransportCoordinate) -> Result<(), ConsumerError> {
        self.endpoint.command::<u64>(
            redis::cmd("XACK")
                .arg(&coordinate.stream)
                .arg(&self.group)
                .arg(&coordinate.entry_id),
        )?;
        Ok(())
    }
}
