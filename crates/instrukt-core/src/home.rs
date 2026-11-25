//! The home: `~/.instrukt`, the fixed place on this computer holding
//! publication and state (architecture overview, "The home").
//!
//! Every path into the home is named here, so no context spells a home path
//! of its own. `INSTRUKT_HOME` selects an isolated root; otherwise the root
//! sits below `HOME`. Secrets belong to the selected Instrukt root.

use std::path::{Path, PathBuf};

use crate::ids::{DomainName, ProjectSlug};

/// The directory name of the home below the user's home directory.
pub const HOME_DIR_NAME: &str = ".instrukt";

/// The home could not be located.
#[derive(Debug, thiserror::Error)]
pub enum HomeError {
    #[error("HOME is not set; instrukt locates its home at $HOME/{HOME_DIR_NAME}")]
    HomeUnset,
    #[error("development execution requires a distinct INSTRUKT_HOME")]
    DevelopmentNeedsIsolation,
}

/// Paths into one home. Holds no open resources; creating one touches
/// nothing on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    user_home: PathBuf,
    root: PathBuf,
    state_dir: PathBuf,
    machine_user_home: PathBuf,
    development: bool,
    inspection: bool,
    state_read_only: bool,
}

// Resolve an existing ancestor even when first use has not created the
// state directory yet.
fn canonical_location(path: &Path) -> PathBuf {
    match path.canonicalize() {
        Ok(path) => path,
        Err(_) => match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => canonical_location(parent).join(name),
            _ => path.to_path_buf(),
        },
    }
}

impl Home {
    /// The home of the user whose `HOME` the process runs with.
    pub fn from_env() -> Result<Self, HomeError> {
        match std::env::var_os("HOME") {
            Some(home) if !home.is_empty() => {
                let mut selected = Self::at_user_home(PathBuf::from(home));
                if let Some(root) = std::env::var_os("INSTRUKT_HOME").filter(|s| !s.is_empty()) {
                    selected.root = PathBuf::from(root);
                    selected.state_dir = canonical_location(&selected.root.join("state"));
                }
                if std::env::var_os("INSTRUKT_DEVELOPMENT").as_deref()
                    == Some(std::ffi::OsStr::new("1"))
                {
                    if selected.root == selected.machine_user_home.join(HOME_DIR_NAME) {
                        return Err(HomeError::DevelopmentNeedsIsolation);
                    }
                    selected.user_home = selected.root.join("user");
                    selected.development = true;
                }
                Ok(selected)
            }
            _ => Err(HomeError::HomeUnset),
        }
    }

    /// The home below an explicit user home directory.
    pub fn at_user_home(user_home: PathBuf) -> Self {
        let root = user_home.join(HOME_DIR_NAME);
        Self {
            machine_user_home: user_home.clone(),
            user_home,
            state_dir: canonical_location(&root.join("state")),
            root,
            development: false,
            inspection: false,
            state_read_only: false,
        }
    }

    /// Select nonmutating owned-configuration reads for this invocation.
    pub fn for_inspection(&self) -> Self {
        let mut home = self.clone();
        home.inspection = true;
        home.state_read_only = true;
        home
    }
    pub fn for_writing(&self) -> Self {
        let mut home = self.clone();
        home.inspection = false;
        home.state_read_only = false;
        home
    }
    pub fn for_configuration_read(&self) -> Self {
        let mut home = self.clone();
        home.inspection = true;
        home.state_read_only = false;
        home
    }
    pub fn is_state_read_only(&self) -> bool {
        self.state_read_only
    }
    pub fn is_inspection(&self) -> bool {
        self.inspection
    }

    pub fn machine_user_home(&self) -> &Path {
        &self.machine_user_home
    }
    pub fn is_development(&self) -> bool {
        self.development
    }

    pub fn secrets_file(&self) -> PathBuf {
        self.root.join("secrets.env")
    }

    pub fn agents_file(&self) -> PathBuf {
        self.root.join("agents.yaml")
    }

    pub fn agents_deployment_file(&self) -> PathBuf {
        self.state_dir().join("agents-deployment.json")
    }

    /// The user-scope output root: the machine user's home in installed use,
    /// or the private user directory inside the development home.
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

    /// `~/.instrukt/companies/` — one folder per company home on this
    /// computer (`company-homes`).
    pub fn companies_dir(&self) -> PathBuf {
        self.root.join("companies")
    }

