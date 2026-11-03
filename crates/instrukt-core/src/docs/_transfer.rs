//! Pending-ownership transfer: every pending carrier naming one todo is
//! made to name another (`local/projects/instrukt/spec/feature/work/dependencies-and-holds`,
//! Structural holders: a new holder's pending documentation moves to its
//! first child).
//!
//! The whole transfer is planned in memory first; a refusal writes nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::_corpus::{SourceFile, read_tree};
use super::_fold::{confined, rewrite_delivered_by};
use super::_identity::companion_shape;
use super::_markers;
use super::_text::split_frontmatter;
use super::DocsError;
use crate::fsx::atomic_write;
use crate::ids::TodoSlug;

/// What a transfer changed, repository-relative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransferOutcome {
    /// Files rewritten in place.
    pub written: Vec<PathBuf>,
    /// Companions renamed `<stem>.<from>.md` → `<stem>.<to>.md`, their
    /// contents rewritten as well.
    pub renamed: Vec<(PathBuf, PathBuf)>,
}

impl TransferOutcome {
    pub fn is_empty(&self) -> bool {
        self.written.is_empty() && self.renamed.is_empty()
    }
}

/// One planned change: the file's new text, and its new path when it is a
/// companion being renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Change {
    pub text: String,
    pub moved_to: Option<String>,
}

