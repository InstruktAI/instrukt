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

mod answers;
mod contract;
mod store;
mod traversal;

pub use answers::{BlastRadius, PathAnswer, UnknownEndpoint, blast_radius, path, repo_relative};
pub use contract::{ContractError, Graph, Link, Location, Node, Relation, SourceFileProblem};
pub use store::{GraphStore, LatestGraph, StoreError};
pub use traversal::Direction;
