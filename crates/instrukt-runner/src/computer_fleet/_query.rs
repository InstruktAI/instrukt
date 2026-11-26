//! The requester half of the fleet query: opening, the request envelope,
//! one combined wait for every computer asked, and what was observed for
//! each.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use instrukt_core::config::Configuration;
use instrukt_core::home::Home;
use instrukt_core::machine::{self, MachineId};
use serde::{Deserialize, Serialize};

use crate::state::RunnerState;
use crate::transport::{EventScope, ScopedTransport, TransportError};

/// How long a request lives, and the longest a consumer may wait.
pub(crate) const REQUEST_LIFETIME: Duration = Duration::from_secs(120);

/// The longest deadline a consumer may ask with.
pub const MAX_DEADLINE: Duration = REQUEST_LIFETIME;

/// A query kind: the key prefix its queues and reply places live under.
/// Each kind travels on its own per-computer queue, so a computer meets only
/// the requests of the kinds it answers and a build that does not know a
/// kind never takes one of its requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind {
    prefix: &'static str,
    wait: Wait,
}

/// The connection a kind's combined wait runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    /// A pooled connection, whose read timeout is the cluster connection's
    /// socket timeout.
    Pooled,
    /// A connection of its own whose read timeout outlasts the deadline,
    /// so the deadline always ends the wait first.
    Outlasting,
}

/// Merged history search: its keys are the ones history always used, and
/// its wait runs as it always did.
pub(crate) const HISTORY: Kind = Kind {
    prefix: "instrukt:history",
    wait: Wait::Pooled,
};

/// The evaluation report's computer answers.
pub const EVALUATION: Kind = Kind {
    prefix: "instrukt:eval",
    wait: Wait::Outlasting,
};

impl Kind {
    /// Where requests of this kind wait for `computer`.
    pub(crate) fn queue(self, computer: &MachineId) -> String {
        format!("{}:requests:{computer}", self.prefix)
    }

    /// The reply place of this kind named by `id`, under the kind's prefix.
    fn reply(self, id: &str) -> String {
        format!("{}:reply:{id}", self.prefix)
    }

    /// Whether `place` is a reply place of this kind; nothing else is ever
    /// written to by a computer answering.
    pub(crate) fn is_reply_place(self, place: &str) -> bool {
        place
            .strip_prefix(self.prefix)
            .and_then(|rest| rest.strip_prefix(":reply:"))
            .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
    }

    /// The readiness key of this kind for `computer`.
    pub(crate) fn ready(self, computer: &MachineId) -> String {
        format!("{}:ready:{computer}", self.prefix)
    }
}

/// A request as it travels: its reply place and expiry around the kind's
/// own payload, whose fields stand beside them.
#[derive(Serialize, Deserialize)]
pub(crate) struct Envelope<P> {
    pub reply: String,
    #[serde(flatten)]
    pub payload: P,
    /// Unix seconds after which the request is left unanswered.
    pub expires: i64,
}

/// Why the query cannot start, for the whole query and before any computer
/// is asked.
#[derive(Debug)]
pub enum Unopened {
    /// No cluster connection is configured on this computer.
    NotConfigured,
    /// A cluster connection is configured but cannot be used: its address
    /// or a credential it refers to cannot be resolved, or is invalid.
    Unusable(TransportError),
}

/// The fleet query over this computer's cluster connection.
#[derive(Debug, Clone)]
pub struct Fleet {
    transport: ScopedTransport,
}

/// Opens the fleet query on `config`'s cluster connection. This is the one
/// reading of whether a cluster connection is configured: an address that
/// is present but cannot be used is never treated as absent.
pub fn open(config: &Configuration, home: &Home) -> Result<Fleet, Unopened> {
    match config.cluster_redis_url() {
        Ok(None) => return Err(Unopened::NotConfigured),
        Ok(Some(_)) => {}
        Err(error) => return Err(Unopened::Unusable(TransportError::Config(error))),
    }
    ScopedTransport::cluster_only(config, &RunnerState::new(home))
        .map(|transport| Fleet { transport })
        .map_err(Unopened::Unusable)
}

/// What the query observed for one computer asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Its answer, as it arrived.
    Answered(String),
    /// The store could not be reached when the query began.
    Unreachable,
    /// The store refused the request, or failed while it was sent.
    NotDelivered,
    /// The store failed while the answer was awaited.
    StoreFailed,
    /// No answer came before the deadline.
    NoAnswer,
}

/// What stops a request from being formed on this computer.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    #[error("no random reply place could be drawn: {0}")]
    Random(std::io::Error),
    #[error("the request could not be encoded: {0}")]
    Encode(#[from] serde_json::Error),
}

impl Fleet {
    pub(crate) fn transport(&self) -> &ScopedTransport {
        &self.transport
    }

