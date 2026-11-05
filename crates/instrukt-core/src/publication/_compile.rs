//! Compilation: a bundle, a master or a global agent entrypoint expanded
//! into one self-contained text. Required reads are inlined ahead of the
//! text that requires them, each once; include fragments are expanded and
//! `@config` values resolved by the docs read pipeline with no runtime
//! session; planned material is dropped and pending snippets are never
//! inlined. A text left with an `@` reference fails compilation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::_tree::BUNDLE_FILE;
use super::PublicationError;
use crate::docs::{
    self, BaselineOwner, Corpus, Fragments, Lens, Position, RefTarget, Reference, RenderContext,
    Rendered, SourceFile, Values, View,
};
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug};
use crate::index::{GlobalIndex, IndexEntry, IndexScope};

/// How deep required reads may nest before compilation gives up.
const MAX_DEPTH: usize = 20;

/// The separator between compiled sections.
const SEPARATOR: &str = "\n\n---\n\n";

/// What one compilation reads.
pub(super) struct Compiler<'a> {
    pub home: &'a Home,
    pub project: &'a ProjectSlug,
    /// The committed documentation of the project.
    pub corpus: &'a Corpus,
    /// The domains the project publishes in this sync.
    pub owned: &'a BTreeSet<DomainName>,
    /// The global index as this sync leaves it.
    pub index: &'a GlobalIndex,
    /// Whether the events module is on: configured responsibilities are
    /// listed.
    pub events: bool,
    pub values: &'a Values<'a>,
    /// The project's primary worktree, against which its `docs/…`
    /// references resolve.
    pub primary: &'a Path,
    /// Bundles compiled so far, by domain.
    bundles: BTreeMap<DomainName, String>,
    compiling: BTreeSet<DomainName>,
}

