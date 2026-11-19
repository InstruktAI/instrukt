//! A company document as a company rendering reads it
//! (`project/instrukt/spec/feature/publication/agent-entrypoints`, "Company
//! home entrypoint"): its text without its `## Required reads` section, each
//! entry of that section as written, the `@` references in its text and its
//! directives. What the entries name is publication's to judge.

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
    /// The `@` references visible in the text, each without its `@`.
    pub references: Vec<String>,
    /// Its `@config`, `@exec` and `@include` directives.
    pub directives: Vec<Directive>,
}

/// Reads `text` as a company document.
pub(crate) fn company_document(text: &str) -> CompanyDocument {
    let lines = _text::lines(text);
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
            let item = ["- ", "* ", "+ "]
                .iter()
                .find_map(|bullet| item.strip_prefix(bullet))
                .map_or(item, str::trim);
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
    let body_lines = _text::lines(&body);
    CompanyDocument {
        references: _text::references(&body_lines)
            .into_iter()
            .map(|reference| reference.target)
            .collect(),
        directives: _text::directives(&body_lines),
        body,
        reads,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_company_document_splits_into_its_text_and_its_reads() {
        let document = company_document(
            "# People\n\nWho works here.\n\n## Required reads\n\n- roles.md\n- [Teams](teams/a.md)\n\n## After\n\nMore.\n",
        );
        assert_eq!(
            document.reads,
            vec!["roles.md".to_owned(), "teams/a.md".to_owned()]
        );
        assert_eq!(
            document.body,
            "# People\n\nWho works here.\n\n## After\n\nMore.\n"
        );
        assert!(document.references.is_empty());
        assert!(document.directives.is_empty());
    }
}
