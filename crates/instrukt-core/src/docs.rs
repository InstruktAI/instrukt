//! The docs context: the snippet model, its structural validation and
//! pending documentation (`docs/project/spec/domain/docs.md`).
//!
//! Callers ask for the parsed documentation of a repository
//! ([`Corpus::load`], or [`Corpus::from_sources`] for a committed
//! revision), for its findings ([`validate`]), for one delivery's fold
//! ([`fold`]), or for moving one todo's pending documentation to another
//! ([`transfer_pending`]); identity derivation, frontmatter reading, reference forms
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
//! - the [`Position`]s documentation lives in, and the documents a
//!   position carries at its root: its [`PositionIndex`] (`index.yaml`)
//!   and its [`PositionSchema`] (`schema.yaml`), parsed with the keys
//!   each position allows;
//! - [`BaselineManifest`]s, [`ThirdPartyDoc`]s and whether a reference resolves
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
//!     pub home: &'a Home,              // expands `~/` references
//!     pub values: &'a Values<'a>,      // Values::read(&configuration)
//!                                      // | Values::publish(&configuration)
//!     pub repo_root: Option<&'a Path>, // resolves `docs/…` references
//!     pub project: Option<&'a ProjectSlug>,
//! }
//! pub struct Rendered {
//!     pub text: String,
//!     pub required_reads: Vec<Reference>,
//!     pub config_refusals: Vec<ConfigRefusal>,
//! }
//! ```
//!
//! A rendering strips frontmatter, resolves planned blocks for the lens, replaces
//! each visible `@config` directive by its configured value, turns
//! `@<path>.md` references into absolute paths and strips the `Required
//! reads` section, returning its references. It runs no command and
//! includes nothing: a leftover `@exec` or `@include` stays exactly as
//! written. The pending gate and the companion are
//! [`Snippet::view`]: render the version a `View::Visible` names.
//! [`Values::read`] resolves in the reader's session and turns a refusal
//! into an inline notice listed in [`Rendered::config_refusals`];
//! [`Values::publish`] resolves with no runtime session and fails the
//! rendering ([`DocsError::ConfigUnresolved`]).
//!
//! Retrieval reads what publication wrote into the home: [`index`] lists
//! the global index's entries for an [`IndexRequest`], [`get`] reads the
//! snippets a [`GetRequest`] names through the read pipeline, with their
//! required reads named or, as a closure, included. Neither writes
//! anything; publication's copying and indexing belong to publication.

mod _corpus;
mod _directives;
mod _finding;
mod _fold;
mod _frontmatter;
mod _identity;
mod _markers;
mod _position;
mod _refs;
mod _render;
mod _retrieval;
mod _text;
mod _transfer;
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
pub use _identity::{
    BaselineOwner, DocDomain, Scope, Taxonomy, published_destination, published_origin,
    published_source,
};
pub use _position::{
    AgentCli, DomainIndex, FolderSections, GENERAL, GeneralIndex, INDEX_FILE, Position,
    PositionIndex, PositionSchema, ProjectIndex, RESERVED_DOMAIN_FOLDERS, SCHEMA_FILE,
    is_reserved_domain_folder,
};
pub use _refs::{RefTarget, Reference};
pub use _render::{ConfigRefusal, RenderContext, Rendered, Values, render};
pub use _retrieval::{GetRequest, IndexRequest, Reading, get, index};
pub use _text::Directive;
pub use _transfer::{TransferOutcome, transfer_pending};
pub use _validate::{ValidationRequest, validate};

/// The `@` references visible in `text`, each without its `@`, as the
/// docs model recognizes them: outside fenced code, inline code and HTML
/// comments, a line holding only a reference or an `@<path>.md` token in
/// prose. Publication refuses a compiled text that still has one.
pub fn visible_references(text: &str) -> Vec<String> {
    _text::references(&_text::lines(text))
        .into_iter()
        .map(|reference| reference.target)
        .collect()
}

/// The `@` references visible in `text` (as [`visible_references`] finds
/// them), each read for a repository declaring `project` whose readers'
/// home directory is `user_home`: what each names, in the forms validation
/// accepts.
pub fn references(
    text: &str,
    project: Option<&crate::ids::ProjectSlug>,
    user_home: &std::path::Path,
) -> Vec<Reference> {
    visible_references(text)
        .iter()
        .map(|raw| _refs::parse(raw, project, user_home))
        .collect()
}

use crate::config::ConfigError;
use crate::fsx::FsError;
use crate::git::GitError;
use crate::ids::{ProjectSlug, SnippetId};
use crate::index::IndexError;
use crate::interfaces::InterfaceError;
use crate::publication::PublicationStateError;

/// The docs context could not do what was asked. Findings are not errors;
/// these are.
#[derive(Debug, thiserror::Error)]
pub enum DocsError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// A project needs a domain no adopted docs repository publishes on
    /// this computer (`project/instrukt/spec/feature/publication/docs-repositories`).
    #[error(
        "domain `{domain}` is not published on this computer, but {required_by} needs it; adopt a docs repository that publishes it"
    )]
    DomainUnpublished {
        domain: crate::ids::DomainName,
        /// What needs it: `workflow.domain`, or the file whose required
        /// read names it.
        required_by: String,
    },
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
    #[error("cannot transfer pending documentation at {}: {reason}", path.display())]
    TransferRefused { path: PathBuf, reason: String },
    #[error(
        "transferring pending documentation failed: {cause}; {}",
        if unrestored.is_empty() {
            "every file already changed was restored".to_owned()
        } else {
            format!(
                "these files could not be restored: {}",
                unrestored.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            )
        }
    )]
    TransferFailed {
        cause: Box<DocsError>,
        unrestored: Vec<PathBuf>,
    },
    #[error("cannot fold {}: {reason}", path.display())]
    FoldRefused { path: PathBuf, reason: String },
    #[error(transparent)]
    Interface(#[from] InterfaceError),
    #[error(
        "{} lies outside the home's published tree or climbs out of it; documentation is read only inside it",
        path.display()
    )]
    OutsideHome { path: PathBuf },
    #[error("the configured value `{path}` cannot be resolved: {reason}")]
    ConfigUnresolved { path: String, reason: String },
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    PublicationState(#[from] PublicationStateError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("unknown snippet id{}: {}{}", plural(ids.len()), ids.join(", "), if *nothing_published {
        "; nothing is published yet: run `instrukt sync` in a project to publish its documentation"
    } else {
        ""
    })]
    UnknownIds {
        ids: Vec<String>,
        nothing_published: bool,
    },
    #[error("unknown domain{}: {}; the published domains are: {}", plural(unknown.len()), unknown.join(", "), valid.join(", "))]
    UnknownDomains {
        unknown: Vec<String>,
        valid: Vec<String>,
    },
    #[error("cannot read {id} as a closure: {reason}")]
    ClosureRefused { id: String, reason: String },
    #[error(
        "the local file of project `{project}` is missing at {}; run `instrukt sync` in that project to republish its documentation",
        path.display()
    )]
    MissingLocalFile { project: ProjectSlug, path: PathBuf },
    #[error(
        "the published file of project `{project}` is missing at {}; run `instrukt sync` in that project to republish its documentation",
        path.display()
    )]
    MissingPublishedFile { project: ProjectSlug, path: PathBuf },
    #[error(
        "the index entry of {id} carries the path {}, which is not that snippet's place; run `instrukt sync` in its project to republish it",
        path.display()
    )]
    IndexMismatch { id: SnippetId, path: PathBuf },
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}
