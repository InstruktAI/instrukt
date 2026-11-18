//! Bounded request/reply over the existing cluster Redis connection; no archive lives there.
use super::{Failure, HistoryError, Operation, SearchResult, local_search};
use crate::{
    state::RunnerState,
    transport::{EventScope, ScopedTransport},
};
use instrukt_core::{
    config::{Configuration, Env},
    home::Home,
    machine::{self, MachineId},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const PREFIX: &str = "instrukt:history";
const QUERY_TTL: u64 = 120;
#[derive(Serialize, Deserialize)]
struct Query {
    reply: String,
    query: String,
    limit: usize,
    expires: i64,
}

pub(super) fn merge(
    home: &Home,
    _env: &Env,
    search: &super::Search<'_>,
) -> Result<SearchResult, HistoryError> {
    let config = Configuration::load_global(home)?;
    let transport = ScopedTransport::cluster_only(&config, &RunnerState::new(home))?;
    let endpoint = transport.endpoint(EventScope::Cluster);
    let own = machine::machine_id(home)?.to_string();
    let targets: BTreeSet<_> = search.computers.iter().filter(|id| **id != own).collect();
    let deadline = Instant::now() + search.timeout;
    let mut result = SearchResult::default();
    let mut pending = std::collections::BTreeMap::new();
    for computer in targets {
        if MachineId::parse(computer).is_none() {
            return Err(HistoryError::Source(format!(
                "invalid computer id: {computer}"
            )));
        }
        let id = machine::random_hex(16).map_err(|source| HistoryError::Io {
            path: home.machine_id_file(),
            source,
        })?;
        let reply = format!("{PREFIX}:reply:{id}");
        let request = serde_json::to_string(&Query {
            reply: reply.clone(),
            query: search.query.into(),
            limit: search.limit,
            expires: jiff::Timestamp::now().as_second() + QUERY_TTL as i64,
        })?;
        let queue = format!("{PREFIX}:requests:{computer}");
        let sent = endpoint.run(|conn| {
            redis::pipe()
                .atomic()
                .cmd("RPUSH")
                .arg(&queue)
                .arg(&request)
                .ignore()
                .cmd("EXPIRE")
                .arg(&queue)
                .arg(QUERY_TTL)
                .ignore()
                .query::<()>(conn)
        });
        match sent {
            Ok(()) => {
                pending.insert(reply, computer.clone());
            }
            Err(error) => result.failures.push(Failure {
                computer: computer.clone(),
                operation: Operation::RemoteSearch,
                detail: {
                    tracing::error!(event="history_remote_operation_failed",error=%error);
                    HistoryError::Transport(error).public_message().into()
                },
            }),
        }
    }
    // Wait on all replies together: a missing computer cannot hide a healthy one's answer.
    while !pending.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let keys: Vec<&str> = pending.keys().map(String::as_str).collect();
        let response = endpoint.command::<Option<(String, String)>>(
            redis::cmd("BLPOP")
                .arg(&keys)
                .arg(remaining.as_secs_f64().max(0.001)),
        );
        match response {
            Ok(Some((reply, body))) => {
                let computer = pending
                    .remove(&reply)
                    .ok_or_else(|| HistoryError::Source("unexpected history reply queue".into()))?;
                match serde_json::from_str::<SearchResult>(&body) {
                    Ok(mut remote) => {
                        result.matches.append(&mut remote.matches);
                        result.failures.append(&mut remote.failures);
                    }
                    Err(error) => result.failures.push(Failure {
                        computer,
                        operation: Operation::RemoteSearch,
                        detail: {
                            tracing::error!(event="history_reply_invalid",error=%error);
                            "remote history response is incompatible".into()
                        },
                    }),
                }
            }
            Ok(None) => break,
            Err(error) => {
                let error = HistoryError::Transport(error);
                for computer in pending.values() {
                    result.failures.push(Failure::new(
                        computer.clone(),
                        Operation::RemoteSearch,
                        &error,
                    ));
                }
                pending.clear();
            }
        }
    }
    for computer in pending.into_values() {
        result.failures.push(Failure {
            computer,
            operation: Operation::RemoteSearch,
            detail: "computer did not answer before the search deadline".into(),
        });
    }
    Ok(result)
}

pub(crate) struct SearchWorker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
/// The worker's name in `runner_workers`.
const SEARCH: &str = "history-search";

