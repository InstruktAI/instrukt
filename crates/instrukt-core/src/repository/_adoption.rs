//! `instrukt init`: adoption of a git repository in one idempotent step.
//! It writes what instrukt needs and nothing else, never overwrites a file
//! that exists (the editor-schema modeline aside), commits nothing, and
//! shows what the repository will claim before anything is published
//! (`docs/project/spec/feature/repository/project-initialization.md`).

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{self, CONFIG_FILE_NAME, InitOutcome, Layer};
use crate::docs::{INDEX_FILE, SCHEMA_FILE};
use crate::fsx::atomic_write;
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug};
use crate::publication::{
    self, ClaimsPreview, EditorChange, EditorChangeKind, PublishedSchema, SCHEMAS_DIR,
};

use super::_contain::Bounds;
use super::_error::RepositoryError;
use super::_git;
use super::_hooks::{self, HooksReport, SectionChange};

/// The root instruction file instrukt renders from the project's index.
pub const GENERATED_FILE: &str = "AGENTS.md";
/// The project position adoption creates; a repository publishes
/// `docs/general/` or a `docs/domain/<x>/` by holding it.
pub const PROJECT_DOCS: &str = "docs/project";
/// Where a hand-written root `AGENTS.md` is kept, byte for byte, while its
/// instructions find their place in the project's documentation.
pub const KEPT_INSTRUCTIONS: &str = ".instrukt/adopted/AGENTS.md";
/// The domain whose published `schema.yaml` a new project is given.
const SCHEMA_DOMAIN: &str = "sdlc";
/// A new project index: it selects nothing yet.
const EMPTY_INDEX: &str = "expanded: []\nlisted: []\n";
/// The lines adoption keeps in `.gitignore`: instrukt's generated folder
/// only. The generated `AGENTS.md` files are committed.
const IGNORED: [&str; 1] = [".instrukt/"];

/// A part of adoption, in the order `init` establishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitPart {
    /// `instrukt.yaml` at the repository root.
    ProjectLayer,
    /// `docs/project/`.
    ProjectDocs,
    /// `docs/project/index.yaml`.
    ProjectIndex,
    /// `docs/project/schema.yaml`, a copy of the published sdlc schema.
    ProjectSchema,
    /// `.instrukt/adopted/AGENTS.md`, the hand-written root `AGENTS.md`.
    KeptInstructions,
    /// `.instrukt/schemas/` and the modelines of the files they describe.
    EditorSchemas,
    /// The generated files' lines in `.gitignore`.
    Ignores,
    /// instrukt's sections in the hooks directory.
    HookSections,
}

/// Why a part was not written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotWritten {
    /// The schema domain is not published on this computer.
    SchemaDomainNotPublished,
    /// The schema domain is published without a `schema.yaml`.
    SchemaDomainHasNoSchema,
}

/// What `init` did about one part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartOutcome {
    Created,
    /// The part existed in part, or at an older version, and was completed.
    Completed,
    AlreadyPresent,
    NotWritten(NotWritten),
}

/// The hand-written root `AGENTS.md` adoption kept aside; the next commit
/// through the hook writes the rendering over the file itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeptInstructions;

/// What `init` did, part by part, and the claims the first sync will make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    /// The working tree adopted.
    pub root: PathBuf,
    pub parts: Vec<(InitPart, PartOutcome)>,
    /// The editor schemas part in detail: each file changed or left
    /// unwritten behind a symbolic link.
    pub editor: Vec<EditorChange>,
    /// The hook sections part in detail.
    pub hooks: HooksReport,
    /// Present while the kept copy of a hand-written root `AGENTS.md`
    /// exists.
    pub kept: Option<KeptInstructions>,
    /// The claims the repository's first sync will make; `None` when it
    /// holds no published position.
    pub claims: Option<ClaimsPreview>,
}

