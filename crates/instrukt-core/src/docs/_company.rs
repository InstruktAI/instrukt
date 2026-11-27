//! A company document as a company rendering reads it
//! (`project/instrukt/spec/feature/publication/agent-entrypoints`, "Company
//! home entrypoint"): its text after any leading YAML frontmatter and without
//! its `## Required reads` section, each entry of that section as written, the
//! references in its body, its directives and the `description` and
//! `visibility` of its frontmatter. What the entries and references name, and
//! what a document without a description means, is publication's to judge.

use std::ops::Range;

use super::_corpus::required_reads_section;
use super::_frontmatter::Frontmatter;
use super::_refs;
use super::_text::{self, Directive, split_frontmatter};
use crate::ids::SnippetId;

/// One company document, read.
pub(crate) struct CompanyDocument {
    /// The text after any leading YAML frontmatter, without its
    /// `## Required reads` section, heading included.
    pub body: String,
    /// Each entry of the `## Required reads` section, in order: a list
    /// item's text, or a link's target.
    pub reads: Vec<String>,
    /// The references in the body without its `## Required reads` section,
    /// each once, in order.
    pub references: Vec<CompanyReference>,
    /// Its `@config`, `@exec` and `@include` directives in the body, the
    /// `## Required reads` section's included.
    pub directives: Vec<Directive>,
    /// What its frontmatter says.
    pub frontmatter: CompanyFrontmatter,
    /// Where each path [`company_paths`] can replace stands in the text.
    places: Places,
}

/// Where the paths of a company document stand in its whole text, by byte
/// range.
#[derive(Default)]
struct Places {
    /// Byte offset where the body starts after any leading frontmatter.
    body_offset: usize,
    /// The `## Required reads` section, heading included.
    section: Option<Range<usize>>,
    /// The path of each entry of that section, in the order of `reads`.
    reads: Vec<Range<usize>>,
    /// Each occurrence of a reference outside that section, with the range
    /// of its path: an `@` reference's without its `@`.
    references: Vec<(CompanyReference, Range<usize>)>,
}

/// A company document's frontmatter, as a company rendering reads it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CompanyFrontmatter {
    /// No frontmatter block, or one that is a YAML mapping: its
    /// `description`, read as any description is (trimmed, inner line breaks
    /// kept), `None` when it is missing or blank; and its `visibility` as
    /// written, whatever its YAML type, `None` when it holds none.
    Read {
        description: Option<String>,
        visibility: Option<serde_yaml_ng::Value>,
    },
    /// A frontmatter block that holds no YAML document, or only `null`: no
    /// mapping, and so no description and no visibility.
    Null,
    /// A frontmatter block that is not a YAML mapping, with why.
    Invalid(String),
}

impl CompanyFrontmatter {
    fn of_yaml(yaml: Option<&str>) -> Self {
        let Some(yaml) = yaml else {
            return Self::Read {
                description: None,
                visibility: None,
            };
        };
        match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(yaml) {
            Ok(serde_yaml_ng::Value::Mapping(mapping)) => {
                let visibility = mapping.get("visibility").cloned();
                Self::Read {
                    description: Frontmatter::read(mapping, None).description,
                    visibility,
                }
            }
            Ok(serde_yaml_ng::Value::Null) => Self::Null,
            Ok(_) => Self::Invalid("it is not a YAML mapping".to_owned()),
            Err(err) => Self::Invalid(format!("it is not YAML: {err}")),
        }
    }
}

/// A reference in a company document's text, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompanyReference {
    /// A name that is no file: an `@` reference in the shape of a snippet
    /// id, or a See Also entry's opening word without `.md`. It references
    /// something only when it names a snippet or a domain.
    Name(String),
    /// A file: an `@` reference ending in `.md` or holding a `/` in no
    /// snippet id's shape, a Markdown link's target, or a See Also entry's
    /// opening path ending in `.md`.
    Path(String),
}

/// A path of a company document, as written, that [`company_paths`] asks
/// to replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompanyPath<'a> {
    /// A reference in its text outside the `## Required reads` section.
    Reference(&'a CompanyReference),
    /// An entry of its `## Required reads` section.
    Read(&'a str),
}

/// Which text of a company document [`company_paths`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompanyText {
    /// The whole text, its frontmatter and `## Required reads` section
    /// included.
    Whole,
    /// The text after any leading YAML frontmatter and without its
    /// `## Required reads` section, as [`CompanyDocument::body`] holds it.
    Body,
}

