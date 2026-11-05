//! How a position's selection reads once rendered: the section shape every
//! rendering of a position index shares — `instrukt docs get --domain <x>`,
//! the global agent files and a project's root `AGENTS.md`
//! (`project/instrukt/spec/feature/publication/agent-entrypoints`,
//! "Rendering a selection"; listing lines from
//! `project/instrukt/spec/feature/docs/progressive-retrieval`).
//!
//! These are text functions only. Which snippets a selection holds, in
//! which order, and how each body passes the read pipeline is the caller's:
//! it hands over the snippets in full in traversal order, the listed ones
//! and the position's triggered procedures.

use std::collections::BTreeSet;

use crate::ids::SnippetId;

/// One snippet a rendering lists rather than expands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub id: SnippetId,
    pub description: String,
    /// A procedure's `activation_trigger`: it is listed in the triggered
    /// form, read with its closure.
    pub trigger: Option<String>,
}

impl Listing {
    /// The listing line: `` - <id>: <description> — `instrukt docs get <id>` ``,
    /// or for a triggered procedure
    /// `` - <id>: <description> — when: <trigger> — `instrukt docs get <id> --closure` ``.
    pub fn line(&self) -> String {
        match self.trigger.as_deref().map(str::trim) {
            Some(trigger) if !trigger.is_empty() => format!(
                "- {}: {} — when: {trigger} — `{}`",
                self.id,
                self.description.trim(),
                closure_command(&self.id)
            ),
            _ => format!(
                "- {}: {} — `instrukt docs get {}`",
                self.id,
                self.description.trim(),
                self.id
            ),
        }
    }
}

/// The command that reads `id` with its closure.
pub fn closure_command(id: &SnippetId) -> String {
    format!("instrukt docs get {id} --closure")
}

/// One snippet a rendering holds in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFull {
    pub id: SnippetId,
    /// The name of the position it belongs to (`general`, a domain,
    /// `project`), whose heading it sits under.
    pub position: String,
    /// Its body after the read pipeline: frontmatter and required reads
    /// stripped, its own H1 first.
    pub body: String,
    /// A procedure's `activation_trigger`, shown directly under its
    /// heading.
    pub trigger: Option<String>,
}

/// What one position's selection renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The position's name: `general`, the domain, `project`.
    pub position: String,
    /// The snippets in full, in traversal order: each required read before
    /// the snippet requiring it, each once.
    pub in_full: Vec<InFull>,
    /// The `listed` snippets, in the order the index lists them.
    pub listed: Vec<Listing>,
    /// The position's procedures carrying an `activation_trigger`.
    pub triggered: Vec<Listing>,
}

/// A position's heading: `# <Name>`, the name's first letter capitalised.
pub fn heading(position: &str) -> String {
    let mut chars = position.chars();
    match chars.next() {
        Some(first) => format!("# {}{}", first.to_uppercase(), chars.as_str()),
        None => "#".to_owned(),
    }
}

/// `text` with every heading one level lower, except inside fenced code;
/// an H6 stays H6.
pub fn demote_headings(text: &str) -> String {
    let lines = super::_text::lines(text);
    let mut out = String::with_capacity(text.len() + lines.len());
    for line in &lines {
        let level = line.text.chars().take_while(|c| *c == '#').count();
        let rest = &line.text[level..];
        let heading = !line.fenced
            && (1..6).contains(&level)
            && (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'));
        if heading {
            out.push('#');
        }
        out.push_str(&text[line.span.clone()]);
    }
    out
}

impl Selection {
    /// The rendered section: the position's heading; each snippet in full,
    /// headings demoted, under the heading of its own position, a heading
    /// opening wherever the traversal moves to another position; then the
    /// listing lines, back under the position's heading; then the triggered
    /// procedures neither rendered in full nor listed.
    pub fn render(&self) -> String {
        let mut blocks: Vec<String> = vec![heading(&self.position)];
        let mut current = self.position.as_str();
        let shown: BTreeSet<&SnippetId> = self
            .in_full
            .iter()
            .map(|s| &s.id)
            .chain(self.listed.iter().map(|l| &l.id))
            .collect();
        for snippet in &self.in_full {
            if snippet.position != current {
                current = &snippet.position;
                blocks.push(heading(current));
            }
            blocks.push(in_full(snippet));
        }
        let lines: Vec<String> = self
            .listed
            .iter()
            .chain(self.triggered.iter().filter(|t| !shown.contains(&t.id)))
            .map(Listing::line)
            .collect();
        if !lines.is_empty() {
            if current != self.position {
                blocks.push(heading(&self.position));
            }
            blocks.push(lines.join("\n"));
        }
        let mut text = blocks.join("\n\n");
        text.push('\n');
        text
    }
}

/// One snippet in full: its body with headings demoted and, for a
/// triggered procedure, the trigger line directly under its heading.
fn in_full(snippet: &InFull) -> String {
    let body = demote_headings(snippet.body.trim_matches('\n'));
    let Some(trigger) = snippet
        .trigger
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return body;
    };
    let line = format!("when: {trigger} — `{}`", closure_command(&snippet.id));
    match body.split_once('\n') {
        Some((first, rest)) if first.starts_with('#') => format!("{first}\n\n{line}\n{rest}"),
        _ => format!("{line}\n\n{body}"),
    }
}

/// Subscribed procedures are configured responsibilities, including while events
/// are off; listing their declaration does not activate them.
pub(crate) fn responsibilities<'a>(
    entries: impl Iterator<Item = &'a crate::index::IndexEntry>,
) -> Option<String> {
    let mut entries: Vec<_> = entries
        .filter(|entry| {
            entry.taxonomy == "procedure"
                && entry.delivered_by.is_empty()
                && !entry.subscriptions.is_empty()
        })
        .collect();
    entries.sort_by(|left, right| left.id.cmp(&right.id));
    if entries.is_empty() {
        return None;
    }
    Some(format!(
        "## Configured Responsibilities — started when their events arrive\n\n{}",
        entries
            .into_iter()
            .map(|entry| format!(
                "- {}: {} — answers: {}",
                entry.id,
                entry.description.trim(),
                entry.subscriptions.join(", ")
            ))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}
