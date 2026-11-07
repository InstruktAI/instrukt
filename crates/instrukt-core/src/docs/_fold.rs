//! The delivery fold: `instrukt docs fold <slug>` applies exactly one
//! delivery's pending carriers to the working tree
//! (`project/instrukt/spec/feature/docs/pending-documentation`,
//! Delivery fold).
//!
//! The whole fold is planned in memory first; a refusal writes nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

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
/// without losing another todo's pending carriers (its `delivered_by`
/// entry, planned blocks or companion), when a file naming `slug` has a
/// malformed marker structure, and when `docs/` or a file to change lies
/// outside the repository.
pub fn fold(repo_root: &Path, slug: &TodoSlug) -> Result<FoldOutcome, DocsError> {
    let started = Instant::now();
    let root = repo_root.canonicalize().map_err(|source| DocsError::Read {
        path: repo_root.to_path_buf(),
        source,
    })?;
    let sources = super::_corpus::read_tree(&root)?;
    let corpus = super::_corpus::Corpus::from_sources(
        crate::config::project_slug(&root)?,
        &crate::home::Home::at_user_home(root.clone()),
        sources.clone(),
    );
    let inventory = crate::specs::Inventory::read(&corpus);
    let bindings = if inventory
        .use_cases()
        .iter()
        .any(|case| case.claim() == &crate::specs::Claim::Held(slug.clone()))
    {
        crate::specs::BindingSet::read(&root).map_err(|error| DocsError::FoldRefused {
            path: root.clone(),
            reason: error.to_string(),
        })?
    } else {
        crate::specs::BindingSet::default()
    };
    let originals: BTreeMap<String, String> = sources
        .iter()
        .map(|file| (file.path.clone(), file.text.clone()))
        .collect();
    let mut plan = plan(sources, slug)?;
    for snippet in corpus.snippets() {
        let file = snippet.path().to_string_lossy().to_string();
        let text = match plan.get(&file) {
            Some(Some(text)) => text,
            Some(None) => continue,
            None => match originals.get(&file) {
                Some(text) => text,
                None => continue,
            },
        };
        if let Some(text) = reserve_bindings(&file, text, slug, &inventory, &bindings)? {
            plan.insert(file, Some(text));
        }
    }
    let targets = plan
        .iter()
        .map(|(path, next)| Ok((confined(&root, path)?, path, next)))
        .collect::<Result<Vec<_>, DocsError>>()?;
    let mut outcome = FoldOutcome::default();
    for (absolute, path, next) in targets {
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
    tracing::debug!(
        event = "docs_fold_completed",
        slug = %slug,
        repo_root = %root.display(),
        written = outcome.written.len(),
        removed = outcome.removed.len(),
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    Ok(outcome)
}

/// Activation removes visibility markers; binding ownership has an independent lifetime.
fn reserve_bindings(
    file: &str,
    text: &str,
    slug: &TodoSlug,
    inventory: &crate::specs::Inventory,
    bindings: &crate::specs::BindingSet,
) -> Result<Option<String>, DocsError> {
    let split = split_frontmatter(text);
    let mut mapping = parse_mapping(split.frontmatter).ok_or_else(|| DocsError::FoldRefused {
        path: PathBuf::from(file),
        reason: "frontmatter is not a mapping".into(),
    })?;
    let original = mapping.get("binding_reservations").cloned();
    let mut reservations = match &original {
        Some(Value::Mapping(map)) => map.clone(),
        None => serde_yaml_ng::Mapping::new(),
        Some(_) => {
            return Err(DocsError::FoldRefused {
                path: PathBuf::from(file),
                reason: "binding_reservations must map use-case ids to todo slugs".into(),
            });
        }
    };
    for case in inventory.use_cases().iter().filter(|case| {
        case.file() == Path::new(file) && case.claim() == &crate::specs::Claim::Held(slug.clone())
    }) {
        let key = Value::String(case.reference().use_case.to_string());
        let bound = bindings
            .bindings()
            .iter()
            .any(|binding| binding.is_attached() && binding.use_case() == Some(case.reference()));
        if bound {
            reservations.remove(&key);
        } else {
            reservations.insert(key, Value::String(slug.to_string()));
        }
    }
    if reservations.is_empty() {
        mapping.remove("binding_reservations");
    } else {
        mapping.insert(
            Value::String("binding_reservations".into()),
            Value::Mapping(reservations),
        );
    }
    if mapping.get("binding_reservations") == original.as_ref() {
        return Ok(None);
    }
    let yaml = serde_yaml_ng::to_string(&mapping).map_err(|error| DocsError::FoldRefused {
        path: PathBuf::from(file),
        reason: error.to_string(),
    })?;
    Ok(Some(format!("---\n{yaml}---\n{}", split.body)))
}

/// The absolute path of repository-relative `rel`, refused unless its
/// directory resolves inside `root` (canonical) and it is not itself a
/// symbolic link.
pub(super) fn confined(root: &Path, rel: &str) -> Result<PathBuf, DocsError> {
    let outside = || DocsError::OutsideRepository {
        path: root.join(rel),
    };
    let absolute = root.join(rel);
    let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) else {
        return Err(outside());
    };
    let parent = parent.canonicalize().map_err(|source| DocsError::Read {
        path: parent.to_path_buf(),
        source,
    })?;
    if !parent.starts_with(root) {
        return Err(outside());
    }
    let target = parent.join(name);
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => Err(outside()),
        _ => Ok(target),
    }
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
        if companion_slug != slug.as_str() {
            continue;
        }
        let Some(parent_text) = original.get(&parent) else {
            continue;
        };
        let foreign = Foreign::of(parent_text, slug);
        let refuse = |other: &str, carrier: &str| DocsError::FoldRefused {
            path: PathBuf::from(&parent),
            reason: format!(
                "folding {slug} would lose the {carrier} of {other}; fold {other} first or carry it into {path}"
            ),
        };
        next.insert(path.clone(), None);
        if text.is_empty() {
            if let Some((other, carrier)) = foreign.first() {
                return Err(refuse(&other, carrier));
            }
            let other_companion = original.keys().find_map(|p| {
                let (of, s) = companion_shape(p)?;
                (of == parent && s != slug.as_str()).then_some(s)
            });
            if let Some(other) = other_companion {
                return Err(refuse(&other, "companion"));
            }
            next.insert(parent, None);
        } else {
            if let Some((other, carrier)) = foreign.lost_in(text) {
                return Err(refuse(&other, carrier));
            }
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

/// Other todos' pending carriers in one file.
struct Foreign {
    reservations: BTreeMap<String, String>,
    delivered_by: Vec<String>,
    /// Each foreign planned block, marker lines included, with its slug.
    blocks: Vec<(String, String)>,
    /// Slugs named by foreign markers, whether or not their blocks are
    /// well formed.
    marker_slugs: Vec<String>,
}

impl Foreign {
    fn of(text: &str, slug: &TodoSlug) -> Self {
        let split = split_frontmatter(text);
        let reservations = reservation_owners(split.frontmatter)
            .into_iter()
            .filter(|(_, owner)| owner != slug.as_str())
            .collect();
        let delivered_by = delivered_by(split.frontmatter)
            .into_iter()
            .filter(|s| s != slug.as_str())
            .collect();
        let scan = _markers::scan(split.body);
        let lines: Vec<&str> = split.body.split_inclusive('\n').collect();
        let blocks = scan
            .blocks
            .iter()
            .filter(|b| b.slug != slug.as_str())
            .map(|b| {
                let text = lines.get(b.open..=b.close).unwrap_or_default().concat();
                (b.slug.clone(), text)
            })
            .collect();
        let marker_slugs = scan
            .slugs
            .iter()
            .filter(|s| *s != slug)
            .map(ToString::to_string)
            .collect();
        Self {
            reservations,
            delivered_by,
            blocks,
            marker_slugs,
        }
    }

    /// The first foreign carrier, for a retirement that would delete it.
    fn first(&self) -> Option<(String, &'static str)> {
        self.reservations
            .values()
            .next()
            .map(|owner| (owner.clone(), "binding reservation"))
            .or_else(|| {
                self.delivered_by
                    .first()
                    .map(|s| (s.clone(), "delivered_by entry"))
                    .or_else(|| {
                        self.marker_slugs
                            .first()
                            .map(|s| (s.clone(), "planned material"))
                    })
            })
    }

    /// The first foreign carrier `replacement` does not carry over.
    fn lost_in(&self, replacement: &str) -> Option<(String, &'static str)> {
        let split = split_frontmatter(replacement);
        let kept = reservation_owners(split.frontmatter);
        if let Some((_, owner)) = self
            .reservations
            .iter()
            .find(|(id, owner)| kept.get(*id) != Some(*owner))
        {
            return Some((owner.clone(), "binding reservation"));
        }
        let kept_by = delivered_by(split.frontmatter);
        if let Some(other) = self.delivered_by.iter().find(|s| !kept_by.contains(s)) {
            return Some((other.clone(), "delivered_by entry"));
        }
        let lost_block = self
            .blocks
            .iter()
            .find(|(_, text)| !split.body.contains(text.as_str()));
        if let Some((other, _)) = lost_block {
            return Some((other.clone(), "planned material"));
        }
        let kept_slugs = _markers::scan(split.body).slugs;
        self.marker_slugs
            .iter()
            .find(|s| !kept_slugs.iter().any(|k| k.as_str() == s.as_str()))
            .map(|s| (s.clone(), "planned material"))
    }
}

fn reservation_owners(yaml: Option<&str>) -> BTreeMap<String, String> {
    parse_mapping(yaml)
        .and_then(|map| map.get("binding_reservations").cloned())
        .and_then(|value| {
            value.as_mapping().map(|map| {
                map.iter()
                    .filter_map(|(id, owner)| {
                        Some((id.as_str()?.to_owned(), owner.as_str()?.to_owned()))
                    })
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// The slugs of a frontmatter's `delivered_by` list.
pub(super) fn delivered_by(yaml: Option<&str>) -> Vec<String> {
    parse_mapping(yaml)
        .and_then(|map| {
            map.get("delivered_by")
                .and_then(Value::as_sequence)
                .cloned()
        })
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
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
    let head = clear_delivered_by(head, split.frontmatter, slug).ok_or_else(|| {
        DocsError::FoldRefused {
            path: PathBuf::from(path),
            reason: format!(
                "its `delivered_by` names {slug} in a form the fold cannot rewrite; write it as a top-level list"
            ),
        }
    })?;
    Ok(format!("{head}{body}"))
}

/// The top-level key a frontmatter line opens, unquoted (`key:`,
/// `"key":`, `'key':`).
fn line_key(line: &str) -> Option<&str> {
    if line.starts_with([' ', '\t', '-', '#']) {
        return None;
    }
    let (key, rest) = match line.chars().next()? {
        quote @ ('"' | '\'') => {
            let inner = &line[1..];
            let end = inner.find(quote)?;
            (&inner[..end], &inner[end + 1..])
        }
        _ => {
            let end = line.find(':')?;
            (&line[..end], &line[end..])
        }
    };
    rest.trim_start().starts_with(':').then_some(key.trim_end())
}

/// A mapping's entries ordered by key, for comparing two mappings whatever
/// their key order.
fn entries(map: serde_yaml_ng::Mapping) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = map
        .into_iter()
        .map(|(k, v)| (format!("{k:?}"), v))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The frontmatter block `head` (fences included) with `slug` removed from
/// `delivered_by`.
fn clear_delivered_by(head: &str, yaml: Option<&str>, slug: &TodoSlug) -> Option<String> {
    rewrite_delivered_by(head, yaml, slug, None)
}

/// The frontmatter block `head` (fences included) with `from` in
/// `delivered_by` replaced by `into`, or removed when `into` is `None`; a
/// slug already listed is not listed twice, and the key goes when no slug
/// remains. Other lines are kept as written. `None` when the rewrite cannot
/// be made, or does not read back as exactly that change.
pub(super) fn rewrite_delivered_by(
    head: &str,
    yaml: Option<&str>,
    from: &TodoSlug,
    into: Option<&TodoSlug>,
) -> Option<String> {
    let Some(yaml) = yaml else {
        return Some(head.to_owned());
    };
    let names = delivered_by(Some(yaml));
    if !names.iter().any(|s| s == from.as_str()) {
        return Some(head.to_owned());
    }
    let mut remaining: Vec<&str> = Vec::new();
    for name in &names {
        let name = if name == from.as_str() {
            match into {
                Some(into) => into.as_str(),
                None => continue,
            }
        } else {
            name.as_str()
        };
        if !remaining.contains(&name) {
            remaining.push(name);
        }
    }
    let lines: Vec<&str> = yaml.split_inclusive('\n').collect();
    let start = lines
        .iter()
        .position(|l| line_key(l) == Some("delivered_by"))?;
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
    let mut expected = parse_mapping(Some(yaml))?;
    expected.remove("delivered_by");
    if !remaining.is_empty() {
        expected.insert(
            Value::from("delivered_by"),
            Value::Sequence(remaining.iter().map(|s| Value::from(*s)).collect()),
        );
    }
    let got = parse_mapping(Some(&rebuilt))?;
    if entries(expected) != entries(got) {
        return None;
    }
    let opening = head.find('\n').map_or(0, |i| i + 1);
    let closing = &head[opening + yaml.len()..];
    Some(format!("{}{rebuilt}{closing}", &head[..opening]))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

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

    const X: &str = "docs/project/spec/x.md";
    const BODY: &str = "body\n";

    /// The `delivered_by` slugs a folded file's frontmatter still carries.
    fn remaining(text: &str) -> Vec<TodoSlug> {
        delivered_by(split_frontmatter(text).frontmatter)
            .iter()
            .map(|s| slug(s))
            .collect()
    }

    /// Whether a text's frontmatter carries `eval: true` — how these tests
    /// tell a companion's next version from its parent.
    fn is_next_version(text: &str) -> bool {
        parse_mapping(split_frontmatter(text).frontmatter)
            .and_then(|map| map.get("eval").and_then(Value::as_bool))
            .unwrap_or(false)
    }

    /// The pending slugs a text's planned blocks still name.
    fn planned(text: &str) -> BTreeSet<TodoSlug> {
        _markers::scan(split_frontmatter(text).body).slugs
    }

    fn fold_one(text: &str, todo: &str) -> Result<Option<String>, DocsError> {
        plan(sources(&[(X, text)]), &slug(todo)).map(|out| out.get(X).cloned().flatten())
    }

    #[test]
    fn delivered_by_is_cleared_and_other_fields_kept() {
        let flow =
            format!("---\ndescription: d\ndelivered_by: [a, b]\nvisibility: public\n---\n{BODY}");
        let out = fold_one(&flow, "a").unwrap().unwrap();
        assert_eq!(remaining(&out), [slug("b")]);
        assert_eq!(split_frontmatter(&out).body, split_frontmatter(&flow).body);
        let before = parse_mapping(split_frontmatter(&flow).frontmatter).unwrap();
        let after = parse_mapping(split_frontmatter(&out).frontmatter).unwrap();
        assert_eq!(after.get("visibility"), before.get("visibility"));
        assert_eq!(after.get("description"), before.get("description"));
    }

    #[test]
    fn delivered_by_is_cleared_in_every_accepted_yaml_form() {
        for head in [
            "delivered_by:\n  - a\neval: true\n",
            "delivered_by:\n- a\neval: true\n",
            "\"delivered_by\": [a]\neval: true\n",
            "'delivered_by': [a]\neval: true\n",
            "delivered_by : [a]\neval: true\n",
            "delivered_by: [\n    a,\n  ]\neval: true\n",
        ] {
            let text = format!("---\ndescription: d\n{head}---\n{BODY}");
            let out = fold_one(&text, "a").unwrap().unwrap();
            let map = parse_mapping(split_frontmatter(&out).frontmatter).unwrap();
            assert!(!map.contains_key("delivered_by"), "{head}");
            assert_eq!(map.get("eval"), Some(&Value::Bool(true)), "{head}");
        }
    }

    #[test]
    fn a_form_the_fold_cannot_rewrite_is_refused_not_left_behind() {
        let text = format!("---\n{{description: d, delivered_by: [a]}}\n---\n{BODY}");
        assert!(matches!(
            fold_one(&text, "a"),
            Err(DocsError::FoldRefused { .. })
        ));
    }

    const NEW: &str = "---\ndescription: next\neval: true\n---\nnew\n";

    #[test]
    fn companions_replace_or_retire_their_parent() {
        let out = plan(
            sources(&[
                (X, "old\n"),
                ("docs/project/spec/x.a.md", NEW),
                ("docs/project/spec/y.md", "gone\n"),
                ("docs/project/spec/y.a.md", ""),
                ("docs/project/spec/z.b.md", "other\n"),
                ("docs/project/spec/z.md", "kept\n"),
            ]),
            &slug("a"),
        )
        .unwrap();
        assert!(out[X].as_deref().is_some_and(is_next_version));
        assert_eq!(out["docs/project/spec/x.a.md"], None);
        assert_eq!(out["docs/project/spec/y.md"], None);
        assert_eq!(out["docs/project/spec/y.a.md"], None);
        assert!(!out.contains_key("docs/project/spec/z.md"));
        assert!(!out.contains_key("docs/project/spec/z.b.md"));
    }

    const FOREIGN_BLOCK: &str = "<!-- planned:b -->\nlater\n<!-- /planned:b -->\n";

    fn refused(files: &[(&str, &str)]) -> bool {
        matches!(
            plan(sources(files), &slug("a")),
            Err(DocsError::FoldRefused { .. })
        )
    }

    #[test]
    fn a_retirement_that_would_lose_foreign_carriers_is_refused() {
        let empty = ("docs/project/spec/x.a.md", "");
        assert!(refused(&[(X, FOREIGN_BLOCK), empty]));
        assert!(refused(&[
            (X, "---\ndescription: d\ndelivered_by: [b]\n---\nx\n"),
            empty
        ]));
        assert!(refused(&[
            (X, "x\n"),
            empty,
            ("docs/project/spec/x.b.md", NEW)
        ]));
        assert!(!refused(&[(X, "x\n"), empty]));
    }

    #[test]
    fn a_replacement_that_would_lose_foreign_carriers_is_refused() {
        let parent = format!("old\n{FOREIGN_BLOCK}");
        assert!(refused(&[(X, &parent), ("docs/project/spec/x.a.md", NEW)]));
        let pending = "---\ndescription: d\ndelivered_by: [a, b]\n---\nold\n";
        assert!(refused(&[(X, pending), ("docs/project/spec/x.a.md", NEW)]));
        let carried = format!("{NEW}{FOREIGN_BLOCK}");
        let out = plan(
            sources(&[(X, &parent), ("docs/project/spec/x.a.md", &carried)]),
            &slug("a"),
        )
        .unwrap();
        let folded = out[X].as_deref().unwrap_or_default();
        assert!(is_next_version(folded));
        assert_eq!(planned(folded), BTreeSet::from([slug("b")]));
    }

    #[test]
    fn malformed_markers_of_the_slug_refuse_the_fold() {
        assert!(refused(&[(X, "<!-- planned:a -->\nnever closed\n")]));
    }

    const ADDED: &str = "add\n";

    #[test]
    fn planned_blocks_of_the_slug_are_applied_and_others_kept() {
        let head = "---\ndescription: d\n---\n";
        let text = format!("{head}<!-- planned:a -->\n{ADDED}<!-- /planned:a -->\n{FOREIGN_BLOCK}");
        let out = fold_one(&text, "a").unwrap().unwrap();
        // Lines: 0-2 frontmatter, 3 open a, 4 add, 5 close a, 6-8 b's block.
        // The addition's marker lines go; its content and b's block stay.
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let kept: String = [0, 1, 2, 4, 6, 7, 8].iter().map(|i| lines[*i]).collect();
        assert_eq!(out, kept);
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_fold_writes_and_removes_files_in_the_working_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, X, "old\n");
        write(root, "docs/project/spec/x.a.md", NEW);
        write(root, "docs/project/spec/keep.md", "untouched\n");
        let outcome = fold(root, &slug("a")).unwrap();
        assert_eq!(outcome.written, [PathBuf::from(X)]);
        assert_eq!(outcome.removed, [PathBuf::from("docs/project/spec/x.a.md")]);
        assert!(is_next_version(
            &std::fs::read_to_string(root.join(X)).unwrap()
        ));
        assert!(!root.join("docs/project/spec/x.a.md").exists());
        assert!(fold(root, &slug("a")).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_fold_never_changes_files_outside_the_repository() {
        let outside = tempfile::tempdir().unwrap();
        let pending = "---\ndescription: d\ndelivered_by: [a]\n---\nx\n";
        write(outside.path(), "local/spec/x.md", pending);
        let before = std::fs::read(outside.path().join("local/spec/x.md")).unwrap();
        let repo = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.path().join("docs")).unwrap();
        assert!(matches!(
            fold(repo.path(), &slug("a")),
            Err(DocsError::OutsideRepository { .. })
        ));

        let inner = tempfile::tempdir().unwrap();
        write(inner.path(), "docs/project/spec/real.md", "y\n");
        std::os::unix::fs::symlink(outside.path(), inner.path().join("docs/project/linked"))
            .unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("local/spec/x.md"),
            inner.path().join("docs/project/spec/linked.md"),
        )
        .unwrap();
        assert!(fold(inner.path(), &slug("a")).unwrap().is_empty());
        assert_eq!(
            std::fs::read(outside.path().join("local/spec/x.md")).unwrap(),
            before
        );
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
            let once = plan(sources(&[(X, &text)]), &slug("a")).unwrap();
            let folded = once.get(X).cloned().flatten().unwrap_or(text);
            let twice = plan(sources(&[(X, &folded)]), &slug("a")).unwrap();
            prop_assert!(twice.is_empty(), "{twice:?}");
        }
    }
}
