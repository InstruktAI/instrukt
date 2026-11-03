//! Pending-ownership transfer: every pending carrier naming one todo is
//! made to name another (`local/projects/instrukt/spec/feature/work/dependencies-and-holds`,
//! Structural holders: a new holder's pending documentation moves to its
//! first child).
//!
//! The whole transfer is planned in memory first; a refusal writes nothing,
//! and a failed write restores what was already written.

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
/// All or nothing. Every file is read and every rewrite planned before the
/// first write, and each file is written atomically. A refusal — a renamed
/// companion landing on an existing file, a `delivered_by` naming `from` in
/// a form that cannot be rewritten, `docs/` or a file lying outside the
/// repository — writes nothing. When a write fails part-way, the files
/// already changed are restored to their original bytes and the failure is
/// reported as [`DocsError::TransferFailed`], naming any file that could not
/// be restored.
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
    let sources = read_tree(&root)?;
    let originals: BTreeMap<String, String> = sources
        .iter()
        .map(|s| (s.path.clone(), s.text.clone()))
        .collect();
    let plan = plan(sources, from, to)?;
    let mut steps = Vec::new();
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
                Some(destination)
            }
            None => None,
        };
        steps.push(Step {
            rel: path.clone(),
            moved_to: change.moved_to.clone(),
            source,
            destination,
            original: originals.get(path).cloned().unwrap_or_default(),
            text: change.text.clone(),
        });
    }
    let outcome = apply(&steps, &mut WorkingTree)?;
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

/// One planned file change, resolved to absolute paths.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    /// Repository-relative path now.
    rel: String,
    /// Repository-relative path after a companion rename.
    moved_to: Option<String>,
    source: PathBuf,
    destination: Option<PathBuf>,
    /// The file's bytes as read before planning.
    original: String,
    text: String,
}

/// The two effects a transfer has on the working tree. Behind a trait so
/// a failure part-way through can be injected and the restore observed.
trait Files {
    fn write(&mut self, path: &Path, text: &str) -> Result<(), DocsError>;
    fn remove(&mut self, path: &Path) -> Result<(), DocsError>;
}

/// The working tree itself: atomic writes and plain removals.
struct WorkingTree;

impl Files for WorkingTree {
    fn write(&mut self, path: &Path, text: &str) -> Result<(), DocsError> {
        Ok(atomic_write(path, text.as_bytes())?)
    }

