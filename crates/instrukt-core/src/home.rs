//! The home: `~/.instrukt`, the fixed place on this computer holding
//! publication and state (architecture overview, "The home").
//!
//! Every path into the home is named here, so no context spells a home path
//! of its own. `INSTRUKT_HOME` selects an isolated root; otherwise the root
//! sits below `HOME`. Machine secrets remain below `HOME` in either case.

use std::path::{Path, PathBuf};

use crate::ids::{DomainName, ProjectSlug};

/// The directory name of the home below the user's home directory.
pub const HOME_DIR_NAME: &str = ".instrukt";

/// The home could not be located.
#[derive(Debug, thiserror::Error)]
pub enum HomeError {
    #[error("HOME is not set; instrukt locates its home at $HOME/{HOME_DIR_NAME}")]
    HomeUnset,
}

/// Paths into one home. Holds no open resources; creating one touches
/// nothing on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    user_home: PathBuf,
    root: PathBuf,
}

impl Home {
    /// The home of the user whose `HOME` the process runs with.
    pub fn from_env() -> Result<Self, HomeError> {
        match std::env::var_os("HOME") {
            Some(home) if !home.is_empty() => {
                let mut selected = Self::at_user_home(PathBuf::from(home));
                if let Some(root) = std::env::var_os("INSTRUKT_HOME").filter(|s| !s.is_empty()) {
                    selected.root = PathBuf::from(root);
                }
                Ok(selected)
            }
            _ => Err(HomeError::HomeUnset),
        }
    }

    /// The home below an explicit user home directory.
    pub fn at_user_home(user_home: PathBuf) -> Self {
        let root = user_home.join(HOME_DIR_NAME);
        Self { user_home, root }
    }

    pub fn secrets_file(&self) -> PathBuf {
        self.user_home.join(HOME_DIR_NAME).join("secrets.env")
    }

    pub fn agents_file(&self) -> PathBuf {
        self.root.join("agents.yaml")
    }

    pub fn agents_deployment_file(&self) -> PathBuf {
        self.state_dir().join("agents-deployment.json")
    }

    /// The user's home directory (`$HOME`), where agent CLIs keep their
    /// user-scope entrypoints and settings.
    pub fn user_home(&self) -> &Path {
        &self.user_home
    }

    /// `~/.instrukt`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `~/.instrukt/instrukt.yaml` — the global configuration layer.
    pub fn global_config(&self) -> PathBuf {
        self.root.join("instrukt.yaml")
    }

    /// `~/.instrukt/docs/` — the published tree: `general/` and one
    /// directory per published domain, beside the third-party docs.
    pub fn docs_dir(&self) -> PathBuf {
        self.root.join("docs")
    }

    /// `~/.instrukt/docs/<domain>/` — one published position: a domain's,
    /// or `general/`.
    pub fn domain_dir(&self, domain: &DomainName) -> PathBuf {
        self.docs_dir().join(domain.as_str())
    }

    /// `~/.instrukt/docs/third-party/` — published third-party docs, at
    /// their path below a project's `docs/third-party/`.
    pub fn third_party_dir(&self) -> PathBuf {
        self.docs_dir().join("third-party")
    }

    /// `~/.instrukt/state/index.yaml` — the global index.
    pub fn index_file(&self) -> PathBuf {
        self.state_dir().join("index.yaml")
    }

    /// `~/.instrukt/state/`.
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    /// `~/.instrukt/state/publication/` — per project, the published commit
    /// and checkout path.
    pub fn publication_state_dir(&self) -> PathBuf {
        self.state_dir().join("publication")
    }

    /// `~/.instrukt/state/publication/claims/` — the ownership record: per
    /// published position, the project that owns it on this computer.
    pub fn claims_dir(&self) -> PathBuf {
        self.publication_state_dir().join("claims")
    }

    /// `~/.instrukt/state/hooks/<project>/` — each hook section's last run.
    pub fn hooks_state_dir(&self, project: &ProjectSlug) -> PathBuf {
        self.state_dir().join("hooks").join(project.as_str())
    }

    /// `~/.instrukt/state/instrukt.db` — the state database: the local
    /// work record and the runner's state (`state-database`).
    pub fn state_db_file(&self) -> PathBuf {
        self.state_dir().join("instrukt.db")
    }

    /// `~/.instrukt/state/writer.lock` — the per-computer writer lock every
    /// state-database write runs under.
    pub fn writer_lock_file(&self) -> PathBuf {
        self.state_dir().join("writer.lock")
    }

    /// `~/.instrukt/state/runner.lock` — serializes runner invocations; it
    /// guards no data.
    pub fn runner_lock_file(&self) -> PathBuf {
        self.state_dir().join("runner.lock")
    }

    /// `~/.instrukt/state/eval-fingerprint/` — per published commit.
    pub fn eval_fingerprint_dir(&self) -> PathBuf {
        self.state_dir().join("eval-fingerprint")
    }

    /// `~/.instrukt/state/blueprint/<project>/` — graphs per SHA.
    pub fn blueprint_dir(&self, project: &ProjectSlug) -> PathBuf {
        self.state_dir().join("blueprint").join(project.as_str())
    }

    /// `~/.instrukt/state/machine-id` — this computer's id, created once;
    /// the holder of its execution claims.
    pub fn machine_id_file(&self) -> PathBuf {
        self.state_dir().join("machine-id")
    }

    /// `~/.instrukt/git/` — ingested external repositories.
    pub fn git_dir(&self) -> PathBuf {
        self.root.join("git")
    }

    /// `~/.instrukt/locks/` — home-wide lock files (the docs lock, the
    /// runner lock is kept in the runner's state directory).
    pub fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    /// `~/.instrukt/state/publication/outputs/` — per project, the files
    /// outside the home its last publication wrote (compiled `AGENTS.md`
    /// files, global agent entrypoints), so a later sync can retract them.
    pub fn publication_outputs_dir(&self) -> PathBuf {
        self.publication_state_dir().join("outputs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_lies_inside_the_home() {
        let home = Home::at_user_home(PathBuf::from("/u"));
        let project = ProjectSlug::parse("demo").unwrap();
        let domain = DomainName::parse("sdlc").unwrap();
        for path in [
            home.global_config(),
            home.domain_dir(&domain),
            home.index_file(),
            home.third_party_dir(),
            home.publication_state_dir(),
            home.publication_outputs_dir(),
            home.claims_dir(),
            home.hooks_state_dir(&project),
            home.state_db_file(),
            home.writer_lock_file(),
            home.runner_lock_file(),
            home.eval_fingerprint_dir(),
            home.blueprint_dir(&project),
            home.machine_id_file(),
            home.git_dir(),
            home.locks_dir(),
        ] {
            assert!(path.starts_with("/u/.instrukt"), "{}", path.display());
        }
    }
}
