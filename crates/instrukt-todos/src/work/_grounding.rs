//! What an artifact version is grounded on when it is written
//! (`artifact-currency`, "Drift evidence"): the canonical branch's head,
//! the code paths and snippets its content declares, and the snippets the
//! todo owns. Prerequisite artifacts are added by the caller, which holds
//! the project's records.

use std::io;
use std::path::Path;

use instrukt_core::docs::{Corpus, RefTarget};
use instrukt_core::git;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SnippetId, TodoSlug};

use super::_error::WorkError;
use crate::evidence::{ContentDigest, Grounding, SnippetDigest};

/// The language tag of a fenced block declaring referenced code.
const REFERENCED_PATHS: &str = "referenced-paths";

/// The grounding of `content`, an artifact of todo `todo` in the project
/// rooted at `root`, whose canonical branch is `base_branch`.
pub(crate) fn ground(
    home: &Home,
    root: &Path,
    project: &ProjectSlug,
    base_branch: &str,
    todo: &TodoSlug,
    content: &str,
) -> Result<Grounding, WorkError> {
    let corpus = Corpus::load(root, home)?;
    let digest_of = |id: &SnippetId| -> Result<Option<ContentDigest>, WorkError> {
        let path = match corpus.snippet(id) {
            Some(snippet) => root.join(snippet.path()),
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
            && !referenced_snippets
                .iter()
                .any(|s: &SnippetDigest| s.id == id)
        {
            let digest = digest_of(&id)?;
            referenced_snippets.push(SnippetDigest { id, digest });
        }
    }
    let mut owned_snippets = Vec::new();
    for snippet in corpus.snippets() {
        if snippet.has_material_for(todo) {
            let digest = digest_of(snippet.id())?;
            owned_snippets.push(SnippetDigest {
                id: snippet.id().clone(),
                digest,
            });
        }
    }
    Ok(Grounding {
        head: head(root, base_branch)?,
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