    /// Asks each of `computers` with `payload` as a request of `kind`, and
    /// waits for all their answers together until each has answered or
    /// `deadline` (at most [`MAX_DEADLINE`]) has passed. Returns what was
    /// observed for every computer asked, by computer. The consumer leaves
    /// its own computer out: it is never asked through the store.
    pub fn ask<P: Serialize>(
        &self,
        kind: Kind,
        computers: &[MachineId],
        payload: &P,
        deadline: Duration,
    ) -> Result<BTreeMap<MachineId, Outcome>, AskError> {
        let deadline = Instant::now() + deadline.min(MAX_DEADLINE);
        let endpoint = self.transport.endpoint(EventScope::Cluster);
        let mut observed = BTreeMap::new();
        // Each computer's reply place, and the computer it belongs to.
        let mut pending: BTreeMap<String, MachineId> = BTreeMap::new();
        let mut first = true;
        for computer in computers {
            if observed.contains_key(computer) || pending.values().any(|c| c == computer) {
                continue;
            }
            let id = machine::random_hex(16).map_err(AskError::Random)?;
            let reply = kind.reply(&id);
            let request = serde_json::to_string(&Envelope {
                reply: reply.clone(),
                payload,
                expires: jiff::Timestamp::now().as_second()
                    + i64::try_from(REQUEST_LIFETIME.as_secs()).unwrap_or(i64::MAX),
            })?;
            let queue = kind.queue(computer);
            let sent = endpoint.run(|conn| {
                redis::pipe()
                    .atomic()
                    .cmd("RPUSH")
                    .arg(&queue)
                    .arg(&request)
                    .ignore()
                    .cmd("EXPIRE")
                    .arg(&queue)
                    .arg(REQUEST_LIFETIME.as_secs())
                    .ignore()
                    .query::<()>(conn)
            });
            match sent {
                Ok(()) => {
                    pending.insert(reply, computer.clone());
                }
                // The store could not be reached at all: nobody is asked.
                Err(error) if first && error.is_connection_failure() => {
                    tracing::warn!(event = "computer_fleet_unreachable", kind = kind.prefix, %error);
                    return Ok(computers
                        .iter()
                        .map(|computer| (computer.clone(), Outcome::Unreachable))
                        .collect());
                }
                Err(error) => {
                    tracing::warn!(event = "computer_fleet_not_delivered", kind = kind.prefix, %computer, %error);
                    observed.insert(computer.clone(), Outcome::NotDelivered);
                }
            }
            first = false;
        }
        // Wait on all replies together: a missing computer cannot hide a
        // healthy one's answer.
        while !pending.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let keys: Vec<&str> = pending.keys().map(String::as_str).collect();
            let mut wait = redis::cmd("BLPOP");
            wait.arg(&keys).arg(remaining.as_secs_f64().max(0.001));
            let response = match kind.wait {
                Wait::Pooled => endpoint.command::<Option<(String, String)>>(&mut wait),
                Wait::Outlasting => endpoint.run_waiting(remaining, |conn| wait.query(conn)),
            };
            match response {
                Ok(Some((reply, body))) => match pending.remove(&reply) {
                    Some(computer) => {
                        observed.insert(computer, Outcome::Answered(body));
                    }
                    None => {
                        tracing::warn!(event = "computer_fleet_reply_unexpected", kind = kind.prefix, %reply);
                    }
                },
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(event = "computer_fleet_wait_failed", kind = kind.prefix, %error);
                    for computer in std::mem::take(&mut pending).into_values() {
                        observed.insert(computer, Outcome::StoreFailed);
                    }
                }
            }
        }
        for computer in pending.into_values() {
            observed.insert(computer, Outcome::NoAnswer);
        }
        Ok(observed)
    }
}

#[cfg(test)]
mod tests {
    use super::{EVALUATION, Envelope, HISTORY};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize)]
    struct HistoryPayload {
        query: String,
        limit: usize,
    }

    /// A history request as builds before the fleet query read it: these
    /// four fields and no others.
    #[derive(Debug, PartialEq, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OlderBuildsQuery {
        reply: String,
        query: String,
        limit: usize,
        expires: i64,
    }

    #[test]
    fn a_history_request_is_the_json_older_builds_read() {
        let (reply, query, limit, expires) = (
            HISTORY.reply("0123456789abcdef0123456789abcdef"),
            "sentinel query".to_owned(),
            20,
            1_900_000_120,
        );
        let request = serde_json::to_string(&Envelope {
            reply: reply.clone(),
            payload: HistoryPayload {
                query: query.clone(),
                limit,
            },
            expires,
        })
        .unwrap();
        assert_eq!(
            serde_json::from_str::<OlderBuildsQuery>(&request).unwrap(),
            OlderBuildsQuery {
                reply,
                query,
                limit,
                expires,
            }
        );
    }

    #[test]
    fn a_reply_place_is_the_kinds_prefix_and_32_hex_digits() {
        let hex = "0123456789abcdef0123456789ABCDEF";
        assert!(HISTORY.is_reply_place(&format!("instrukt:history:reply:{hex}")));
        assert!(EVALUATION.is_reply_place(&format!("instrukt:eval:reply:{hex}")));
        for place in [
            format!("instrukt:eval:reply:{hex}"),
            format!("instrukt:history:reply:{hex}0"),
            format!("instrukt:history:reply:{}", &hex[1..]),
            "instrukt:history:reply:0123456789abcdef0123456789abcdeg".to_owned(),
            format!("instrukt:history:requests:{hex}"),
            format!("instrukt:historyx:reply:{hex}"),
            format!("other:instrukt:history:reply:{hex}"),
        ] {
            assert!(!HISTORY.is_reply_place(&place), "{place}");
        }
    }
}