    /// `~/.instrukt/companies/<name>/` — one company's home.
    pub fn company_dir(&self, company: &ProjectSlug) -> PathBuf {
        self.companies_dir().join(company.as_str())
    }

    /// `~/.instrukt/companies/<name>/secrets.env` — the company's own
    /// protected secrets.
    pub fn company_secrets_file(&self, company: &ProjectSlug) -> PathBuf {
        self.company_dir(company).join("secrets.env")
    }

    /// `~/.instrukt/docs/` — the published tree: `general/` and one
    /// directory per published domain, beside `project/<slug>/` and third-party docs.
    pub fn docs_dir(&self) -> PathBuf {
        self.root.join("docs")
    }

    /// `~/.instrukt/docs/<domain>/` — one published position: a domain's,
    /// or `general/`.
    pub fn domain_dir(&self, domain: &DomainName) -> PathBuf {
        self.docs_dir().join(domain.as_str())
    }

    /// The committed project position in the published store.
    pub fn project_docs_dir(&self, project: &ProjectSlug) -> PathBuf {
        self.docs_dir().join("project").join(project.as_str())
    }

    /// `~/.instrukt/docs/third-party/` — published third-party docs, at
    /// their path below a project's `docs/third-party/`.
    pub fn third_party_dir(&self) -> PathBuf {
        self.docs_dir().join("third-party")
    }

    /// `~/.instrukt/state/index.yaml` — the global index.
    /// Publication keeps these paths relative to the original home root so
    /// its confinement check can inspect the original path components.
    pub fn index_file(&self) -> PathBuf {
        self.root.join("state").join("index.yaml")
    }

    /// `~/.instrukt/state/`.
    pub fn state_dir(&self) -> PathBuf {
        self.state_dir.clone()
    }

    /// `~/.instrukt/state/publication/` — per project, the published commit
    /// and checkout path.
    pub fn publication_state_dir(&self) -> PathBuf {
        self.root.join("state").join("publication")
    }

    /// `~/.instrukt/state/publication/claims/` — the ownership record: per
    /// published position, the project that owns it on this computer.
    pub fn claims_dir(&self) -> PathBuf {
        self.publication_state_dir().join("claims")
    }

    /// `~/.instrukt/state/hook-runs/<key>/` — each hook section's last run
    /// in one repository, keyed by its git common directory.
    pub fn hook_runs_dir(&self, key: &str) -> PathBuf {
        self.state_dir().join("hook-runs").join(key)
    }

    /// `~/.instrukt/state/hooks/` — where an earlier release kept hook runs
    /// per project; the first hook run that finds it removes it.
    pub fn retired_hook_records_dir(&self) -> PathBuf {
        self.state_dir().join("hooks")
    }

    /// `~/.instrukt/state/instrukt.db` — the state database: the local
    /// work record and the runner's state (`state-database`).
    pub fn state_db_file(&self) -> PathBuf {
        self.state_dir().join("instrukt.db")
    }

    /// The Redis socket address keeps the home spelling. A short home
    /// alias can fit the Unix socket address limit when the resolved path does not.
    pub fn redis_socket(&self) -> PathBuf {
        self.root.join("state/redis/redis.sock")
    }

    /// `~/.instrukt/state/writer.lock` — the per-computer writer lock every
    /// state-database write runs under.
    pub fn writer_lock_file(&self) -> PathBuf {
        self.state_dir().join("writer.lock")
    }

    /// `~/.instrukt/state/reflection.lock` — the per-computer reflection
    /// lock: GitHub reflection holds it across its requests and takes the
    /// writer lock only for each local transaction recording what it learned
    /// (lock order reflection → writer).
    pub fn reflection_lock_file(&self) -> PathBuf {
        self.state_dir().join("reflection.lock")
    }

    /// `~/.instrukt/state/withdrawal.lock` — held by a withdrawal while its
    /// recorded intent is open, so a withdrawal in progress is told apart
    /// from an interrupted one; taken before the writer lock and never while
    /// it is held.
    pub fn withdrawal_lock_file(&self) -> PathBuf {
        self.state_dir().join("withdrawal.lock")
    }