impl SearchWorker {
    /// The query responder for other computers' merged searches. Cluster
    /// Redis is retried inside its thread after a growing delay, and its
    /// failure is recorded in `runner_workers` while it lasts.
    pub(crate) fn start(
        home: &Home,
        env: &Env,
        current: instrukt_core::config::Current,
        transport: &ScopedTransport,
    ) -> Result<Self, HistoryError> {
        let computer = machine::machine_id(home)?.to_string();
        let home = home.clone();
        let env = env.clone();
        let transport = transport.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let handle = std::thread::Builder::new()
            .name("history-search".into())
            .spawn(move || {
                let endpoint = transport.endpoint(EventScope::Cluster);
                let queue = format!("{PREFIX}:requests:{computer}");
                let mut failing: Option<String> = None;
                // An earlier process's record is cleared by this worker's first
                // success; a later failure records again.
                let mut shown_healthy = false;
                let mut delay = Duration::from_secs(1);
                while !stopping.load(Ordering::Relaxed) {
                    let outcome = (|| -> Result<(), HistoryError> {
                        if !current.get().history_enabled()? {
                            return Err(HistoryError::Disabled);
                        }
                        endpoint.command::<()>(
                            redis::cmd("SET")
                                .arg(format!("{PREFIX}:ready:{computer}"))
                                .arg("1")
                                .arg("EX")
                                .arg(5),
                        )?;
                        let Some((_, body)) = endpoint.command::<Option<(String, String)>>(
                            redis::cmd("BLPOP").arg(&queue).arg(1),
                        )?
                        else {
                            return Ok(());
                        };
                        let request: Query = serde_json::from_str(&body)?;
                        if request.expires < jiff::Timestamp::now().as_second() {
                            return Ok(());
                        }
                        let valid_reply = request
                            .reply
                            .strip_prefix(&format!("{PREFIX}:reply:"))
                            .is_some_and(|id| {
                                id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
                            });
                        if !valid_reply || !(1..=1000).contains(&request.limit) {
                            return Err(HistoryError::Source(
                                "invalid remote history request".into(),
                            ));
                        }
                        if !current.get().history_enabled()? { return Err(HistoryError::Disabled); }
                        let answer = match local_search(&home, &env, &request.query, request.limit)
                        {
                            Ok(answer) => answer,
                            Err(error) => SearchResult {
                                matches: Vec::new(),
                                failures: vec![Failure {
                                    computer: computer.clone(),
                                    operation: Operation::RemoteSearch,
                                    detail: {tracing::error!(event="history_remote_operation_failed",error=%error);error.public_message().into()},
                                }],
                            },
                        };
                        let body = serde_json::to_string(&answer)?;
                        endpoint.run(|conn| {
                            redis::pipe()
                                .atomic()
                                .cmd("RPUSH")
                                .arg(&request.reply)
                                .arg(body)
                                .ignore()
                                .cmd("EXPIRE")
                                .arg(&request.reply)
                                .arg(QUERY_TTL)
                                .ignore()
                                .query::<()>(conn)
                        })?;
                        Ok(())
                    })();
                    match outcome {
                        Ok(()) => {
                            if let Some(cause) = failing.take() {
                                tracing::info!(event = "history_worker_recovered", worker = SEARCH, cause = %cause);
                            }
                            if !shown_healthy {
                                crate::state::worker_state(&home, SEARCH, None);
                                shown_healthy = true;
                            }
                            delay = Duration::from_secs(1);
                        }
                        Err(HistoryError::Disabled) => break,
                        Err(error @ HistoryError::Transport(_)) => {
                            let cause = error.to_string();
                            shown_healthy = false;
                            if failing.as_deref() != Some(cause.as_str()) {
                                tracing::warn!(event = "history_worker_failing", worker = SEARCH, error = %error);
                                crate::state::worker_state(&home, SEARCH, Some(&cause));
                                failing = Some(cause);
                            }
                            if !stopping.load(Ordering::Relaxed) {
                                std::thread::park_timeout(delay);
                            }
                            delay = (delay * 2).min(Duration::from_secs(60));
                        }
                        Err(error) => tracing::error!(event="history_query_failed",error=%error),
                    }
                }
                // A worker that ends leaves no record of an earlier failure.
                crate::state::worker_state(&home, SEARCH, None);
            })
            .map_err(|source| HistoryError::Io {
                path: std::path::PathBuf::from("history search worker"),
                source,
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
    /// Whether the worker's thread has ended: history switched off, or
    /// unwound.
    pub(crate) fn ended(&self) -> bool {
        self.handle
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
    }
}
impl Drop for SearchWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = &self.handle {
            handle.thread().unpark();
        }
        if let Some(handle) = self.handle.take()
            && let Err(error) = handle.join()
        {
            tracing::error!(event="history_query_worker_panicked",error=?error);
        }
    }
}
