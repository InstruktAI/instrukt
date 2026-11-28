//! Queues durable verdict notices (`runtime_inputs`, kind `verdict`) for every
//! session that asked for a requested review of a head, once work has recorded
//! a verdict version (`requested-review`, Returning the outcome).

use std::collections::BTreeMap;
use std::path::PathBuf;

use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use instrukt_core::state_db::StoreError;
use jiff::Timestamp;

use crate::inputs::{self, Entry, InputKind, Intent};
use crate::interfaces::HeadVerdictValue;
use crate::state::{RunnerState, StateError};

use super::_store;

/// A verdict version work has recorded, handed to the runner so it can queue
/// notices for sessions that asked for a requested review of the head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedVersion {
    pub project: ProjectSlug,
    pub head: CommitSha,
    pub number: u64,
    pub verdict: HeadVerdictValue,
    pub report_digest: String,
    pub base: CommitSha,
    pub reviewer: SessionRef,
    pub recorded_at: Timestamp,
}

/// Failure while reading asks or queuing verdict notices.
#[derive(Debug, thiserror::Error)]
pub enum NoticeError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error("cannot serialize verdict notice: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Queues one verdict notice per distinct asking session recorded in unclosed
/// asks for `(version.project, version.head)`. Uses each asker's latest
/// unclosed ask's report destination. Performs one unlocked read of the
/// requested-review store and one state-database write; makes no runtime call
/// and pumps nothing.
pub fn queue_verdict_notices(
    home: &Home,
    version: &RecordedVersion,
    now: Timestamp,
) -> Result<usize, NoticeError> {
    let asks = _store::asks(home, &version.project, &version.head)?;
    let mut recipients = BTreeMap::<SessionRef, PathBuf>::new();
    for ask in asks {
        if ask.closed_at.is_some() {
            continue;
        }
        if let Some(asker) = ask.asker {
            recipients.insert(asker, ask.report);
        }
    }
    if recipients.is_empty() {
        return Ok(0);
    }

    let id = format!(
        "verdict:{}:{}:{}:{}",
        version.project, version.head, version.number, version.report_digest
    );
    let source_at = version.recorded_at.to_string();
    let show = format!("instrukt review show {}", version.head);
    let mut queued = Vec::with_capacity(recipients.len());
    for (asker, report) in recipients {
        let payload = serde_json::json!({
            "project": version.project.as_str(),
            "head": version.head.as_str(),
            "base": version.base.as_str(),
            "verdict": version.verdict.as_str(),
            "version": version.number,
            "report_digest": version.report_digest.as_str(),
            "reviewer": version.reviewer.to_string(),
            "report": report.to_string_lossy(),
            "show": show,
        });
        let content = serde_json::to_string(&payload)?;
        queued.push((asker, content));
    }

    let count = queued.len();
    let state = RunnerState::new(home);
    state.write::<_, StateError>(|tx| {
        for (asker, content) in &queued {
            inputs::retain(
                tx,
                asker,
                &Entry {
                    id: &id,
                    kind: InputKind::Verdict,
                    assignment: None,
                    source_at: Some(&source_at),
                    arrived_at: now,
                    intent: Intent::Asap,
                    content,
                },
            )?;
        }
        Ok(())
    })?;
    tracing::info!(
        event = "requested_review_notices_queued",
        project = %version.project,
        head = %version.head,
        version = version.number,
        count
    );
    Ok(count)
}
