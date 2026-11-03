//! The delivery fold: `instrukt docs fold <slug>` applies exactly one
//! delivery's pending carriers to the working tree
//! (`local/projects/instrukt/spec/feature/docs/pending-documentation`,
//! Delivery fold).
//!
//! The whole fold is planned in memory first; a refusal writes nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_yaml_ng::Value;

use super::_corpus::SourceFile;
use super::_frontmatter::parse_mapping;
use super::_identity::companion_shape;
use super::_markers::{self, Side};
use super::_text::split_frontmatter;
use super::DocsError;
use crate::fsx::atomic_write;
use crate::ids::TodoSlug;

/// What a fold changed, repository-relative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldOutcome {
    /// Files rewritten.
    pub written: Vec<PathBuf>,
    /// Files removed.
    pub removed: Vec<PathBuf>,
}

impl FoldOutcome {
    pub fn is_empty(&self) -> bool {
        self.written.is_empty() && self.removed.is_empty()
    }
}

/// Folds `slug`'s pending carriers into the documentation of the working
/// tree rooted at `repo_root`: clears it from `delivered_by`, replaces each
/// live parent with its non-empty companion, removes each empty companion
/// with its parent, unwraps its planned additions and applies its planned
/// replacements. Every other todo's material is left as it is.
///
/// Refuses, changing nothing, when a carrier of `slug` cannot be folded
/// without losing other material: a malformed marker structure in a file
/// naming `slug`, or a retirement of a parent that carries another todo's
/// pending material.
pub fn fold(repo_root: &Path, slug: &TodoSlug) -> Result<FoldOutcome, DocsError> {
    let sources = super::_corpus::read_tree(repo_root)?;
    let plan = plan(sources, slug)?;
    let mut outcome = FoldOutcome::default();
    for (path, next) in plan {
        let absolute = repo_root.join(&path);
        match next {
            Some(text) => {
                atomic_write(&absolute, text.as_bytes())?;
                outcome.written.push(PathBuf::from(path));
            }
            None => {
                std::fs::remove_file(&absolute).map_err(|source| DocsError::Remove {
                    path: absolute.clone(),
                    source,
                })?;
                outcome.removed.push(PathBuf::from(path));
            }
        }
    }
    Ok(outcome)
}

/// The changes folding `slug` makes: path to new text, or `None` to remove.
/// Only changed paths are listed.
pub(super) fn plan(
    sources: Vec<SourceFile>,
    slug: &TodoSlug,
) -> Result<BTreeMap<String, Option<String>>, DocsError> {
    let original: BTreeMap<String, String> =
        sources.into_iter().map(|s| (s.path, s.text)).collect();
    let mut next: BTreeMap<String, Option<String>> = original
        .iter()
        .map(|(path, text)| (path.clone(), Some(text.clone())))
        .collect();
    for (path, text) in &original {
        let Some((parent, companion_slug)) = companion_shape(path) else {
            continue;
        };
        if companion_slug != slug.as_str() || !original.contains_key(&parent) {
            continue;
        }
        next.insert(path.clone(), None);
        if text.is_empty() {
            retire(&original, &parent, slug)?;
            next.insert(parent, None);
        } else {
            next.insert(parent, Some(text.clone()));
        }
    }
    for (path, text) in next.iter_mut() {
        let Some(current) = text else {
            continue;
        };
        *current = fold_text(path, current, slug)?;
    }
    Ok(next
        .into_iter()
        .filter(|(path, text)| original.get(path) != text.as_ref())
        .collect())
}

/// Refuses a retirement that would delete another todo's pending material.
fn retire(
    original: &BTreeMap<String, String>,
    parent: &str,
    slug: &TodoSlug,
) -> Result<(), DocsError> {
    let refuse = |reason: String| DocsError::FoldRefused {
        path: PathBuf::from(parent),
        reason,
    };
    let text = original.get(parent).map(String::as_str).unwrap_or_default();
    let scan = _markers::scan(split_frontmatter(text).body);
    if let Some(other) = scan.slugs.iter().find(|s| *s != slug) {
        return Err(refuse(format!(
            "retired by {slug} but carries pending material of {other}; fold {other} first"
        )));
    }
    let other_companion = original.keys().find_map(|path| {
        let (p, s) = companion_shape(path)?;
        (p == parent && s != slug.as_str()).then_some(s)
    });
    if let Some(other) = other_companion {
        return Err(refuse(format!(
            "retired by {slug} but has a companion of {other}; fold {other} first"
        )));
    }
    Ok(())
}

