//! The work record of one todo, as the local store keeps it and as its
//! GitHub record envelope carries it (`work` domain, "Todo" aggregate).

use std::path::PathBuf;

use instrukt_core::ids::{CommitSha, ProjectSlug, RoleName, SessionRef, TodoSlug};
use instrukt_core::machine::MachineId;
use instrukt_runner::interfaces::{AgentPreference, CatchUp, TaskSchedule, TodoKind};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::evidence::{
    Actor, ArtifactName, ContentDigest, Grounding, HoldRecord, StampFact, StampRecord, TodoStatus,
    Upstream,
};

/// The record schema this build writes and reads.
pub(crate) const SCHEMA: u32 = 3;

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

/// How a command addresses a todo (`todos`, "Identity and creation"): a
/// plain slug means the live item; `<slug>@<identity-prefix>` addresses
/// an exact entry, retained ones included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoAddress {
    pub slug: TodoSlug,
    /// A prefix of the durable identity (lowercase hex).
    pub identity: Option<String>,
}

/// The shortest identity prefix commands show for `<slug>@<prefix>`.
pub(crate) const SHOWN_PREFIX: usize = 8;

impl TodoAddress {
    /// The address of `record`'s exact entry.
    pub fn exact(record: &TodoRecord) -> Self {
        Self {
            slug: record.slug.clone(),
            identity: Some(record.id.as_str()[..SHOWN_PREFIX].to_owned()),
        }
    }

    /// Whether `record` is the entry this address names (for a plain slug:
    /// any entry with that slug).
    pub(crate) fn names(&self, record: &TodoRecord) -> bool {
        record.slug == self.slug
            && self
                .identity
                .as_deref()
                .is_none_or(|prefix| record.id.as_str().starts_with(prefix))
    }
}

impl std::fmt::Display for TodoAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.identity {
            Some(prefix) => write!(f, "{}@{prefix}", self.slug),
            None => write!(f, "{}", self.slug),
        }
    }
}

/// A todo address that is neither a slug nor `<slug>@<identity-prefix>`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a todo address: a slug, or `<slug>@<identity-prefix>` for a retained entry")]
pub struct AddressError(String);

impl std::str::FromStr for TodoAddress {
    type Err = AddressError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused = || AddressError(text.to_owned());
        let (slug, identity) = match text.split_once('@') {
            Some((slug, prefix)) => {
                let valid = !prefix.is_empty()
                    && prefix.len() <= 32
                    && prefix
                        .chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
                if !valid {
                    return Err(refused());
                }
                (slug, Some(prefix.to_owned()))
            }
            None => (text, None),
        };
        Ok(Self {
            slug: TodoSlug::parse(slug).map_err(|_| refused())?,
            identity,
        })
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prior_slugs: Vec<TodoSlug>,
    pub kind: TodoKind,
    pub status: TodoStatus,
    pub created: Timestamp,
    pub created_by: Actor,
    /// The audit-only `--human-involved` declaration of the filing act.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_involved: Option<bool>,
    /// Incremented by every accepted change; the remote record carries it.
    pub revision: u64,
    /// The last record-order number given to an artifact version or stamp.
    #[serde(default)]
    pub sequence: u64,
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
    /// Attributable GitHub label actions, shared through reflection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_actions: Vec<LabelAction>,
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

/// A processed inbound action; identity survives reconstruction on another computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelAction {
    pub issue: u64,
    pub id: u64,
    pub action: String,
    pub login: String,
    pub at: String,
    pub outcome: String,
}

/// A withdrawal: when, and the status restoration returns to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removal {
    pub at: Timestamp,
    pub from: TodoStatus,
    /// The working resources withdrawal released, kept as recovery
    /// evidence: the work branch and the commit it pointed at, and the
    /// worktrees removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<ReleasedBranch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worktrees: Vec<PathBuf>,
}

