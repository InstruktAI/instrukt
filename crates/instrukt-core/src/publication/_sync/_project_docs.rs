//! Publication of a project's own documentation within a sync: its
//! position, its third-party docs and the index the sync leaves. A
//! company's repository publishes none of them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::super::_source::Project;
use super::super::_tree::{self, ProjectClaimRecord};
use super::super::_tx::{Transaction, own_destination};
use super::super::PublicationError;
use super::{Prepared, relative_to};
use crate::docs::Position;
use crate::home::Home;
use crate::ids::ProjectSlug;

/// The project's position, its third-party docs and the index this sync
/// leaves, all through `tx`.
pub(super) fn publish_project_docs(
    home: &Home,
    project: &Project,
    prepared: &Prepared,
    staging: &tempfile::TempDir,
    tx: &mut Transaction,
) -> Result<(), PublicationError> {
    let root = home.root();
    let in_home = |path: &Path| own_destination(root, &relative_to(root, path)?);
    // Project positions use the same transaction and whole-tree replacement.
    let project_dir = in_home(&home.project_docs_dir(&project.slug))?;
    let claim_path = in_home(&_tree::project_claim_path(home, &project.slug))?;
    if prepared.committed.below("docs/project/").next().is_some() {
        let staged = staging.path().join("project").join(project.slug.as_str());
        _tree::stage_position(&prepared.committed, &Position::Project, &staged)?;
        _tree::stage_units(&prepared.committed, &Position::Project, &staged)?;
        tx.replace_dir(&staged, &project_dir)?;
        let claim = ProjectClaimRecord {
            project: project.slug.clone(),
            owner: project.slug.clone(),
        };
        let text = serde_yaml_ng::to_string(&claim).map_err(PublicationError::Render)?;
        tx.write(&claim_path, text.as_bytes())?;
    } else {
        tx.remove_dir(&project_dir)?;
        tx.remove_file(&claim_path)?;
    }
    // Third-party docs: file by file, owner-scoped.
    let docs = home.docs_dir();
    let mine = |owner: &ProjectSlug| owner == &project.slug;
    for doc in prepared
        .patched
        .third_party
        .iter()
        .filter(|d| mine(&d.source_project))
    {
        let source = Path::new("docs").join(&doc.path);
        if let Some(bytes) = prepared.committed.file(&source.to_string_lossy()) {
            tx.write(&in_home(&docs.join(&doc.path))?, bytes)?;
        }
    }
    let third_party: BTreeSet<&PathBuf> = prepared
        .patched
        .third_party
        .iter()
        .map(|d| &d.path)
        .collect();
    for old in prepared
        .index
        .third_party
        .iter()
        .filter(|d| mine(&d.source_project))
    {
        if !third_party.contains(&old.path) {
            tx.remove_file(&in_home(&docs.join(&old.path))?)?;
        }
    }
    let index_text =
        serde_yaml_ng::to_string(&prepared.patched).map_err(PublicationError::Render)?;
    tx.write(&in_home(&home.index_file())?, index_text.as_bytes())?;
    Ok(())
}