/// One file's text with `slug`'s planned blocks resolved and `slug`
/// cleared from `delivered_by`.
fn fold_text(path: &str, text: &str, slug: &TodoSlug) -> Result<String, DocsError> {
    let split = split_frontmatter(text);
    let scan = _markers::scan(split.body);
    if scan.names(slug) && !scan.problems.is_empty() {
        return Err(DocsError::FoldRefused {
            path: PathBuf::from(path),
            reason: format!(
                "its pending markers are malformed; run `instrukt docs validate {path}`"
            ),
        });
    }
    let body = _markers::resolve(split.body, &scan, |block| {
        if block.slug == slug.as_str() {
            Side::Delivered
        } else {
            Side::Untouched
        }
    });
    let head = &text[..split.body_offset];
    Ok(format!(
        "{}{body}",
        clear_delivered_by(head, split.frontmatter, slug)
    ))
}

/// The frontmatter block `head` (fences included) with `slug` removed from
/// `delivered_by`; the key goes when no slug remains. Other lines are kept
/// as written.
fn clear_delivered_by(head: &str, yaml: Option<&str>, slug: &TodoSlug) -> String {
    let Some(yaml) = yaml else {
        return head.to_owned();
    };
    let Some(slugs) = parse_mapping(Some(yaml)).and_then(|map| {
        map.get("delivered_by")
            .and_then(Value::as_sequence)
            .cloned()
    }) else {
        return head.to_owned();
    };
    let names: Vec<&str> = slugs.iter().filter_map(Value::as_str).collect();
    if !names.contains(&slug.as_str()) {
        return head.to_owned();
    }
    let remaining: Vec<&str> = names.into_iter().filter(|s| *s != slug.as_str()).collect();
    let lines: Vec<&str> = yaml.split_inclusive('\n').collect();
    let Some(start) = lines.iter().position(|l| l.starts_with("delivered_by:")) else {
        return head.to_owned();
    };
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| {
            let t = l.trim_end_matches(['\n', '\r']);
            !t.is_empty() && !t.starts_with([' ', '\t', '-'])
        })
        .map_or(lines.len(), |(i, _)| i);
    let mut rebuilt: String = lines[..start].concat();
    if !remaining.is_empty() {
        rebuilt.push_str(&format!("delivered_by: [{}]\n", remaining.join(", ")));
    }
    rebuilt.push_str(&lines[end..].concat());
    let opening = head.find('\n').map_or(0, |i| i + 1);
    let closing = &head[opening + yaml.len()..];
    format!("{}{rebuilt}{closing}", &head[..opening])
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

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

    #[test]
    fn delivered_by_is_cleared_and_other_slugs_kept() {
        let text = "---\ndescription: d\ndelivered_by: [a, b]\nvisibility: public\n---\nbody\n";
        let out = plan(sources(&[("docs/local/spec/x.md", text)]), &slug("a")).unwrap();
        assert_eq!(
            out["docs/local/spec/x.md"].as_deref(),
            Some("---\ndescription: d\ndelivered_by: [b]\nvisibility: public\n---\nbody\n")
        );
        let block = "---\ndescription: d\ndelivered_by:\n  - a\neval: true\n---\nbody\n";
        let out = plan(sources(&[("docs/local/spec/x.md", block)]), &slug("a")).unwrap();
        assert_eq!(
            out["docs/local/spec/x.md"].as_deref(),
            Some("---\ndescription: d\neval: true\n---\nbody\n")
        );
    }

    #[test]
    fn companions_replace_or_retire_their_parent() {
        let out = plan(
            sources(&[
                ("docs/local/spec/x.md", "old\n"),
                ("docs/local/spec/x.a.md", "new\n"),
                ("docs/local/spec/y.md", "gone\n"),
                ("docs/local/spec/y.a.md", ""),
                ("docs/local/spec/z.b.md", "other\n"),
                ("docs/local/spec/z.md", "kept\n"),
            ]),
            &slug("a"),
        )
        .unwrap();
        assert_eq!(out["docs/local/spec/x.md"].as_deref(), Some("new\n"));
        assert_eq!(out["docs/local/spec/x.a.md"], None);
        assert_eq!(out["docs/local/spec/y.md"], None);
        assert_eq!(out["docs/local/spec/y.a.md"], None);
        assert!(!out.contains_key("docs/local/spec/z.md"));
        assert!(!out.contains_key("docs/local/spec/z.b.md"));
    }

    #[test]
    fn a_retirement_over_foreign_material_is_refused() {
        let refused = plan(
            sources(&[
                (
                    "docs/local/spec/y.md",
                    "<!-- planned:b -->\nx\n<!-- /planned:b -->\n",
                ),
                ("docs/local/spec/y.a.md", ""),
            ]),
            &slug("a"),
        );
        assert!(matches!(refused, Err(DocsError::FoldRefused { .. })));
    }

    #[test]
    fn malformed_markers_of_the_slug_refuse_the_fold() {
        let refused = plan(
            sources(&[("docs/local/spec/y.md", "<!-- planned:a -->\nnever closed\n")]),
            &slug("a"),
        );
        assert!(matches!(refused, Err(DocsError::FoldRefused { .. })));
    }

    #[test]
    fn planned_blocks_of_the_slug_are_applied_and_others_kept() {
        let text = "---\ndescription: d\n---\n<!-- planned:a -->\nadd\n<!-- /planned:a -->\n\
                    <!-- planned:b -->\nlater\n<!-- /planned:b -->\n";
        let out = plan(sources(&[("docs/local/spec/x.md", text)]), &slug("a")).unwrap();
        assert_eq!(
            out["docs/local/spec/x.md"].as_deref(),
            Some("---\ndescription: d\n---\nadd\n<!-- planned:b -->\nlater\n<!-- /planned:b -->\n")
        );
    }

    #[test]
    fn a_fold_writes_and_removes_files_in_the_working_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("docs/local/spec/x.md", "old\n");
        write("docs/local/spec/x.a.md", "new\n");
        write("docs/local/spec/keep.md", "untouched\n");
        let outcome = fold(root, &slug("a")).unwrap();
        assert_eq!(outcome.written, [PathBuf::from("docs/local/spec/x.md")]);
        assert_eq!(outcome.removed, [PathBuf::from("docs/local/spec/x.a.md")]);
        assert_eq!(
            std::fs::read_to_string(root.join("docs/local/spec/x.md")).unwrap(),
            "new\n"
        );
        assert!(!root.join("docs/local/spec/x.a.md").exists());
        assert!(fold(root, &slug("a")).unwrap().is_empty());
    }

    proptest! {
        /// Folding a slug twice changes nothing the first fold did not.
        #[test]
        fn folding_is_idempotent(
            mine in prop::collection::vec(prop::bool::ANY, 0..6),
            pending in prop::bool::ANY,
        ) {
            let head = if pending { "---\ndescription: d\ndelivered_by: [a, b]\n---\n" } else { "---\ndescription: d\n---\n" };
            let body: String = mine
                .iter()
                .map(|m| {
                    let s = if *m { "a" } else { "b" };
                    format!("p\n<!-- planned-change:{s} -->\nold\n<!-- change:{s} -->\nnew\n<!-- /planned-change:{s} -->\n")
                })
                .collect();
            let text = format!("{head}{body}");
            let once = plan(sources(&[("docs/local/spec/x.md", &text)]), &slug("a")).unwrap();
            let folded = once.get("docs/local/spec/x.md").cloned().flatten().unwrap_or(text);
            let twice = plan(sources(&[("docs/local/spec/x.md", &folded)]), &slug("a")).unwrap();
            prop_assert!(twice.is_empty(), "{twice:?}");
        }
    }
}
