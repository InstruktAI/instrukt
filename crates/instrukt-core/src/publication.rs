//! The publication context: committed-state publication into the home,
//! index patching by owner, agent-file rendering and the eval fingerprint
//! (`docs/project/spec/domain/publication.md`).
//!
//! Installed publication reads committed canonical main; explicit development
//! execution reads the development worktree's committed HEAD. Both read:
//! `docs/general/`, each `docs/domain/<x>/` and `docs/project/`. Each is claimed
//! in the ownership record in the selected home's publication state.
//!
//! Two operations. [`render_repository`] renders a repository's committed
//! entrypoints from one view of it and nothing from the computer, as a
//! [`RenderPlan`] its caller applies ([`write_rendering`] for the working
//! tree). [`sync`]: publish the project the caller stands in from
//! the selected committed source, or do nothing when that commit is already
//! published. What it reads (the committed tree), how it patches the
//! index, stages and swaps the published tree, renders agent files and
//! undoes a failed run stays behind that call. Readers of its state use
//! [`PublishedRecord`] and [`EvalFingerprint`].

mod _adoption;
mod _agents;
mod _company;
mod _editor;
mod _fingerprint;
mod _patch;
pub(crate) use _patch::target_entries;
mod _plugin;
mod _render;
mod _source;
mod _sync;
mod _tree;
mod _tx;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use _adoption::{
    ClaimStatus, ClaimsPreview, GlobalFiles, PlannedGlobalFile, PublishedSchema, preview_claims,
    published_schema,
};
pub use _agents::{InstructionOutcome, InstructionTarget, NonDelivery};
pub use _company::{CompanyIndexKey, CompanyIndexProblem, CompanyReadProblem};
pub use _editor::{
    EditorChange, EditorChangeKind, SCHEMAS_DIR, committed_modeline, editor_schema_describes,
    refresh_editor_schemas,
};
pub use _fingerprint::EvalFingerprint;
pub use _patch::Claim;
pub use _plugin::{
    CompanyName, CompanyNameError, PluginDrift, PluginFinding, PluginPlan, PluginVersion,
};
pub use _render::{
    GENERATED_LINE, PlannedFile, RenderFinding, RenderFindingKind, RenderPlan, is_rendering,
};
pub use _sync::{
    EnsuredLink, GlobalDestination, LinkState, Refusal, SyncError, SyncFinding, SyncOutcome,
    SyncReport, SyncRequest, sync,
};

use crate::config::ConfigError;
use crate::docs::DocsError;
use crate::fsx::FsError;
use crate::git::GitError;
use crate::home::Home;
use crate::ids::{CommitSha, ProjectSlug};
use crate::index::IndexError;

/// The view of a repository its entrypoints are rendered from.
#[derive(Debug, Clone, Copy)]
pub enum RepositoryView<'a> {
    /// The working tree whose root is this directory, tracked and
    /// untracked files alike.
    WorkingTree(&'a Path),
    /// The index a commit is being made from in the repository whose root
    /// is `root`: `index` (git's `GIT_INDEX_FILE` for the hook run), or the
    /// repository's own index when `None`.
    Index {
        root: &'a Path,
        index: Option<&'a Path>,
    },
}

/// Renders every entrypoint of the repository in `view` from that view
/// alone (`project/instrukt/spec/feature/publication/agent-entrypoints`,
/// "Committed entrypoints"): what to write, what to remove, what the
/// rendering found and noticed. Reads nothing from the computer and writes
/// nothing.
pub fn render_repository(view: RepositoryView<'_>) -> Result<RenderPlan, PublicationError> {
    // An `AGENTS.override.md` is never committed by instrukt: the view
    // holds it, or, for a commit, the working tree beside the index does.
    let (tree, override_beside_root) = match view {
        RepositoryView::WorkingTree(root) => {
            let tree = _source::Committed::working_tree(root)?;
            let present = tree.file(_render::OVERRIDE_FILE).is_some();
            (tree, present)
        }
        RepositoryView::Index { root, index } => {
            let path = root.join(_render::OVERRIDE_FILE);
            let present = match std::fs::symlink_metadata(&path) {
                Ok(_) => true,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                Err(source) => return Err(PublicationError::Io { path, source }),
            };
            (_source::Committed::read_index(root, index)?, present)
        }
    };
    _render::render_tree(&tree, override_beside_root)
}

