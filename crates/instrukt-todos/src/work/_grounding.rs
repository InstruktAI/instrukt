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
/// rooted at `project_root`, with todo-owned pending snippets read from
/// `owned_root` (the active worktree when present, else `project_root`),
/// whose canonical branch is `base_branch`.
pub(crate) fn ground(
    home: &Home,
    project_root: &Path,
    owned_root: &Path,
    project: &ProjectSlug,
    base_branch: &str,
    todo: &TodoSlug,
    content: &str,
) -> Result<Grounding, WorkError> {
    let project_corpus = Corpus::load(project_root, home)?;
    let digest_in = |corpus: &Corpus,
                     root: &Path,
                     id: &SnippetId|
     -> Result<Option<ContentDigest>, WorkError> {
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
            let digest = digest_in(&project_corpus, project_root, &id)?;
            referenced_snippets.push(SnippetDigest { id, digest });
        }
    }
    let separate_owned = owned_root != project_root;
    let owned_corpus = if separate_owned {
        Some(Corpus::load(owned_root, home)?)
    } else {
        None
    };
    let active_corpus = owned_corpus.as_ref().unwrap_or(&project_corpus);
    let mut owned_snippets = Vec::new();
    for snippet in active_corpus.snippets() {
        if snippet.has_material_for(todo) {
            let digest = digest_in(active_corpus, owned_root, snippet.id())?;
            owned_snippets.push(SnippetDigest {
                id: snippet.id().clone(),
                digest,
            });
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
    fn ground_reads_referenced_snippets_from_primary_and_owned_snippets_from_worktree() {
        let home_dir = tempfile::tempdir().unwrap();
        let primary = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        for dir in [primary.path(), worktree.path()] {
            std::fs::write(dir.join("instrukt.yaml"), "project: demo\n").unwrap();
            std::fs::create_dir_all(dir.join("docs/project/policy")).unwrap();
        }
        let primary_ref = "---\ndescription: ref.\n---\n# Upstream\n\nNew main text.\n";
        let stale_ref = "---\ndescription: ref.\n---\n# Upstream\n\nOld worktree text.\n";
        let primary_owned = "---\ndescription: owned.\n---\n# Owned\n\nBase.\n";
        let worktree_owned = "---\ndescription: owned.\n---\n# Owned\n\nBase.\n<!-- planned:my-todo -->\nPending.\n<!-- /planned:my-todo -->\n";
        std::fs::write(
            primary.path().join("docs/project/policy/upstream.md"),
            primary_ref,
        )
        .unwrap();
        std::fs::write(
            worktree.path().join("docs/project/policy/upstream.md"),
            stale_ref,
        )
        .unwrap();
        std::fs::write(
            primary.path().join("docs/project/policy/owned.md"),
            primary_owned,
        )
        .unwrap();
        std::fs::write(
            worktree.path().join("docs/project/policy/owned.md"),
            worktree_owned,
        )
        .unwrap();

        git::run(primary.path(), ["init", "-b", "main"]).unwrap();
        git::run(primary.path(), ["config", "user.name", "Test"]).unwrap();
        git::run(primary.path(), ["config", "user.email", "test@example.com"]).unwrap();
        git::run(primary.path(), ["add", "."]).unwrap();
        git::run(primary.path(), ["commit", "-m", "init"]).unwrap();

        let home = Home::at_user_home(home_dir.path().to_path_buf());
        let project = ProjectSlug::parse("demo").unwrap();
        let todo = TodoSlug::parse("my-todo").unwrap();
        let artifact = "See @docs/project/policy/upstream.md for the upstream rule.\n";
        let grounding = ground(
            &home,
            primary.path(),
            worktree.path(),
            &project,
            "main",
            &todo,
            artifact,
        )
        .unwrap();
        assert_eq!(
            grounding.referenced_snippets,
            vec![SnippetDigest {
                id: SnippetId::parse("project/demo/policy/upstream").unwrap(),
                digest: Some(ContentDigest::of(primary_ref)),
            }]
        );
        assert_eq!(
            grounding.owned_snippets,
            vec![SnippetDigest {
                id: SnippetId::parse("project/demo/policy/owned").unwrap(),
                digest: Some(ContentDigest::of(worktree_owned)),
            }]
        );
    }
}