/// Reads `text` as a company document: splits any leading YAML frontmatter
/// once, parses its frontmatter metadata, and reads the body after it.
pub(crate) fn company_document(text: &str) -> CompanyDocument {
    let split = split_frontmatter(text);
    let frontmatter = CompanyFrontmatter::of_yaml(split.frontmatter);
    parse_body(split.body, split.body_offset, frontmatter)
}

/// Reads `body`, a text whose frontmatter has already been split (such as a
/// compressed unit's body or a company document's delivered prose), without
/// splitting frontmatter again.
pub(crate) fn company_body(body: &str) -> CompanyDocument {
    parse_body(
        body,
        0,
        CompanyFrontmatter::Read {
            description: None,
            visibility: None,
        },
    )
}

fn parse_body(
    raw_body: &str,
    body_offset: usize,
    frontmatter: CompanyFrontmatter,
) -> CompanyDocument {
    let lines = _text::lines(raw_body);
    let directives = _text::directives(&lines);
    let mut reads = Vec::new();
    let mut places = Places {
        body_offset,
        ..Places::default()
    };
    let mut body = raw_body.to_owned();
    let mut section_in_body = None;
    if let Some(section) = required_reads_section(&lines) {
        for line in lines
            .get(section.clone())
            .unwrap_or_default()
            .iter()
            .filter(|line| !line.fenced)
        {
            let visible = blank_comments(line.text);
            let item = visible.trim();
            if item.is_empty() {
                continue;
            }
            let item = list_item(item).unwrap_or(item);
            let read = _refs::link(item).map_or(item, |(_, target)| target);
            let at = body_offset + line.span.start + offset(&visible, read);
            places.reads.push(at..at + read.len());
            reads.push(read.to_owned());
        }
        let from = lines
            .get(section.start.saturating_sub(1))
            .map_or(raw_body.len(), |line| line.span.start);
        let to = lines
            .get(section.end)
            .map_or(raw_body.len(), |line| line.span.start);
        body = format!("{}{}", &raw_body[..from], &raw_body[to..]);
        section_in_body = Some(from..to);
        places.section = Some(body_offset + from..body_offset + to);
    }
    // The body's references, placed in the whole text: no line of the body
    // spans the cut, which falls between two lines.
    places.references = located_references(&body)
        .into_iter()
        .map(|(reference, range)| {
            let shift = match &section_in_body {
                Some(cut) if range.start >= cut.start => body_offset + (cut.end - cut.start),
                _ => body_offset,
            };
            (reference, range.start + shift..range.end + shift)
        })
        .collect();
    let mut references: Vec<CompanyReference> = Vec::new();
    for (reference, _) in &places.references {
        if !references.contains(reference) {
            references.push(reference.clone());
        }
    }
    CompanyDocument {
        references,
        directives,
        body,
        reads,
        frontmatter,
        places,
    }
}