/// Whether `rel`, a path relative to the root of the working tree at `root`,
/// is that repository's company index, which rendering reads and the
/// documentation's validation leaves alone
/// (`project/instrukt/spec/feature/publication/agent-entrypoints`): it is
/// `docs/index.yaml`, and the tree renders as a company home by the rule
/// rendering decides by, with the root `instrukt.yaml` read as a view of the
/// tree reads it.
pub fn is_company_index(root: &Path, rel: &str) -> Result<bool, PublicationStateError> {
    if rel != _company::COMPANY_INDEX {
        return Ok(false);
    }
    let read = |path: &Path| {
        let path = path.to_path_buf();
        move |source| PublicationStateError::Read { path, source }
    };
    let document = _source::root_document_on_disk(root)
        .map_err(read(&root.join(crate::config::CONFIG_FILE_NAME)))?;
    let absent = _source::entry_index_absent_on_disk(root, rel).map_err(read(&root.join(rel)))?;
    Ok(_render::company_home(
        &crate::config::classify_root(document.as_deref()),
        absent,
    ))
}

/// The findings of rendering the company home whose root is `home_dir`,
/// from its working tree: what `instrukt config validate --company` reports
/// beside configuration's own findings.
pub fn company_findings(home_dir: &Path) -> Result<Vec<RenderFinding>, PublicationError> {
    Ok(render_repository(RepositoryView::WorkingTree(home_dir))?.findings)
}

/// Applies `plan` to the working tree whose root is `root`: each rendering
/// that differs from its file is written, and each removal made, one file
/// at a time and each atomically. Returns the paths it changed, relative to
/// `root`.
pub fn write_rendering(root: &Path, plan: &RenderPlan) -> Result<Vec<PathBuf>, PublicationError> {
    let mut changed = Vec::new();
    for file in plan.writes.iter().filter(|file| file.differs()) {
        let target = _tx::own_destination(root, &file.path)?;
        crate::fsx::atomic_write(&target, file.text.as_bytes())?;
        changed.push(file.path.clone());
    }
    for path in &plan.removals {
        let target = _tx::own_destination(root, path)?;
        match crate::fsx::remove_file(&target) {
            Ok(()) => changed.push(path.clone()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(PublicationError::Io {
                    path: target,
                    source,
                });
            }
        }
    }
    Ok(changed)
}