impl<'a> Compiler<'a> {
    #[allow(
        clippy::too_many_arguments,
        reason = "one field per input a compilation reads"
    )]
    pub(super) fn new(
        home: &'a Home,
        project: &'a ProjectSlug,
        corpus: &'a Corpus,
        owned: &'a BTreeSet<DomainName>,
        index: &'a GlobalIndex,
        events: bool,
        values: &'a Values<'a>,
        primary: &'a Path,
    ) -> Self {
        Self {
            home,
            project,
            corpus,
            owned,
            index,
            events,
            values,
            primary,
            bundles: BTreeMap::new(),
            compiling: BTreeSet::new(),
        }
    }

    fn project_context(&self) -> RenderContext<'_> {
        RenderContext {
            fragments: Fragments::CorpusFirst(self.corpus),
            values: self.values,
            repo_root: Some(self.primary),
            project: Some(self.project),
        }
    }

    fn home_context(&self) -> RenderContext<'_> {
        RenderContext {
            fragments: Fragments::Home(self.home),
            values: self.values,
            repo_root: None,
            project: None,
        }
    }

    /// The bundle of a domain this project publishes: a title, its baseline
    /// manifest compiled, then its progressive baseline index, activatable
    /// procedures and (with events on) configured responsibilities.
    pub(super) fn bundle(&mut self, domain: &DomainName) -> Result<String, PublicationError> {
        if let Some(done) = self.bundles.get(domain) {
            return Ok(done.clone());
        }
        let file = format!("{}/{BUNDLE_FILE}", Position::of_domain(domain).dir());
        if !self.compiling.insert(domain.clone()) {
            return Err(compile_error(&file, "its baseline requires its own bundle"));
        }
        let owner = BaselineOwner::Domain(domain.clone());
        let manifests: Vec<(bool, Vec<Reference>, String)> = self
            .corpus
            .baselines()
            .iter()
            .filter(|m| m.owner() == &owner)
            .map(|m| {
                (
                    m.is_progressive(),
                    m.entries().into_iter().cloned().collect(),
                    m.path().display().to_string(),
                )
            })
            .collect();
        let mut sections = Vec::new();
        let mut seen = BTreeSet::new();
        for (_, entries, path) in manifests.iter().filter(|(progressive, ..)| !progressive) {
            for reference in entries {
                sections.extend(self.read(reference, path, &mut seen, MAX_DEPTH)?);
            }
        }
        let progressive: Vec<&Reference> = manifests
            .iter()
            .filter(|(progressive, ..)| *progressive)
            .flat_map(|(_, entries, _)| entries)
            .collect();
        let scope = |e: &IndexEntry| e.scope == IndexScope::Global && e.domain == domain.as_str();
        let listed = self.listings(&progressive, scope, "Domain", &file)?;
        let mut text = format!(
            "# {domain} — Platform Baseline\n\n{}",
            sections.join(SEPARATOR)
        );
        if !listed.is_empty() {
            text = format!("{}{SEPARATOR}{listed}\n", text.trim_end());
        }
        let text = finish(&text);
        self_contained(&file, &text)?;
        self.compiling.remove(domain);
        self.bundles.insert(domain.clone(), text.clone());
        Ok(text)
    }

    /// The `AGENTS.md` of the master at `file` (repository-relative). The
    /// repository-root master gains the project's baseline index,
    /// activatable procedures and (with events on) configured
    /// responsibilities.
    pub(super) fn master(&mut self, file: &str, text: &str) -> Result<String, PublicationError> {
        let mut compiled = self.document(file, text)?;
        if file == super::_source::MASTER_FILE_NAME {
            let local: Vec<Reference> = self
                .corpus
                .baselines()
                .iter()
                .filter(|m| m.owner() == &BaselineOwner::Project)
                .flat_map(|m| m.entries().into_iter().cloned())
                .collect();
            let refs: Vec<&Reference> = local.iter().collect();
            let project = self.project.clone();
            let scope =
                move |e: &IndexEntry| e.scope == IndexScope::Local && e.source_project == project;
            let listed = self.listings(&refs, scope, "Project", file)?;
            if !listed.is_empty() {
                compiled = format!("{}{SEPARATOR}{listed}\n", compiled.trim_end());
            }
        }
        let compiled = finish(&compiled);
        self_contained(file, &compiled)?;
        Ok(compiled)
    }

    /// A global agent entrypoint: the global master compiled, the agent's
    /// addendum, then `roster`.
    pub(super) fn entrypoint(
        &mut self,
        master: (&str, &str),
        addendum: Option<(&str, &str)>,
        roster: &str,
    ) -> Result<String, PublicationError> {
        let (file, text) = master;
        let mut compiled = self.document(file, text)?;
        if let Some((addendum_file, addendum_text)) = addendum {
            let rendered = self.render(addendum_file, addendum_text)?;
            if !rendered.required_reads.is_empty() {
                return Err(compile_error(
                    addendum_file,
                    "an addendum has no required reads",
                ));
            }
            let body = rendered.text.trim();
            if !body.is_empty() {
                compiled = format!("{}{SEPARATOR}{body}", compiled.trim_end());
            }
        }
        if !roster.is_empty() {
            compiled = format!("{}{SEPARATOR}{roster}\n", compiled.trim_end());
        }
        let compiled = finish(&compiled);
        self_contained(file, &compiled)?;
        Ok(compiled)
    }

    /// The domains whose bundle `text`'s required reads name.
    pub(super) fn inlined_bundles(
        &self,
        file: &str,
        text: &str,
    ) -> Result<BTreeSet<DomainName>, PublicationError> {
        Ok(self
            .render(file, text)?
            .required_reads
            .iter()
            .filter_map(|r| match &r.target {
                Some(RefTarget::Bundle(domain)) => Some(domain.clone()),
                _ => None,
            })
            .collect())
    }

    /// The refusal for a required read of `file` into `domain` when no
    /// adopted docs repository publishes that domain on this computer;
    /// `None` when the domain is published (the read is unresolved for
    /// another reason).
    fn missing_domain(&self, domain: &DomainName, file: &str) -> Option<PublicationError> {
        (!self.owned.contains(domain) && !self.home.domain_dir(domain).is_dir()).then(|| {
            PublicationError::Docs(docs::DocsError::DomainUnpublished {
                domain: domain.clone(),
                required_by: file.to_owned(),
            })
        })
    }

    fn render(&self, file: &str, text: &str) -> Result<Rendered, PublicationError> {
        docs::render(text, Lens::Current, &self.project_context())
            .map_err(|e| compile_error(file, &e.to_string()))
    }

    /// A non-snippet source (a master): its required reads, then its text.
    fn document(&mut self, file: &str, text: &str) -> Result<String, PublicationError> {
        let rendered = self.render(file, text)?;
        let mut seen = BTreeSet::new();
        let mut sections = Vec::new();
        for reference in &rendered.required_reads {
            sections.extend(self.read(reference, file, &mut seen, MAX_DEPTH)?);
        }
        let body = strip_leading_separators(rendered.text.trim());
        if !body.is_empty() {
            sections.push(body.to_owned());
        }
        Ok(sections.join(SEPARATOR))
    }

    /// The sections one required read contributes: its own required reads
    /// first, then itself. Each target is read once per compilation; a
    /// pending snippet contributes nothing.
    fn read(
        &mut self,
        reference: &Reference,
        file: &str,
        seen: &mut BTreeSet<RefTarget>,
        depth: usize,
    ) -> Result<Vec<String>, PublicationError> {
        let Some(target) = reference.target.clone() else {
            return Err(compile_error(
                file,
                &format!("cannot resolve the required read `@{}`", reference.raw),
            ));
        };
        if depth == 0 {
            return Err(compile_error(
                file,
                &format!("required reads nest too deep at `@{}`", reference.raw),
            ));
        }
        if !seen.insert(target.clone()) {
            return Ok(Vec::new());
        }
        let unresolved = || {
            compile_error(
                file,
                &format!("cannot resolve the required read `@{}`", reference.raw),
            )
        };
        match &target {
            RefTarget::Bundle(domain) => {
                let text = if self.owned.contains(domain) {
                    self.bundle(domain)?
                } else {
                    let path = self.home.domain_dir(domain).join(BUNDLE_FILE);
                    match std::fs::read_to_string(&path) {
                        Ok(text) => text,
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                            return Err(self
                                .missing_domain(domain, file)
                                .unwrap_or_else(unresolved));
                        }
                        Err(source) => return Err(PublicationError::Io { path, source }),
                    }
                };
                let text = strip_leading_separators(text.trim()).to_owned();
                Ok(if text.is_empty() {
                    Vec::new()
                } else {
                    vec![text]
                })
            }
            RefTarget::LocalFile(path) => {
                let entries: Vec<Reference> = self
                    .corpus
                    .baselines()
                    .iter()
                    .filter(|m| m.path() == Path::new(path))
                    .flat_map(|m| m.entries().into_iter().cloned())
                    .collect();
                if entries.is_empty()
                    && !self
                        .corpus
                        .baselines()
                        .iter()
                        .any(|m| m.path() == Path::new(path))
                {
                    return Err(unresolved());
                }
                let mut sections = Vec::new();
                for entry in &entries {
                    sections.extend(self.read(entry, path, seen, depth - 1)?);
                }
                Ok(sections)
            }
            RefTarget::Snippet(id) => {
                let own = target.is_local()
                    || id
                        .as_str()
                        .split('/')
                        .next()
                        .and_then(|d| DomainName::parse(d).ok())
                        .is_some_and(|d| self.owned.contains(&d));
                let (rendered, source) = if own {
                    let snippet = self.corpus.snippet(id).ok_or_else(unresolved)?;
                    match snippet.view(Lens::Current) {
                        View::Visible { version, .. } => (
                            version
                                .render(Lens::Current, &self.project_context())
                                .map_err(|e| {
                                    compile_error(
                                        &snippet.path().display().to_string(),
                                        &e.to_string(),
                                    )
                                })?,
                            snippet.path().display().to_string(),
                        ),
                        View::Withheld { .. } | View::Retired => return Ok(Vec::new()),
                    }
                } else {
                    let path = self.home.docs_dir().join(format!("{}.md", id.as_str()));
                    let text = match std::fs::read_to_string(&path) {
                        Ok(text) => text,
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                            let domain = id
                                .as_str()
                                .split('/')
                                .next()
                                .and_then(|d| DomainName::parse(d).ok());
                            return Err(domain
                                .and_then(|d| self.missing_domain(&d, file))
                                .unwrap_or_else(unresolved));
                        }
                        Err(source) => return Err(PublicationError::Io { path, source }),
                    };
                    let source = docs::published_source(id).ok_or_else(unresolved)?;
                    let corpus = Corpus::from_sources(
                        None,
                        self.home,
                        [SourceFile {
                            path: source.clone(),
                            text,
                        }],
                    );
                    let snippet = corpus.snippet(id).ok_or_else(unresolved)?;
                    match snippet.view(Lens::Current) {
                        View::Visible { version, .. } => (
                            version
                                .render(Lens::Current, &self.home_context())
                                .map_err(|e| {
                                    compile_error(&path.display().to_string(), &e.to_string())
                                })?,
                            path.display().to_string(),
                        ),
                        View::Withheld { .. } | View::Retired => return Ok(Vec::new()),
                    }
                };
                let mut sections = Vec::new();
                for nested in &rendered.required_reads {
                    sections.extend(self.read(nested, &source, seen, depth - 1)?);
                }
                let body = strip_leading_separators(rendered.text.trim());
                if !body.is_empty() {
                    sections.push(format!("id: {}\n\n{body}", id.as_str()));
                }
                Ok(sections)
            }
        }
    }

    /// The generated listings of one scope: the baseline index its
    /// progressive manifest names, its activatable procedures and, with
    /// events on, its configured responsibilities. Sections without lines
    /// are left out.
    fn listings(
        &self,
        progressive: &[&Reference],
        in_scope: impl Fn(&IndexEntry) -> bool,
        label: &str,
        file: &str,
    ) -> Result<String, PublicationError> {
        let by_id: BTreeMap<&str, &IndexEntry> = self
            .index
            .snippets
            .iter()
            .map(|e| (e.id.as_str(), e))
            .collect();
        let mut baseline = Vec::new();
        for reference in progressive {
            let Some(RefTarget::Snippet(id)) = &reference.target else {
                return Err(compile_error(
                    file,
                    &format!(
                        "the progressive baseline names `@{}`, which is no snippet",
                        reference.raw
                    ),
                ));
            };
            let entry = by_id.get(id.as_str()).ok_or_else(|| {
                compile_error(
                    file,
                    &format!(
                        "the progressive baseline names `@{}`, which is not indexed",
                        reference.raw
                    ),
                )
            })?;
            if entry.delivered_by.is_empty() {
                baseline.push(listing_line(entry, None));
            }
        }
        let live =
            |e: &&IndexEntry| in_scope(e) && e.taxonomy == "procedure" && e.delivered_by.is_empty();
        let activatable: Vec<String> = self
            .index
            .snippets
            .iter()
            .filter(live)
            .filter_map(|e| {
                let trigger = e.activation_trigger.as_deref()?.trim();
                (!trigger.is_empty()).then(|| listing_line(e, Some(trigger)))
            })
            .collect();
        let configured: Vec<String> = if self.events {
            self.index
                .snippets
                .iter()
                .filter(live)
                .filter(|e| !e.subscriptions.is_empty())
                .map(|e| listing_line(e, Some(&format!("answers: {}", e.subscriptions.join(", ")))))
                .collect()
        } else {
            Vec::new()
        };
        let blocks: Vec<String> = [
            (
                format!("## {label} baseline index — load via `instrukt docs get <id>` when relevant"),
                baseline,
            ),
            (
                "## Activatable Procedures — load via `instrukt docs get <id> --closure` before acting".to_owned(),
                activatable,
            ),
            (
                "## Configured Responsibilities — started when their events arrive".to_owned(),
                configured,
            ),
        ]
        .into_iter()
        .filter(|(_, lines)| !lines.is_empty())
        .map(|(header, lines)| format!("{header}\n\n{}", lines.join("\n")))
        .collect();
        Ok(blocks.join("\n\n"))
    }
}

