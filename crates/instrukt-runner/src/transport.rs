//! Scoped Redis publication. A source identity names an endpoint/database, never a credential.

mod _endpoint;

use std::sync::Arc;

use instrukt_core::config::{Configuration, Env};
use serde::{Deserialize, Serialize};

use crate::event_log::DbEventLog;
use crate::events::{EnvelopeError, EventEnvelope};
use crate::state::RunnerState;

pub(crate) use _endpoint::Endpoint;

pub const EVENT_STREAM: &str = "instrukt:events";
pub const DEAD_LETTER_STREAM: &str = "instrukt:events:dlq";
pub const STREAM_MAXLEN: u64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventScope {
    Local,
    Cluster,
}

impl EventScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Cluster => "cluster",
        }
    }
}

/// Retired platform vocabulary stays local. Relayed producers explicitly use cluster.
pub fn scope_of(event_type: &str) -> EventScope {
    match event_type.split('.').next() {
        Some("instrukt" | "runner" | "workflow") => EventScope::Local,
        _ => EventScope::Cluster,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportCoordinate {
    pub source: String,
    pub stream: String,
    pub entry_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),
    #[error(transparent)]
    Log(#[from] crate::events::EventLogError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(
        "invalid redis.{scope}.url; configure a supported Redis endpoint without inline credentials"
    )]
    Url { scope: &'static str },
    #[error("Redis source {source_id} failed ({kind})")]
    Redis { source_id: String, kind: String },
}

#[derive(Debug, Clone)]
pub struct ScopedTransport {
    local: Arc<Endpoint>,
    cluster: Arc<Endpoint>,
}

impl ScopedTransport {
    /// Cluster-only consumers neither resolve local credentials nor create local scopes.
    pub fn cluster_only(
        config: &Configuration,
        state: &RunnerState,
    ) -> Result<Self, TransportError> {
        let endpoint = Arc::new(Endpoint::new(&config.cluster_redis()?, "cluster", state)?);
        Ok(Self {
            local: Arc::clone(&endpoint),
            cluster: endpoint,
        })
    }

    pub fn from_config(
        config: &Configuration,
        env: &Env,
        state: &RunnerState,
    ) -> Result<Self, TransportError> {
        let settings = config.redis(env)?;
        let cluster = Arc::new(Endpoint::new(&settings.cluster, "cluster", state)?);
        let local = match settings.local {
            None => {
                tracing::warn!(
                    event = "local_redis_fallback",
                    "local Redis is unconfigured; local events use cluster Redis"
                );
                Arc::clone(&cluster)
            }
            Some(settings) => {
                let endpoint = Endpoint::new(&settings, "local", state)?;
                if endpoint.identity == cluster.identity {
                    Arc::clone(&cluster)
                } else {
                    Arc::new(endpoint)
                }
            }
        };
        Ok(Self { local, cluster })
    }

    pub(crate) fn endpoint(&self, scope: EventScope) -> &Arc<Endpoint> {
        match scope {
            EventScope::Local => &self.local,
            EventScope::Cluster => &self.cluster,
        }
    }

    pub(crate) fn distinct(&self, scopes: &[EventScope]) -> Vec<Arc<Endpoint>> {
        let mut endpoints = Vec::new();
        for scope in scopes {
            let endpoint = self.endpoint(*scope);
            if !endpoints
                .iter()
                .any(|known: &Arc<Endpoint>| known.identity == endpoint.identity)
            {
                endpoints.push(Arc::clone(endpoint));
            }
        }
        endpoints
    }

    /// Relay boundaries must pass Cluster regardless of the producer's vocabulary.
    pub fn publish(
        &self,
        scope: EventScope,
        event: &EventEnvelope,
    ) -> Result<TransportCoordinate, TransportError> {
        let endpoint = self.endpoint(scope);
        let entry_id = endpoint.command(
            redis::cmd("XADD")
                .arg(EVENT_STREAM)
                .arg("MAXLEN")
                .arg("~")
                .arg(STREAM_MAXLEN)
                .arg("*")
                .arg("event")
                .arg(event.encode()?),
        )?;
        Ok(TransportCoordinate {
            source: endpoint.identity.clone(),
            stream: EVENT_STREAM.into(),
            entry_id,
        })
    }

    /// A claim is confirmed only after append. An append error leaves no false completed claim.
    pub fn publish_claimed(
        &self,
        event: &EventEnvelope,
        claim: &str,
        ttl_seconds: u64,
    ) -> Result<TransportCoordinate, TransportError> {
        let key = format!(
            "instrukt:ingress:claim:{}",
            crate::events::occurrence_id(&[claim])
        );
        let script = redis::Script::new(
            r#"
            local prior = redis.call('GET', KEYS[2])
            if prior then return prior end
            local id = redis.call('XADD', KEYS[1], 'MAXLEN', '~', ARGV[1], '*', 'event', ARGV[2])
            redis.call('SET', KEYS[2], id, 'EX', ARGV[3])
            return id
        "#,
        );
        let encoded = event.encode()?;
        let entry_id = self.cluster.run(|conn| {
            script
                .key(EVENT_STREAM)
                .key(key)
                .arg(STREAM_MAXLEN)
                .arg(encoded)
                .arg(ttl_seconds)
                .invoke(conn)
        })?;
        Ok(TransportCoordinate {
            source: self.cluster.identity.clone(),
            stream: EVENT_STREAM.into(),
            entry_id,
        })
    }

    /// Each scope retries independently. No network operation is inside a local write section.
    pub fn publish_pending(&self, log: &DbEventLog) -> Result<(), TransportError> {
        let pending = log.unconfirmed()?;
        std::thread::scope(|threads| {
            for scope in [EventScope::Local, EventScope::Cluster] {
                let pending = &pending;
                threads.spawn(move || {
                    for publication in pending.iter().filter(|entry| entry.scope == scope) {
                        match self.publish(scope, &publication.envelope) {
                            Ok(coordinate) => {
                                if let Err(error) = log.confirm(&publication.envelope.id, &coordinate) {
                                    tracing::warn!(event = "event_publication_unconfirmed", error = %error);
                                }
                            }
                            Err(error) => {
                                if let Err(record_error) = log.publication_failed(&publication.envelope.id, &error.to_string()) {
                                    tracing::warn!(event = "event_publication_failure_not_recorded", error = %record_error);
                                }
                                tracing::warn!(event = "event_publication_pending", id = %publication.envelope.id, error = %error);
                                // A failed endpoint gets one opportunity per bounded drain.
                                break;
                            }
                        }
                    }
                });
            }
        });
        Ok(())
    }
}