/// Plans the rendering of the company plugin of `key`, named `name`, from
/// the working tree whose root is `root`
/// (`project/instrukt/spec/feature/publication/company-plugin`): what it
/// found, and what to write and remove wherever that can be known, read
/// from the working tree alone, the plugin's folder and the catalog
/// included. Writes nothing.
pub fn plan_plugin(
    root: &Path,
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<PluginPlan, PublicationError> {
    _plugin::plan(&plugin_view(root, key)?, key, name)
}

/// Every finding of the company plugin of `key`, named `name`, in the
/// working tree whose root is `root`, as [`plan_plugin`] finds them, without
/// comparing any file with the rendering. Writes nothing.
pub fn validate_plugin(
    root: &Path,
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<Vec<PluginFinding>, PublicationError> {
    _plugin::findings(&plugin_view(root, key)?, key, name)
}

/// The view of the working tree whose root is `root` that the plugin of
/// `key` is rendered from: the plugin's folder and the catalog included.
fn plugin_view(root: &Path, key: &ProjectSlug) -> Result<_source::Committed, PublicationError> {
    let folder = _plugin::folder(key);
    _source::Committed::working_tree_places(
        root,
        _source::Places {
            folder: &folder,
            file: _plugin::CATALOG,
        },
    )
}

/// Applies `plan` to the working tree whose root is `root`, and nothing
/// while it holds a finding. Each removal comes first, with each folder it
/// leaves empty inside the plugin's folder, so a new file never meets an
/// old file or an emptied folder on its way. Then each write follows, one file at a
/// time: the plugin's other files, then its manifest, then the catalog.
/// Each write is atomic and nothing passes through a symbolic link. A
/// rendering cut short leaves exactly the removals and writes made before
/// the cut; the next rendering compares against what the folder then
/// holds. Returns the paths it changed, relative to `root`.
pub fn write_plugin(root: &Path, plan: &PluginPlan) -> Result<Vec<PathBuf>, PublicationError> {
    let mut changed = Vec::new();
    let Some(drift) = plan.drift.as_ref().filter(|_| plan.findings.is_empty()) else {
        return Ok(changed);
    };
    for path in &drift.removals {
        let target = _tx::own_destination(root, path)?;
        match crate::fsx::remove_file(&target) {
            Ok(()) => changed.push(path.clone()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(PublicationError::Io {
                    path: target,
                    source,
                });
            }
        }
        remove_emptied_folders(root, &plan.folder, path)?;
    }
    for file in drift.writes.iter().chain(&plan.catalog) {
        let target = _tx::own_destination(root, &file.path)?;
        crate::fsx::atomic_write(&target, file.text.as_bytes())?;
        changed.push(file.path.clone());
    }
    Ok(changed)
}

/// Removes each folder above the removed file `removed` that the removal
/// left empty, up to the plugin's folder `folder` and never that folder
/// itself, each through [`_tx::own_destination`]. A folder that still holds
/// anything, or is gone, ends the climb.
fn remove_emptied_folders(
    root: &Path,
    folder: &Path,
    removed: &Path,
) -> Result<(), PublicationError> {
    let mut at = removed.parent();
    while let Some(rel) = at.filter(|rel| rel.starts_with(folder) && *rel != folder) {
        let dir = _tx::own_destination(root, rel)?;
        match crate::fsx::remove_dir(&dir) {
            Ok(()) => at = rel.parent(),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                ) =>
            {
                break;
            }
            Err(source) => return Err(PublicationError::Io { path: dir, source }),
        }
    }
    Ok(())
}

/// A sync could not run to its end. Refusals are not errors (see
/// [`Refusal`]); these are.
#[derive(Debug, thiserror::Error)]
pub enum PublicationError {
    #[error("{} is not inside a git repository", dir.display())]
    NotInRepository { dir: PathBuf },
    #[error("{} does not exist; a project is adopted with `instrukt config init`", path.display())]
    NoProjectLayer { path: PathBuf },
    #[error("{} declares no `project` slug", path.display())]
    NoProjectSlug { path: PathBuf },
    #[error(
        "the committed {CONFIG} declares project `{committed}`, the working tree `{working}`; commit the change to publish it",
        CONFIG = crate::config::CONFIG_FILE_NAME
    )]
    SlugMismatch {
        committed: String,
        working: ProjectSlug,
    },
    #[error("cannot find the primary worktree of the repository at {}", dir.display())]
    NoPrimaryWorktree { dir: PathBuf },
    #[error("cannot tell the default branch of the repository at {}", dir.display())]
    NoDefaultBranch { dir: PathBuf },
    #[error("the default branch `{branch}` has no commit in {}", dir.display())]
    MainHasNoCommit { dir: PathBuf, branch: String },
    #[error("git returned no content for {path} ({object})")]
    UnreadableObject { object: String, path: String },
    #[error("committed path `{path}` leaves the repository")]
    UnsafePath { path: String },
    #[error(
        "{} is a symbolic link or lies behind one; publication writes only inside its root",
        path.display()
    )]
    Unconfined { path: PathBuf },
    #[error(
        "{} is a symbolic link that leads nowhere; publication changes nothing through it",
        path.display()
    )]
    DanglingLink { path: PathBuf },
    #[error("{} cannot take the entrypoint-loading key: {reason}", path.display())]
    Settings { path: PathBuf, reason: String },
    #[error("cannot render {file}: {reason}")]
    Rendering { file: String, reason: String },
    /// A document a committed rendering would hold reads a configured
    /// value, which differs per computer.
    #[error("{document} reads a configured value, which a committed rendering cannot hold")]
    CommittedConfig { document: String },
    /// A document a committed rendering would hold carries a directive the
    /// format does not support.
    #[error("{}", findings.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    CommittedDirectives { findings: Vec<crate::docs::Finding> },
    #[error("cannot access {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} is not valid: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("cannot render a publication record: {0}")]
    Render(serde_yaml_ng::Error),
    #[error(
        "project `{project}` has no published baseline; run `instrukt sync` in the primary checkout, then retry checkout rendering"
    )]
    CheckoutUnpublished { project: ProjectSlug },
    #[error(
        "checkout bootstrap was refused: {0}; resolve the finding and run `instrukt sync` in the primary checkout"
    )]
    CheckoutRefused(String),
    #[error("checkout bootstrap failed: {0}")]
    CheckoutPublication(#[source] Box<SyncError>),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Docs(#[from] DocsError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    State(#[from] PublicationStateError),
}