    fn remove(&mut self, path: &Path) -> Result<(), DocsError> {
        std::fs::remove_file(path).map_err(|source| DocsError::Remove {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// What has been done for one step, so it can be undone.
#[derive(Debug, Clone, Copy, Default)]
struct Done {
    /// The new text is written (in place, or at the destination).
    written: bool,
    /// The renamed companion's old file is removed.
    removed: bool,
}

/// Applies every step; on the first failure, undoes the steps done so far
/// and reports the failure with the files that could not be restored.
fn apply(steps: &[Step], files: &mut dyn Files) -> Result<TransferOutcome, DocsError> {
    let mut done: Vec<Done> = Vec::new();
    for step in steps {
        let mut progress = Done::default();
        let result = (|| {
            let target = step.destination.as_ref().unwrap_or(&step.source);
            files.write(target, &step.text)?;
            progress.written = true;
            if step.destination.is_some() {
                files.remove(&step.source)?;
                progress.removed = true;
            }
            Ok::<(), DocsError>(())
        })();
        done.push(progress);
        if let Err(cause) = result {
            let unrestored = undo(steps, &done, files);
            return Err(DocsError::TransferFailed {
                cause: Box::new(cause),
                unrestored,
            });
        }
    }
    let mut outcome = TransferOutcome::default();
    for step in steps {
        match &step.moved_to {
            Some(next) => outcome
                .renamed
                .push((PathBuf::from(&step.rel), PathBuf::from(next))),
            None => outcome.written.push(PathBuf::from(&step.rel)),
        }
    }
    Ok(outcome)
}

/// Restores each step's original bytes, newest first; returns the files
/// that could not be restored.
fn undo(steps: &[Step], done: &[Done], files: &mut dyn Files) -> Vec<PathBuf> {
    let mut unrestored = Vec::new();
    for (step, progress) in steps.iter().zip(done).rev() {
        match &step.destination {
            None if progress.written => {
                if files.write(&step.source, &step.original).is_err() {
                    unrestored.push(step.source.clone());
                }
            }
            Some(destination) => {
                if progress.removed && files.write(&step.source, &step.original).is_err() {
                    unrestored.push(step.source.clone());
                }
                if progress.written && files.remove(destination).is_err() {
                    unrestored.push(destination.clone());
                }
            }
            None => {}
        }
    }
    unrestored
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

    /// Files in memory; the operation numbered `fail_from` and every later
    /// one fails, until `fail_until` (exclusive) when given.
    struct Memory {
        files: BTreeMap<PathBuf, String>,
        ops: usize,
        fail_from: usize,
        fail_until: usize,
    }

    impl Memory {
        fn new(files: &[(&str, &str)], fail_from: usize, fail_until: usize) -> Self {
            Self {
                files: files
                    .iter()
                    .map(|(p, t)| (PathBuf::from(p), (*t).to_owned()))
                    .collect(),
                ops: 0,
                fail_from,
                fail_until,
            }
        }

        fn tick(&mut self, path: &Path) -> Result<(), DocsError> {
            self.ops += 1;
            if (self.fail_from..self.fail_until).contains(&self.ops) {
                return Err(DocsError::OutsideRepository {
                    path: path.to_path_buf(),
                });
            }
            Ok(())
        }
    }

    impl Files for Memory {
        fn write(&mut self, path: &Path, text: &str) -> Result<(), DocsError> {
            self.tick(path)?;
            self.files.insert(path.to_path_buf(), text.to_owned());
            Ok(())
        }

        fn remove(&mut self, path: &Path) -> Result<(), DocsError> {
            self.tick(path)?;
            self.files.remove(path);
            Ok(())
        }
    }

    const A: &str = "a.md";
    const B: &str = "b.md";
    const OLD: &str = "c.parent.md";
    const NEW: &str = "c.child.md";

    fn step(path: &str, original: &str, text: &str, moved_to: Option<&str>) -> Step {
        Step {
            rel: path.to_owned(),
            moved_to: moved_to.map(str::to_owned),
            source: PathBuf::from(path),
            destination: moved_to.map(PathBuf::from),
            original: original.to_owned(),
            text: text.to_owned(),
        }
    }

    fn steps() -> Vec<Step> {
        vec![
            step(A, "a0", "a1", None),
            step(OLD, "c0", "c1", Some(NEW)),
            step(B, "b0", "b1", None),
        ]
    }

    const BEFORE: [(&str, &str); 3] = [(A, "a0"), (B, "b0"), (OLD, "c0")];

    fn state(memory: &Memory) -> Vec<(PathBuf, String)> {
        memory.files.clone().into_iter().collect()
    }

    #[test]
    fn a_completed_transfer_writes_and_renames_every_file() {
        let mut memory = Memory::new(&BEFORE, usize::MAX, usize::MAX);
        let outcome = apply(&steps(), &mut memory).unwrap();
        assert_eq!(outcome.written, [PathBuf::from(A), PathBuf::from(B)]);
        assert_eq!(outcome.renamed, [(PathBuf::from(OLD), PathBuf::from(NEW))]);
        let after = Memory::new(&[(A, "a1"), (B, "b1"), (NEW, "c1")], 0, 0);
        assert_eq!(state(&memory), state(&after));
    }

    /// Applies the steps with operation `fail_at` failing, and checks the
    /// transfer failed with every file back as it was.
    fn assert_restored_after_failing(fail_at: usize) {
        let mut memory = Memory::new(&BEFORE, fail_at, fail_at + 1);
        let result = apply(&steps(), &mut memory);
        assert!(
            matches!(&result, Err(DocsError::TransferFailed { unrestored, .. }) if unrestored.is_empty()),
            "{result:?}"
        );
        assert_eq!(state(&memory), state(&Memory::new(&BEFORE, 0, 0)));
    }

    // Operations in order: 1 write a, 2 write c.child, 3 remove c.parent,
    // 4 write b.

    #[test]
    fn a_failed_first_write_changes_nothing() {
        assert_restored_after_failing(1);
    }

    #[test]
    fn a_failed_write_of_a_renamed_companion_restores_the_earlier_writes() {
        assert_restored_after_failing(2);
    }

    #[test]
    fn a_failed_removal_of_a_renamed_companions_old_file_restores_the_companion() {
        assert_restored_after_failing(3);
    }

    #[test]
    fn a_failed_later_write_restores_the_earlier_writes_and_the_rename() {
        assert_restored_after_failing(4);
    }

    #[test]
    fn a_file_that_cannot_be_restored_is_named() {
        // The write of b fails, and so does every restore after it.
        let mut memory = Memory::new(&BEFORE, 4, usize::MAX);
        let result = apply(&steps(), &mut memory);
        let Err(DocsError::TransferFailed { unrestored, .. }) = result else {
            panic!("{result:?}");
        };
        assert_eq!(
            unrestored,
            [PathBuf::from(OLD), PathBuf::from(NEW), PathBuf::from(A)]
        );
    }
}
