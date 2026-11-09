//! Retained project dispositions. Signal classification belongs to the hunter;
//! this store compares the signal identity the caller explicitly supplies.
use instrukt_core::{
    fsx::{LockGuard, atomic_write},
    home::Home,
    ids::ProjectSlug,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suppression {
    pub id: String,
    pub project: ProjectSlug,
    pub signal: String,
    pub reason: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    schema_version: u32,
    suppressions: Vec<Suppression>,
}
#[derive(Debug, thiserror::Error)]
pub enum SuppressionError {
    #[error(
        "known-issue suppressions are unreadable; restore the retained set before classifying signals: {0}"
    )]
    Read(#[from] std::io::Error),
    #[error(
        "known-issue suppressions are invalid; restore the retained set before classifying signals: {0}"
    )]
    Parse(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Write(#[from] instrukt_core::fsx::FsError),
}
pub struct KnownIssues {
    file: PathBuf,
    project: ProjectSlug,
}
impl KnownIssues {
    pub fn new(home: &Home, project: &ProjectSlug) -> Self {
        Self {
            file: home
                .state_dir()
                .join("known-issues")
                .join(format!("{project}.json")),
            project: project.clone(),
        }
    }
    pub fn list(&self) -> Result<Vec<Suppression>, SuppressionError> {
        let text = match std::fs::read(&self.file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let retained: Retained = serde_json::from_slice(&text)?;
        let mut ids = std::collections::BTreeSet::new();
        if retained.schema_version != 1
            || retained.suppressions.iter().any(|record| {
                !ids.insert(record.id.clone())
                    || record.id
                        != instrukt_runner::events::occurrence_id(&[
                            self.project.as_str(),
                            &record.signal,
                        ])
                    || record.project != self.project
                    || record.signal.trim().is_empty()
                    || record.reason.trim().is_empty()
            })
        {
            return Err(SuppressionError::Invalid("unsupported or invalid retained suppression set; restore it before classifying signals".into()));
        }
        Ok(retained.suppressions)
    }
    pub fn disposition(&self, signal: &str) -> Result<Option<Suppression>, SuppressionError> {
        Ok(self
            .list()?
            .into_iter()
            .find(|record| record.signal == signal))
    }
    pub fn retain(&self, signal: &str, reason: &str) -> Result<Suppression, SuppressionError> {
        if signal.trim().is_empty() || reason.trim().is_empty() {
            return Err(SuppressionError::Invalid(
                "signal identity and reason must be nonblank".into(),
            ));
        }
        let _lock = LockGuard::acquire(&self.file.with_extension("lock"), Duration::from_secs(10))?;
        let mut records = self.list()?;
        let id = instrukt_runner::events::occurrence_id(&[self.project.as_str(), signal]);
        let record = Suppression {
            id: id.clone(),
            project: self.project.clone(),
            signal: signal.into(),
            reason: reason.into(),
        };
        records.retain(|existing| existing.id != id);
        records.push(record.clone());
        self.write(records)?;
        tracing::info!(event="known_issue_retained",project=%self.project,suppression=%record.id);
        Ok(record)
    }
    pub fn remove(&self, id: &str) -> Result<(), SuppressionError> {
        let _lock = LockGuard::acquire(&self.file.with_extension("lock"), Duration::from_secs(10))?;
        let mut records = self.list()?;
        if !records.iter().any(|record| record.id == id) {
            return Err(SuppressionError::Invalid(format!(
                "unknown suppression {id}; inspect `instrukt bugs suppressions`"
            )));
        }
        records.retain(|record| record.id != id);
        self.write(records)?;
        tracing::info!(event="known_issue_removed",project=%self.project,suppression=id);
        Ok(())
    }
    fn write(&self, suppressions: Vec<Suppression>) -> Result<(), SuppressionError> {
        atomic_write(
            &self.file,
            &serde_json::to_vec_pretty(&Retained {
                schema_version: 1,
                suppressions,
            })?,
        )?;
        Ok(())
    }
}
