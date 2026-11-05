//! The blueprint module: revision-keyed structural graphs of a project and
//! the questions answered from them (`docs/project/spec/domain/blueprint.md`).
//!
//! - [`Graph`] — the graph contract and its validation of engine output.
//! - [`GraphStore`] — the per-project store in the home: the latest complete
//!   graph and the one-step publication generation uses.
//! - [`Graph::reach`] and [`Graph::shortest_path`] — traversal over a graph.
//! - [`blast_radius`] and [`path`] — the answers computed from the contract.
//! - [`Engine`] — the engine adapter: the shipped adapter or a configured
//!   command, and the questions only an engine answers.
//! - [`Generation`] — core's `GraphGeneration`, wired by the CLI when the
//!   module is switched on: queues a revision and starts the detached
//!   [`run_worker`], which publishes through [`GraphStore::publish`];
//!   [`last_generation`] is the outcome `blueprint status` shows.
//! - [`trace_use_case`] and [`trace_files`] — use-case trace.
//!
//! Depends on core only.

mod _answers;
mod _contract;
mod _engine;
mod _generation;
mod _store;
mod _trace;
mod _traversal;

pub use _answers::{BlastRadius, PathAnswer, UnknownEndpoint, blast_radius, path, repo_relative};
pub use _contract::{
    ContractError, Graph, Link, Location, Node, Relation, SourceFileProblem, normalize_path,
};
pub use _engine::{Engine, EngineError, Question, SHIPPED_ENGINE};
pub use _generation::{
    Generation, GenerationError, GenerationState, LastGeneration, WORKER_COMMAND, last_generation,
    run_worker,
};
pub use _store::{CleanupFailure, GraphStore, LatestGraph, Published, StoreError};
pub use _trace::{
    AtStake, FileTrace, SpecFolder, UntracedFile, UseCaseTrace, trace_files, trace_use_case,
};
pub use _traversal::Direction;
