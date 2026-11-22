//! Retains source conversations locally and answers searches with explicit source failures.
mod _db;
mod _fleet;
mod _legacy;
mod _projection;
mod _store;
mod _watch;

use instrukt_core::{
    config::{Current, Env},
    home::Home,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) use _fleet::SearchWorker;

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error(
        "the history module is off; switch it on with `modules.history: true` in instrukt.yaml"
    )]
    Disabled,
    #[error(transparent)]
    Db(#[from] instrukt_core::state_db::StateDbError),
    #[error(transparent)]
    Store(#[from] instrukt_core::state_db::StoreError),
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
    Machine(#[from] instrukt_core::machine::MachineError),
    #[error(transparent)]
    Transport(#[from] crate::transport::TransportError),
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
    #[error(transparent)]
    ConversationSource(#[from] crate::runtime::conversations::SourceError),
    #[error(transparent)]
    Runtime(#[from] crate::sessions::SessionsError),
}

impl HistoryError {
    /// What a caller is told, here and across the fleet boundary: no path,
    /// version or internal detail; a refusal names its recovery.
    pub fn public_message(&self) -> Cow<'static, str> {
        use instrukt_core::state_db::{StateDbError, StoreError};
        match self {
            Self::Disabled => {
                "the history module is off; switch it on with `modules.history: true` in instrukt.yaml".into()
            }
            Self::Store(StoreError::Newer { .. }) | Self::Db(StateDbError::Newer { .. }) => format!(
                "history storage is newer than this instrukt knows; upgrade with `{}`",
                instrukt_core::INSTALLER_COMMAND
            )
            .into(),
            Self::Store(StoreError::Symlink { .. }) | Self::Db(StateDbError::Symlink { .. }) => {
                "history storage is a symbolic link; link the instrukt home or state/ directory instead".into()
            }
            Self::Db(_) | Self::Store(_) | Self::Sql(_) => {
                "history storage or source schema is unavailable; inspect this computer's history logs".into()
            }
            Self::Runtime(crate::sessions::SessionsError::MissingCapability { .. }) => {
                "selected runtime does not support durable conversation extraction".into()
            }
            Self::Runtime(crate::sessions::SessionsError::NoActiveRuntime) => {
                "choose a runtime before extracting conversation history".into()
            }
            Self::ConversationSource(crate::runtime::conversations::SourceError::Watch(_)) => {
                "history source notifications are unavailable; inspect this computer's history logs".into()
            }
            _ => {
                "history source or operation is unavailable or incompatible; inspect this computer's history logs".into()
            }
        }
    }
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
    pub operation: Operation,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    CatchUp,
    RemoteSearch,
}
impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::CatchUp => "catch_up",
            Self::RemoteSearch => "remote_search",
        })
    }
}
impl Failure {
    pub(super) fn new(computer: String, operation: Operation, error: &HistoryError) -> Self {
        tracing::error!(event="history_operation_failed",%computer,%operation,error=%error);
        Self {
            computer,
            operation,
            detail: error.public_message().into(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SearchResult {
    pub matches: Vec<Match>,
    pub failures: Vec<Failure>,
}

#[derive(Debug, Default, Serialize)]
pub struct Extraction {
    pub loaded: usize,
    pub position: Value,
}

/// Catch up through T3's ordered state into the history store, under the
/// history lock alone; the state database's writer lock spans no runtime I/O.
pub fn extract(home: &Home, env: &Env) -> Result<Extraction, HistoryError> {
    let source = source(home, env)?;
    let mut db = _db::open(home)?;
    let mut position = _projection::position(&db, source.identity())?;
    let mut report = Extraction::default();
    loop {
        let started = std::time::Instant::now();
        let batch =
            source.changed_since(position.as_ref(), crate::runtime::conversations::Scope::All)?;
        let changed = position.as_ref() != Some(&batch.position);
        let loaded = _projection::apply(home, &mut db, source.identity(), &batch)?;
        if changed || !batch.records.is_empty() {
            tracing::debug!(event="history_extract_completed",source=source.identity(),loaded,position=?batch.position,duration_ms=started.elapsed().as_millis() as u64);
        }
        report.loaded += loaded;
        report.position = serde_json::to_value(&batch.position)?;
        position = Some(batch.position);
        if !batch.has_more {
            break;
        }
    }
    Ok(report)
}

/// Imports the TeleClaude mirror store once, in resumable batches: an
/// interrupted import continues after its last committed mirror, and a
/// completed one is preserved without opening the source again. Reports the
/// conversations this run loaded and the last committed mirror id.
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
                    result.failures.push(Failure::new(
                        computer.clone(),
                        Operation::RemoteSearch,
                        &error,
                    ));
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
        .map(|error| Failure::new(computer, Operation::CatchUp, &error))
        .into_iter()
        .collect();
    Ok(SearchResult { matches, failures })
}

/// Read the retained source document, including its original metadata and aliases.
pub fn show(home: &Home, source: &str, conversation: &str) -> Result<Value, HistoryError> {
    _store::show(home, source, conversation)
}

pub(super) fn source(
    home: &Home,
    env: &Env,
) -> Result<Box<dyn crate::runtime::conversations::ConversationSource>, HistoryError> {
    let runtime = instrukt_core::config::Configuration::load_global(home)?.runner_runtime()?;
    Ok(crate::runtime::adapter(runtime).conversation_source(home, env)?)
}

/// Whether history extraction is on: the machine's switch while no project
/// is published here, otherwise any admitted project's.
pub fn enabled(home: &Home) -> Result<bool, HistoryError> {
    let candidate = instrukt_core::config::Admitted::load(home)
        .map_err(|error| HistoryError::Source(error.to_string()))?;
    Ok(candidate.generation.history_enabled()?)
}

/// Owns independent source and query readers for the long-lived runner.
#[derive(Default)]
pub(crate) struct Readers {
    extraction: Option<_watch::Worker>,
    search: Option<SearchWorker>,
    /// The cluster transport settings the search worker was started on.
    search_transport: Option<serde_yaml_ng::Value>,
}
impl Readers {
    /// Starts what the current generation switches on and stops what it
    /// switches off. A worker whose thread ended is started again; each
    /// worker retries what it depends on inside its own thread.
    pub(crate) fn refresh(&mut self, home: &Home, env: &Env, current: &Current) {
        let admitted = current.get();
        match admitted.history_enabled() {
            Ok(true) => {}
            Ok(false) => {
                self.search = None;
                self.extraction = None;
                // Nothing demanded is failing: no record stays, an earlier
                // process's included.
                crate::state::worker_state(home, "history-extraction", None);
                crate::state::worker_state(home, "history-search", None);
                return;
            }
            Err(error) => {
                tracing::warn!(event = "history_enablement_unreadable", %error);
                return;
            }
        }
        if self.extraction.as_ref().is_some_and(_watch::Worker::ended) {
            tracing::warn!(
                event = "history_worker_restarted",
                worker = "history-extraction"
            );
            self.extraction = None;
        }
        // A search worker holds the transport it started on: a generation
        // admitting other cluster settings starts it again on them.
        let transport_settings = admitted.global().get(Some("redis"), env).ok();
        if self.search.is_some() && self.search_transport != transport_settings {
            tracing::info!(
                event = "history_worker_restarted",
                worker = "history-search",
                cause = "transport settings changed"
            );
            self.search = None;
        }
        if self.search.as_ref().is_some_and(SearchWorker::ended) {
            tracing::warn!(
                event = "history_worker_restarted",
                worker = "history-search"
            );
            self.search = None;
        }
        let supported = admitted.global().runner_runtime().is_ok_and(|runtime| {
            crate::runtime::adapter(runtime)
                .supports(crate::runtime::Capability::ConversationSource)
        });
        if self.extraction.is_none() && supported {
            match _watch::Worker::start(home, env, current.clone()) {
                Ok(worker) => self.extraction = Some(worker),
                Err(error) => {
                    tracing::error!(event = "history_worker_unstarted", worker = "history-extraction", %error)
                }
            }
        }
        if self.search.is_none() {
            match crate::transport::ScopedTransport::cluster_only(
                admitted.global(),
                &crate::state::RunnerState::new(home),
            ) {
                Ok(transport) => {
                    match SearchWorker::start(home, env, current.clone(), &transport) {
                        Ok(worker) => {
                            self.search = Some(worker);
                            self.search_transport = transport_settings;
                        }
                        Err(error) => {
                            tracing::error!(event = "history_worker_unstarted", worker = "history-search", %error)
                        }
                    }
                }
                Err(error) => tracing::warn!(event="history_merged_search_unavailable", %error),
            }
        }
    }
}
