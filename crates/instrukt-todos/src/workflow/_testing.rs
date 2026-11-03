//! Test fixtures shared by the engine's colocated tests.

use std::path::PathBuf;

use instrukt_core::ids::{CommitSha, ProjectSlug, TodoSlug};
use tempfile::TempDir;

use super::_derive::Subject;
use super::_facts::{FactValue, Facts};
use super::_render::ConfigLookupError;

/// The sdlc flavor's authoring source in this repository.
pub(crate) fn sdlc_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/global/sdlc/workflow")
}

/// A flavor directory holding `files` (relative path, content).
pub(crate) fn flavor_dir(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    dir
}

pub(crate) fn subject(facts: Facts) -> Subject {
    Subject {
        project: ProjectSlug::parse("demo").unwrap(),
        slug: TodoSlug::parse("add-login").unwrap(),
        facts,
    }
}

pub(crate) fn facts(entries: &[(&str, FactValue)]) -> Facts {
    let mut facts = Facts::new();
    for (name, value) in entries {
        facts.insert(name, value.clone()).unwrap();
    }
    facts
}

pub(crate) fn text(value: &str) -> FactValue {
    FactValue::Text(value.to_owned())
}

pub(crate) fn sha(fill: char) -> CommitSha {
    CommitSha::parse(&fill.to_string().repeat(40)).unwrap()
}

/// Configuration with nothing in it.
pub(crate) fn no_config(_: &str) -> Result<Option<String>, ConfigLookupError> {
    Ok(None)
}

/// Configuration inside a runtime session `sess-1`.
pub(crate) fn session_config(path: &str) -> Result<Option<String>, ConfigLookupError> {
    Ok((path == "runtime.session_id").then(|| "sess-1".to_owned()))
}

/// Builds a todo's evidence in record order: each artifact version and
/// stamp takes the next record sequence and is made one second after the
/// previous one, or, on a frozen clock, at the same instant.
pub(crate) struct EvidenceBuilder {
    evidence: crate::evidence::TodoEvidence,
    tick: i64,
    frozen: bool,
}

/// The evidence of a fresh active todo `add-login` of `kind` in `demo`.
pub(crate) fn todo(kind: instrukt_runner::interfaces::TodoKind) -> EvidenceBuilder {
    EvidenceBuilder {
        evidence: crate::evidence::TodoEvidence {
            id: crate::work::TodoId::parse(&"ab".repeat(16)).unwrap(),
            project: ProjectSlug::parse("demo").unwrap(),
            slug: TodoSlug::parse("add-login").unwrap(),
            kind,
            status: crate::evidence::TodoStatus::Active,
            artifacts: Vec::new(),
            stamps: Vec::new(),
            claim: None,
            bindings: Vec::new(),
            hold: None,
            prerequisites: Vec::new(),
            delivery: None,
            timing: None,
            agent: None,
            implementation_plan_seeded: false,
        },
        tick: 0,
        frozen: false,
    }
}

pub(crate) fn session(id: &str) -> instrukt_core::ids::SessionRef {
    instrukt_core::ids::SessionRef::parse(&format!("cli:{id}")).unwrap()
}

pub(crate) fn digest(value: &str) -> crate::evidence::ContentDigest {
    crate::evidence::ContentDigest::new(value)
}

impl EvidenceBuilder {
    fn next(&mut self) -> jiff::Timestamp {
        self.tick += 1;
        let offset = if self.frozen { 0 } else { self.tick };
        jiff::Timestamp::from_second(1_800_000_000 + offset).unwrap()
    }

    /// Every later mutation is made at one instant, as under `INSTRUKT_NOW`.
    pub(crate) fn frozen(mut self) -> Self {
        self.frozen = true;
        self
    }

    /// Saves a new version of artifact `name` with digest `value`, grounded
    /// on `grounding`.
    pub(crate) fn grounded(
        mut self,
        name: &str,
        value: &str,
        grounding: crate::evidence::Grounding,
    ) -> Self {
        self = self.derived(name, value, &[]);
        if let Some(version) = self
            .evidence
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.name.as_str() == name)
            .and_then(|artifact| artifact.versions.last_mut())
        {
            version.grounding = grounding;
        }
        self
    }

    /// Saves a new version of artifact `name` with digest `value`.
    pub(crate) fn artifact(self, name: &str, value: &str) -> Self {
        self.derived(name, value, &[])
    }

    /// Saves a new version of artifact `name` with digest `value`, derived
    /// from `upstream`: each upstream artifact with the digest it had
    /// then (`None` when it did not exist).
    pub(crate) fn derived(
        mut self,
        name: &str,
        value: &str,
        upstream: &[(&str, Option<&str>)],
    ) -> Self {
        use crate::evidence::{Actor, ArtifactHistory, ArtifactName, ArtifactVersion, Upstream};
        let version = ArtifactVersion {
            digest: digest(value),
            writer: Actor::Session(session("writer")),
            created: self.next(),
            sequence: u64::try_from(self.tick).unwrap_or_default(),
            grounding: Default::default(),
            derived_from: upstream
                .iter()
                .map(|(artifact, value)| Upstream {
                    artifact: ArtifactName::new(*artifact),
                    digest: value.map(digest),
                })
                .collect(),
        };
        match self
            .evidence
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.name.as_str() == name)
        {
            Some(artifact) => artifact.versions.push(version),
            None => self.evidence.artifacts.push(ArtifactHistory {
                name: ArtifactName::new(name),
                versions: vec![version],
            }),
        }
        self
    }

    /// Records stamp `fact`.
    pub(crate) fn stamp(mut self, fact: crate::evidence::StampFact) -> Self {
        let at = self.next();
        self.evidence.stamps.push(crate::evidence::StampRecord {
            fact,
            at,
            actor: crate::evidence::Actor::Session(session("stamper")),
            model: None,
            eval_fingerprint: None,
            role: None,
            human_involved: None,
            sequence: u64::try_from(self.tick).unwrap_or_default(),
            model_provenance: Default::default(),
        });
        self
    }

    pub(crate) fn claimed(mut self) -> Self {
        self.evidence.claim = Some(crate::evidence::ClaimRecord {
            session: Some(session("orchestrator")),
            at: jiff::Timestamp::from_second(1_800_000_000).unwrap(),
        });
        self
    }

    /// Binds session `id` for `role`, retaining `worktree` when given.
    pub(crate) fn bound(mut self, role: &str, id: &str, worktree: Option<&str>) -> Self {
        self.evidence.bindings.push(crate::evidence::RoleBinding {
            role: instrukt_core::ids::RoleName::parse(role).unwrap(),
            binding: instrukt_runner::interfaces::SessionBinding {
                session: session(id),
                worktree: worktree.map(PathBuf::from),
            },
        });
        self
    }

    /// The filer supplied the implementation plan with the filing.
    pub(crate) fn seeded(mut self) -> Self {
        self.evidence.implementation_plan_seeded = true;
        self
    }

    pub(crate) fn status(mut self, status: crate::evidence::TodoStatus) -> Self {
        self.evidence.status = status;
        self
    }

    pub(crate) fn build(self) -> crate::evidence::TodoEvidence {
        self.evidence
    }
}