/// `text`, read as `document`, as `part` says, with each path `replace`
/// gives a replacement for replaced. Only the path's characters are
/// replaced: an `@`, a link's text, an anchor or a query after the path
/// and the rest of the line stay as written.
pub(crate) fn company_paths(
    text: &str,
    document: &CompanyDocument,
    part: CompanyText,
    mut replace: impl FnMut(CompanyPath<'_>) -> Option<String>,
) -> String {
    let places = &document.places;
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    match (part, &places.section) {
        (CompanyText::Body, Some(section)) => edits.push((section.clone(), String::new())),
        (CompanyText::Body, None) => {}
        (CompanyText::Whole, _) => {
            for (read, range) in document.reads.iter().zip(&places.reads) {
                if let Some(path) = replace(CompanyPath::Read(read)) {
                    edits.push((range.clone(), path));
                }
            }
        }
    }
    for (reference, range) in &places.references {
        if let Some(path) = replace(CompanyPath::Reference(reference)) {
            edits.push((range.clone(), path));
        }
    }
    edits.sort_by_key(|(range, _)| (range.start, range.end));
    let mut at = match part {
        CompanyText::Whole => 0,
        CompanyText::Body => places.body_offset,
    };
    let mut out = String::with_capacity(text.len().saturating_sub(at));
    for (range, replacement) in edits {
        // One path read twice, as a link's target and as an `@` reference
        // inside it, is replaced once, by the reading that starts first.
        if range.start < at {
            continue;
        }
        out.push_str(&text[at..range.start]);
        out.push_str(&replacement);
        at = range.end;
    }
    out.push_str(&text[at..]);
    out
}

/// The references of `text` read as one body, its `## Required reads`
/// section included: its `@` references, its Markdown links' file targets
/// and its See Also entries, outside code and comments, each occurrence in
/// order.
pub(crate) fn company_references(text: &str) -> Vec<CompanyReference> {
    located_references(text)
        .into_iter()
        .map(|(reference, _)| reference)
        .collect()
}

/// The references of `body` with the byte range of each one's path: its
/// `@` references, its Markdown links' file targets and its See Also
/// entries, outside fenced code, inline code and HTML comments, each
/// occurrence in order.
fn located_references(body: &str) -> Vec<(CompanyReference, Range<usize>)> {
    let lines = _text::lines(body);
    let see_also = see_also_section(&lines);
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.fenced {
            continue;
        }
        let visible = blank_comments(&_text::mask_inline_code(line.text));
        let mut found = at_references(&visible);
        found.extend(
            link_targets(&visible)
                .into_iter()
                .map(|(target, range)| (CompanyReference::Path(target), range)),
        );
        if see_also
            .as_ref()
            .is_some_and(|range| range.contains(&index))
            && let Some(entry) = see_also_entry(line.text)
        {
            found.push(entry);
        }
        let start = line.span.start;
        out.extend(
            found
                .into_iter()
                .map(|(reference, range)| (reference, start + range.start..start + range.end)),
        );
    }
    out
}

/// `line` with each HTML comment outside inline code blanked to spaces, so
/// byte offsets keep their meaning.
fn blank_comments(line: &str) -> String {
    let masked = _text::mask_inline_code(line);
    let comments = _text::comments(&masked);
    line.char_indices()
        .map(|(at, ch)| {
            if comments.iter().any(|comment| comment.whole.contains(&at)) {
                " ".repeat(ch.len_utf8())
            } else {
                ch.to_string()
            }
        })
        .collect()
}

/// The byte offset of `inner`, a slice of `outer`, in `outer`.
fn offset(outer: &str, inner: &str) -> usize {
    (inner.as_ptr() as usize).saturating_sub(outer.as_ptr() as usize)
}

/// A list item's text without its marker.
fn list_item(line: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|bullet| line.trim().strip_prefix(bullet))
        .map(str::trim)
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn is_path_char(ch: char) -> bool {
    is_word(ch) || matches!(ch, '~' | '.' | '/' | '-')
}

/// The `@` references of a line, each with the range of its path, its `@`
/// aside: an `@` not preceded by a word character, `@` or a backtick,
/// followed by path characters in a reference's shape — holding a `/` or
/// ending in `.md`; a trailing full stop ends the sentence, not the
/// reference. A bare `@word` mentions someone: it is prose.
fn at_references(line: &str) -> Vec<(CompanyReference, Range<usize>)> {
    let mut out = Vec::new();
    let mut previous: Option<char> = None;
    for (at, ch) in line.char_indices() {
        let starts = ch == '@' && previous.is_none_or(|p| !is_word(p) && p != '@' && p != '`');
        previous = Some(ch);
        if !starts {
            continue;
        }
        let rest = &line[at + 1..];
        let run = rest
            .char_indices()
            .find(|(_, c)| !is_path_char(*c))
            .map_or(rest, |(end, _)| &rest[..end])
            .trim_end_matches('.');
        if !run.ends_with(".md") && !run.contains('/') {
            continue;
        }
        let written = format!("@{run}");
        let reference = if !run.ends_with(".md") && SnippetId::parse(run).is_ok() {
            CompanyReference::Name(written)
        } else {
            CompanyReference::Path(written)
        };
        out.push((reference, at + 1..at + 1 + run.len()));
    }
    out
}

