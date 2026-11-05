//! Pure reading of snippet text: the frontmatter split, line context
//! (fenced code, inline code), headings, `@` references and directives.
//!
//! Nothing here knows about identity, taxonomy or the filesystem; the
//! model and validation build on these readings.

use std::ops::Range;

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

/// `text` with every inline code span blanked to spaces, so byte offsets
/// keep their meaning. A span opens with a run of backticks and closes with
/// the next run of the same length; an unclosed run is literal text.
pub(super) fn mask_inline_code(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut blank = vec![false; bytes.len()];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run = bytes[i..].iter().take_while(|b| **b == b'`').count();
        let mut j = i + run;
        let mut closed = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let len = bytes[j..].iter().take_while(|b| **b == b'`').count();
                if len == run {
                    closed = Some(j + len);
                    break;
                }
                j += len;
            } else {
                j += 1;
            }
        }
        match closed {
            Some(end) => {
                blank[i..end].iter_mut().for_each(|flag| *flag = true);
                i = end;
            }
            None => i += run,
        }
    }
    blanked(text, &blank)
}

/// `text` with the bytes flagged in `blank` replaced by spaces. Only whole
/// characters are ever flagged, so the result stays valid UTF-8.
fn blanked(text: &str, blank: &[bool]) -> String {
    text.char_indices()
        .map(|(at, ch)| {
            if blank.get(at).copied().unwrap_or(false) {
                " ".repeat(ch.len_utf8())
            } else {
                ch.to_string()
            }
        })
        .collect()
}

/// One HTML comment on a line: byte ranges of the whole comment and of the
/// text between `<!--` and `-->`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Comment {
    pub whole: Range<usize>,
    pub inner: Range<usize>,
}

/// The HTML comments of one line, in order.
pub(super) fn comments(line: &str) -> Vec<Comment> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(open) = line[from..].find("<!--").map(|i| from + i) {
        let inner_start = open + 4;
        let Some(close) = line[inner_start..].find("-->").map(|i| inner_start + i) else {
            break;
        };
        out.push(Comment {
            whole: open..close + 3,
            inner: inner_start..close,
        });
        from = close + 3;
    }
    out
}

/// The comments of `line` that are not inside inline code.
pub(super) fn visible_comments(line: &str) -> Vec<Comment> {
    let masked = mask_inline_code(line);
    comments(line)
        .into_iter()
        .filter(|c| masked.get(c.whole.start..c.whole.start + 4) == Some("<!--"))
        .collect()
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
    /// The byte range of the reference, its `@` included, in the body.
    pub span: Range<usize>,
}

/// The target of a line holding only a reference: `@x`, or a list item
/// `- @x` (`*`, `+` too), with the byte offset of its `@` in the line.
fn reference_line(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim();
    let item = ["- ", "* ", "+ ", "-\t", "*\t", "+\t"]
        .iter()
        .find_map(|bullet| trimmed.strip_prefix(bullet))
        .map_or(trimmed, str::trim_start);
    let target = item.strip_prefix('@')?;
    // `item` is a suffix of `trimmed`, which starts where `line`'s leading
    // whitespace ends.
    let at = (line.len() - line.trim_start().len()) + (trimmed.len() - item.len());
    (!target.is_empty() && !target.contains(char::is_whitespace)).then_some((at, target))
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn is_path_char(ch: char) -> bool {
    is_word(ch) || matches!(ch, '~' | '.' | '/' | '-')
}

/// The `@<path>.md` tokens of a line of prose: an `@` not preceded by a
/// word character, `@` or a backtick, followed by path characters up to the
/// last `.md` that ends a word; each with the byte offset of its `@`.
fn reference_tokens(line: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut previous: Option<char> = None;
    for (at, ch) in line.char_indices() {
        let starts = ch == '@' && previous.is_none_or(|p| !is_word(p) && p != '@' && p != '`');
        previous = Some(ch);
        if !starts {
            continue;
        }
        let rest = &line[at + 1..];
        let run_len = rest
            .char_indices()
            .find(|(_, c)| !is_path_char(*c))
            .map_or(rest.len(), |(i, _)| i);
        let run = &rest[..run_len];
        let token = run
            .match_indices(".md")
            .map(|(i, _)| i + 3)
            .filter(|end| rest[*end..].chars().next().is_none_or(|c| !is_word(c)))
            .filter(|end| *end > 3)
            .last();
        if let Some(end) = token {
            out.push((at, run[..end].to_owned()));
        }
    }
    out
}

/// The `@` references on the lines of `body`, outside fenced code, inline
/// code and HTML comments: a line holding only a reference (`@x` or
/// `- @x`), or an `@<path>.md` token in prose.
pub(super) fn references(lines: &[Line<'_>]) -> Vec<RawRef> {
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.fenced {
            continue;
        }
        let code_masked = mask_inline_code(line.text);
        let mut blank = vec![false; code_masked.len()];
        for comment in comments(&code_masked) {
            blank[comment.whole]
                .iter_mut()
                .for_each(|flag| *flag = true);
        }
        let masked = blanked(&code_masked, &blank);
        let targets = match reference_line(&masked) {
            Some((at, target)) => vec![(at, target.to_owned())],
            None => reference_tokens(&masked),
        };
        out.extend(targets.into_iter().map(|(at, target)| {
            let start = line.span.start + at;
            RawRef {
                span: start..start + 1 + target.len(),
                target,
                line: index,
            }
        }));
    }
    out
}

/// A directive in a body: an `@include` of a fragment, a `@config`
/// configured value, or a leftover `@exec`, which the format does not have:
/// it is read only so validation can report it, and is never run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    /// `<!-- @include: <path> -->` — the fragment path as written
    /// (`docs/snippets/<name>.md`).
    Include { path: String, span: Range<usize> },
    /// `<!-- @config: <path> -->` — the configuration path as written.
    Config { path: String, span: Range<usize> },
    /// `<!-- @exec: <command> -->` — not a directive of the format; inert.
    Exec { command: String, span: Range<usize> },
}

