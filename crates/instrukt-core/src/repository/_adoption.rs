//! `instrukt init`: adoption of a git repository in one idempotent step.
//! It writes what instrukt needs and nothing else, never overwrites a file
//! that exists and commits nothing.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{self, InitOutcome, Layer};
use crate::fsx::atomic_write;
use crate::home::Home;
use crate::ids::ProjectSlug;

use super::_error::RepositoryError;
use super::_filter::{self, FILTER_PATTERN};
use super::_git;
use super::_hooks::{self, HooksReport, SectionChange};

/// The master instruction file at the repository root.
pub const MASTER_FILE: &str = "AGENTS.master.md";
/// The generated instruction file a master compiles into.
pub const GENERATED_FILE: &str = "AGENTS.md";
/// The project's own baseline manifest a seeded master requires.
const LOCAL_BASELINE: &str = "docs/local/baseline.md";
/// The docs folders adoption creates.
const DOCS_FOLDERS: [&str; 2] = ["docs/global", "docs/local"];

/// A part of adoption, in the order `init` establishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitPart {
    /// `instrukt.yaml` at the repository root.
    ProjectLayer,
    /// `docs/global/` and `docs/local/`.
    DocsFolders,
    /// `AGENTS.master.md` at the repository root.
    Master,
    /// The `instrukt-docs` filter in the repository's git configuration.
    DocsFilter,
    /// The filter's line in `.gitattributes`.
    FilterPatterns,
    /// The ignore of generated `AGENTS.md` files in `.gitignore`.
    GeneratedIgnore,
    /// instrukt's sections in the hooks directory.
    HookSections,
}

/// What `init` did about one part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartOutcome {
    Created,
    /// The part existed in part, or at an older version, and was completed.
    Completed,
    AlreadyPresent,
    /// Not written: a hand-written `AGENTS.md` exists without a master, so
    /// its content must first move into a `docs/local` snippet.
    Withheld,
}

/// What `init` did, part by part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    /// The working tree adopted.
    pub root: PathBuf,
    pub parts: Vec<(InitPart, PartOutcome)>,
    /// The hook sections part in detail.
    pub hooks: HooksReport,
}

/// Adopts the repository whose working tree contains `cwd`. `slug`
/// overrides the slug derived from the primary worktree's folder name.
/// Refused outside a working tree; run again it changes nothing.
pub fn init(
    home: &Home,
    cwd: &Path,
    slug: Option<&ProjectSlug>,
) -> Result<InitReport, RepositoryError> {
    let started = Instant::now();
    let root = _git::root(cwd)?;
    let mut parts = Vec::new();
    let layer = match config::init(home, &Layer::Project(root.clone()), slug)? {
        InitOutcome::Created => PartOutcome::Created,
        InitOutcome::Migrated { .. } => PartOutcome::Completed,
        InitOutcome::AlreadyCurrent => PartOutcome::AlreadyPresent,
    };
    parts.push((InitPart::ProjectLayer, layer));
    parts.push((InitPart::DocsFolders, docs_folders(&root)?));
    parts.push((InitPart::Master, master(&root)?));
    parts.push((InitPart::DocsFilter, docs_filter(&root)?));
    parts.push((
        InitPart::FilterPatterns,
        ensure_line(&root.join(".gitattributes"), FILTER_PATTERN)?,
    ));
    parts.push((
        InitPart::GeneratedIgnore,
        ensure_line(&root.join(".gitignore"), GENERATED_FILE)?,
    ));
    let hooks = _hooks::install_at(home, &root)?;
    parts.push((InitPart::HookSections, hooks_outcome(&hooks)));
    tracing::debug!(
        event = "repository_initialized",
        repo_root = %root.display(),
        changed_parts = parts.iter().filter(|(_, outcome)| matches!(outcome, PartOutcome::Created | PartOutcome::Completed)).count(),
        duration_ms = started.elapsed().as_millis() as u64,
    );
    Ok(InitReport { root, parts, hooks })
}

fn docs_folders(root: &Path) -> Result<PartOutcome, RepositoryError> {
    let mut created = 0;
    for folder in DOCS_FOLDERS {
        let path = root.join(folder);
        if path.is_dir() {
            continue;
        }
        std::fs::create_dir_all(&path).map_err(|source| RepositoryError::Create {
            path: path.clone(),
            source,
        })?;
        created += 1;
    }
    Ok(match created {
        0 => PartOutcome::AlreadyPresent,
        n if n == DOCS_FOLDERS.len() => PartOutcome::Created,
        _ => PartOutcome::Completed,
    })
}

fn master(root: &Path) -> Result<PartOutcome, RepositoryError> {
    let path = root.join(MASTER_FILE);
    if exists(&path)? {
        return Ok(PartOutcome::AlreadyPresent);
    }
    if exists(&root.join(GENERATED_FILE))? {
        return Ok(PartOutcome::Withheld);
    }
    let name = match config::project_slug(root)? {
        Some(slug) => slug.to_string(),
        None => _git::primary_worktree(root)?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    let text = format!("## Purpose\n\n{name}\n\n## Required reads\n\n@{LOCAL_BASELINE}\n");
    atomic_write(&path, text.as_bytes())?;
    Ok(PartOutcome::Created)
}

fn docs_filter(root: &Path) -> Result<PartOutcome, RepositoryError> {
    let (matching, total) = _filter::defined_keys(root)?;
    if matching == total {
        return Ok(PartOutcome::AlreadyPresent);
    }
    _filter::define(root)?;
    Ok(if matching == 0 {
        PartOutcome::Created
    } else {
        PartOutcome::Completed
    })
}

/// Appends `line` to the file at `path` unless a line equal to it is there,
/// keeping every other line.
fn ensure_line(path: &Path, line: &str) -> Result<PartOutcome, RepositoryError> {
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
    let Some(text) = text else {
        atomic_write(path, format!("{line}\n").as_bytes())?;
        return Ok(PartOutcome::Created);
    };
    if text.lines().any(|existing| existing.trim() == line) {
        return Ok(PartOutcome::AlreadyPresent);
    }
    let mut out = text;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    atomic_write(path, out.as_bytes())?;
    Ok(PartOutcome::Completed)
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
