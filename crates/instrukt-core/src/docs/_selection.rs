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
//! and the position's triggered procedures. Which listing-line form a
//! rendering uses, inline ([`Selection::render`]) or headed
//! ([`Selection::render_headed`]), is the caller's too.

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
        match self.trigger_text() {
            Some(trigger) => format!(
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

    /// The listing line of the headed form, carrying no command:
    /// `- <id>: <description>`, or for a triggered procedure
    /// `- <id>: <description> — when: <trigger>`. Its heading names the
    /// command ([`LISTED_HEADING`], [`TRIGGERED_HEADING`]).
    pub fn headed_line(&self) -> String {
        match self.trigger_text() {
            Some(trigger) => format!(
                "- {}: {} — when: {trigger}",
                self.id,
                self.description.trim()
            ),
            None => format!("- {}: {}", self.id, self.description.trim()),
        }
    }

    /// The trigger the listing carries, when it has a non-empty one.
    fn trigger_text(&self) -> Option<&str> {
        self.trigger
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
    }
}

/// The heading over a run of listed snippets in the headed form: it names
/// the command each line's snippet is read with.
pub const LISTED_HEADING: &str = "## Listed — read each with `instrukt docs get <id>`";

/// The heading over a run of triggered procedures in the headed form: it
/// names the command each line's procedure is read with, its closure
/// included.
pub const TRIGGERED_HEADING: &str =
    "## Triggered procedures — read each with `instrukt docs get <id> --closure`";

/// How a rendering's listing lines lead to their command
/// (`project/instrukt/spec/feature/docs/progressive-retrieval`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListingForm {
    /// Each line carries its command.
    Inline,
    /// A heading over each run of lines of one kind carries the command once.
    Headed,
}

/// The line naming a snippet another repository publishes:
/// `` - <id> — `instrukt docs get <id> --closure` ``.
pub fn reference_line(id: &SnippetId) -> String {
    format!("- {id} — `{}`", closure_command(id))
}

/// The line naming a domain another repository publishes:
/// `` - <x> — `instrukt docs get --domain <x>` ``.
pub fn domain_reference_line(domain: &str) -> String {
    format!("- {domain} — `instrukt docs get --domain {domain}`")
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

/// A required read a rendering does not hold, named by its line in the
/// place its body would have had: under the heading of its own position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    /// The position the read belongs to (a domain, `general`, `project`),
    /// whose heading it sits under.
    pub position: String,
    /// Its [`reference_line`] or [`domain_reference_line`].
    pub line: String,
}

/// One entry of a selection's traversal: a snippet in full, or a read the
/// rendering does not hold, named. Both take the same place and open the
/// heading of their position the same way, so a rendering that holds a read
/// and one that names it differ only in body against line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    InFull(InFull),
    Named(Named),
}

impl Entry {
    /// The position whose heading the entry sits under.
    pub fn position(&self) -> &str {
        match self {
            Entry::InFull(snippet) => &snippet.position,
            Entry::Named(named) => &named.position,
        }
    }
}