/// The targets of a line's Markdown links (`[text](target)`, an image's
/// included) and of a link reference definition (`[label]: target`) that
/// name a file: not a URL or an address with a scheme, and not only an
/// anchor in the same document. A target loses its anchor and query; its
/// range is that of what remains.
fn link_targets(line: &str) -> Vec<(String, Range<usize>)> {
    let mut raw: Vec<Range<usize>> = Vec::new();
    let mut from = 0;
    while let Some(open) = line[from..].find("](") {
        let start = from + open + 2;
        let Some(close) = line[start..].find(')') else {
            break;
        };
        raw.push(start..start + close);
        from = start + close + 1;
    }
    let trimmed = line.trim();
    if let Some(definition) = trimmed.strip_prefix('[')
        && let Some((_, target)) = definition.split_once("]:")
    {
        let start = offset(line, target);
        raw.push(start..start + target.len());
    }
    raw.into_iter()
        .filter_map(|range| {
            let target = line[range].split_whitespace().next()?;
            let target = target.trim_start_matches('<').trim_end_matches('>');
            let target = target.split(['#', '?']).next().unwrap_or_default();
            let start = offset(line, target);
            (!target.is_empty() && !has_scheme(target))
                .then(|| (target.to_owned(), start..start + target.len()))
        })
        .collect()
}

/// Whether `target` opens with a URL scheme (`https:`, `mailto:`).
fn has_scheme(target: &str) -> bool {
    target.split_once(':').is_some_and(|(scheme, _)| {
        scheme
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
    })
}