    /// `~/.instrukt/state/delivered-release.lock` — held by work for the
    /// length of one release of a delivered todo's worktree and work branch,
    /// across its git and network steps, outside the writer lock; taken
    /// without waiting, so one such release runs at a time.
    pub fn delivered_release_lock_file(&self) -> PathBuf {
        self.state_dir().join("delivered-release.lock")
    }

    /// `~/.instrukt/state/runner.lock` — serializes runner invocations; it
    /// guards no data.
    pub fn runner_lock_file(&self) -> PathBuf {
        self.state_dir().join("runner.lock")
    }

    /// `~/.instrukt/state/runner-hosts.json` — the connectors the running
    /// `instrukt run` hosts, published by that run.
    pub fn runner_hosts_file(&self) -> PathBuf {
        self.state_dir().join("runner-hosts.json")
    }

    /// `~/.instrukt/state/runner-service.json` — an earlier release's record
    /// of what the runner service executed; the service deletes it when it
    /// starts, and nothing reads it.
    pub fn runner_service_file(&self) -> PathBuf {
        self.state_dir().join("runner-service.json")
    }

    /// `~/.instrukt/state/history.lock` — the history store's
    /// write-isolation unit: every history write runs under it, and once the
    /// one-time move of earlier history out of the state database is
    /// complete, none takes the state database's writer lock.
    pub fn history_lock_file(&self) -> PathBuf {
        self.state_dir().join("history.lock")
    }

    /// `~/.instrukt/state/history.db` — the history store: retained
    /// conversations, their index and extraction positions, kept apart from
    /// the state database (`project/instrukt/spec/feature/runner/history`).
    pub fn history_db_file(&self) -> PathBuf {
        self.state_dir().join("history.db")
    }

    /// `~/.instrukt/state/review.lock` — the write-isolation unit of the
    /// review evidence store, taken only by the commands that write it
    /// (`project/instrukt/spec/feature/work/github-apps`, Review evidence
    /// storage).
    pub fn review_lock_file(&self) -> PathBuf {
        self.state_dir().join("review.lock")
    }

    /// `~/.instrukt/state/review.db` — the review evidence store: head
    /// verdicts with their versions, exemptions and imported derived
    /// approvals.
    pub fn review_db_file(&self) -> PathBuf {
        self.state_dir().join("review.db")
    }

    /// `~/.instrukt/state/review-publication.lock` — the write-isolation unit
    /// of the review publication store, taken by the tick alone
    /// (`project/instrukt/spec/feature/work/github-apps`, Review evidence
    /// storage).
    pub fn review_publication_lock_file(&self) -> PathBuf {
        self.state_dir().join("review-publication.lock")
    }

    /// `~/.instrukt/state/review-publication.db` — what GitHub shows of each
    /// verdict, recognised verdicts and derived approvals.
    pub fn review_publication_db_file(&self) -> PathBuf {
        self.state_dir().join("review-publication.db")
    }

    /// `~/.instrukt/state/review-observation.lock` — the write-isolation unit
    /// of the pull-request observation store, taken by the tick alone
    /// (`project/instrukt/spec/feature/runner/pull-request-review`,
    /// Observation store).
    pub fn review_observation_lock_file(&self) -> PathBuf {
        self.state_dir().join("review-observation.lock")
    }

    /// `~/.instrukt/state/review-observation.db` — the pages of each opted-in
    /// project's pull-request listing with their conditional validators.
    pub fn review_observation_db_file(&self) -> PathBuf {
        self.state_dir().join("review-observation.db")
    }

    /// `~/.instrukt/state/eval-fingerprint/` — per published commit.
    pub fn eval_fingerprint_dir(&self) -> PathBuf {
        self.root.join("state").join("eval-fingerprint")
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

    /// `~/.instrukt/state/publication/outputs/` — where an earlier release
    /// kept a record per project; the first sync that publishes a project
    /// deletes its record there unparsed.
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
            home.company_dir(&project),
            home.company_secrets_file(&project),
            home.domain_dir(&domain),
            home.index_file(),
            home.third_party_dir(),
            home.publication_state_dir(),
            home.publication_outputs_dir(),
            home.claims_dir(),
            home.hook_runs_dir("0123456789abcdef"),
            home.retired_hook_records_dir(),
            home.state_db_file(),
            home.writer_lock_file(),
            home.reflection_lock_file(),
            home.withdrawal_lock_file(),
            home.delivered_release_lock_file(),
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
