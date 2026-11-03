//! The specs context: use-case inventory, test bindings, coverage and
//! reservation (`docs/local/spec/domain/specs.md`).
//!
//! Callers ask for a project's coverage ([`Coverage::measure`], then
//! [`Coverage::check`] or [`Coverage::update_baseline`]) or for a todo's
//! reservation check ([`Reservation::check`]). Blueprint's trace consumes
//! the published reads: the [`Inventory`] of feature-spec use cases (from a
//! docs [`Corpus`](crate::docs::Corpus)) and the [`BindingSet`] of the
//! project's tests. Everything is read fresh from the working tree;
//! nothing is stored except the ratchet baseline.

mod _bindings;
mod _coverage;
mod _inventory;
mod _reserve;

use std::path::PathBuf;

pub use _bindings::{Binding, BindingSet, Citation, TestLocation};
pub use _coverage::{BASELINE_FILE, Baseline, Check, Counts, Coverage, Debt, Ratchet};
pub use _inventory::{AmbiguousClaim, AnonymousScenario, Claim, Inventory, UseCase};
pub use _reserve::{Answer, MalformedBinding, RequestedBinding, Reservation, Verdict};

use crate::docs::DocsError;
use crate::fsx::FsError;
use crate::git::GitError;

/// The specs context could not do what was asked.
#[derive(Debug, thiserror::Error)]
pub enum SpecsError {
    #[error(transparent)]
    Docs(#[from] DocsError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a coverage baseline: {source}")]
    Baseline {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(transparent)]
    Write(#[from] FsError),
}
