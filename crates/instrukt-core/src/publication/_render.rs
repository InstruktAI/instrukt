//! Rendering: a position's `index.yaml` selection, a project's root
//! `AGENTS.md`, a nested master and a global agent file, each as one
//! self-contained text
//! (`project/instrukt/spec/feature/publication/agent-entrypoints`).
//!
//! Snippets in full come with their closure — each required read before
//! the snippet requiring it, each once per file — in traversal order, under
//! the heading of the position each belongs to; the section shapes are the
//! docs context's ([`docs::Selection`]). `@config` values resolve against the
//! publishing project's configuration with no runtime session; planned
//! material is dropped and a pending snippet is never inlined. A required
//! read that cannot be resolved, or a text left with a reference, fails
//! rendering.

use std::collections::{BTreeMap, BTreeSet};

use super::PublicationError;
use crate::docs::{
    self, AgentCli, Corpus, InFull, Lens, Listing, PositionIndex, RefTarget, Reference,
    RenderContext, Rendered, Selection, SourceFile, Taxonomy, Values, View,
};
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug, SnippetId};
use crate::index::{GlobalIndex, IndexEntry, IndexScope};

/// The name of the position a project's own snippets belong to.
const PROJECT: &str = "project";

/// What one sync's renderings read.
pub(super) struct Renderer<'a> {
    pub home: &'a Home,
    pub project: &'a ProjectSlug,
    /// The committed documentation of the project.
    pub corpus: &'a Corpus,
    /// The domains the project publishes in this sync.
    pub owned: &'a BTreeSet<DomainName>,
    /// The global index as this sync leaves it.
    pub index: &'a GlobalIndex,
    /// Every published position's `index.yaml` as this sync leaves the
    /// computer, by the domain name it is addressed by (`general`
    /// included); a position without one is absent.
    pub indexes: &'a BTreeMap<DomainName, PositionIndex>,
    /// Whether the events module is on: configured responsibilities are
    /// listed in the root `AGENTS.md`.
    pub events: bool,
    pub values: &'a Values<'a>,
}

/// The snippets one rendered file holds in full so far, in traversal
/// order, and the domains whose `expanded` set a bare-domain read already
/// brought in.
#[derive(Default, Clone)]
struct Expansion {
    order: Vec<InFull>,
    seen: BTreeSet<SnippetId>,
    domains: BTreeSet<DomainName>,
}

impl Expansion {
    /// The ids rendered in full or listed so far: a triggered procedure
    /// among them is not listed again.
    fn shown<'s>(&'s self, listed: &'s [Listing]) -> BTreeSet<&'s SnippetId> {
        self.order
            .iter()
            .map(|s| &s.id)
            .chain(listed.iter().map(|l| &l.id))
            .collect()
    }
}

