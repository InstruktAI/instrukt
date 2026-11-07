//! One command module per context. Each owns its argument types and its
//! `run` functions, and translates between the command line and its
//! context's library API; behavior lives in the libraries.

pub mod agents;
pub mod blueprint;
pub mod config;
pub mod docs;
pub mod eval;
pub mod events;
pub mod repository;
pub mod runner;
pub mod specs;
pub mod specs_trace;
pub mod sync;
pub mod todo;
pub mod todo_flow;
pub mod webhooks;

use std::process::ExitCode;

/// The answer of a command whose context is not built yet.
pub(crate) fn not_built(command: &str) -> anyhow::Result<ExitCode> {
    anyhow::bail!("`instrukt {command}` is not available in this build")
}
