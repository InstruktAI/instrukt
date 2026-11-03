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
//! Rendering — the read pipeline's text steps, shared by `docs get` and by
//! publication's compilation of bundles and entrypoints:
//!
//! ```text
//! impl Version {
//!     pub fn render(&self, lens: Lens<'_>, cx: &RenderContext<'_>)
//!         -> Result<Rendered, DocsError>;
//! }
//! pub fn render(text: &str, lens: Lens<'_>, cx: &RenderContext<'_>)
//!     -> Result<Rendered, DocsError>;
//!
//! pub struct RenderContext<'a> {
//!     pub fragments: Fragments<'a>,    // Home(&Home) | CorpusFirst(&Corpus)
//!     pub exec: &'a ExecPolicy,        // ExecPolicy::read(cwd)
//!                                      // | ExecPolicy::publish(root, runtimes)
//!     pub repo_root: Option<&'a Path>, // resolves `docs/…` references
//!     pub project: Option<&'a ProjectSlug>,
//! }
//! pub struct Rendered {
//!     pub text: String,
//!     pub required_reads: Vec<Reference>,
//!     pub exec_failures: Vec<ExecFailure>,
//! }
//! ```
//!
//! A rendering strips frontmatter, expands `@include` fragments (a missing
//! one leaves its directive), resolves planned blocks for the lens, runs
//! each visible `@exec` (`/bin/sh -c`, [`EXEC_TIME_LIMIT`]), turns `@<path>.md`
//! references into absolute paths and strips the `Required reads` section,
//! returning its references. The pending gate and the companion are
//! [`Snippet::view`]: render the version a `View::Visible` names.
//! [`ExecPolicy::read`] runs in the caller's directory and environment and
//! turns a failure into an inline notice listed in
//! [`Rendered::exec_failures`]; [`ExecPolicy::publish`] runs in the project
//! root without the runtime mapping's session variables and fails the
//! rendering ([`DocsError::ExecFailed`]). [`Fragments::Home`] reads the
//! home's published fragments; [`Fragments::CorpusFirst`] reads the
//! corpus's own `docs/global/snippets/` first, then the home's.
//!
//! Reading the home and the index is retrieval (`docs index`, `docs get`);
//! publication's copying and indexing belong to publication.

mod _corpus;
mod _finding;
mod _fold;
mod _frontmatter;
mod _identity;
mod _markers;
mod _refs;
mod _render;
mod _text;
mod _validate;

use std::path::PathBuf;

pub use _corpus::{
    BaselineManifest, Companion, Corpus, Lens, Snippet, SourceFile, ThirdPartyDoc, Version, View,
};
pub use _finding::{Finding, FindingCode};
pub use _fold::{FoldOutcome, fold};
pub use _frontmatter::{
    Dispatch, EventDelivery, Frontmatter, Parameter, ProcedureEntry, ThinkingMode, Visibility,
};
pub use _identity::{BaselineOwner, DocDomain, Scope, Taxonomy};
pub use _refs::{RefTarget, Reference};
pub use _render::{
    EXEC_TIME_LIMIT, ExecFailure, ExecOutcome, ExecPolicy, Fragments, RenderContext, Rendered,
    render,
};
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
    #[error(
        "{} lies outside the home's published tree or climbs out of it; documentation is read only inside it",
        path.display()
    )]
    OutsideHome { path: PathBuf },
    #[error("cannot run the read-time command `{command}`: {source}")]
    Exec {
        command: String,
        source: std::io::Error,
    },
    #[error(
        "the read-time command `{command}` failed: {outcome}{}",
        stderr_suffix(stderr)
    )]
    ExecFailed {
        command: String,
        outcome: ExecOutcome,
        stderr: String,
    },
}

fn stderr_suffix(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}
