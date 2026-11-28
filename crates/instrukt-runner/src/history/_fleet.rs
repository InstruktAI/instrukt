//! Merged history search as the history kind of the computer fleet's query
//! (`crate::computer_fleet`): its payload, its answer, and how each
//! computer's missing answer is worded.
use super::{Failure, HistoryError, Operation, SearchResult, local_search};
use crate::computer_fleet::{
    self, Answering, FailureFile, Outcome, Responder, ResponderSpec, StartError, Unopened,
};
use crate::transport::{ScopedTransport, TransportError};
use instrukt_core::{
    config::{ConfigError, Configuration, Current, Env},
    home::Home,
    machine::{self, MachineId},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// The history kind's payload.
#[derive(Serialize, Deserialize)]
struct Query {
    query: String,
    limit: usize,
}

pub(super) fn merge(
    home: &Home,
    _env: &Env,
    search: &super::Search<'_>,
) -> Result<SearchResult, HistoryError> {
    let config = Configuration::load_global(home)?;
    let fleet = match computer_fleet::open(&config, home, computer_fleet::Reachability::Unrecorded)
    {
        Ok(fleet) => fleet,
        Err(Unopened::Unusable(cause)) => return Err(HistoryError::Transport(cause)),
        Err(Unopened::NotConfigured) => {
            return Err(HistoryError::Transport(TransportError::Config(
                config
                    .cluster_redis()
                    .err()
                    .unwrap_or_else(|| ConfigError::InvalidValue {
                        key: "redis.cluster.url".into(),
                        message: "required for enabled Redis transport".into(),
                    }),
            )));
        }
    };
    let own = machine::machine_id(home)?.to_string();
    let targets: BTreeSet<_> = search.computers.iter().filter(|id| **id != own).collect();
    let mut computers = Vec::with_capacity(targets.len());
    for computer in targets {
        computers.push(
            MachineId::parse(computer)
                .ok_or_else(|| HistoryError::Source(format!("invalid computer id: {computer}")))?,
        );
    }
    let observed = fleet
        .ask(
            computer_fleet::HISTORY,
            &computers,
            &Query {
                query: search.query.into(),
                limit: search.limit,
            },
            search.timeout,
        )
        .map_err(|error| match error {
            computer_fleet::AskError::Random(source) => HistoryError::Io {
                path: home.machine_id_file(),
                source,
            },
            computer_fleet::AskError::Encode(error) => HistoryError::Json(error),
        })?;
    let mut result = SearchResult::default();
    for (computer, outcome) in observed {
        let computer = computer.to_string();
        let unanswered = |detail: String| Failure {
            computer: computer.clone(),
            operation: Operation::RemoteSearch,
            detail,
        };
        match outcome {
            Outcome::Answered(body) => match serde_json::from_str::<SearchResult>(&body) {
                Ok(mut remote) => {
                    result.matches.append(&mut remote.matches);
                    result.failures.append(&mut remote.failures);
                }
                Err(error) => {
                    tracing::error!(event = "history_reply_invalid", error = %error);
                    result
                        .failures
                        .push(unanswered("remote history response is incompatible".into()));
                }
            },
            Outcome::Unreachable | Outcome::NotDelivered => result.failures.push(unanswered(
                HistoryError::Fleet("the request was not delivered")
                    .public_message()
                    .into(),
            )),
            Outcome::StoreFailed => result.failures.push(Failure::new(
                computer.clone(),
                Operation::RemoteSearch,
                &HistoryError::Fleet("the store failed while the answer was awaited"),
            )),
            Outcome::NoAnswer => result.failures.push(unanswered(
                "computer did not answer before the search deadline".into(),
            )),
        }
    }
    Ok(result)
}

/// The worker's name in its failure file and `instrukt events status`.
const SEARCH: &str = "history-search";

/// The query responder for other computers' merged searches, while history
/// is switched on here.
pub(crate) fn search_worker(
    home: &Home,
    env: &Env,
    current: Current,
    runtimes: crate::runtime::RuntimeRegistry,
    transport: &ScopedTransport,
) -> Result<Responder, StartError> {
    Responder::start(
        home,
        ResponderSpec {
            kind: computer_fleet::HISTORY,
            worker: SEARCH,
            ready: true,
        },
        transport,
        Searching {
            computer: machine::machine_id(home)?.to_string(),
            home: home.clone(),
            env: env.clone(),
            current,
            runtimes,
        },
        FailureFile::worker(home, SEARCH, std::sync::Arc::new(jiff::Timestamp::now)),
    )
}

/// History's answering on this computer.
struct Searching {
    computer: String,
    home: Home,
    env: Env,
    current: Current,
    runtimes: crate::runtime::RuntimeRegistry,
}

impl Answering for Searching {
    fn enabled(&self) -> Result<bool, String> {
        self.current
            .get()
            .history_enabled()
            .map_err(|error| error.to_string())
    }

    fn accepts(&self, payload: &Map<String, Value>) -> bool {
        serde_json::from_value::<Query>(Value::Object(payload.clone()))
            .is_ok_and(|request| (1..=1000).contains(&request.limit))
    }

    fn answer(&self, payload: Map<String, Value>) -> Result<String, serde_json::Error> {
        let request: Query = serde_json::from_value(Value::Object(payload))?;
        let answer = match local_search(
            &self.home,
            &self.env,
            &self.runtimes,
            &request.query,
            request.limit,
        ) {
            Ok(answer) => answer,
            Err(error) => SearchResult {
                matches: Vec::new(),
                failures: vec![Failure {
                    computer: self.computer.clone(),
                    operation: Operation::RemoteSearch,
                    detail: {
                        tracing::error!(event = "history_remote_operation_failed", error = %error);
                        error.public_message().into()
                    },
                }],
            },
        };
        serde_json::to_string(&answer)
    }
}