/// Makes every pending carrier naming `from` in the documentation of the
/// working tree rooted at `repo_root` name `to` instead: its `delivered_by`
/// entries, its `planned`, `planned-change` and `change` markers, and its
/// companions `<stem>.<from>.md`, renamed to `<stem>.<to>.md`. Every other
/// todo's carriers are left as they are. Nothing is committed.
///
/// Refuses, changing nothing, when a renamed companion would land on an
/// existing file, when a `delivered_by` naming `from` is in a form that
/// cannot be rewritten, and when `docs/` or a file to change lies outside
/// the repository.
pub fn transfer_pending(
    repo_root: &Path,
    from: &TodoSlug,
    to: &TodoSlug,
) -> Result<TransferOutcome, DocsError> {
    let started = Instant::now();
    let root = repo_root.canonicalize().map_err(|source| DocsError::Read {
        path: repo_root.to_path_buf(),
        source,
    })?;
    let plan = plan(read_tree(&root)?, from, to)?;
    let mut targets = Vec::new();
    for (path, change) in &plan {
        let source = confined(&root, path)?;
        let destination = match &change.moved_to {
            Some(next) => {
                let destination = confined(&root, next)?;
                if std::fs::symlink_metadata(&destination).is_ok() {
                    return Err(DocsError::TransferRefused {
                        path: PathBuf::from(next),
                        reason: format!("a companion of {to} is already there"),
                    });
                }
                destination
            }
            None => source.clone(),
        };
        targets.push((source, destination, path, change));
    }
    let mut outcome = TransferOutcome::default();
    for (source, destination, path, change) in targets {
        atomic_write(&destination, change.text.as_bytes())?;
        match &change.moved_to {
            Some(next) => {
                std::fs::remove_file(&source).map_err(|err| DocsError::Remove {
                    path: source.clone(),
                    source: err,
                })?;
                outcome
                    .renamed
                    .push((PathBuf::from(path), PathBuf::from(next)));
            }
            None => outcome.written.push(PathBuf::from(path)),
        }
    }
    tracing::debug!(
        event = "docs_transfer_completed",
        from = %from,
        to = %to,
        repo_root = %root.display(),
        written = outcome.written.len(),
        renamed = outcome.renamed.len(),
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    Ok(outcome)
}

/// The changes transferring `from`'s carriers to `to` makes, by current
/// path. Only changed files are listed.
pub(super) fn plan(
    sources: Vec<SourceFile>,
    from: &TodoSlug,
    to: &TodoSlug,
) -> Result<BTreeMap<String, Change>, DocsError> {
    let mut out = BTreeMap::new();
    if from == to {
        return Ok(out);
    }
    let files: BTreeMap<String, String> = sources.into_iter().map(|s| (s.path, s.text)).collect();
    for (path, text) in &files {
        let moved_to = match companion_shape(path) {
            Some((parent, slug)) if slug == from.as_str() && files.contains_key(&parent) => {
                let stem = parent.strip_suffix(".md").unwrap_or(&parent);
                let next = format!("{stem}.{to}.md");
                if files.contains_key(&next) {
                    return Err(DocsError::TransferRefused {
                        path: PathBuf::from(next),
                        reason: format!("a companion of {to} is already there"),
                    });
                }
                Some(next)
            }
            _ => None,
        };
        let rewritten = rewrite(path, text, from, to)?;
        if moved_to.is_some() || rewritten != *text {
            out.insert(
                path.clone(),
                Change {
                    text: rewritten,
                    moved_to,
                },
            );
        }
    }
    Ok(out)
}

/// One file's text with `from` replaced by `to` in its `delivered_by` and
/// its markers.
fn rewrite(path: &str, text: &str, from: &TodoSlug, to: &TodoSlug) -> Result<String, DocsError> {
    let split = split_frontmatter(text);
    let head = &text[..split.body_offset];
    let head = rewrite_delivered_by(head, split.frontmatter, from, Some(to)).ok_or_else(|| {
        DocsError::TransferRefused {
            path: PathBuf::from(path),
            reason: format!(
                "its `delivered_by` names {from} in a form that cannot be rewritten; write it as a top-level list"
            ),
        }
    })?;
    Ok(format!("{head}{}", _markers::rename(split.body, from, to)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::docs::_fold::delivered_by;

    fn slug(s: &str) -> TodoSlug {
        TodoSlug::parse(s).unwrap()
    }

    fn sources(files: &[(&str, &str)]) -> Vec<SourceFile> {
        files
            .iter()
            .map(|(p, t)| SourceFile {
                path: (*p).to_owned(),
                text: (*t).to_owned(),
            })
            .collect()
    }

    fn transfer(files: &[(&str, &str)]) -> Result<BTreeMap<String, Change>, DocsError> {
        plan(sources(files), &slug("parent"), &slug("child"))
    }

    /// The todos a text's `delivered_by` and its markers name.
    fn owners(text: &str) -> (Vec<TodoSlug>, BTreeSet<TodoSlug>) {
        let split = split_frontmatter(text);
        let delivered = delivered_by(split.frontmatter)
            .iter()
            .map(|s| slug(s))
            .collect();
        (delivered, _markers::scan(split.body).slugs)
    }

    const X: &str = "docs/local/spec/x.md";

    #[test]
    fn delivered_by_entries_move_to_the_new_owner() {
        let text = "---\ndescription: d\ndelivered_by: [parent, other]\neval: true\n---\nbody\n";
        let out = transfer(&[(X, text)]).unwrap();
        let (delivered, _) = owners(&out[X].text);
        assert_eq!(delivered, [slug("child"), slug("other")]);
        assert_eq!(out[X].moved_to, None);
    }

    #[test]
    fn an_owner_already_listed_is_not_listed_twice() {
        let text = "---\ndescription: d\ndelivered_by:\n  - child\n  - parent\n---\nbody\n";
        let out = transfer(&[(X, text)]).unwrap();
        assert_eq!(owners(&out[X].text).0, [slug("child")]);
    }

    #[test]
    fn markers_move_and_other_todos_markers_stay() {
        // Lines: 0 open, 1 add, 2 close, 3 change-open, 4 old, 5 separator,
        // 6 new, 7 change-close, 8-10 another todo's block, 11 a sample.
        let body = "<!-- planned:parent -->\nadd\n<!-- /planned:parent -->\n\
                    <!--  planned-change:parent -->\nold\n<!-- change:parent -->\nnew\n\
                    <!-- /planned-change:parent -->\n\
                    <!-- planned:other -->\nlater\n<!-- /planned:other -->\n\
                    `<!-- planned:parent -->`\n";
        let text = format!("---\ndescription: d\n---\n{body}");
        let out = transfer(&[(X, &text)]).unwrap();
        let folded = &out[X].text;
        let (_, marker_owners) = owners(folded);
        assert_eq!(
            marker_owners,
            BTreeSet::from([slug("child"), slug("other")])
        );
        let before = _markers::scan(body);
        let after = _markers::scan(split_frontmatter(folded).body);
        assert!(after.problems.is_empty());
        let shape = |scan: &_markers::Scan| -> Vec<(usize, Option<usize>, usize)> {
            scan.blocks
                .iter()
                .map(|b| (b.open, b.separator, b.close))
                .collect()
        };
        assert_eq!(shape(&after), shape(&before));
        let unchanged =
            |i: usize| body.lines().nth(i) == split_frontmatter(folded).body.lines().nth(i);
        assert!([1, 4, 6, 8, 9, 10, 11].into_iter().all(unchanged));
    }

    #[test]
    fn companions_are_renamed_with_their_contents_rewritten() {
        let companion =
            "---\ndescription: d\n---\n<!-- planned:parent -->\nx\n<!-- /planned:parent -->\n";
        let out = transfer(&[
            (X, "---\ndescription: d\n---\nlive\n"),
            ("docs/local/spec/x.parent.md", companion),
            ("docs/local/spec/x.other.md", ""),
            ("docs/local/spec/y.md", "---\ndescription: d\n---\nlive\n"),
            ("docs/local/spec/y.parent.md", ""),
        ])
        .unwrap();
        let moved: BTreeMap<&str, Option<&str>> = out
            .iter()
            .map(|(path, change)| (path.as_str(), change.moved_to.as_deref()))
            .collect();
        assert_eq!(
            moved,
            BTreeMap::from([
                (
                    "docs/local/spec/x.parent.md",
                    Some("docs/local/spec/x.child.md")
                ),
                (
                    "docs/local/spec/y.parent.md",
                    Some("docs/local/spec/y.child.md")
                ),
            ])
        );
        let (_, marker_owners) = owners(&out["docs/local/spec/x.parent.md"].text);
        assert_eq!(marker_owners, BTreeSet::from([slug("child")]));
        assert!(out["docs/local/spec/y.parent.md"].text.is_empty());
    }

    #[test]
    fn a_companion_landing_on_an_existing_one_is_refused() {
        let refused = transfer(&[
            (X, "live\n"),
            ("docs/local/spec/x.parent.md", "mine\n"),
            ("docs/local/spec/x.child.md", "theirs\n"),
        ]);
        assert!(matches!(refused, Err(DocsError::TransferRefused { .. })));
    }

    #[test]
    fn a_delivered_by_that_cannot_be_rewritten_is_refused() {
        let text = "---\n{description: d, delivered_by: [parent]}\n---\nbody\n";
        assert!(matches!(
            transfer(&[(X, text)]),
            Err(DocsError::TransferRefused { .. })
        ));
    }

    #[test]
    fn nothing_naming_the_old_owner_means_nothing_changes() {
        let text = "---\ndescription: d\ndelivered_by: [other]\n---\n<!-- planned:other -->\nx\n<!-- /planned:other -->\n";
        assert!(
            transfer(&[(X, text), ("docs/local/spec/x.other.md", "")])
                .unwrap()
                .is_empty()
        );
        assert!(
            plan(sources(&[(X, text)]), &slug("other"), &slug("other"))
                .unwrap()
                .is_empty()
        );
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_transfer_rewrites_and_renames_files_in_the_working_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            X,
            "---\ndescription: d\ndelivered_by: [parent]\n---\nbody\n",
        );
        write(
            root,
            "docs/local/spec/y.md",
            "---\ndescription: d\n---\nlive\n",
        );
        write(root, "docs/local/spec/y.parent.md", "");
        write(
            root,
            "docs/local/spec/keep.md",
            "---\ndescription: d\n---\nother\n",
        );
        let before = std::fs::read(root.join("docs/local/spec/keep.md")).unwrap();
        let outcome = transfer_pending(root, &slug("parent"), &slug("child")).unwrap();
        assert_eq!(outcome.written, [PathBuf::from(X)]);
        assert_eq!(
            outcome.renamed,
            [(
                PathBuf::from("docs/local/spec/y.parent.md"),
                PathBuf::from("docs/local/spec/y.child.md")
            )]
        );
        let x = std::fs::read_to_string(root.join(X)).unwrap();
        assert_eq!(owners(&x).0, [slug("child")]);
        assert!(root.join("docs/local/spec/y.child.md").exists());
        assert!(!root.join("docs/local/spec/y.parent.md").exists());
        assert_eq!(
            std::fs::read(root.join("docs/local/spec/keep.md")).unwrap(),
            before
        );
        assert!(
            transfer_pending(root, &slug("parent"), &slug("child"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_refused_transfer_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            X,
            "---\ndescription: d\ndelivered_by: [parent]\n---\nbody\n",
        );
        write(root, "docs/local/spec/y.md", "live\n");
        write(root, "docs/local/spec/y.parent.md", "mine\n");
        write(root, "docs/local/spec/y.child.md", "theirs\n");
        let before = std::fs::read(root.join(X)).unwrap();
        assert!(matches!(
            transfer_pending(root, &slug("parent"), &slug("child")),
            Err(DocsError::TransferRefused { .. })
        ));
        assert_eq!(std::fs::read(root.join(X)).unwrap(), before);
        assert!(root.join("docs/local/spec/y.parent.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_transfer_never_changes_files_outside_the_repository() {
        let outside = tempfile::tempdir().unwrap();
        let pending = "---\ndescription: d\ndelivered_by: [parent]\n---\nx\n";
        write(outside.path(), "local/spec/x.md", pending);
        let before = std::fs::read(outside.path().join("local/spec/x.md")).unwrap();
        let repo = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.path().join("docs")).unwrap();
        assert!(matches!(
            transfer_pending(repo.path(), &slug("parent"), &slug("child")),
            Err(DocsError::OutsideRepository { .. })
        ));
        let inner = tempfile::tempdir().unwrap();
        write(inner.path(), "docs/local/spec/real.md", "y\n");
        std::os::unix::fs::symlink(
            outside.path().join("local/spec/x.md"),
            inner.path().join("docs/local/spec/linked.md"),
        )
        .unwrap();
        assert!(
            transfer_pending(inner.path(), &slug("parent"), &slug("child"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::read(outside.path().join("local/spec/x.md")).unwrap(),
            before
        );
    }
}
