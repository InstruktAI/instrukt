//! Canonical, open received facts and emission context, with no transport policy.

use std::collections::BTreeMap;

use instrukt_core::event_type::is_type_name;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::{EventHome, EventHomeOwner, EventRef, ReferenceKind};

#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("an event reference must be `<kind>:<identity>` with nonempty parts: `{0}`")]
    Reference(String),
    #[error("invalid event origin `{0}`")]
    Home(String),
    #[error("invalid event project owner `{0}`")]
    Owner(String),
    #[error("event {0} must be nonempty")]
    Empty(&'static str),
    #[error("invalid dotted owner-qualified event type `{0}`")]
    Type(String),
    #[error("invalid RFC 3339 event time `{0}`")]
    Time(String),
    #[error("event does not match its declaration: {0}")]
    Declaration(String),
    #[error("cannot encode or decode event facts: {0}")]
    Json(#[from] serde_json::Error),
}

/// Producer keys are length-framed before hashing: separators inside identities
/// cannot alias a different tuple. Payload serialization never participates.
pub fn occurrence_id(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// A validated timestamp retaining its original offset and precision on the
/// wire. Re-encoding received facts must not rewrite their timestamp spelling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EventTime {
    text: String,
    instant: Timestamp,
}

impl EventTime {
    pub fn parse(text: &str) -> Result<Self, EnvelopeError> {
        // Jiff also accepts Temporal annotations and abbreviated times. The
        // event contract uses RFC 3339; Jiff validates calendar/offset values
        // after this narrower lexical check.
        if !rfc3339_shape(text) {
            return Err(EnvelopeError::Time(text.into()));
        }
        let instant = text.parse().map_err(|_| EnvelopeError::Time(text.into()))?;
        Ok(Self {
            text: text.into(),
            instant,
        })
    }

    pub fn timestamp(&self) -> Timestamp {
        self.instant
    }
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

fn rfc3339_shape(text: &str) -> bool {
    let Some((date, time)) = text.split_once(['T', 't']) else {
        return false;
    };
    if date.len() != 10
        || !date.bytes().enumerate().all(|(at, byte)| {
            if matches!(at, 4 | 7) {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return false;
    }
    let clock = if let Some(clock) = time.strip_suffix(['Z', 'z']) {
        clock
    } else {
        let Some(at) = time.rfind(['+', '-']) else {
            return false;
        };
        let (clock, offset) = time.split_at(at);
        if offset.len() != 6
            || !offset.bytes().enumerate().all(|(at, byte)| match at {
                0 => matches!(byte, b'+' | b'-'),
                3 => byte == b':',
                _ => byte.is_ascii_digit(),
            })
        {
            return false;
        }
        clock
    };
    let (seconds, fraction) = clock
        .split_once('.')
        .map_or((clock, None), |(seconds, fraction)| {
            (seconds, Some(fraction))
        });
    seconds.len() == 8
        && seconds.bytes().enumerate().all(|(at, byte)| {
            if matches!(at, 2 | 5) {
                byte == b':'
            } else {
                byte.is_ascii_digit()
            }
        })
        && fraction
            .is_none_or(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

impl TryFrom<String> for EventTime {
    type Error = EnvelopeError;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<EventTime> for String {
    fn from(value: EventTime) -> Self {
        value.text
    }
}

impl TryFrom<Timestamp> for EventTime {
    type Error = EnvelopeError;
    fn try_from(instant: Timestamp) -> Result<Self, Self::Error> {
        Self::parse(&instant.to_string())
    }
}

/// Facts attached by an emission boundary. The caller is actual runtime context;
/// automation supplies None. Failed ownership lookup supplies Unresolved.
#[derive(Debug, Clone)]
pub struct EmissionContext {
    pub source: String,
    pub time: Timestamp,
    pub project_owner: EventHomeOwner,
    pub caller_session: Option<String>,
}

/// Received events are open facts: unfamiliar attributes, data and reference
/// kinds round-trip, while known envelope invariants are validated on decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "WireEnvelope")]
pub struct EventEnvelope {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    pub subject: EventRef,
    pub time: EventTime,
    pub data: Map<String, Value>,
    pub home: EventHome,
    pub home_owner: EventHomeOwner,
    #[serde(default)]
    pub related: Vec<EventRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causationid: Option<String>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct WireEnvelope {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    source: String,
    subject: EventRef,
    time: EventTime,
    data: Map<String, Value>,
    home: EventHome,
    home_owner: EventHomeOwner,
    #[serde(default)]
    related: Vec<EventRef>,
    causationid: Option<String>,
    #[serde(flatten)]
    extensions: BTreeMap<String, Value>,
}

impl TryFrom<WireEnvelope> for EventEnvelope {
    type Error = EnvelopeError;
    fn try_from(wire: WireEnvelope) -> Result<Self, Self::Error> {
        let event = Self {
            id: wire.id,
            event_type: wire.event_type,
            source: wire.source,
            subject: wire.subject,
            time: wire.time,
            data: wire.data,
            home: wire.home,
            home_owner: wire.home_owner,
            related: wire.related,
            causationid: wire.causationid,
            extensions: wire.extensions,
        };
        event.validate()?;
        Ok(event)
    }
}

impl EventEnvelope {
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        if self.extensions.keys().any(|key| {
            matches!(
                key.as_str(),
                "id" | "type"
                    | "source"
                    | "subject"
                    | "time"
                    | "data"
                    | "home"
                    | "home_owner"
                    | "related"
                    | "causationid"
            )
        }) {
            return Err(EnvelopeError::Declaration(
                "extension duplicates a canonical attribute".into(),
            ));
        }
        if self.id.trim().is_empty() {
            return Err(EnvelopeError::Empty("id"));
        }
        if self.source.trim().is_empty() {
            return Err(EnvelopeError::Empty("source"));
        }
        if !is_type_name(&self.event_type) || !self.event_type.contains('.') {
            return Err(EnvelopeError::Type(self.event_type.clone()));
        }
        if self
            .causationid
            .as_ref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err(EnvelopeError::Empty("causationid"));
        }
        Ok(())
    }

    /// Construct received/relayed facts without catalog registration or routing.
    /// Decode uses this same validation and retains every unknown attribute.
    pub fn decode(json: &str) -> Result<Self, EnvelopeError> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn encode(&self) -> Result<String, EnvelopeError> {
        self.validate()?;
        Ok(serde_json::to_string(self)?)
    }

    pub(super) fn attach_context(
        &mut self,
        context: &EmissionContext,
    ) -> Result<(), EnvelopeError> {
        self.home_owner = if matches!(self.home, EventHome::Project(_)) {
            if context.project_owner == EventHomeOwner::None {
                EventHomeOwner::Unresolved
            } else {
                context.project_owner.clone()
            }
        } else {
            EventHomeOwner::None
        };
        if let Some(session) = &context.caller_session {
            let reference = EventRef::new(ReferenceKind::Session, session)?;
            if !self.related.contains(&reference) {
                self.related.push(reference);
            }
        }
        self.validate()
    }
}
