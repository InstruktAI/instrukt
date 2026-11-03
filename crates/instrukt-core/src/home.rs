//! The home: `~/.instrukt`, the fixed place on this computer holding
//! publication and state (architecture overview, "The home").
//!
//! Every path into the home is named here, so no context spells a home path
//! of its own. The home is always derived from the `HOME` environment
//! variable of the running process, which is what lets tests point a real
//! binary at a temporary home.

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
            Some(home) if !home.is_empty() => Ok(Self::at_user_home(PathBuf::from(home))),
            _ => Err(HomeError::HomeUnset),
        }
    }

    /// The home below an explicit user home directory.
    pub fn at_user_home(user_home: PathBuf) -> Self {
        let root = user_home.join(HOME_DIR_NAME);
        Self { user_home, root }
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

    /// `~/.instrukt/docs/` — the published tree.
    pub fn docs_dir(&self) -> PathBuf {
        self.root.join("docs")
    }

    /// `~/.instrukt/docs/<domain>/` — one published domain.
    pub fn domain_dir(&self, domain: &DomainName) -> PathBuf {
        self.docs_dir().join(domain.as_str())
    }

    /// `~/.instrukt/docs/snippets/` — published include fragments, named by
    /// their path below a project's `docs/global/snippets/` (`@include:
    /// docs/snippets/<name>.md` resolves to `<this>/<name>.md`).
    pub fn fragments_dir(&self) -> PathBuf {
        self.docs_dir().join("snippets")
    }

    /// `~/.instrukt/docs/third-party/` — published third-party docs, at
    /// their path below a project's `docs/third-party/`.
    pub fn third_party_dir(&self) -> PathBuf {
        self.docs_dir().join("third-party")
    }

    /// `~/.instrukt/docs/index.yaml` — the global index.
    pub fn index_file(&self) -> PathBuf {
        self.docs_dir().join("index.yaml")
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

    /// `~/.instrukt/state/hooks/<project>/` — each hook section's last run.
    pub fn hooks_state_dir(&self, project: &ProjectSlug) -> PathBuf {
        self.state_dir().join("hooks").join(project.as_str())
    }

    /// `~/.instrukt/state/runner/` — event log, cursors, validators,
    /// bindings, runner lock and last wake-up.
    pub fn runner_state_dir(&self) -> PathBuf {
        self.state_dir().join("runner")
    }

    /// `~/.instrukt/state/eval-fingerprint/` — per published commit.
    pub fn eval_fingerprint_dir(&self) -> PathBuf {
        self.state_dir().join("eval-fingerprint")
    }

    /// `~/.instrukt/state/blueprint/<project>/` — graphs per SHA.
    pub fn blueprint_dir(&self, project: &ProjectSlug) -> PathBuf {
        self.state_dir().join("blueprint").join(project.as_str())
    }

    /// `~/.instrukt/state/work/` — the local work record.
    pub fn work_state_dir(&self) -> PathBuf {
        self.state_dir().join("work")
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
            home.fragments_dir(),
            home.third_party_dir(),
            home.publication_state_dir(),
            home.hooks_state_dir(&project),
            home.runner_state_dir(),
            home.eval_fingerprint_dir(),
            home.blueprint_dir(&project),
            home.work_state_dir(),
            home.git_dir(),
            home.locks_dir(),
        ] {
            assert!(path.starts_with("/u/.instrukt"), "{}", path.display());
        }
    }
}
