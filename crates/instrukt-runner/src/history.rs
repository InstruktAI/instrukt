//! Retains source conversations locally and answers searches with explicit source failures.
mod _fleet;
mod _legacy;
mod _store;
mod _t3;

use instrukt_core::{config::Env, home::Home};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) use _fleet::SearchWorker;

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("history stream has no new frame within the read bound")]
    StreamIdle,
    #[error(transparent)]
    Db(#[from] instrukt_core::state_db::StateDbError),
    #[error("history SQLite operation failed: {0}")]
    Sql(#[from] instrukt_core::state_db::rusqlite::Error),
    #[error("history source I/O failed at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid history source data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("history source unavailable: {0}")]
    Source(String),
    #[error(transparent)]
    Lock(#[from] instrukt_core::fsx::FsError),
    #[error(transparent)]
    Machine(#[from] instrukt_core::machine::MachineError),
    #[error(transparent)]
    Transport(#[from] crate::transport::TransportError),
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Match {
    pub store_computer: String,
    pub computer: String,
    pub source: String,
    pub conversation: String,
    pub title: String,
    pub parent: Option<String>,
    pub aliases: Vec<String>,
    pub excerpt: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failure {
    pub computer: String,
    pub operation: String,
    pub detail: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SearchResult {
    pub matches: Vec<Match>,
    pub failures: Vec<Failure>,
}

#[derive(Debug, Default, Serialize)]
pub struct Extraction {
    pub loaded: usize,
    pub position: u64,
}

/// Catch up through T3's ordered state; no local writer lock spans runtime I/O.
pub fn extract(home: &Home, env: &Env) -> Result<Extraction, HistoryError> {
    _t3::extract(home, env, false)
}

/// One atomic, repeat-safe import. A completed source is never opened again.
pub fn import_teleclaude(
    home: &Home,
    path: Option<&std::path::Path>,
) -> Result<Extraction, HistoryError> {
    _legacy::import(home, path)
}

/// Every participating computer catches up before searching its own retained store.
pub struct Search<'a> {
    pub query: &'a str,
    pub limit: usize,
    pub computers: &'a [String],
    pub timeout: Duration,
}

pub fn search(home: &Home, env: &Env, request: &Search<'_>) -> Result<SearchResult, HistoryError> {
    let Search {
        query,
        limit,
        computers,
        timeout: _,
    } = *request;
    if query.trim().is_empty() || !(1..=1000).contains(&limit) {
        return Err(HistoryError::Source(
            "a nonempty FTS5 query and limit between 1 and 1000 are required".into(),
        ));
    }
    let mut result = local_search(home, env, query, limit)?;
    if !computers.is_empty() {
        match _fleet::merge(home, env, request) {
            Ok(mut remote) => {
                result.matches.append(&mut remote.matches);
                result.failures.append(&mut remote.failures);
            }
            Err(error) => {
                for computer in computers {
                    result.failures.push(Failure {
                        computer: computer.clone(),
                        operation: "remote_search".into(),
                        detail: error.to_string(),
                    });
                }
            }
        }
    }
    result.matches.sort_by(|a, b| {
        (&a.store_computer, &a.source, &a.conversation).cmp(&(
            &b.store_computer,
            &b.source,
            &b.conversation,
        ))
    });
    result.matches.dedup_by(|a, b| {
        a.store_computer == b.store_computer
            && a.source == b.source
            && a.conversation == b.conversation
    });
    result.matches.truncate(limit);
    Ok(result)
}

pub(super) fn local_search(
    home: &Home,
    env: &Env,
    query: &str,
    limit: usize,
) -> Result<SearchResult, HistoryError> {
    let computer = instrukt_core::machine::machine_id(home)?.to_string();
    let caught_up = extract(home, env);
    let mut matches = _store::search(home, query, limit)?;
    for found in &mut matches {
        found.store_computer = computer.clone();
    }
    let failures = caught_up
        .err()
        .map(|error| Failure {
            computer,
            operation: "catch_up".into(),
            detail: error.to_string(),
        })
        .into_iter()
        .collect();
    Ok(SearchResult { matches, failures })
}

/// Read the retained source document, including its original metadata and aliases.
pub fn show(home: &Home, source: &str, conversation: &str) -> Result<Value, HistoryError> {
    _store::show(home, source, conversation)
}

pub(crate) fn start(home: &Home, env: &Env) -> Result<_t3::Worker, HistoryError> {
    _t3::Worker::start(home, env)
}
