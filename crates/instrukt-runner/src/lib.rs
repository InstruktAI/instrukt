//! The runner context: module-aware dispatch, standing agents, event
//! sources, the sessions facade and task timing
//! (`docs/local/spec/domain/runner.md`). Shared by the `todos` and `events`
//! modules; depends on core only.
//!
//! The runner reaches work and workflow only through the interfaces it
//! declares in [`interfaces`]; the todos crate implements them and the CLI
//! wires them in when `todos` is on.

pub mod events;
pub mod interfaces;
