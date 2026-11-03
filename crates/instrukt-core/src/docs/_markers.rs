//! In-body pending markers: `planned:<slug>` additions and
//! `planned-change:<slug>` … `change:<slug>` … replacements.
//!
//! The grammar is whole-line and non-nesting
//! (`general/spec/snippet-authoring-schema`, in-body marker grammar). One
//! scan yields the well-formed blocks a lens or a fold resolves, and the
//! problems validation reports; both read the same grammar, so every
//! accepted marker is one the fold can resolve.

use std::collections::BTreeSet;

use super::_text::visible_comments;
use crate::ids::TodoSlug;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Planned,
    PlannedChange,
    Change,
}

/// One live marker occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Marker {
    closing: bool,
    kind: Kind,
    slug: String,
    text: String,
    line: usize,
}

/// A well-formed pending block, by line indices of the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Block {
    pub slug: String,
    pub open: usize,
    /// The `change:` separator line of a replacement block; `None` for an
    /// addition.
    pub separator: Option<usize>,
    pub close: usize,
}

/// A marker the grammar rejects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Problem {
    /// The marker shares its line with other text.
    Inline(String),
    /// A block opens inside another open block.
    Nested(String),
    /// A closing or separator marker without its block, or a block never
    /// closed.
    Unbalanced(String),
    /// A block is closed by a marker of another slug or kind.
    Overlapping(String),
    /// The marker's slug is not a well-formed todo slug.
    InvalidSlug(String),
}

/// What one scan of a body found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Scan {
    pub blocks: Vec<Block>,
    pub problems: Vec<Problem>,
    /// Every well-formed slug named by a live marker, whether or not its
    /// block is well formed.
    pub slugs: BTreeSet<TodoSlug>,
}

impl Scan {
    /// Whether any live marker names `slug`.
    pub fn names(&self, slug: &TodoSlug) -> bool {
        self.slugs.contains(slug)
    }
}

/// A placeholder slug in sample syntax (`<slug>`, `{slug}`) is not a live
/// marker.
fn is_placeholder(slug: &str) -> bool {
    (slug.starts_with('{') && slug.ends_with('}')) || slug.starts_with('<')
}

/// Reads the text between `<!--` and `-->` as a marker:
/// `[/](planned-change|planned|change):<slug>`, padded by spaces or tabs
/// only, the slug free of whitespace and `>`.
fn parse_marker(inner: &str) -> Option<(bool, Kind, &str)> {
    let inner = inner.trim_matches([' ', '\t']);
    let (closing, rest) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, inner),
    };
    let (kind, slug) = [
        ("planned-change:", Kind::PlannedChange),
        ("planned:", Kind::Planned),
        ("change:", Kind::Change),
    ]
    .into_iter()
    .find_map(|(prefix, kind)| rest.strip_prefix(prefix).map(|slug| (kind, slug)))?;
    let valid = !slug.is_empty() && !slug.contains(|c: char| c.is_whitespace() || c == '>');
    valid.then_some((closing, kind, slug))
}

fn markers(body: &str) -> (Vec<Marker>, Vec<Problem>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for (index, raw) in body.split_inclusive('\n').enumerate() {
        let line = raw.trim_end_matches(['\n', '\r']);
        let on_line: Vec<_> = visible_comments(line)
            .into_iter()
            .filter_map(|comment| {
                let (closing, kind, slug) = parse_marker(&line[comment.inner.clone()])?;
                (!is_placeholder(slug)).then(|| Marker {
                    closing,
                    kind,
                    slug: slug.to_owned(),
                    text: line[comment.whole].to_owned(),
                    line: index,
                })
            })
            .collect();
        let own_line = on_line.len() == 1
            && on_line
                .first()
                .is_some_and(|marker| line.trim() == marker.text);
        for marker in on_line {
            if TodoSlug::parse(&marker.slug).is_err() {
                problems.push(Problem::InvalidSlug(marker.slug.clone()));
            }
            if own_line {
                found.push(marker);
            } else {
                problems.push(Problem::Inline(marker.text));
            }
        }
    }
    (found, problems)
}

/// `body` with the slug of every live marker naming `from` rewritten to
/// `to`; everything else, padding inside the markers included, is kept.
pub(super) fn rename(body: &str, from: &TodoSlug, to: &TodoSlug) -> String {
    let mut out = String::with_capacity(body.len());
    for raw in body.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\n', '\r']);
        let mut cuts: Vec<std::ops::Range<usize>> = Vec::new();
        for comment in visible_comments(line) {
            let inner = &line[comment.inner.clone()];
            let Some((_, _, slug)) = parse_marker(inner) else {
                continue;
            };
            if slug != from.as_str() {
                continue;
            }
            let end = comment.inner.start + inner.trim_end_matches([' ', '\t']).len();
            cuts.push(end - slug.len()..end);
        }
        let mut at = 0;
        for cut in cuts {
            out.push_str(&raw[at..cut.start]);
            out.push_str(to.as_str());
            at = cut.end;
        }
        out.push_str(&raw[at..]);
    }
    out
}