/// What one position's selection renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The position's name: `general`, the domain, `project`.
    pub position: String,
    /// The traversal, in order: each required read before the entry
    /// requiring it, each once — a snippet in full, or, in a committed
    /// rendering, a read it does not hold, named.
    pub in_full: Vec<Entry>,
    /// The `listed` snippets the rendering does not hold, named by their
    /// [`reference_line`]: only a committed rendering has any.
    pub references: Vec<String>,
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
    /// The rendered section: the position's heading; each entry of the
    /// traversal under the heading of its own position, a heading opening
    /// wherever the traversal moves to another position — a snippet in full
    /// with its headings demoted, a named read as its line, consecutive
    /// lines as one list; then the named listed snippets and the listing
    /// lines, back under the position's heading; then the triggered
    /// procedures neither rendered in full nor listed. Each listing line
    /// carries its command (the inline form).
    pub fn render(&self) -> String {
        self.render_as(ListingForm::Inline)
    }

    /// The rendered section of [`Selection::render`] with its listing lines
    /// in the headed form: the listed snippets without a trigger under
    /// [`LISTED_HEADING`], then every triggered procedure — one `listed`
    /// names included — under [`TRIGGERED_HEADING`], each line without its
    /// command.
    pub fn render_headed(&self) -> String {
        self.render_as(ListingForm::Headed)
    }

    fn render_as(&self, form: ListingForm) -> String {
        let mut blocks: Vec<String> = vec![heading(&self.position)];
        let mut current = self.position.as_str();
        let shown: BTreeSet<&SnippetId> = self
            .in_full
            .iter()
            .filter_map(|entry| match entry {
                Entry::InFull(snippet) => Some(&snippet.id),
                Entry::Named(_) => None,
            })
            .chain(self.listed.iter().map(|l| &l.id))
            .collect();
        let mut listing = false;
        for entry in &self.in_full {
            if entry.position() != current {
                current = entry.position();
                blocks.push(heading(current));
                listing = false;
            }
            match entry {
                Entry::InFull(snippet) => {
                    blocks.push(in_full(snippet));
                    listing = false;
                }
                Entry::Named(named) => match blocks.last_mut() {
                    Some(lines) if listing => {
                        lines.push('\n');
                        lines.push_str(&named.line);
                    }
                    _ => {
                        blocks.push(named.line.clone());
                        listing = true;
                    }
                },
            }
        }
        let listings = self
            .listed
            .iter()
            .chain(self.triggered.iter().filter(|t| !shown.contains(&t.id)));
        let tail: Vec<String> = match form {
            ListingForm::Inline => {
                let lines: Vec<String> = self
                    .references
                    .iter()
                    .cloned()
                    .chain(listings.map(Listing::line))
                    .collect();
                if lines.is_empty() {
                    Vec::new()
                } else {
                    vec![lines.join("\n")]
                }
            }
            ListingForm::Headed => {
                let (triggered, plain): (Vec<&Listing>, Vec<&Listing>) =
                    listings.partition(|l| l.trigger_text().is_some());
                let mut tail = Vec::new();
                if !self.references.is_empty() {
                    tail.push(self.references.join("\n"));
                }
                for (title, run) in [(LISTED_HEADING, plain), (TRIGGERED_HEADING, triggered)] {
                    if !run.is_empty() {
                        let lines: Vec<String> = run.iter().map(|l| l.headed_line()).collect();
                        tail.push(format!("{title}\n\n{}", lines.join("\n")));
                    }
                }
                tail
            }
        };
        if !tail.is_empty() {
            if current != self.position {
                blocks.push(heading(&self.position));
            }
            blocks.extend(tail);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> SnippetId {
        SnippetId::parse(value).unwrap()
    }

    fn listing(value: &str, trigger: Option<&str>) -> Listing {
        Listing {
            id: id(value),
            description: format!("What {value} says."),
            trigger: trigger.map(str::to_owned),
        }
    }

    fn selection(listed: Vec<Listing>, triggered: Vec<Listing>) -> Selection {
        Selection {
            position: "general".to_owned(),
            in_full: vec![Entry::InFull(InFull {
                id: id("general/procedure/held"),
                position: "general".to_owned(),
                body: "# Held — Procedure\n\nDo the held thing.\n".to_owned(),
                trigger: Some("Use when holding.".to_owned()),
            })],
            references: Vec::new(),
            listed,
            triggered,
        }
    }

    #[test]
    fn headed_form_heads_plain_listings_with_the_get_command() {
        let rendered = selection(
            vec![
                listing("general/policy/first", None),
                listing("general/policy/second", None),
            ],
            Vec::new(),
        )
        .render_headed();
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn headed_form_heads_a_listed_triggered_procedure_with_the_closure_command() {
        let rendered = selection(
            vec![
                listing("general/policy/plain", None),
                listing("general/procedure/listed", Some("Use when listed.")),
            ],
            vec![listing("general/procedure/other", Some("Use when other."))],
        )
        .render_headed();
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn headed_form_does_not_relist_a_triggered_procedure_shown_in_full() {
        let rendered = selection(
            Vec::new(),
            vec![
                listing("general/procedure/held", Some("Use when holding.")),
                listing("general/procedure/other", Some("Use when other.")),
            ],
        )
        .render_headed();
        insta::assert_snapshot!(rendered);
    }
}
