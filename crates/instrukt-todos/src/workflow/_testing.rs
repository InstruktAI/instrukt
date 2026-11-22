//! Test fixtures shared by workflow's colocated tests.

use std::path::PathBuf;

use instrukt_core::ids::{CommitSha, ProjectSlug, TodoSlug};
use tempfile::TempDir;

/// The sdlc flavor's authoring source in this repository.
pub(crate) fn sdlc_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/domain/sdlc/workflow")
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

pub(crate) fn sha(fill: char) -> CommitSha {
    CommitSha::parse(&fill.to_string().repeat(40)).unwrap()
}

pub(crate) struct EvidenceBuilder {
    evidence: crate::evidence::TodoEvidence,
    tick: i64,
}

pub(crate) fn todo(kind: instrukt_runner::interfaces::TodoKind) -> EvidenceBuilder {
    EvidenceBuilder {
        evidence: crate::evidence::TodoEvidence {
            branch: None,
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
        jiff::Timestamp::from_second(1_800_000_000 + self.tick).unwrap()
    }

    pub(crate) fn artifact(mut self, name: &str, value: &str) -> Self {
        use crate::evidence::{Actor, ArtifactHistory, ArtifactName, ArtifactVersion};
        let version = ArtifactVersion {
            digest: digest(value),
            writer: Actor::Session(session("writer")),
            created: self.next(),
            sequence: u64::try_from(self.tick).unwrap_or_default(),
            grounding: Default::default(),
            derived_from: Vec::new(),
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

    pub(crate) fn claimed(mut self) -> Self {
        self.evidence.claim = Some(crate::evidence::ClaimRecord {
            session: Some(session("orchestrator")),
            at: jiff::Timestamp::from_second(1_800_000_000).unwrap(),
        });
        self
    }

    pub(crate) fn build(self) -> crate::evidence::TodoEvidence {
        self.evidence
    }
}
