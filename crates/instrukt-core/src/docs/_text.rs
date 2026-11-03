//! Pure reading of snippet text: the frontmatter split, line context
//! (fenced code, inline code), headings, `@` references and directives.
//!
//! Nothing here knows about identity, taxonomy or the filesystem; the
//! model and validation build on these readings.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

/// A source split into its frontmatter and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Split<'a> {
    /// The YAML between the fences; `None` when the source has no closed
    /// frontmatter block.
    pub frontmatter: Option<&'a str>,
    /// Everything after the closing fence (the whole source without
    /// frontmatter).
    pub body: &'a str,
    /// The byte offset of `body` in the source.
    pub body_offset: usize,
}

/// Splits `text` at a leading `---` frontmatter block. An unclosed block is
/// no frontmatter: the whole text is the body.
pub(super) fn split_frontmatter(text: &str) -> Split<'_> {
    let whole = Split {
        frontmatter: None,
        body: text,
        body_offset: 0,
    };
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return whole;
    };
    if first.trim_end() != "---" {
        return whole;
    }
    let mut offset = first.len();
    for line in lines {
        if line.trim_end() == "---" {
            let yaml = &text[first.len()..offset];
            let body_offset = offset + line.len();
            return Split {
                frontmatter: Some(yaml),
                body: &text[body_offset..],
                body_offset,
            };
        }
        offset += line.len();
    }
    whole
}

/// One line of a body with the context that decides how it is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Line<'a> {
    /// The line without its line ending.
    pub text: &'a str,
    /// The line's byte range in the body, line ending included.
    pub span: Range<usize>,
    /// Inside a fenced code block, or a fence line itself.
    pub fenced: bool,
}

/// The lines of `body`, each marked when it belongs to fenced code.
pub(super) fn lines(body: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut fence: Option<(char, usize)> = None;
    for raw in body.split_inclusive('\n') {
        let text = raw.trim_end_matches(['\n', '\r']);
        let trimmed = text.trim_start();
        let opener = fence_opener(trimmed);
        let fenced = match (fence, opener) {
            (None, Some(open)) => {
                fence = Some(open);
                true
            }
            (Some((ch, len)), Some((close_ch, close_len)))
                if ch == close_ch && close_len >= len && trimmed.trim_end().len() == close_len =>
            {
                fence = None;
                true
            }
            (Some(_), _) => true,
            (None, None) => false,
        };
        out.push(Line {
            text,
            span: offset..offset + raw.len(),
            fenced,
        });
        offset += raw.len();
    }
    out
}

fn fence_opener(trimmed: &str) -> Option<(char, usize)> {
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|c| *c == ch).count();
    (len >= 3).then_some((ch, len))
}

static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| compile(r"`+[^`]*`+"));
static HTML_COMMENT: LazyLock<Regex> = LazyLock::new(|| compile(r"<!--.*?-->"));

/// `text` with every inline code span blanked to spaces, so byte offsets
/// keep their meaning.
pub(super) fn mask_inline_code(text: &str) -> String {
    blank(text, &INLINE_CODE)
}

fn blank(text: &str, pattern: &Regex) -> String {
    pattern
        .replace_all(text, |caps: &regex::Captures<'_>| {
            " ".repeat(caps.get(0).map_or(0, |m| m.as_str().len()))
        })
        .into_owned()
}

/// A Markdown ATX heading outside fenced code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Heading {
    pub level: usize,
    pub title: String,
    /// Index into the body's lines.
    pub line: usize,
}

/// The headings of a body, in order, ignoring fenced code.
pub(super) fn headings(lines: &[Line<'_>]) -> Vec<Heading> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.fenced)
        .filter_map(|(index, line)| {
            let level = line.text.chars().take_while(|c| *c == '#').count();
            let rest = &line.text[level..];
            let valid = (1..=6).contains(&level)
                && (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'));
            valid.then(|| Heading {
                level,
                title: rest.trim().trim_end_matches('#').trim().to_owned(),
                line: index,
            })
        })
        .collect()
}

/// A heading title reduced to lowercase words, so `See Also` and
/// `See also` name the same section.
pub(super) fn normalize_title(title: &str) -> String {
    title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// One `@` reference written in a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RawRef {
    /// The reference without its `@`.
    pub target: String,
    /// Index into the body's lines.
    pub line: usize,
}

static REF_LINE: LazyLock<Regex> = LazyLock::new(|| compile(r"^\s*(?:[-*+]\s+)?@(\S+)\s*$"));
static REF_TOKEN: LazyLock<Regex> = LazyLock::new(|| compile(r"(?:^|[^\w@`])@([\w~./\-]+\.md)\b"));

/// The `@` references on the lines of `body`, outside fenced code, inline
/// code and HTML comments: a line holding only a reference (`@x` or
/// `- @x`), or an `@<path>.md` token in prose.
pub(super) fn references(lines: &[Line<'_>]) -> Vec<RawRef> {
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.fenced {
            continue;
        }
        let masked = blank(&mask_inline_code(line.text), &HTML_COMMENT);
        if let Some(caps) = REF_LINE.captures(&masked) {
            if let Some(target) = caps.get(1) {
                out.push(RawRef {
                    target: target.as_str().to_owned(),
                    line: index,
                });
            }
            continue;
        }
        for caps in REF_TOKEN.captures_iter(&masked) {
            if let Some(target) = caps.get(1) {
                out.push(RawRef {
                    target: target.as_str().to_owned(),
                    line: index,
                });
            }
        }
    }
    out
}

/// A directive in a body: an `@include` of a fragment or an `@exec`
/// read-time command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    /// `<!-- @include: <path> -->` — the fragment path as written
    /// (`docs/snippets/<name>.md`).
    Include { path: String, span: Range<usize> },
    /// `<!-- @exec: <command> -->` — the command as written.
    Exec { command: String, span: Range<usize> },
}

impl Directive {
    /// The directive's byte range in the body it was read from; replacing
    /// it there replaces the directive.
    pub fn span(&self) -> Range<usize> {
        match self {
            Directive::Include { span, .. } | Directive::Exec { span, .. } => span.clone(),
        }
    }
}

static DIRECTIVE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"<!--[ \t]*@(include|exec):[ \t]*(.*?)[ \t]*-->"));

