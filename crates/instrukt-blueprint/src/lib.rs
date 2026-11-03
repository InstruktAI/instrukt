//! The blueprint module: revision-keyed structural graphs of a project and
//! the questions answered from them (`docs/local/spec/domain/blueprint.md`).
//!
//! - [`Graph`] — the graph contract and its validation of engine output.
//! - [`GraphStore`] — the per-project store in the home: the latest complete
//!   graph and the one-step publication generation uses.
//! - [`Graph::reach`] and [`Graph::shortest_path`] — traversal over a graph.
//! - [`blast_radius`] and [`path`] — the answers computed from the contract.
//!
//! Depends on core only. Generation — core's `GraphGeneration` interface,
//! wired by the CLI when the module is switched on — publishes through
//! [`GraphStore::publish`].

mod _answers;
mod _contract;
mod _store;
mod _traversal;

pub use _answers::{BlastRadius, PathAnswer, UnknownEndpoint, blast_radius, path, repo_relative};
pub use _contract::{
    ContractError, Graph, Link, Location, Node, Relation, SourceFileProblem, normalize_path,
};
pub use _store::{CleanupFailure, GraphStore, LatestGraph, Published, StoreError};
pub use _traversal::Direction;
