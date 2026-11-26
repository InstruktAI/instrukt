//! The judgment vocabulary: what a try returns and what the gate answers.
use std::fmt;
use std::time::Duration;

use jiff::Timestamp;

use crate::_gate::Wait;

/// Why a service is temporarily unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Its name did not resolve or no connection opened.
    Unreachable,
    /// It did not answer in time, or the connection broke after sending.
    TimedOut,
    /// It limited the calls.
    RateLimited,
    /// It said it is unavailable; the HTTP status when it gave one.
    Unavailable { status: Option<u16> },
    /// A refusal the part's service rules count as temporary.
    Refused,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable => f.write_str("unreachable"),
            Self::TimedOut => f.write_str("timed out"),
            Self::RateLimited => f.write_str("rate limited"),
            Self::Unavailable {
                status: Some(status),
            } => write!(f, "unavailable (HTTP {status})"),
            Self::Unavailable { status: None } => f.write_str("unavailable"),
            Self::Refused => f.write_str("refused"),
        }
    }
}

/// What a temporary unavailability applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extent {
    /// Only what within the scope the call used.
    Key,
    /// The whole scope.
    Scope,
    /// Only the request that received it: no other call is held back.
    Call,
}

impl fmt::Display for Extent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Key => "key",
            Self::Scope => "scope",
            Self::Call => "call",
        })
    }
}

/// The moment a service named for the next try.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Named {
    /// A delay, counted from the moment the answer was received on the host's
    /// clock and rounded up to whole seconds.
    After {
        delay: Duration,
        received_at: Timestamp,
    },
    /// A date.
    At(Timestamp),
}

impl Named {
    /// The moment on the host's clock; one too far to represent is the
    /// farthest representable.
    pub(crate) fn moment(self) -> Timestamp {
        match self {
            Self::At(at) => at,
            Self::After { delay, received_at } => {
                let seconds = delay
                    .as_secs()
                    .saturating_add(u64::from(delay.subsec_nanos() > 0));
                i64::try_from(seconds)
                    .ok()
                    .and_then(|seconds| {
                        received_at
                            .checked_add(jiff::SignedDuration::from_secs(seconds))
                            .ok()
                    })
                    .unwrap_or(Timestamp::MAX)
            }
        }
    }
}

/// A temporary unavailability as a part's adapter judged it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unavailable {
    pub reason: Reason,
    pub extent: Extent,
    /// The moment the service named, when it named one.
    pub named: Option<Named>,
}

/// What one try through the part's own client returns.
#[derive(Debug)]
pub enum Tried<T, E> {
    Done(T),
    /// Temporarily unavailable, with the part's own error for it.
    Unavailable(Unavailable, E),
    /// Anything else; never retried by the gate.
    Defect(E),
}

/// What the gate answers a part.
#[derive(Debug)]
pub enum Attempted<T, E> {
    Done(T),
    Defect(E),
    /// The key or scope waits; `error` is the part's own error when a try
    /// ran, `None` when the wait answered without one.
    Wait {
        wait: Wait,
        error: Option<E>,
    },
}