/// The roster of published domains a global entrypoint ends with: one
/// line per domain named in `descriptions` or carrying live global entries
/// in `index`, except those in `inlined`, with its live global snippet
/// count, bundle path and, when its `index.yaml` gives one, its
/// description.
pub(super) fn roster(
    index: &GlobalIndex,
    descriptions: &BTreeMap<DomainName, Option<String>>,
    inlined: &BTreeSet<DomainName>,
) -> Result<String, PublicationError> {
    let mut counts: BTreeMap<String, usize> = descriptions
        .keys()
        .map(|domain| (domain.to_string(), 0))
        .collect();
    for entry in index
        .snippets
        .iter()
        .filter(|e| e.scope == IndexScope::Global && e.delivered_by.is_empty())
    {
        *counts.entry(entry.domain.clone()).or_default() += 1;
    }
    let mut lines = Vec::new();
    for (domain, count) in counts {
        let name =
            DomainName::parse(&domain).map_err(|e| compile_error(&domain, &e.to_string()))?;
        if inlined.contains(&name) {
            continue;
        }
        let mut line = format!(
            "- `{domain}` — {count} snippet(s); bundle: `~/.instrukt/docs/{domain}/{BUNDLE_FILE}`"
        );
        if let Some(Some(description)) = descriptions.get(&name) {
            line.push_str(&format!(" — {description}"));
        }
        lines.push(line);
    }
    if lines.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "## Published Domains — load the relevant domain's bundle when needed\n\n{}",
        lines.join("\n")
    ))
}