impl Renderer<'_> {
    /// The global agent file of `cli`: `general`'s section, one group per
    /// other published domain with an `index.yaml`, by name, then the
    /// CLI's `agents` snippets in full with their closure.
    pub(super) fn global_file(&self, cli: AgentCli) -> Result<String, PublicationError> {
        let file = "docs/general/index.yaml";
        let general =
            DomainName::parse(docs::GENERAL).map_err(|e| render_error(file, &e.to_string()))?;
        let index = self.indexes.get(&general);
        let mut expansion = Expansion::default();
        let mut listed = Vec::new();
        if let Some(index) = index {
            self.expand(index.expanded(), file, &mut expansion)?;
            listed = self.listings(index.listed(), file)?;
        }
        let mut shown: BTreeSet<SnippetId> =
            expansion.shown(&listed).into_iter().cloned().collect();
        let triggered = self.triggered(|e| global_of(e, &general), &shown);
        shown.extend(triggered.iter().map(|l| l.id.clone()));
        let mut blocks = vec![
            Selection {
                position: general.as_str().to_owned(),
                in_full: expansion.order.clone(),
                listed,
                triggered,
            }
            .render(),
        ];
        for (domain, index) in self.indexes {
            if domain == &general {
                continue;
            }
            let PositionIndex::Domain(domain_index) = index else {
                continue;
            };
            let mut lines = vec![domain_line(domain, domain_index.description.as_deref())];
            let triggered = self.triggered(|e| global_of(e, domain), &shown);
            shown.extend(triggered.iter().map(|l| l.id.clone()));
            lines.extend(triggered.iter().map(Listing::line));
            blocks.push(format!(
                "{}\n\n{}\n",
                docs::heading(domain.as_str()),
                lines.join("\n")
            ));
        }
        if let Some(PositionIndex::General(general_index)) = index
            && let Some(ids) = general_index.agents.get(&cli)
        {
            // What the general section holds in full is not repeated.
            let mut own = Expansion {
                order: Vec::new(),
                seen: expansion.seen.clone(),
                domains: expansion.domains.clone(),
            };
            self.expand(ids, file, &mut own)?;
            if let Some(first) = own.order.first() {
                blocks.push(
                    Selection {
                        position: first.position.clone(),
                        in_full: own.order,
                        listed: Vec::new(),
                        triggered: Vec::new(),
                    }
                    .render(),
                );
            }
        }
        let text = finish(&blocks.join("\n"));
        self_contained(file, &text)?;
        Ok(text)
    }

    /// The root `AGENTS.md`: the master's text, then the rendering of
    /// `docs/project/index.yaml` (or, without one, the project's triggered
    /// procedures) and, with events on, the configured responsibilities.
    pub(super) fn root_agents(
        &self,
        file: &str,
        master: &str,
        project_index: Option<&PositionIndex>,
    ) -> Result<String, PublicationError> {
        let rendered = self.render_master(file, master)?;
        let mut expansion = Expansion::default();
        let mut listed = Vec::new();
        let index_file = "docs/project/index.yaml";
        if let Some(index) = project_index {
            self.expand(index.expanded(), index_file, &mut expansion)?;
            listed = self.listings(index.listed(), index_file)?;
        }
        let shown: BTreeSet<SnippetId> = expansion.shown(&listed).into_iter().cloned().collect();
        let triggered = self.triggered(|e| self.local(e), &shown);
        let mut blocks = vec![rendered.text.trim().to_owned()];
        if !expansion.order.is_empty() || !listed.is_empty() || !triggered.is_empty() {
            blocks.push(
                Selection {
                    position: PROJECT.to_owned(),
                    in_full: expansion.order,
                    listed,
                    triggered,
                }
                .render(),
            );
        }
        if self.events {
            let configured: Vec<String> = self
                .index
                .snippets
                .iter()
                .filter(|e| self.local(e) && live_procedure(e) && !e.subscriptions.is_empty())
                .map(|e| {
                    format!(
                        "- {}: {} — answers: {}",
                        e.id,
                        e.description.trim(),
                        e.subscriptions.join(", ")
                    )
                })
                .collect();
            if !configured.is_empty() {
                blocks.push(format!(
                    "## Configured Responsibilities — started when their events arrive\n\n{}",
                    configured.join("\n")
                ));
            }
        }
        let text = finish(&blocks.join("\n\n"));
        self_contained(file, &text)?;
        Ok(text)
    }

    /// A nested master's `AGENTS.md`: its own text, then its required
    /// reads in full with their closure.
    pub(super) fn nested(&self, file: &str, master: &str) -> Result<String, PublicationError> {
        let rendered = self.render_master(file, master)?;
        let mut expansion = Expansion::default();
        for reference in &rendered.required_reads {
            self.follow(reference, file, &mut expansion)?;
        }
        let mut blocks = vec![rendered.text.trim().to_owned()];
        if let Some(first) = expansion.order.first() {
            blocks.push(
                Selection {
                    position: first.position.clone(),
                    in_full: expansion.order.clone(),
                    listed: Vec::new(),
                    triggered: Vec::new(),
                }
                .render(),
            );
        }
        let text = finish(&blocks.join("\n\n"));
        self_contained(file, &text)?;
        Ok(text)
    }

    fn render_master(&self, file: &str, text: &str) -> Result<Rendered, PublicationError> {
        let cx = RenderContext {
            home: self.home,
            values: self.values,
            project: Some(self.project),
        };
        docs::render(text, Lens::Current, &cx).map_err(|e| render_error(file, &e.to_string()))
    }

    /// Whether `entry` is one of this project's own snippets.
    fn local(&self, entry: &IndexEntry) -> bool {
        entry.scope == IndexScope::Local && &entry.source_project == self.project
    }

    /// `ids` into `into` in traversal order: each after its transitive
    /// required reads, each once.
    fn expand(
        &self,
        ids: &[SnippetId],
        file: &str,
        into: &mut Expansion,
    ) -> Result<(), PublicationError> {
        for id in ids {
            self.follow(
                &Reference {
                    raw: id.as_str().to_owned(),
                    target: Some(RefTarget::Snippet(id.clone())),
                },
                file,
                into,
            )?;
        }
        Ok(())
    }

    /// One required read of `file` into `into`: a snippet after its own
    /// closure, a bare domain as that domain's `expanded` set.
    fn follow(
        &self,
        reference: &Reference,
        file: &str,
        into: &mut Expansion,
    ) -> Result<(), PublicationError> {
        let unresolved = || {
            render_error(
                file,
                &format!("cannot resolve the required read `{}`", reference.raw),
            )
        };
        match &reference.target {
            Some(RefTarget::Snippet(id)) => {
                if !into.seen.insert(id.clone()) {
                    return Ok(());
                }
                let Some((rendered, source)) = self.load(id, file)? else {
                    return Ok(());
                };
                for nested in &rendered.required_reads {
                    self.follow(nested, &source, into)?;
                }
                into.order.push(InFull {
                    id: id.clone(),
                    position: position_name(id),
                    body: rendered.text,
                    trigger: self.entry(id).and_then(trigger_of).map(str::to_owned),
                });
                Ok(())
            }
            Some(RefTarget::Domain(domain)) => {
                if !into.domains.insert(domain.clone()) {
                    return Ok(());
                }
                match self.indexes.get(domain) {
                    Some(index) => self.expand(index.expanded(), file, into),
                    None => Err(self.missing_domain(domain, file).unwrap_or_else(unresolved)),
                }
            }
            Some(RefTarget::Bundle(_) | RefTarget::LocalFile(_)) | None => Err(unresolved()),
        }
    }

    /// The snippet `id` through the read pipeline, with the file it is read
    /// from; `None` when it is pending.
    fn load(
        &self,
        id: &SnippetId,
        file: &str,
    ) -> Result<Option<(Rendered, String)>, PublicationError> {
        let unresolved = || render_error(file, &format!("cannot resolve the required read `{id}`"));
        let own = id.domain().is_none_or(|d| self.owned.contains(&d));
        if own {
            let snippet = self.corpus.snippet(id).ok_or_else(unresolved)?;
            let source = snippet.path().display().to_string();
            let View::Visible { version, .. } = snippet.view(Lens::Current) else {
                return Ok(None);
            };
            let cx = RenderContext {
                home: self.home,
                values: self.values,
                project: Some(self.project),
            };
            let rendered = version
                .render(Lens::Current, &cx)
                .map_err(|e| render_error(&source, &e.to_string()))?;
            return Ok(Some((rendered, source)));
        }
        let path = self.home.docs_dir().join(format!("{}.md", id.as_str()));
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(id
                    .domain()
                    .and_then(|d| self.missing_domain(&d, file))
                    .unwrap_or_else(unresolved));
            }
            Err(source) => return Err(PublicationError::Io { path, source }),
        };
        let source = docs::published_source(id).ok_or_else(unresolved)?;
        let corpus = Corpus::from_sources(None, self.home, [SourceFile { path: source, text }]);
        let snippet = corpus.snippet(id).ok_or_else(unresolved)?;
        let View::Visible { version, .. } = snippet.view(Lens::Current) else {
            return Ok(None);
        };
        let cx = RenderContext {
            home: self.home,
            values: self.values,
            project: None,
        };
        let shown = path.display().to_string();
        let rendered = version
            .render(Lens::Current, &cx)
            .map_err(|e| render_error(&shown, &e.to_string()))?;
        Ok(Some((rendered, shown)))
    }

    /// The listing line of each delivered id of `ids`.
    fn listings(&self, ids: &[SnippetId], file: &str) -> Result<Vec<Listing>, PublicationError> {
        let mut listed = Vec::new();
        for id in ids {
            let entry = self
                .entry(id)
                .ok_or_else(|| render_error(file, &format!("`{id}` names no published snippet")))?;
            if entry.delivered_by.is_empty() {
                listed.push(listing(entry));
            }
        }
        Ok(listed)
    }

    /// The delivered procedures carrying an `activation_trigger` that
    /// `in_scope` selects and `shown` does not hold yet, by id.
    fn triggered(
        &self,
        in_scope: impl Fn(&IndexEntry) -> bool,
        shown: &BTreeSet<SnippetId>,
    ) -> Vec<Listing> {
        let mut lines: Vec<Listing> = self
            .index
            .snippets
            .iter()
            .filter(|e| in_scope(e) && trigger_of(e).is_some() && !shown.contains(&e.id))
            .map(listing)
            .collect();
        lines.sort_by(|a, b| a.id.cmp(&b.id));
        lines
    }

    fn entry(&self, id: &SnippetId) -> Option<&IndexEntry> {
        self.index.snippets.iter().find(|e| &e.id == id)
    }

    /// The refusal for a read of `file` into `domain` when no adopted docs
    /// repository publishes that domain on this computer; `None` when the
    /// domain is published (the read is unresolved for another reason).
    fn missing_domain(&self, domain: &DomainName, file: &str) -> Option<PublicationError> {
        (!self.owned.contains(domain) && !self.home.domain_dir(domain).is_dir()).then(|| {
            PublicationError::Docs(docs::DocsError::DomainUnpublished {
                domain: domain.clone(),
                required_by: file.to_owned(),
            })
        })
    }
}

