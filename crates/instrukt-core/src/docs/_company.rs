//! A company document as a company rendering reads it
//! (`project/instrukt/spec/feature/publication/agent-entrypoints`, "Company
//! home entrypoint"): its text without its `## Required reads` section, each
//! entry of that section as written, the references in its text and its
//! directives. What the entries and references name is publication's to
//! judge.

use std::ops::Range;

use super::_corpus::required_reads_section;
use super::_refs;
use super::_text::{self, Directive};

/// One company document, read.
pub(crate) struct CompanyDocument {
    /// The text without its `## Required reads` section, heading included.
    pub body: String,
    /// Each entry of the `## Required reads` section, in order: a list
    /// item's text, or a link's target.
    pub reads: Vec<String>,
    /// The references in the text without its `## Required reads`
    /// section, each once, in order.
    pub references: Vec<CompanyReference>,
    /// Its `@config`, `@exec` and `@include` directives, the `## Required
    /// reads` section's included.
    pub directives: Vec<Directive>,
}

/// A reference in a company document's text, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompanyReference {
    /// A name that is no file: an `@` reference without `.md`, or a See
    /// Also entry's opening word without `.md`. It references something
    /// only when it names a snippet or a domain.
    Name(String),
    /// A file: an `@` reference ending in `.md`, a Markdown link's target,
    /// or a See Also entry's opening path ending in `.md`.
    Path(String),
}

/// Reads `text` as a company document.
pub(crate) fn company_document(text: &str) -> CompanyDocument {
    let lines = _text::lines(text);
    let directives = _text::directives(&lines);
    let mut reads = Vec::new();
    let mut body = text.to_owned();
    if let Some(section) = required_reads_section(&lines) {
        for line in lines
            .get(section.clone())
            .unwrap_or_default()
            .iter()
            .filter(|line| !line.fenced)
        {
            let visible = _text::without_comments(line.text);
            let item = visible.trim();
            if item.is_empty() {
                continue;
            }
            let item = list_item(item).unwrap_or(item);
            let read = _refs::link(item).map_or(item, |(_, target)| target);
            reads.push(read.to_owned());
        }
        let from = lines
            .get(section.start.saturating_sub(1))
            .map_or(text.len(), |line| line.span.start);
        let to = lines
            .get(section.end)
            .map_or(text.len(), |line| line.span.start);
        body = format!("{}{}", &text[..from], &text[to..]);
    }
    CompanyDocument {
        references: references(&body),
        directives,
        body,
        reads,
    }
}

/// The references of `body`: its `@` references, its Markdown links' file
/// targets and its See Also entries, outside fenced code, inline code and
/// HTML comments.
fn references(body: &str) -> Vec<CompanyReference> {
    let lines = _text::lines(body);
    let see_also = see_also_section(&lines);
    let mut out: Vec<CompanyReference> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.fenced {
            continue;
        }
        let visible = _text::without_comments(&_text::mask_inline_code(line.text));
        let mut found = at_references(&visible);
        found.extend(
            link_targets(&visible)
                .into_iter()
                .map(CompanyReference::Path),
        );
        if see_also
            .as_ref()
            .is_some_and(|range| range.contains(&index))
            && let Some(entry) = see_also_entry(line.text)
        {
            found.push(entry);
        }
        for reference in found {
            if !out.contains(&reference) {
                out.push(reference);
            }
        }
    }
    out
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

/// The `@` references of a line: an `@` not preceded by a word character,
/// `@` or a backtick, followed by path characters; a trailing full stop
/// ends the sentence, not the reference.
fn at_references(line: &str) -> Vec<CompanyReference> {
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
        if run.is_empty() {
            continue;
        }
        let written = format!("@{run}");
        out.push(if run.ends_with(".md") {
            CompanyReference::Path(written)
        } else {
            CompanyReference::Name(written)
        });
    }
    out
}

/// The targets of a line's Markdown links (`[text](target)`, an image's
/// included) and of a link reference definition (`[label]: target`) that
/// name a file: not a URL or an address with a scheme, and not only an
/// anchor in the same document. A target loses its anchor and query.
fn link_targets(line: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find("](") {
        let after = &rest[open + 2..];
        let Some(close) = after.find(')') else {
            break;
        };
        targets.push(&after[..close]);
        rest = &after[close + 1..];
    }
    if let Some(definition) = line.trim().strip_prefix('[')
        && let Some((_, target)) = definition.split_once("]:")
    {
        targets.push(target);
    }
    targets
        .into_iter()
        .filter_map(|target| {
            let target = target.split_whitespace().next()?;
            let target = target.trim_start_matches('<').trim_end_matches('>');
            let target = target.split(['#', '?']).next().unwrap_or_default();
            (!target.is_empty() && !has_scheme(target)).then(|| target.to_owned())
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
/// reference, which the line's own reading finds.
fn see_also_entry(line: &str) -> Option<CompanyReference> {
    let visible = _text::without_comments(line);
    let item = list_item(&visible)?;
    if item.starts_with('[') || item.starts_with('@') {
        return None;
    }
    let first = item.split_whitespace().next()?.trim_matches('`');
    if first.is_empty() {
        return None;
    }
    Some(if first.ends_with(".md") {
        CompanyReference::Path(first.to_owned())
    } else {
        CompanyReference::Name(first.to_owned())
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
            "# Company\n\nSee @general/policy/commits and @general.\nRead [Outside](../secret.md#top) or <https://example.com>, [site](https://example.com).\nAsk @docs/people.md, not `@hidden` or a@b.\n\n[def]: docs/roles.md\n\n## See Also\n\n- general/policy/commits\n- `docs/teams.md` — the teams\n",
        );
        assert_eq!(
            document.references,
            vec![
                CompanyReference::Name("@general/policy/commits".to_owned()),
                CompanyReference::Name("@general".to_owned()),
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
}
