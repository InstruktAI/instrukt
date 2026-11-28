//! The outside gate (`docs/project/design/architecture/outside-gate.md`): the
//! one mechanism a part brings each call to an outside service through. The
//! part's own client makes the request; the gate judges the outcome
//! temporarily unavailable or a defect, keeps the wait per key and per scope,
//! and tells the operator once that it waits and why. It depends on nothing
//! of instrukt, so every crate reaches it.
//!
//! - [`Gate`] — made once by the host of the parts and handed down;
//!   [`Gate::attempt`] tries at most once and never pauses, [`Gate::call`]
//!   pauses within the caller's wait.
//! - [`Key`] — what a call reaches: a service, a scope and optionally an item
//!   within it.
//! - [`Tried`] — what a part's try returns; [`Attempted`] — what the gate
//!   answers, a [`Wait`] among it.
//! - [`http`] — the HTTP adapter: status semantics and `Retry-After`.
//! - [`Report`] — where the episode records go.
mod _gate;
mod _http;
mod _judgment;
mod _key;
mod _report;
mod _runtime;

pub use _gate::{Backoff, BackoffError, CallerWait, Clock, Gate, Monotonic, Settings, Wait};
pub use _judgment::{Attempted, Extent, Named, Reason, Tried, Unavailable};
pub use _key::Key;
pub use _report::{Episode, Record, Report};

/// The HTTP adapter of the outside gate.
pub mod http {
    pub use crate::_http::{
        BodyReadError, ClientConfig, HttpAnswer, Judgment, Rule, TransportFailure, build_client,
        ensure_crypto_provider, judge, judge_reqwest_error, judge_response, read_bounded_body,
        redact_url, retry_after,
    };
}

/// Shared Tokio runtime bridge for sync callers driving async SDK clients.
pub mod runtime {
    pub use crate::_runtime::{block_on, runtime};
}
