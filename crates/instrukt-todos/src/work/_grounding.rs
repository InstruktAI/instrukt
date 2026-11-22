//! What an artifact version is grounded on when it is written
//! (`artifact-currency`, "Drift evidence"): the canonical branch's head,
//! the code paths and snippets its content declares, and the snippets the
//! todo owns. Prerequisite artifacts are added by the caller, which holds
//! the project's records.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use instrukt_core::docs::{Corpus, RefTarget};
use instrukt_core::git;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SnippetId, TodoSlug};
use instrukt_runner::interfaces::Lane;

use super::_error::WorkError;
use crate::evidence::{ContentDigest, Grounding, SnippetDigest};

/// The language tag of a fenced block declaring referenced code.
const REFERENCED_PATHS: &str = "referenced-paths";

/// Candidate worktree paths for a todo's own pending carriers, newest
/// work-lane placement first, then newest preparation-lane placement.
pub(crate) fn candidate_worktrees<'a>(
    placements: impl DoubleEndedIterator<Item = (Option<Lane>, Option<&'a Path>)> + Clone,
) -> Vec<&'a Path> {
    let mut out = Vec::new();
    for target in [Lane::Work, Lane::Prepare] {
        for (lane, path) in placements.clone().rev() {
            if lane == Some(target)
                && let Some(path) = path
                && !out.contains(&path)
            {
                out.push(path);
            }
        }
    }
    out
}

/// Low-level I/O or git failure while resolving a todo's owned-snippet checkout
/// or hashing an owned snippet and its companion.
#[derive(Debug)]
pub(crate) enum GroundingIoError {
    Git(git::GitError),
    Io { path: PathBuf, source: io::Error },
}

impl From<git::GitError> for GroundingIoError {
    fn from(err: git::GitError) -> Self {
        Self::Git(err)
    }
}

impl From<GroundingIoError> for WorkError {
    fn from(err: GroundingIoError) -> Self {
        match err {
            GroundingIoError::Git(err) => WorkError::Git(err),
            GroundingIoError::Io { path, source } => WorkError::Io { path, source },
        }
    }
}

/// Resolve the live checkout that holds a todo's own pending snippets: the
/// newest existing work-lane or preparation-lane worktree rooted exactly at
/// its recorded path, falling back to `project_root` once removed or absent.
pub(crate) fn owned_snippet_root(
    project_root: &Path,
    candidates: &[&Path],
) -> Result<PathBuf, GroundingIoError> {
    for candidate in candidates {
        let joined = project_root.join(candidate);
        let canon = match joined.canonicalize() {
            Ok(path) => path,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(GroundingIoError::Io {
                    path: joined,
                    source,
                });
            }
        };
        let Some(root) = git::worktree_root(&canon)? else {
            continue;
        };
        let root = root.canonicalize().map_err(|source| GroundingIoError::Io {
            path: root.clone(),
            source,
        })?;
        if root == canon {
            return Ok(canon);
        }
    }
    Ok(project_root.to_path_buf())
}

/// Deterministic digest combining a live parent snippet with its todo companion
/// file so companion-only edits and empty retirement companions are distinct
/// from both each other and from a companion-less parent.
fn combine_owned_digest(parent: &str, companion: &str) -> ContentDigest {
    let mut combined = String::with_capacity(parent.len() + companion.len() + 32);
    combined.push_str("parent:");
    combined.push_str(&parent.len().to_string());
    combined.push('\n');
    combined.push_str(parent);
    combined.push_str("\ncompanion:");
    combined.push_str(&companion.len().to_string());
    combined.push('\n');
    combined.push_str(companion);
    ContentDigest::of(&combined)
}

/// Digest of a todo-owned snippet read solely from `root` (never falling back
/// to the published store). When `todo` has a companion `<stem>.<slug>.md`,
/// hashes parent and companion together; when none exists, hashes the snippet
/// file directly.
pub(crate) fn owned_snippet_digest(
    root: &Path,
    corpus: &Corpus,
    todo: &TodoSlug,
    id: &SnippetId,
) -> Result<Option<ContentDigest>, GroundingIoError> {
    let Some(snippet) = corpus.snippet(id) else {
        return Ok(None);
    };
    let parent_path = root.join(snippet.path());
    let parent_text = match std::fs::read_to_string(&parent_path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(GroundingIoError::Io {
                path: parent_path,
                source,
            });
        }
    };
    let Some(companion) = snippet.companions().iter().find(|c| c.slug() == todo) else {
        return Ok(Some(ContentDigest::of(&parent_text)));
    };
    let companion_path = root.join(companion.path());
    let companion_text = match std::fs::read_to_string(&companion_path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(GroundingIoError::Io {
                path: companion_path,
                source,
            });
        }
    };
    Ok(Some(combine_owned_digest(&parent_text, &companion_text)))
}

/// Record absent previously-owned snippet IDs with `digest: None` so a drift
/// judgment that accepted deletion detects later restoration.
fn append_absent_owned(
    owned_snippets: &mut Vec<SnippetDigest>,
    owned_ids: &mut BTreeSet<SnippetId>,
    prior_owned: &[SnippetId],
) {
    for id in prior_owned {
        if owned_ids.insert(id.clone()) {
            owned_snippets.push(SnippetDigest {
                id: id.clone(),
                digest: None,
            });
        }
    }
}