/// Adopts the repository whose working tree contains `cwd`. `slug`
/// overrides the slug derived from the primary worktree's folder name;
/// `runtimes` is the selected runtime's adapter, asked which global agent
/// files the first sync writes. Refused outside a working tree; run again it
/// changes nothing.
pub fn init(
    home: &Home,
    cwd: &Path,
    slug: Option<&ProjectSlug>,
    runtimes: Option<&dyn crate::interfaces::ConfiguredAgentRuntimes>,
) -> Result<InitReport, RepositoryError> {
    let started = Instant::now();
    let root = _git::root(cwd)?;
    // Every target is resolved, and the hook edits decided, before anything
    // is written: a target outside the repository refuses the whole init.
    let bounds = Bounds::of(&[&root])?;
    bounds.resolve(&root.join(CONFIG_FILE_NAME))?;
    let docs = bounds.resolve(&root.join(PROJECT_DOCS))?;
    let index = bounds.resolve(&root.join(PROJECT_DOCS).join(INDEX_FILE))?;
    let schema = bounds.resolve(&root.join(PROJECT_DOCS).join(SCHEMA_FILE))?;
    let generated = bounds.resolve(&root.join(GENERATED_FILE))?;
    let kept_path = bounds.resolve(&root.join(KEPT_INSTRUCTIONS))?;
    let ignore = bounds.resolve(&root.join(".gitignore"))?;
    let hooks_plan = _hooks::plan_install_at(home, &root)?;
    // Read before anything is written: a root `AGENTS.md` that is not
    // instrukt's rendering is the developer's own, whether or not the
    // project's index already exists.
    let own_instructions = match std::fs::read(&generated) {
        Ok(bytes) => (!publication::is_rendering(&bytes)).then_some(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(RepositoryError::Read {
                path: generated,
                source,
            });
        }
    };

    let mut parts = Vec::new();
    // An existing project layer is committed to the repository: it is checked
    // and converted in memory, and only a commit brings it current.
    let project_layer = Layer::Project(root.clone());
    let layer = if exists(&project_layer.path(home))? {
        config::bring_current(home, &project_layer)?;
        PartOutcome::AlreadyPresent
    } else {
        match config::init(home, &project_layer, slug)? {
            InitOutcome::Created => PartOutcome::Created,
            InitOutcome::Migrated { .. } => PartOutcome::Completed,
            InitOutcome::AlreadyCurrent => PartOutcome::AlreadyPresent,
        }
    };
    parts.push((InitPart::ProjectLayer, layer));
    let project = project_slug(&root)?;
    // A company's repository keeps its knowledge in its own `docs/`: it is
    // given no project position and publishes none
    // (`project/instrukt/spec/feature/repository/project-initialization`).
    let company = publication::is_company_repository(&root)?;
    if !company {
        parts.push((InitPart::ProjectDocs, project_docs(&docs)?));
        parts.push((
            InitPart::ProjectIndex,
            create(&index, EMPTY_INDEX.as_bytes())?,
        ));
        parts.push((InitPart::ProjectSchema, project_schema(home, &schema)?));
    }
    // The copy is kept whenever none has been: on a first adoption, and on
    // the retry of one interrupted before the copy was written. Once a copy
    // exists it is never written again.
    if let Some(bytes) = &own_instructions {
        parts.push((InitPart::KeptInstructions, create(&kept_path, bytes)?));
    } else if exists(&kept_path)? {
        parts.push((InitPart::KeptInstructions, PartOutcome::AlreadyPresent));
    }
    let kept = if exists(&kept_path)? {
        Some(KeptInstructions)
    } else {
        None
    };
    let schemas_existed = root.join(SCHEMAS_DIR).is_dir();
    let editor = publication::refresh_editor_schemas(home.root(), &root)?;
    parts.push((
        InitPart::EditorSchemas,
        editor_outcome(schemas_existed, &editor),
    ));
    parts.push((InitPart::Ignores, ensure_lines(&ignore, &IGNORED)?));
    let hooks = hooks_plan.apply()?;
    parts.push((InitPart::HookSections, hooks_outcome(&hooks)));
    let claims = if company {
        None
    } else {
        let claims = publication::preview_claims(home, &root, &project, runtimes)?;
        (!claims.is_empty()).then_some(claims)
    };
    tracing::debug!(
        event = "repository_initialized",
        repo_root = %root.display(),
        changed_parts = parts.iter().filter(|(_, outcome)| matches!(outcome, PartOutcome::Created | PartOutcome::Completed)).count(),
        claimed_positions = claims.as_ref().map_or(0, |claims| claims.positions.len()),
        duration_ms = elapsed_ms(started),
    );
    Ok(InitReport {
        root,
        parts,
        editor,
        hooks,
        kept,
        claims,
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The project layer's slug, which `config::init` has just ensured.
fn project_slug(root: &Path) -> Result<ProjectSlug, RepositoryError> {
    config::project_slug(root)?.ok_or_else(|| RepositoryError::NoProjectSlug {
        path: root.join(CONFIG_FILE_NAME),
    })
}

fn project_docs(path: &Path) -> Result<PartOutcome, RepositoryError> {
    if path.is_dir() {
        return Ok(PartOutcome::AlreadyPresent);
    }
    crate::fsx::create_dir_all(path).map_err(|source| RepositoryError::Create {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(PartOutcome::Created)
}

/// Writes `bytes` at `path` (resolved) when nothing is there; an existing
/// file is never touched.
fn create(path: &Path, bytes: &[u8]) -> Result<PartOutcome, RepositoryError> {
    if exists(path)? {
        return Ok(PartOutcome::AlreadyPresent);
    }
    atomic_write(path, bytes)?;
    Ok(PartOutcome::Created)
}

/// Gives the project a copy of the published sdlc `schema.yaml` at `path`
/// (resolved) when it has none; once it exists the file is the project's
/// own.
fn project_schema(home: &Home, path: &Path) -> Result<PartOutcome, RepositoryError> {
    if exists(path)? {
        return Ok(PartOutcome::AlreadyPresent);
    }
    let domain = DomainName::parse(SCHEMA_DOMAIN)?;
    match publication::published_schema(home, &domain)? {
        PublishedSchema::NotPublished => Ok(PartOutcome::NotWritten(
            NotWritten::SchemaDomainNotPublished,
        )),
        PublishedSchema::NoSchema => {
            Ok(PartOutcome::NotWritten(NotWritten::SchemaDomainHasNoSchema))
        }
        PublishedSchema::Schema(bytes) => {
            atomic_write(path, &bytes)?;
            Ok(PartOutcome::Created)
        }
    }
}

fn editor_outcome(schemas_existed: bool, changes: &[EditorChange]) -> PartOutcome {
    let written = changes
        .iter()
        .any(|change| change.kind != EditorChangeKind::Linked);
    match (written, schemas_existed) {
        (false, _) => PartOutcome::AlreadyPresent,
        (true, false) => PartOutcome::Created,
        (true, true) => PartOutcome::Completed,
    }
}

/// Appends each of `lines` missing from the file at `path` (already
/// resolved), keeping every other line.
fn ensure_lines(path: &Path, lines: &[&str]) -> Result<PartOutcome, RepositoryError> {
    let text = match std::fs::read(path) {
        Ok(bytes) => Some(
            String::from_utf8(bytes).map_err(|_| RepositoryError::NotText {
                path: path.to_path_buf(),
            })?,
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(RepositoryError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let existing = text.clone().unwrap_or_default();
    let missing: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| !existing.lines().any(|present| present.trim() == *line))
        .collect();
    if missing.is_empty() {
        return Ok(PartOutcome::AlreadyPresent);
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for line in &missing {
        out.push_str(line);
        out.push('\n');
    }
    atomic_write(path, out.as_bytes())?;
    Ok(if text.is_none() {
        PartOutcome::Created
    } else {
        PartOutcome::Completed
    })
}

fn hooks_outcome(report: &HooksReport) -> PartOutcome {
    let changed = report
        .hooks
        .iter()
        .filter(|hook| {
            matches!(
                hook.change,
                SectionChange::Written | SectionChange::Updated | SectionChange::Removed
            )
        })
        .count();
    let kept = report.hooks.len() - changed;
    match (changed, kept) {
        (0, _) => PartOutcome::AlreadyPresent,
        (_, 0) => PartOutcome::Created,
        _ => PartOutcome::Completed,
    }
}

fn exists(path: &Path) -> Result<bool, RepositoryError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RepositoryError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}
