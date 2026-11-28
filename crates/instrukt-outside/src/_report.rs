//! The operator's report of a wait: one record when it starts, one when a try
//! at its level next succeeds.
use std::fmt;
use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use jiff::Timestamp;

use crate::_judgment::{Extent, Reason};

/// The log target of the gate's records.
const LOG_TARGET: &str = "instrukt_outside";

/// Where a wait's episode stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Episode {
    Started,
    /// A try at its level succeeded after the wait lasted `waited`.
    Ended {
        waited: Duration,
    },
}

/// One record of the report. It names no credential, request or answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub episode: Episode,
    pub service: &'static str,
    /// The key or scope waited on, as [`crate::Key`] renders it.
    pub key: String,
    pub extent: Extent,
    pub reason: Reason,
    /// The moment of the next try when the wait started; when it ended, the
    /// last moment it held.
    pub next_try: Timestamp,
}

impl fmt::Display for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.episode {
            Episode::Started => write!(
                f,
                "waiting on {} {}: {}; next try at {}",
                self.service, self.key, self.reason, self.next_try
            ),
            Episode::Ended { waited } => write!(
                f,
                "{} {} available again after {} s",
                self.service,
                self.key,
                waited.as_secs()
            ),
        }
    }
}

type Sink = Arc<dyn Fn(&Record) + Send + Sync>;

/// Where the gate's records go: the log of the command it runs in, the log
/// and a foreground command's standard error, or a sink of the host's.
#[derive(Clone)]
pub struct Report {
    log: bool,
    sink: Option<Sink>,
}

impl Report {
    /// The command's log, under the gate's event names.
    pub fn log() -> Self {
        Self {
            log: true,
            sink: None,
        }
    }

    /// The command's log and one line per record on `writer`, a foreground
    /// command's standard error.
    pub fn foreground(writer: Arc<Mutex<dyn Write + Send>>) -> Self {
        Self {
            log: true,
            sink: Some(Arc::new(move |record: &Record| {
                let mut writer = writer.lock().unwrap_or_else(PoisonError::into_inner);
                if let Err(error) = writeln!(writer, "{record}") {
                    tracing::debug!(target: LOG_TARGET, event = "outside_report_unwritten", %error);
                }
            })),
        }
    }

    /// Only `sink`.
    pub fn to(sink: impl Fn(&Record) + Send + Sync + 'static) -> Self {
        Self {
            log: false,
            sink: Some(Arc::new(sink)),
        }
    }

    pub(crate) fn emit(&self, record: &Record) {
        if self.log {
            let Record {
                episode,
                service,
                key,
                extent,
                reason,
                next_try,
            } = record;
            match episode {
                Episode::Started => {
                    tracing::warn!(target: LOG_TARGET, event = "outside_wait_started", service, key = %key, extent = %extent, reason = %reason, next_try = %next_try);
                }
                Episode::Ended { waited } => {
                    tracing::info!(target: LOG_TARGET, event = "outside_wait_ended", service, key = %key, extent = %extent, reason = %reason, next_try = %next_try, waited = ?waited);
                }
            }
        }
        if let Some(sink) = &self.sink {
            sink(record);
        }
    }
}