/// A work branch withdrawal deleted, and the commit it pointed at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasedBranch {
    pub name: String,
    pub head: CommitSha,
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

    /// The next record-order number, for an artifact version or stamp
    /// being written.
    pub(crate) fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    /// Gives record-order numbers to versions and stamps written before
    /// the record kept them: in time order, ties in stored order (versions
    /// of earlier-stored artifacts first, then stamps). Numbered records are
    /// left as they are.
    pub(crate) fn number_unsequenced(&mut self) {
        let unnumbered = self
            .artifacts
            .iter()
            .flat_map(|a| &a.versions)
            .any(|v| v.sequence == 0)
            || self.stamps.iter().any(|s| s.sequence == 0);
        if !unnumbered {
            return;
        }
        // (time, stored position, artifact index or stamp marker, index)
        let mut order: Vec<(Timestamp, usize, Option<usize>, usize)> = Vec::new();
        let mut position = 0;
        for (a, artifact) in self.artifacts.iter().enumerate() {
            for (v, version) in artifact.versions.iter().enumerate() {
                order.push((version.created, position, Some(a), v));
                position += 1;
            }
        }
        for (s, stamp) in self.stamps.iter().enumerate() {
            order.push((stamp.at, position, None, s));
            position += 1;
        }
        order.sort_by_key(|(at, position, _, _)| (*at, *position));
        for (number, (_, _, artifact, index)) in order.into_iter().enumerate() {
            let sequence = number as u64 + 1;
            match artifact {
                Some(a) => self.artifacts[a].versions[index].sequence = sequence,
                None => self.stamps[index].sequence = sequence,
            }
        }
        self.sequence = position as u64;
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
    /// Its place in the todo's record order.
    #[serde(default)]
    pub sequence: u64,
    /// What it was grounded on when written.
    #[serde(default)]
    pub grounding: Grounding,
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
    pub session: Option<SessionRef>,
    pub worktree: Option<PathBuf>,
    #[serde(flatten)]
    pub attempt: Attempt,
}

/// Immutable assignment facts and retained completion/failure evidence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub runtime: Option<instrukt_core::config::Runtime>,
    pub native_placement: Option<super::DriverPlacement>,
    pub assignment: instrukt_runner::interfaces::AssignmentKind,
    pub occurrence: Option<Timestamp>,
    #[serde(rename = "attempt")]
    pub number: u64,
    pub semantic_key: String,
    pub native_assignment: Option<super::NativeAssignment>,
    pub lane: Option<instrukt_runner::interfaces::Lane>,
    pub start_from: Option<String>,
    pub no_start_reason: Option<String>,
    pub branch: Option<String>,
    pub base_head: Option<CommitSha>,
    pub completion_sequence: Option<u64>,
    pub completed_head: Option<CommitSha>,
    pub failure_reason: Option<String>,
    pub failure_head: Option<CommitSha>,
    pub failure_workspace: Option<String>,
    pub failed_at: Option<Timestamp>,
    pub released_at: Option<Timestamp>,
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
    ContentDigest::of(content)
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
    fn records_written_before_sequencing_are_numbered_in_time_then_stored_order() {
        let at = |s: i64| Timestamp::from_second(s).unwrap();
        let version = |created: Timestamp| Version {
            digest: digest_of("x"),
            writer: Actor::Person { name: "dev".into() },
            created,
            derived_from: Vec::new(),
            sequence: 0,
            grounding: Grounding::default(),
            remote: None,
            content: None,
        };
        let stamp = |when: Timestamp| StampRecord {
            source_comment: None,
            fact: StampFact::ArchitectureSkipped { reason: "r".into() },
            at: when,
            actor: Actor::Person { name: "dev".into() },
            model: None,
            eval_fingerprint: None,
            role: None,
            human_involved: None,
            sequence: 0,
            model_provenance: Default::default(),
        };
        let mut record: TodoRecord = serde_json::from_value(serde_json::json!({
            "schema": 1, "id": "ab".repeat(16), "project": "demo", "slug": "depth",
            "kind": "change", "status": "active", "created": "2026-01-01T00:00:00Z",
            "created_by": {"person": {"name": "dev"}}, "revision": 1,
        }))
        .unwrap();
        record.artifacts.push(Artifact {
            name: ArtifactName::new("prd"),
            address: None,
            versions: vec![version(at(5)), version(at(9))],
        });
        record.stamps = vec![stamp(at(5)), stamp(at(1))];
        record.number_unsequenced();
        let versions: Vec<u64> = record.artifacts[0]
            .versions
            .iter()
            .map(|v| v.sequence)
            .collect();
        let stamps: Vec<u64> = record.stamps.iter().map(|s| s.sequence).collect();
        assert_eq!((versions, stamps), (vec![2, 4], vec![3, 1]));
        assert_eq!(record.next_sequence(), 5);
    }

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