#[derive(Debug)]
enum Open {
    Addition(Marker),
    Current(Marker),
    Replacement(Marker, usize),
}

/// Scans `body` for pending markers.
pub(super) fn scan(body: &str) -> Scan {
    let (found, mut problems) = markers(body);
    let slugs = found
        .iter()
        .filter_map(|marker| TodoSlug::parse(&marker.slug).ok())
        .collect();
    let mut blocks = Vec::new();
    let mut open: Option<Open> = None;
    for marker in found {
        let opener = !marker.closing && marker.kind != Kind::Change;
        match (open.take(), opener) {
            (None, true) => open = Some(start(marker)),
            (None, false) => problems.push(Problem::Unbalanced(marker.text)),
            (Some(_), true) => {
                problems.push(Problem::Nested(marker.text.clone()));
                open = Some(start(marker));
            }
            (Some(current), false) => {
                open = advance(current, marker, &mut blocks, &mut problems);
            }
        }
    }
    if let Some(left) = open {
        let marker = match left {
            Open::Addition(m) | Open::Current(m) | Open::Replacement(m, _) => m,
        };
        problems.push(Problem::Unbalanced(marker.text));
    }
    Scan {
        blocks,
        problems,
        slugs,
    }
}

fn start(marker: Marker) -> Open {
    match marker.kind {
        Kind::Planned => Open::Addition(marker),
        _ => Open::Current(marker),
    }
}

fn advance(
    current: Open,
    marker: Marker,
    blocks: &mut Vec<Block>,
    problems: &mut Vec<Problem>,
) -> Option<Open> {
    let same = |open: &Marker| open.slug == marker.slug;
    match current {
        Open::Addition(open) if marker.kind == Kind::Planned && marker.closing && same(&open) => {
            blocks.push(Block {
                slug: open.slug,
                open: open.line,
                separator: None,
                close: marker.line,
            });
            None
        }
        Open::Current(open) if marker.kind == Kind::Change && !marker.closing && same(&open) => {
            let line = marker.line;
            Some(Open::Replacement(open, line))
        }
        Open::Replacement(open, separator)
            if marker.kind == Kind::PlannedChange && marker.closing && same(&open) =>
        {
            blocks.push(Block {
                slug: open.slug,
                open: open.line,
                separator: Some(separator),
                close: marker.line,
            });
            None
        }
        _ => {
            problems.push(Problem::Overlapping(marker.text));
            None
        }
    }
}

/// How one block is resolved when a body is rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    /// Left as written, markers included.
    Untouched,
    /// The current contract: an addition disappears, a replacement shows
    /// its current side.
    Current,
    /// The delivered contract: an addition's content stays, a replacement
    /// shows its replacement side; markers removed.
    Delivered,
}