/// Publication state could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum PublicationStateError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a valid publication record: {source}")]
    Parse {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("cannot render the publication record: {0}")]
    Render(serde_yaml_ng::Error),
    #[error(transparent)]
    Write(#[from] FsError),
}

/// What publication records per project under `state/publication/`: the
/// selected source commit last published (canonical main in installed use),
/// the project's checkout path and
/// its repository's remote. Docs compares it with main for the behind-main
/// warning and names it as the source of what the project publishes; the
/// runner finds projects through it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedRecord {
    pub project: ProjectSlug,
    pub commit: CommitSha,
    /// The source checkout: the primary worktree for installed publication,
    /// or the selected development worktree for explicit development execution.
    pub checkout: PathBuf,
    /// The URL of the repository's `origin` remote; `None` without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

impl PublishedRecord {
    /// The record of `project`; `None` when it never published.
    pub fn load(home: &Home, project: &ProjectSlug) -> Result<Option<Self>, PublicationStateError> {
        let path = record_path(home, project);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(PublicationStateError::Read { path, source }),
        };
        serde_yaml_ng::from_str(&text)
            .map(Some)
            .map_err(|source| PublicationStateError::Parse { path, source })
    }

    /// Every project's record, in project order.
    pub fn load_all(home: &Home) -> Result<Vec<Self>, PublicationStateError> {
        let dir = home.publication_state_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(PublicationStateError::Read { path: dir, source }),
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| PublicationStateError::Read {
                path: dir.clone(),
                source,
            })?;
            let name = entry.file_name();
            let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".yaml")) else {
                continue;
            };
            let Ok(project) = ProjectSlug::parse(stem) else {
                continue;
            };
            if let Some(record) = Self::load(home, &project)? {
                records.push(record);
            }
        }
        records.sort_by(|a, b| a.project.cmp(&b.project));
        Ok(records)
    }
}

/// Every claimed published position's owner, by the domain name the
/// position is addressed by (`general` for `docs/general/`).
pub fn position_owners(
    home: &Home,
) -> Result<std::collections::BTreeMap<crate::ids::DomainName, ProjectSlug>, PublicationStateError>
{
    let dir = home.claims_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(std::collections::BTreeMap::new());
        }
        Err(source) => return Err(PublicationStateError::Read { path: dir, source }),
    };
    let mut owners = std::collections::BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|source| PublicationStateError::Read {
            path: dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "yaml") {
            continue;
        }
        let text =
            std::fs::read_to_string(&path).map_err(|source| PublicationStateError::Read {
                path: path.clone(),
                source,
            })?;
        let record: _tree::ClaimRecord = serde_yaml_ng::from_str(&text)
            .map_err(|source| PublicationStateError::Parse { path, source })?;
        owners.insert(record.position, record.owner);
    }
    Ok(owners)
}