/// `- `<id>` — <description>[ — <tail>]`.
fn listing_line(entry: &IndexEntry, tail: Option<&str>) -> String {
    let mut line = format!("- `{}`", entry.id.as_str());
    if !entry.description.is_empty() {
        line.push_str(&format!(" — {}", entry.description));
    }
    if let Some(tail) = tail {
        line.push_str(&format!(" — {tail}"));
    }
    line
}

fn compile_error(file: &str, reason: &str) -> PublicationError {
    PublicationError::Compile {
        file: file.to_owned(),
        reason: reason.to_owned(),
    }
}

/// `text` without leading `---` separator lines.
fn strip_leading_separators(text: &str) -> &str {
    let mut rest = text;
    loop {
        let trimmed = rest.trim_start_matches(['\n', ' ', '\t', '\r']);
        match trimmed.strip_prefix("---") {
            Some(after) if after.is_empty() || after.starts_with('\n') => rest = after,
            _ => return trimmed,
        }
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

/// Refuses a compiled text that still carries an `@` reference the docs
/// model sees: an agent CLI would try to load it.
fn self_contained(file: &str, text: &str) -> Result<(), PublicationError> {
    match docs::visible_references(text).into_iter().next() {
        Some(reference) => Err(compile_error(
            file,
            &format!("the compiled text still references `@{reference}`"),
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
}
