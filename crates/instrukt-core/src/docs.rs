//! The docs context: the snippet model, its structural validation and
//! pending documentation (`docs/local/spec/domain/docs.md`).
//!
//! Callers ask for the parsed documentation of a repository
//! ([`Corpus::load`], or [`Corpus::from_sources`] for a committed
//! revision), for its findings ([`validate`]), or for one delivery's fold
//! ([`fold`]); identity derivation, frontmatter reading, reference forms
//! and the pending-marker grammar stay behind those calls.
//!
//! What a corpus offers its customers:
//!
//! - every [`Snippet`] with its derived identity, taxonomy, scope and
//!   domain, its live [`Version`] (frontmatter, body, required reads,
//!   directives) and its [`Companion`]s;
//! - pending state: [`Snippet::delivered_by`], [`Snippet::superseded_by`]
//!   and [`Snippet::view`], which applies a [`Lens`] — the current contract
//!   or one todo's future — through the pending gate, the companion and the
//!   planned blocks;
//! - [`BaselineManifest`]s, [`ThirdPartyDoc`]s, fragments
//!   ([`Corpus::fragment`]) and whether a reference resolves
//!   ([`Corpus::resolves`]).
//!
//! Reading the home, running `@exec` directives and expanding inline
//! references belong to the read pipeline; publication's copying and
//! indexing belong to publication.

mod _corpus;
mod _finding;
mod _fold;
mod _frontmatter;
mod _identity;
mod _markers;
mod _refs;
mod _text;
mod _validate;

use std::path::PathBuf;

pub use _corpus::{
    BaselineManifest, Companion, Corpus, Lens, PendingBlock, Snippet, SourceFile, ThirdPartyDoc,
    Version, View,
};
pub use _finding::{Finding, FindingCode};
pub use _fold::{FoldOutcome, fold};
pub use _frontmatter::{
    Dispatch, EventDelivery, Frontmatter, Parameter, ProcedureEntry, ThinkingMode, Visibility,
};
pub use _identity::{BaselineOwner, DocDomain, Scope, Taxonomy};
pub use _refs::{RefTarget, Reference};
pub use _text::Directive;
pub use _validate::{ValidationRequest, validate};

use crate::config::ConfigError;
use crate::fsx::FsError;
use crate::interfaces::InterfaceError;

/// The docs context could not do what was asked. Findings are not errors;
/// these are.
#[derive(Debug, thiserror::Error)]
pub enum DocsError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot list {path}: {message}")]
    Walk { path: PathBuf, message: String },
    #[error("cannot remove {path}: {source}")]
    Remove {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Write(#[from] FsError),
    #[error("no such file or directory: {}", path.display())]
    NoSuchPath { path: PathBuf },
    #[error(
        "{} is a symbolic link or lies outside the repository; documentation is read and changed only inside it",
        path.display()
    )]
    OutsideRepository { path: PathBuf },
    #[error("cannot fold {}: {reason}", path.display())]
    FoldRefused { path: PathBuf, reason: String },
    #[error(transparent)]
    Interface(#[from] InterfaceError),
}