/// `body` with each well-formed block resolved by `side`. Lines outside
/// blocks, malformed markers included, are kept.
pub(super) fn resolve(body: &str, scan: &Scan, side: impl Fn(&Block) -> Side) -> String {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let mut keep = vec![true; lines.len()];
    for block in &scan.blocks {
        let drop_range = |keep: &mut Vec<bool>, from: usize, to: usize| {
            for flag in keep.iter_mut().take(to + 1).skip(from) {
                *flag = false;
            }
        };
        match (side(block), block.separator) {
            (Side::Untouched, _) => {}
            (Side::Current, None) => drop_range(&mut keep, block.open, block.close),
            (Side::Delivered, None) => {
                drop_range(&mut keep, block.open, block.open);
                drop_range(&mut keep, block.close, block.close);
            }
            (Side::Current, Some(separator)) => {
                drop_range(&mut keep, block.open, block.open);
                drop_range(&mut keep, separator, block.close);
            }
            (Side::Delivered, Some(separator)) => {
                drop_range(&mut keep, block.open, separator);
                drop_range(&mut keep, block.close, block.close);
            }
        }
    }
    lines
        .iter()
        .zip(keep)
        .filter_map(|(line, kept)| kept.then_some(*line))
        .collect()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const A: &str = "a\n";
    const ADD: &str = "add\n";
    const B: &str = "b\n";
    const OLD: &str = "old\n";
    const NEW: &str = "new\n";
    const C: &str = "c\n";

    fn body() -> String {
        format!(
            "{A}<!-- planned:x -->\n{ADD}<!-- /planned:x -->\n{B}<!-- planned-change:y -->\n{OLD}\
             <!-- change:y -->\n{NEW}<!-- /planned-change:y -->\n{C}"
        )
    }

    #[test]
    fn well_formed_blocks_are_found_without_problems() {
        let scan = scan(&body());
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(scan.blocks.len(), 2);
        assert_eq!(scan.slugs.len(), 2);
    }

    /// The fixture's own lines at `indices`, joined: the expected result
    /// taken from the input by position.
    fn pick(text: &str, indices: &[usize]) -> String {
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        indices.iter().map(|i| lines[*i]).collect()
    }

    #[test]
    fn current_and_delivered_sides_resolve_each_block() {
        // Lines: 0 a, 1 open x, 2 add, 3 close x, 4 b, 5 open y, 6 old,
        // 7 change y, 8 new, 9 close y, 10 c.
        let text = body();
        let s = scan(&text);
        let resolved = |side: Side| resolve(&text, &s, |_| side);
        assert_eq!(resolved(Side::Current), pick(&text, &[0, 4, 6, 10]));
        assert_eq!(resolved(Side::Delivered), pick(&text, &[0, 2, 4, 8, 10]));
        assert_eq!(resolved(Side::Untouched), text);
    }

    #[test]
    fn a_marker_after_inline_code_on_its_line_is_inline() {
        let s = scan("`x` <!-- planned:a -->\n");
        assert!(matches!(s.problems[..], [Problem::Inline(_)]));
    }

    #[test]
    fn inline_nested_unbalanced_and_overlapping_markers_are_problems() {
        let inline = scan("text <!-- planned:x --> more\n");
        assert!(matches!(inline.problems[..], [Problem::Inline(_)]));

        let nested = scan(
            "<!-- planned:x -->\n<!-- planned:y -->\n<!-- /planned:y -->\n<!-- /planned:x -->\n",
        );
        assert!(
            nested
                .problems
                .iter()
                .any(|p| matches!(p, Problem::Nested(_)))
        );

        let unbalanced = scan("<!-- planned:x -->\nnever closed\n");
        assert!(matches!(unbalanced.problems[..], [Problem::Unbalanced(_)]));

        let overlapping = scan("<!-- planned:x -->\n<!-- /planned:y -->\n");
        assert!(matches!(
            overlapping.problems[..],
            [Problem::Overlapping(_)]
        ));
    }

    #[test]
    fn placeholders_and_inline_code_samples_are_not_markers() {
        let s = scan("`<!-- planned:x -->`\n<!-- planned:{slug} -->\n<!-- planned:<slug> -->\n");
        assert_eq!(s, Scan::default());
    }

    #[test]
    fn a_malformed_slug_is_reported_and_the_block_still_reads() {
        let s = scan("<!-- planned:Bad_Slug -->\nx\n<!-- /planned:Bad_Slug -->\n");
        assert!(matches!(
            s.problems[..],
            [Problem::InvalidSlug(_), Problem::InvalidSlug(_)]
        ));
        assert_eq!(s.blocks.len(), 1);
    }

    fn block(slug: &'static str, change: bool) -> String {
        if change {
            format!(
                "<!-- planned-change:{slug} -->\nold {slug}\n<!-- change:{slug} -->\nnew {slug}\n<!-- /planned-change:{slug} -->\n"
            )
        } else {
            format!("<!-- planned:{slug} -->\nadd {slug}\n<!-- /planned:{slug} -->\n")
        }
    }

    proptest! {
        /// Delivering one slug twice is delivering it once, and leaves every
        /// other slug's blocks as they were.
        #[test]
        fn delivering_is_idempotent_and_local(
            parts in prop::collection::vec((prop::bool::ANY, prop::bool::ANY, "[a-z ]{0,8}"), 0..8)
        ) {
            let body: String = parts
                .iter()
                .map(|(mine, change, prose)| {
                    format!("{prose}\n{}", block(if *mine { "x" } else { "y" }, *change))
                })
                .collect();
            let deliver = |text: &str| {
                let s = scan(text);
                resolve(text, &s, |b| if b.slug == "x" { Side::Delivered } else { Side::Untouched })
            };
            let once = deliver(&body);
            prop_assert_eq!(deliver(&once), once.clone());
            let others = scan(&body).blocks.iter().filter(|b| b.slug == "y").count();
            prop_assert_eq!(scan(&once).blocks.len(), others);
            prop_assert!(scan(&once).problems.is_empty());
        }
    }
}
