//! Opaque received references and typed producer reference vocabulary.

use std::fmt;

use instrukt_core::ids::ProjectSlug;
use serde::{Deserialize, Serialize};

use super::EnvelopeError;

/// Known producer reference kinds. Received references also retain unknown kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Todo,
    Session,
    Operation,
    Job,
    EventType,
    Computer,
    Configuration,
    StreamEntry,
    CapturedItem,
    InboundSource,
    SignalItem,
    SignalCluster,
    SignalSynthesis,
    Deployment,
    SchemaProposal,
    Escalation,
    MemoryScope,
    CiRun,
    Commit,
    Install,
    Release,
}

impl ReferenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Session => "session",
            Self::Operation => "operation",
            Self::Job => "job",
            Self::EventType => "event_type",
            Self::Computer => "computer",
            Self::Configuration => "configuration",
            Self::StreamEntry => "stream_entry",
            Self::CapturedItem => "captured_item",
            Self::InboundSource => "inbound_source",
            Self::SignalItem => "signal_item",
            Self::SignalCluster => "signal_cluster",
            Self::SignalSynthesis => "signal_synthesis",
            Self::Deployment => "deployment",
            Self::SchemaProposal => "schema_proposal",
            Self::Escalation => "escalation",
            Self::MemoryScope => "memory_scope",
            Self::CiRun => "ci_run",
            Self::Commit => "commit",
            Self::Install => "install",
            Self::Release => "release",
        }
    }
}

/// A wire reference splits at its first colon; its identity remains opaque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EventRef {
    kind: String,
    identity: String,
}

impl EventRef {
    pub fn new(kind: ReferenceKind, identity: &str) -> Result<Self, EnvelopeError> {
        Self::parse(&format!("{}:{identity}", kind.as_str()))
    }

    pub fn parse(token: &str) -> Result<Self, EnvelopeError> {
        let (kind, identity) = token
            .split_once(':')
            .ok_or_else(|| EnvelopeError::Reference(token.into()))?;
        if kind.trim().is_empty() || identity.trim().is_empty() {
            return Err(EnvelopeError::Reference(token.into()));
        }
        Ok(Self {
            kind: kind.into(),
            identity: identity.into(),
        })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
}

impl fmt::Display for EventRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.identity)
    }
}

impl TryFrom<String> for EventRef {
    type Error = EnvelopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<EventRef> for String {
    fn from(value: EventRef) -> Self {
        value.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomeKind {
    Project,
    Person,
    Company,
}

/// Origin, never a consumer destination. Unknown and none make different claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EventHome {
    Project(ProjectSlug),
    Person(ProjectSlug),
    Company(ProjectSlug),
    None,
    Unknown,
}

impl EventHome {
    pub fn kind(&self) -> Option<HomeKind> {
        match self {
            Self::Project(_) => Some(HomeKind::Project),
            Self::Person(_) => Some(HomeKind::Person),
            Self::Company(_) => Some(HomeKind::Company),
            Self::None | Self::Unknown => None,
        }
    }
}

impl TryFrom<String> for EventHome {
    type Error = EnvelopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "none" => return Ok(Self::None),
            "unknown" => return Ok(Self::Unknown),
            _ => {}
        }
        let (kind, slug) = value
            .split_once(':')
            .ok_or_else(|| EnvelopeError::Home(value.clone()))?;
        let slug = ProjectSlug::parse(slug).map_err(|_| EnvelopeError::Home(value.clone()))?;
        match kind {
            "project" => Ok(Self::Project(slug)),
            "person" => Ok(Self::Person(slug)),
            "company" => Ok(Self::Company(slug)),
            _ => Err(EnvelopeError::Home(value)),
        }
    }
}

impl From<EventHome> for String {
    fn from(value: EventHome) -> Self {
        match value {
            EventHome::Project(slug) => format!("project:{slug}"),
            EventHome::Person(slug) => format!("person:{slug}"),
            EventHome::Company(slug) => format!("company:{slug}"),
            EventHome::None => "none".into(),
            EventHome::Unknown => "unknown".into(),
        }
    }
}

/// Ownership at emission; unavailable metadata stays unresolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EventHomeOwner {
    Company(ProjectSlug),
    Person(ProjectSlug),
    Ownerless,
    Unresolved,
    None,
}

impl TryFrom<String> for EventHomeOwner {
    type Error = EnvelopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "ownerless" => Ok(Self::Ownerless),
            "unresolved" => Ok(Self::Unresolved),
            "none" => Ok(Self::None),
            _ => match EventHome::try_from(value.clone())? {
                EventHome::Company(slug) => Ok(Self::Company(slug)),
                EventHome::Person(slug) => Ok(Self::Person(slug)),
                _ => Err(EnvelopeError::Owner(value)),
            },
        }
    }
}

impl From<EventHomeOwner> for String {
    fn from(value: EventHomeOwner) -> Self {
        match value {
            EventHomeOwner::Company(slug) => format!("company:{slug}"),
            EventHomeOwner::Person(slug) => format!("person:{slug}"),
            EventHomeOwner::Ownerless => "ownerless".into(),
            EventHomeOwner::Unresolved => "unresolved".into(),
            EventHomeOwner::None => "none".into(),
        }
    }
}