/// The grounding of `content`, an artifact of todo `todo` in the project
/// rooted at `project_root`, with todo-owned pending snippets read solely from
/// `owned_root` (the active todo worktree when present, else `project_root`),
/// whose canonical branch is `base_branch`. `prior_owned` retains previously
/// recorded owned IDs as absent (`digest: None`) when deleted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ground(
    home: &Home,
    project_root: &Path,
    owned_root: &Path,
    project: &ProjectSlug,
    base_branch: &str,
    todo: &TodoSlug,
    content: &str,
    prior_owned: &[SnippetId],
) -> Result<Grounding, WorkError> {
    let project_corpus = Corpus::load(project_root, home)?;
    let separate_owned = owned_root != project_root;
    let owned_corpus = if separate_owned {
        Some(Corpus::load(owned_root, home)?)
    } else {
        None
    };
    let active_corpus = owned_corpus.as_ref().unwrap_or(&project_corpus);
    let mut owned_snippets = Vec::new();
    let mut owned_ids = BTreeSet::new();
    for snippet in active_corpus.snippets() {
        if snippet.has_material_for(todo) {
            let digest = owned_snippet_digest(owned_root, active_corpus, todo, snippet.id())?;
            owned_ids.insert(snippet.id().clone());
            owned_snippets.push(SnippetDigest {
                id: snippet.id().clone(),
                digest,
            });
        }
    }
    append_absent_owned(&mut owned_snippets, &mut owned_ids, prior_owned);
    let digest_in_project = |id: &SnippetId| -> Result<Option<ContentDigest>, WorkError> {
        let path = match project_corpus.snippet(id) {
            Some(snippet) => project_root.join(snippet.path()),
            None => home.docs_dir().join(format!("{id}.md")),
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(ContentDigest::of(&text))),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(WorkError::Io { path, source }),
        }
    };
    let mut referenced_snippets = Vec::new();
    for reference in instrukt_core::docs::references(content, Some(project), home.user_home()) {
        if let Some(RefTarget::Snippet(id)) = reference.target
            && !owned_ids.contains(&id)
            && !referenced_snippets
                .iter()
                .any(|s: &SnippetDigest| s.id == id)
        {
            let digest = digest_in_project(&id)?;
            referenced_snippets.push(SnippetDigest { id, digest });
        }
    }
    Ok(Grounding {
        head: head(project_root, base_branch)?,
        referenced_paths: referenced_paths(content),
        referenced_snippets,
        owned_snippets,
        dependencies: Vec::new(),
    })
}

/// The canonical branch's head commit; `None` when it has none.
fn head(root: &Path, base_branch: &str) -> Result<Option<CommitSha>, WorkError> {
    let spec = format!("{base_branch}^{{commit}}");
    match git::run(root, ["rev-parse", "--verify", "--quiet", spec.as_str()]) {
        Ok(sha) => Ok(CommitSha::parse(sha.trim()).ok()),
        Err(git::GitError::Failed { .. }) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// The paths of the content's fenced `referenced-paths` blocks, one path or
/// directory prefix per line, `#` lines ignored; sorted, each once.
pub(crate) fn referenced_paths(content: &str) -> Vec<String> {
    let mut paths = std::collections::BTreeSet::new();
    let mut inside = false;
    for raw in content.lines() {
        let line = raw.trim();
        if inside {
            if line.starts_with("```") {
                inside = false;
            } else if !line.is_empty() && !line.starts_with('#') {
                paths.insert(line.to_owned());
            }
            continue;
        }
        if let Some(info) = line.strip_prefix("```") {
            inside = info.split_whitespace().next() == Some(REFERENCED_PATHS);
        }
    }
    paths.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_worktrees_prefers_newest_work_then_prepare_and_deduplicates() {
        let prep_old = Path::new("/wt/prep-1");
        let prep_new = Path::new("/wt/prep-2");
        let work_tree = Path::new("/wt/work-1");
        let placements = [
            (Some(Lane::Prepare), Some(prep_old)),
            (Some(Lane::Prepare), Some(prep_new)),
            (Some(Lane::Work), Some(work_tree)),
            (Some(Lane::Prepare), Some(prep_new)),
            (None, Some(Path::new("/wt/ignored"))),
        ];
        assert_eq!(
            candidate_worktrees(placements.into_iter()),
            vec![work_tree, prep_new, prep_old]
        );
    }

    #[test]
    fn candidate_worktrees_selects_prepare_tree_when_no_work_binding_exists() {
        let prep = Path::new("/wt/prep-only");
        let placements = [(Some(Lane::Prepare), Some(prep))];
        assert_eq!(candidate_worktrees(placements.into_iter()), vec![prep]);
    }

    #[test]
    fn combine_owned_digest_distinguishes_companion_edit_and_empty_retirement_companion() {
        let parent = "# Spec\n\nLive body.\n";
        let parent_only = ContentDigest::of(parent);
        let with_v1 = combine_owned_digest(parent, "# Spec\n\nNext v1.\n");
        let with_v2 = combine_owned_digest(parent, "# Spec\n\nNext v2.\n");
        let empty_retire = combine_owned_digest(parent, "");
        assert_ne!(with_v1, with_v2);
        assert_ne!(empty_retire, parent_only);
        assert_ne!(empty_retire, with_v1);
    }

    #[test]
    fn append_absent_owned_retains_deleted_prior_ids_as_none_without_duplicating_present() {
        let present = SnippetId::parse("project/demo/policy/present").unwrap();
        let deleted = SnippetId::parse("project/demo/policy/deleted").unwrap();
        let mut owned = vec![SnippetDigest {
            id: present.clone(),
            digest: Some(ContentDigest::of("body")),
        }];
        let mut ids: BTreeSet<SnippetId> = [present.clone()].into_iter().collect();
        append_absent_owned(&mut owned, &mut ids, &[present, deleted.clone()]);
        assert_eq!(
            owned,
            vec![
                SnippetDigest {
                    id: SnippetId::parse("project/demo/policy/present").unwrap(),
                    digest: Some(ContentDigest::of("body")),
                },
                SnippetDigest {
                    id: deleted,
                    digest: None,
                },
            ]
        );
    }
}