/// The directives of a body, outside fenced and inline code. A directive
/// with nothing after its colon is not one.
pub(super) fn directives(lines: &[Line<'_>]) -> Vec<Directive> {
    let mut out = Vec::new();
    for line in lines.iter().filter(|line| !line.fenced) {
        let masked = mask_inline_code(line.text);
        for caps in DIRECTIVE.captures_iter(&masked) {
            let (Some(whole), Some(kind), Some(arg)) = (caps.get(0), caps.get(1), caps.get(2))
            else {
                continue;
            };
            let argument = arg.as_str().trim();
            if argument.is_empty() {
                continue;
            }
            let span = line.span.start + whole.start()..line.span.start + whole.end();
            out.push(match kind.as_str() {
                "include" => Directive::Include {
                    path: argument.to_owned(),
                    span,
                },
                _ => Directive::Exec {
                    command: argument.to_owned(),
                    span,
                },
            });
        }
    }
    out
}

/// Compiles a pattern literal of this crate.
#[allow(
    clippy::expect_used,
    reason = "only built-in literal patterns reach here, and every one is compiled by the unit tests"
)]
pub(super) fn compile(pattern: &str) -> Regex {
    Regex::new(pattern).expect("built-in pattern compiles")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_splits_at_the_closing_fence() {
        let text = "---\ndescription: x\n---\n\n# T — Spec\n";
        let split = split_frontmatter(text);
        assert_eq!(split.frontmatter, Some("description: x\n"));
        assert_eq!(split.body, "\n# T — Spec\n");
        assert_eq!(&text[split.body_offset..], split.body);
    }

    #[test]
    fn an_unclosed_or_absent_frontmatter_leaves_the_whole_body() {
        assert_eq!(split_frontmatter("---\nx: 1\n").frontmatter, None);
        assert_eq!(split_frontmatter("# T\n").body, "# T\n");
    }

    #[test]
    fn fenced_lines_are_marked_and_their_headings_ignored() {
        let body = "## A\n```\n## not\n```\n## B\n";
        let lines = lines(body);
        let titles: Vec<_> = headings(&lines).into_iter().map(|h| h.title).collect();
        assert_eq!(titles, ["A", "B"]);
    }

    #[test]
    fn titles_normalize_case_and_punctuation() {
        assert_eq!(normalize_title("See Also"), "see also");
        assert_eq!(normalize_title("Inputs/Outputs"), "inputs outputs");
    }

    #[test]
    fn references_skip_code_comments_and_emails() {
        let body = "- @~/.instrukt/docs/a/policy/b.md\nsee @docs/local/spec/x.md now\n\
                    `@docs/local/spec/y.md`\nmail me@host.md\n<!-- @docs/local/z.md -->\n";
        let refs: Vec<_> = references(&lines(body))
            .into_iter()
            .map(|r| r.target)
            .collect();
        assert_eq!(
            refs,
            ["~/.instrukt/docs/a/policy/b.md", "docs/local/spec/x.md"]
        );
    }

    #[test]
    fn directives_are_read_outside_code_with_their_spans() {
        let body = "a\n<!-- @exec: instrukt config get x -->\n`<!-- @exec: no -->`\n\
                    <!-- @include: docs/snippets/f.md -->\n";
        let found = directives(&lines(body));
        assert_eq!(found.len(), 2);
        assert_eq!(
            &body[found[0].span()],
            "<!-- @exec: instrukt config get x -->"
        );
        assert!(
            matches!(&found[1], Directive::Include { path, .. } if path == "docs/snippets/f.md")
        );
    }
}