/// The lines of the level-two `See Also` section of `lines`.
fn see_also_section(lines: &[_text::Line<'_>]) -> Option<Range<usize>> {
    let headings = _text::headings(lines);
    let start = headings
        .iter()
        .position(|h| h.level == 2 && _text::normalize_title(&h.title) == "see also")?;
    let to = headings
        .iter()
        .skip(start + 1)
        .find(|h| h.level <= 2)
        .map_or(lines.len(), |h| h.line);
    Some(headings[start].line + 1..to)
}

/// The reference a See Also entry opens with, when it is no link or `@`
/// reference, which the line's own reading finds, with its range in the
/// line.
fn see_also_entry(line: &str) -> Option<(CompanyReference, Range<usize>)> {
    let visible = blank_comments(line);
    let item = list_item(&visible)?;
    if item.starts_with('[') || item.starts_with('@') {
        return None;
    }
    let first = item.split_whitespace().next()?.trim_matches('`');
    if first.is_empty() {
        return None;
    }
    let start = offset(&visible, first);
    let range = start..start + first.len();
    Some(if first.ends_with(".md") {
        (CompanyReference::Path(first.to_owned()), range)
    } else {
        (CompanyReference::Name(first.to_owned()), range)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_company_document_splits_into_its_text_and_its_reads() {
        let text = "# People\n\nWho works here.\n\n## Required reads\n\n- roles.md\n- [Teams](teams/a.md)\n\n## After\n\nMore.\n";
        let document = company_document(text);
        assert_eq!(
            document.reads,
            vec!["roles.md".to_owned(), "teams/a.md".to_owned()]
        );
        assert!(!document.body.is_empty());
        assert!(document.references.is_empty());
        assert!(document.directives.is_empty());
    }

    #[test]
    fn every_reference_form_in_a_company_documents_text_is_read() {
        let document = company_document(
            "# Company\n\nSee @general/policy/commits, not @maurice or @general.\nRead [Outside](../secret.md#top) or <https://example.com>, [site](https://example.com).\nAsk @docs/people.md, not `@hidden` or a@b.\n\n[def]: docs/roles.md\n\n## See Also\n\n- general/policy/commits\n- `docs/teams.md` — the teams\n",
        );
        assert_eq!(
            document.references,
            vec![
                CompanyReference::Name("@general/policy/commits".to_owned()),
                CompanyReference::Path("../secret.md".to_owned()),
                CompanyReference::Path("@docs/people.md".to_owned()),
                CompanyReference::Path("docs/roles.md".to_owned()),
                CompanyReference::Name("general/policy/commits".to_owned()),
                CompanyReference::Path("docs/teams.md".to_owned()),
            ]
        );
    }

    #[test]
    fn a_directive_in_the_required_reads_is_read() {
        let document =
            company_document("# Company\n\n## Required reads\n\n<!-- @config: company -->\n");
        assert!(document.reads.is_empty());
        assert!(matches!(
            document.directives.as_slice(),
            [Directive::Config { path, .. }] if path == "company"
        ));
    }

    /// The path each reference to a file in `text` occupies, then the path
    /// each required read occupies, each as the text holds it there.
    fn placed(text: &str) -> (Vec<&str>, Vec<&str>) {
        let document = company_document(text);
        let references = document
            .places
            .references
            .iter()
            .filter(|(reference, _)| matches!(reference, CompanyReference::Path(_)))
            .map(|(_, range)| &text[range.clone()])
            .collect();
        let reads = document
            .places
            .reads
            .iter()
            .map(|range| &text[range.clone()])
            .collect();
        (references, reads)
    }

    /// `text` as `part` says, with each path replaced by itself as written,
    /// `@` aside: only where a path's characters stand is anything replaced.
    fn kept(text: &str, part: CompanyText) -> String {
        let document = company_document(text);
        company_paths(text, &document, part, |path| {
            let written = match path {
                CompanyPath::Reference(CompanyReference::Path(file)) => file,
                CompanyPath::Reference(CompanyReference::Name(_)) => return None,
                CompanyPath::Read(read) => read,
            };
            Some(written.strip_prefix('@').unwrap_or(written).to_owned())
        })
    }

    #[test]
    fn an_at_references_path_is_placed_without_its_at() {
        let text = "# A\n\nAsk @people.md. Not `@x.md`.\n";
        assert_eq!(placed(text), (vec!["people.md"], vec![]));
        assert_eq!(kept(text, CompanyText::Whole), text);
    }

    #[test]
    fn a_links_path_is_placed_without_its_text_anchor_query_or_brackets() {
        let text = "# A\n\nSee [the roles](roles.md#lead) and <!-- [x](hidden.md) --> ![map](<maps/a.md?v=1>).\n";
        assert_eq!(placed(text), (vec!["roles.md", "maps/a.md"], vec![]));
        assert_eq!(kept(text, CompanyText::Whole), text);
    }

    #[test]
    fn a_link_definitions_path_is_placed_without_its_title() {
        let text = "# A\n\n[roles]: roles.md \"Roles\"\n";
        assert_eq!(placed(text), (vec!["roles.md"], vec![]));
        assert_eq!(kept(text, CompanyText::Whole), text);
    }

    #[test]
    fn a_see_also_entrys_opening_path_is_placed_and_a_name_is_not() {
        let text = "# A\n\n## See Also\n\n- `teams.md` — the teams\n- general/policy/commits\n";
        assert_eq!(placed(text), (vec!["teams.md"], vec![]));
        assert_eq!(kept(text, CompanyText::Whole), text);
    }

    #[test]
    fn a_required_reads_path_is_placed_and_the_body_leaves_its_section_out() {
        let text = "# A\n\n## Required reads\n\n- roles.md\n- [Teams](teams/a.md) <!-- why -->\n\n## Rules\n\nSee @roles.md.\n";
        assert_eq!(
            placed(text),
            (vec!["roles.md"], vec!["roles.md", "teams/a.md"])
        );
        assert_eq!(kept(text, CompanyText::Whole), text);
        let document = company_document(text);
        assert_eq!(kept(text, CompanyText::Body), document.body);
        let body = company_document(&document.body);
        assert!(body.reads.is_empty());
        assert_eq!(body.references, document.references);
    }

    #[test]
    fn a_documents_visibility_is_read_as_written_whatever_its_type() {
        let visibility = |text: &str| match company_document(text).frontmatter {
            CompanyFrontmatter::Read { visibility, .. } => visibility,
            other => panic!("unread frontmatter: {other:?}"),
        };
        assert_eq!(
            visibility("---\nvisibility: public\n---\n\n# A\n"),
            Some(serde_yaml_ng::Value::String("public".to_owned()))
        );
        assert_eq!(
            visibility("---\nvisibility: [a, b]\n---\n\n# A\n"),
            Some(serde_yaml_ng::from_str("[a, b]").unwrap())
        );
        assert_eq!(visibility("---\ndescription: A.\n---\n\n# A\n"), None);
        assert_eq!(visibility("# A\n"), None);
    }

    fn description(text: &str) -> Option<String> {
        match company_document(text).frontmatter {
            CompanyFrontmatter::Read { description, .. } => description,
            CompanyFrontmatter::Null => None,
            CompanyFrontmatter::Invalid(reason) => panic!("invalid frontmatter: {reason}"),
        }
    }

    #[test]
    fn a_company_documents_description_is_read_from_its_frontmatter() {
        let described = description("---\ndescription: Who works here.\n---\n\n# People\n");
        assert_eq!(described.as_deref(), Some("Who works here."));
        assert_eq!(
            description("---\nvisibility: internal\n---\n\n# People\n"),
            None
        );
        assert_eq!(description("# People\n"), None);
        assert_eq!(
            description("---\ndescription: '  '\n---\n\n# People\n"),
            None
        );
        let across = description(
            "---\ndescription: |\n  Who works here,\n  and since when.\n---\n\n# People\n",
        );
        assert_eq!(across.as_deref(), Some("Who works here,\nand since when."));
    }

    #[test]
    fn frontmatter_is_omitted_from_body_and_unscanned_while_kept_in_whole_and_metadata() {
        let frontmatter = "---\ndescription: >-\n  Read @general/policy/commits and [old](../secret.md) <!-- @config: company -->.\nvisibility: internal\n---\n";
        let body_part = "\n# Company\n\nSee [Roles](roles.md#lead).\n\n## Required reads\n\n- people.md\n\n## See Also\n\n- teams/a.md — teams\n";
        let text = format!("{frontmatter}{body_part}");
        let with_fm = company_document(&text);
        let without_fm = company_document(body_part);
        assert_eq!(with_fm.body, without_fm.body);
        assert_eq!(with_fm.reads, without_fm.reads);
        assert_eq!(with_fm.references, without_fm.references);
        assert_eq!(with_fm.directives, without_fm.directives);
        assert_eq!(
            with_fm.frontmatter,
            CompanyFrontmatter::Read {
                description: Some(
                    "Read @general/policy/commits and [old](../secret.md) <!-- @config: company -->."
                        .to_owned()
                ),
                visibility: Some(serde_yaml_ng::Value::String("internal".to_owned())),
            }
        );
        assert_eq!(placed(&text), placed(body_part));
        assert_eq!(kept(&text, CompanyText::Whole), text);
        assert_eq!(kept(&text, CompanyText::Body), without_fm.body);
        let rewrite = |src: &str, part: CompanyText| {
            let doc = company_document(src);
            company_paths(src, &doc, part, |path| {
                let written = match path {
                    CompanyPath::Reference(CompanyReference::Path(file)) => file,
                    CompanyPath::Reference(CompanyReference::Name(_)) => return None,
                    CompanyPath::Read(read) => read,
                };
                Some(format!(
                    "references/{}",
                    written.strip_prefix('@').unwrap_or(written)
                ))
            })
        };
        assert_eq!(
            rewrite(&text, CompanyText::Body),
            rewrite(body_part, CompanyText::Body)
        );
        assert_eq!(
            rewrite(&text, CompanyText::Whole),
            format!("{frontmatter}{}", rewrite(body_part, CompanyText::Whole))
        );
    }

    #[test]
    fn a_pre_split_company_body_does_not_strip_a_second_leading_fence_block() {
        let body = "---\n## Required reads\n\n- extra.md\n\n## Rules\n---\n- rule\n";
        let document = company_body(body);
        assert_eq!(document.reads, vec!["extra.md".to_owned()]);
        assert_eq!(document.body, "---\n## Rules\n---\n- rule\n");
    }

    #[test]
    fn frontmatter_that_is_no_yaml_mapping_is_invalid_not_a_missing_description() {
        for text in [
            "---\ndescription: Who works here: and why\n---\n\n# People\n",
            "---\n- a list\n---\n\n# People\n",
        ] {
            assert!(
                matches!(
                    company_document(text).frontmatter,
                    CompanyFrontmatter::Invalid(_)
                ),
                "{text:?}"
            );
        }
    }
}