/// Whether `entry` is a published snippet of `domain`.
fn global_of(entry: &IndexEntry, domain: &DomainName) -> bool {
    entry.scope == IndexScope::Global && entry.domain == domain.as_str()
}

fn live_procedure(entry: &IndexEntry) -> bool {
    Taxonomy::from_dir(&entry.taxonomy) == Some(Taxonomy::Procedure)
        && entry.delivered_by.is_empty()
}

/// A delivered procedure's non-empty `activation_trigger`.
fn trigger_of(entry: &IndexEntry) -> Option<&str> {
    live_procedure(entry)
        .then_some(entry.activation_trigger.as_deref())
        .flatten()
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn listing(entry: &IndexEntry) -> Listing {
    Listing {
        id: entry.id.clone(),
        description: entry.description.clone(),
        trigger: trigger_of(entry).map(str::to_owned),
    }
}

/// The name of the position a snippet belongs to: its domain, or
/// `project`.
fn position_name(id: &SnippetId) -> String {
    id.domain()
        .map_or_else(|| PROJECT.to_owned(), |d| d.as_str().to_owned())
}

/// A domain's line in the global files: its description and the command
/// that renders it.
fn domain_line(domain: &DomainName, description: Option<&str>) -> String {
    let command = format!("`instrukt docs get --domain {domain}`");
    match description.map(str::trim).filter(|d| !d.is_empty()) {
        Some(description) => format!("{description} — {command}"),
        None => command,
    }
}

fn render_error(file: &str, reason: &str) -> PublicationError {
    PublicationError::Rendering {
        file: file.to_owned(),
        reason: reason.to_owned(),
    }
}

/// The final text: authoring lock sentinels removed, runs of blank lines
/// collapsed, one trailing newline.
fn finish(text: &str) -> String {
    let kept: Vec<&str> = text
        .split('\n')
        .filter(|line| {
            let line = line.trim();
            !(line.starts_with("<!--") && line.ends_with("-->") && line.contains("LOCKED_FILE"))
        })
        .collect();
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in kept {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    let trimmed = out.trim_matches('\n');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

/// Refuses a rendered text that still carries a reference the docs model
/// sees: an agent CLI would try to load it.
fn self_contained(file: &str, text: &str) -> Result<(), PublicationError> {
    match docs::visible_references(text).into_iter().next() {
        Some(reference) => Err(render_error(
            file,
            &format!("the rendered text still references `@{reference}`"),
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finishing_drops_lock_sentinels_and_collapses_blank_runs() {
        let sentinel = "<!-- LOCKED_FILE: keep -->";
        let text = format!("{sentinel}\n\n\n# A\n\n\n\nb\n\n");
        assert_eq!(finish(&text), finish("# A\n\nb"));
        assert_eq!(finish("# A\n\nb").lines().count(), 3);
    }

    #[test]
    fn a_domain_line_without_a_description_is_its_command() {
        let domain = DomainName::parse("sdlc").unwrap();
        assert_eq!(domain_line(&domain, Some(" ")), domain_line(&domain, None));
        assert_ne!(
            domain_line(&domain, Some("Code work")),
            domain_line(&domain, None)
        );
    }
}