impl Directive {
    /// The directive's byte range in the body it was read from; replacing
    /// it there replaces the directive.
    pub fn span(&self) -> Range<usize> {
        match self {
            Directive::Include { span, .. }
            | Directive::Config { span, .. }
            | Directive::Exec { span, .. } => span.clone(),
        }
    }
}

/// The directives of a body, outside fenced and inline code. The argument
/// is read from the source as written, so backticks inside it stay. A
/// directive with nothing after its colon is not one.
pub(super) fn directives(lines: &[Line<'_>]) -> Vec<Directive> {
    let mut out = Vec::new();
    for line in lines.iter().filter(|line| !line.fenced) {
        for comment in visible_comments(line.text) {
            let inner = &line.text[comment.inner.clone()];
            let padded = inner.trim_start_matches([' ', '\t']);
            let Some((kind, raw)) = ["@include:", "@config:", "@exec:"]
                .into_iter()
                .find_map(|prefix| padded.strip_prefix(prefix).map(|raw| (prefix, raw)))
            else {
                continue;
            };
            let argument = raw.trim().to_owned();
            if argument.is_empty() {
                continue;
            }
            let span = line.span.start + comment.whole.start..line.span.start + comment.whole.end;
            out.push(match kind {
                "@include:" => Directive::Include {
                    path: argument,
                    span,
                },
                "@config:" => Directive::Config {
                    path: argument,
                    span,
                },
                _ => Directive::Exec {
                    command: argument,
                    span,
                },
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_splits_at_the_closing_fence() {
        let text = "---\ndescription: x\n---\n\n# T — Spec\n";
        let split = split_frontmatter(text);
        assert!(split.frontmatter.is_some());
        assert_eq!(split.body_offset, 23);
        assert_eq!(split.body_offset + split.body.len(), text.len());
    }

    #[test]
    fn an_unclosed_or_absent_frontmatter_leaves_the_whole_body() {
        for text in ["---\nx: 1\n", "# T\n"] {
            let split = split_frontmatter(text);
            assert_eq!(split.frontmatter, None);
            assert_eq!(split.body_offset, 0);
            assert_eq!(split.body.len(), text.len());
        }
    }

    #[test]
    fn fenced_lines_are_marked_and_their_headings_ignored() {
        let body = "## A\n```\n## not\n```\n## B\n";
        let lines = lines(body);
        let found: Vec<(usize, usize)> = headings(&lines)
            .into_iter()
            .map(|h| (h.level, h.line))
            .collect();
        assert_eq!(found, [(2, 0), (2, 4)]);
        assert_eq!(lines.iter().filter(|l| l.fenced).count(), 3);
    }

    #[test]
    fn titles_normalize_case_and_punctuation() {
        assert_eq!(normalize_title("See Also"), normalize_title("see also"));
        assert_eq!(
            normalize_title("Inputs/Outputs"),
            normalize_title("inputs outputs")
        );
        assert_ne!(normalize_title("See Also"), normalize_title("See"));
    }

    #[test]
    fn references_skip_code_comments_and_emails() {
        let body = "- @~/.instrukt/docs/a/policy/b.md\nsee @docs/project/spec/x.md now\n\
                    `@docs/project/spec/y.md`\nmail me@host.md\n<!-- @docs/project/z.md -->\n";
        let found: Vec<usize> = references(&lines(body))
            .into_iter()
            .map(|r| r.line)
            .collect();
        assert_eq!(found, [0, 1]);
    }

    #[test]
    fn a_reference_span_covers_its_at_sign_and_target() {
        let body = "x\n  - @~/.instrukt/docs/a/policy/b.md\nsee `c` @docs/project/spec/x.md now\n";
        for r in references(&lines(body)) {
            assert_eq!(body[r.span.clone()], format!("@{}", r.target));
        }
        assert_eq!(references(&lines(body)).len(), 2);
    }

    #[test]
    fn directive_arguments_keep_their_backticks() {
        // The path `a`b`` spans bytes 14..18; masking inline code must not
        // shorten it.
        let body = "<!-- @config: a`b` -->\n";
        let found = directives(&lines(body));
        assert!(
            matches!(&found[..], [Directive::Config { path, .. }] if path.as_str() == &body[14..18])
        );
        assert_eq!(found[0].span(), 0..22);
    }

    #[test]
    fn directives_are_read_outside_code_with_their_spans() {
        let body = "a\n<!-- @config: runtime.session_id -->\n`<!-- @config: no -->`\n\
                    <!-- @include: docs/snippets/f.md -->\n<!-- @exec: touch x -->\n";
        let found = directives(&lines(body));
        assert!(matches!(
            &found[..],
            [
                Directive::Config { .. },
                Directive::Include { .. },
                Directive::Exec { .. }
            ]
        ));
        assert_eq!(found[0].span(), 2..38);
        assert!(
            matches!(&found[1], Directive::Include { path, .. } if path.as_str() == &body[77..95])
        );
    }

    #[test]
    fn a_directive_in_inline_code_or_fences_is_not_one() {
        let body = "`<!-- @config: x -->`\n```\n<!-- @exec: y -->\n```\n";
        assert!(directives(&lines(body)).is_empty());
    }
}
