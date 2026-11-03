//! The runner context: module-aware dispatch, standing agents, event
//! sources, the sessions facade and task timing
//! (`docs/local/spec/domain/runner.md`). Shared by the `todos` and `events`
//! modules; depends on core only.
//!
//! - [`interfaces`] — what the runner reads from work and workflow and
//!   writes through work's command contract. The todos crate implements
//!   them; the CLI wires them in when `todos` is on.
//! - [`events`] — the event vocabulary and the event-log append the runner
//!   implements and work's finalize-ready stamp uses.
//!
//! Everything else here (run/tick, the lock, the event log and its
//! sources, dispatch, the sessions facade, task timing, anomalies) is the
//! runner builder's in wave 3.

pub mod events;
pub mod interfaces;