/// The project that owns the published position `domain` addresses
/// (`general` for `docs/general/`), from the ownership record; `None` when
/// no project publishes it.
pub fn domain_owner(
    home: &Home,
    domain: &crate::ids::DomainName,
) -> Result<Option<ProjectSlug>, PublicationStateError> {
    let path = _tree::claim_path(home, domain);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(PublicationStateError::Read { path, source }),
    };
    serde_yaml_ng::from_str::<_tree::ClaimRecord>(&text)
        .map(|record| Some(record.owner))
        .map_err(|source| PublicationStateError::Parse { path, source })
}

fn record_path(home: &Home, project: &ProjectSlug) -> PathBuf {
    home.publication_state_dir()
        .join(format!("{}.yaml", project.as_str()))
}

pub mod status;

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_rendering_replaces_an_emptied_folder_with_the_document_now_at_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let key = ProjectSlug::parse("acme").unwrap();
        let name = CompanyName::parse("Acme").unwrap();
        let references = root.join("plugins/acme/skills/acme/references");
        write(
            root,
            ".claude-plugin/marketplace.json",
            "{\"plugins\": []}\n",
        );
        write(root, "docs/index.yaml", "listed: [guide.md/spring.md]\n");
        write(
            root,
            "docs/guide.md/spring.md",
            "---\ndescription: Spring.\n---\n\n# Spring\n",
        );
        write_plugin(root, &plan_plugin(root, &key, &name).unwrap()).unwrap();
        assert!(references.join("guide.md/spring.md").is_file());

        std::fs::remove_dir_all(root.join("docs/guide.md")).unwrap();
        write(root, "docs/index.yaml", "listed: [guide.md]\n");
        write(
            root,
            "docs/guide.md",
            "---\ndescription: The guide.\n---\n\n# Guide\n",
        );
        let plan = plan_plugin(root, &key, &name).unwrap();
        assert!(plan.findings.is_empty());
        write_plugin(root, &plan).unwrap();
        assert!(references.join("guide.md").is_file());
        let again = plan_plugin(root, &key, &name).unwrap().drift.unwrap();
        assert!(again.writes.is_empty() && again.removals.is_empty());
    }

    #[test]
    fn a_tracked_folder_replaced_by_a_document_reads_as_gone_before_it_is_staged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let key = ProjectSlug::parse("acme").unwrap();
        let name = CompanyName::parse("Acme").unwrap();
        crate::git::run(root, ["init", "--quiet"]).unwrap();
        write(
            root,
            ".claude-plugin/marketplace.json",
            "{\"plugins\": []}\n",
        );
        write(root, "docs/index.yaml", "listed: [guide.md/spring.md]\n");
        write(
            root,
            "docs/guide.md/spring.md",
            "---\ndescription: Spring.\n---\n\n# Spring\n",
        );
        write_plugin(root, &plan_plugin(root, &key, &name).unwrap()).unwrap();
        // The document and its generated file are tracked.
        crate::git::run(root, ["add", "--all"]).unwrap();

        // The document's folder becomes a document, nothing staged: git still
        // lists both the old document and its generated file.
        std::fs::remove_dir_all(root.join("docs/guide.md")).unwrap();
        write(root, "docs/index.yaml", "listed: [guide.md]\n");
        write(
            root,
            "docs/guide.md",
            "---\ndescription: The guide.\n---\n\n# Guide\n",
        );
        let plan = plan_plugin(root, &key, &name).unwrap();
        assert!(plan.findings.is_empty(), "{:?}", plan.findings);
        write_plugin(root, &plan).unwrap();
        let again = plan_plugin(root, &key, &name).unwrap();
        assert!(again.findings.is_empty(), "{:?}", again.findings);
        let drift = again.drift.unwrap();
        assert!(drift.writes.is_empty() && drift.removals.is_empty());
        assert!(again.catalog.is_none());
    }
}
