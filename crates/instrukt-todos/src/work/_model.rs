//! The work record of one todo, as the local store keeps it and as its
//! GitHub record envelope carries it (`work` domain, "Todo" aggregate).

use std::path::PathBuf;

use instrukt_core::ids::{CommitSha, ProjectSlug, RoleName, SessionRef, TodoSlug};
use instrukt_core::machine::MachineId;
use instrukt_runner::interfaces::{AgentPreference, CatchUp, TaskSchedule, TodoKind};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::evidence::{
    Actor, ArtifactName, ContentDigest, HoldRecord, StampFact, StampRecord, TodoStatus, Upstream,
};

/// The record schema this build writes and reads.
pub(crate) const SCHEMA: u32 = 1;

/// A todo's durable identity: survives rename, removal and recovery.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TodoId(String);

impl TodoId {
    pub(crate) fn new(hex: String) -> Self {
        Self(hex)
    }

    /// Validates `value` as an identity: 32 lowercase hex characters.
    pub fn parse(value: &str) -> Option<Self> {
        let ok = value.len() == 32
            && value
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        ok.then(|| Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TodoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A reference to another todo of the project: its identity, and the slug
/// it was last known under (renames update references).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoRef {
    pub id: TodoId,
    pub slug: TodoSlug,
}

/// One todo's record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoRecord {
    pub schema: u32,
    pub id: TodoId,
    pub project: ProjectSlug,
    pub slug: TodoSlug,
    pub kind: TodoKind,
    pub status: TodoStatus,
    pub created: Timestamp,
    pub created_by: Actor,
    /// The audit-only `--human-involved` declaration of the filing act.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_involved: Option<bool>,
    /// Incremented by every accepted change; the remote record carries it.
    pub revision: u64,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// The stamp ledger, oldest first.
    #[serde(default)]
    pub stamps: Vec<StampRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<Claim>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<HoldRecord>,
    /// Released holds, oldest first: the hold audit.
    #[serde(default)]
    pub hold_history: Vec<ReleasedHold>,
    /// Prerequisites.
    #[serde(default)]
    pub after: Vec<TodoRef>,
    /// The structural holder this todo is grouped under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<TodoRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<TaskSchedule>,
    #[serde(default)]
    pub catch_up: CatchUp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentPreference>,
    /// The work branch recorded at build start, for the linked pull request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
    /// The withdrawal, while the todo is removed: starts its recovery
    /// window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removal: Option<Removal>,
}

/// A withdrawal: when, and the status restoration returns to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removal {
    pub at: Timestamp,
    pub from: TodoStatus,
}

impl TodoRecord {
    pub fn reference(&self) -> TodoRef {
        TodoRef {
            id: self.id.clone(),
            slug: self.slug.clone(),
        }
    }

    pub fn artifact(&self, name: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| a.name.as_str() == name)
    }

    /// The current version's digest of the artifact `name`.
    pub fn current_digest(&self, name: &str) -> Option<&ContentDigest> {
        self.artifact(name)
            .and_then(|a| a.versions.last())
            .map(|v| &v.digest)
    }

    /// Whether the todo is addressable by its slug: active, iceboxed or a
    /// structural holder.
    pub fn is_live(&self) -> bool {
        matches!(
            self.status,
            TodoStatus::Active | TodoStatus::Iceboxed | TodoStatus::StructuralHolder
        )
    }

    /// Whether implementation has started (a build-start stamp exists).
    pub fn implementation_started(&self) -> bool {
        self.stamps
            .iter()
            .any(|s| matches!(s.fact, StampFact::BuildStarted { .. }))
    }

    /// The record as the GitHub envelope carries it: artifact contents are
    /// published as native artifacts, never inside the envelope.
    pub(crate) fn without_contents(&self) -> Self {
        let mut copy = self.clone();
        for artifact in &mut copy.artifacts {
            for version in &mut artifact.versions {
                version.content = None;
            }
        }
        copy
    }
}

/// Named work evidence with its retained versions, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub name: ArtifactName,
    /// The provider address (the issue artifact's number) once reflected:
    /// the artifact's identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<u64>,
    pub versions: Vec<Version>,
}

/// One saved version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub digest: ContentDigest,
    pub writer: Actor,
    pub created: Timestamp,
    /// The upstream artifacts as they stood when this version was written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_from: Vec<Upstream>,
    /// The provider's version number once uploaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<u64>,
    /// The content, where this computer holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// The item-level execution claim: held by a computer, through a session
/// when one claimed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub machine: MachineId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRef>,
    pub at: Timestamp,
}

/// A started session recorded per todo and role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub role: RoleName,
    pub session: SessionRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<PathBuf>,
}

/// A released dispatch hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasedHold {
    pub hold: HoldRecord,
    pub released: Timestamp,
    pub by: Actor,
}

/// Confirmed completion evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub commit: CommitSha,
    pub at: Timestamp,
    pub by: Actor,
}

/// The digest of an artifact version's content: `sha256:<hex>`.
pub(crate) fn digest_of(content: &str) -> ContentDigest {
    ContentDigest::new(format!(
        "sha256:{}",
        hex::encode(Sha256::digest(content.as_bytes()))
    ))
}

/// The artifact name the record accepts: lowercase letters, digits, `_`
/// and `-`, starting with a letter.
pub(crate) fn valid_artifact_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_name_their_algorithm_and_change_with_content() {
        let a = digest_of("one");
        assert_eq!(a, digest_of("one"));
        assert_ne!(a, digest_of("two"));
        assert_eq!(a.as_str().len(), "sha256:".len() + 64);
    }

    #[test]
    fn artifact_names_are_lowercase_words() {
        assert!(valid_artifact_name("implementation_plan"));
        assert!(valid_artifact_name("log-2"));
        for bad in ["", "Plan", "2nd", "a b", "../x"] {
            assert!(!valid_artifact_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn identities_are_32_hex_characters() {
        assert!(TodoId::parse(&"a".repeat(32)).is_some());
        assert!(TodoId::parse(&"a".repeat(31)).is_none());
        assert!(TodoId::parse(&"A".repeat(32)).is_none());
    }
}
