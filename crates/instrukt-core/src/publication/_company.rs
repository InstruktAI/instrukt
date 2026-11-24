//! A company home's committed `AGENTS.md`
//! (`project/instrukt/spec/feature/publication/agent-entrypoints`, "Company
//! home entrypoint"): the generated line, the company passage, each document
//! `docs/index.yaml` expands, in full and in the index's order, its required
//! reads before it and each once, then one line for each document it lists.
//!
//! A company's knowledge stands alone. The index and the company's documents
//! name only the home's own Markdown documents: an index entry by its path
//! relative to `docs/`, a required read by its path relative to the file that
//! reads it, each confined to the home's `docs/` folder and ending in `.md`.
//! An entry, a read or a reference to a snippet, a domain or a file outside
//! `docs/`, a file the home does not hold or one reached through a symbolic
//! link, a cycle of required reads, and an `@config`, `@exec` or `@include`
//! are findings, and a home with a finding is not rendered.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_yaml_ng::Value;

use super::_render::{
    AGENTS_FILE, GENERATED_LINE, OVERRIDE_FILE, PlannedFile, RenderFinding, RenderFindingKind,
    RenderPlan, finish,
};
use super::_source::{Committed, EntryIndex};
use crate::docs::{self, CompanyDocument, CompanyFrontmatter, CompanyReference, Directive};
use crate::ids::{DomainName, SnippetId};

/// The home's documents are below this folder.
const DOCS: &str = "docs/";

/// The company index, which selects the home's rendering.
pub(super) const COMPANY_INDEX: &str = "docs/index.yaml";

/// A key a company index holds: the closed set of two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanyIndexKey {
    /// The documents held in full.
    Expanded,
    /// The documents named by one line.
    Listed,
}

impl CompanyIndexKey {
    const ALL: [Self; 2] = [Self::Expanded, Self::Listed];

    /// The key as the index spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Listed => "listed",
        }
    }

    /// The key the index spells `name`; `None` for any other.
    fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.as_str() == name)
    }
}

impl std::fmt::Display for CompanyIndexKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A company's `docs/index.yaml`: the documents its agents start with in
/// full, in order, and those named by one line. No other key is allowed.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CompanyIndex {
    /// Each document held in full, with its required reads.
    #[serde(default)]
    expanded: Vec<CompanyEntry>,
    /// Each document named by one line.
    #[serde(default)]
    listed: Vec<CompanyEntry>,
}

/// An entry of a company index: a document's path relative to `docs/`,
/// ending in `.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CompanyEntry(String);

impl CompanyEntry {
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl CompanyEntry {
    /// What an entry's path must match: it ends in `.md`.
    const PATTERN: &'static str = r"\.md$";
}

impl JsonSchema for CompanyEntry {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CompanyEntry".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": Self::PATTERN,
            "description": "A company document's path relative to `docs/`, ending in `.md`."
        })
    }
}

impl CompanyIndex {
    /// The editor schema of a company index, generated from the type the
    /// index is read with: the empty document, or a mapping of `expanded`
    /// and `listed`, each a list of `.md` paths, and no other key.
    pub(super) fn editor_schema() -> serde_json::Value {
        schemars::schema_for!(Option<CompanyIndex>).to_value()
    }
}

impl<'de> Deserialize<'de> for CompanyEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let path = String::deserialize(deserializer)?;
        if path.ends_with(".md") {
            Ok(Self(path))
        } else {
            Err(serde::de::Error::custom(format!(
                "`{path}` does not end in `.md`"
            )))
        }
    }
}

/// What is wrong with a company index as a whole, or with one of its
/// entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompanyIndexProblem {
    /// It is not YAML, with why.
    NotYaml(String),
    /// It holds a document that is neither a mapping of keys nor empty,
    /// such as a list or a text.
    NotAMapping,
    /// It holds a key other than `expanded` and `listed`.
    KeyNotAllowed(String),
    /// The value of `key` is not a list of document paths.
    NotAList { key: CompanyIndexKey },
    /// An entry of `key` names no document the home holds.
    Entry {
        key: CompanyIndexKey,
        entry: String,
        problem: CompanyReadProblem,
    },
}

impl std::fmt::Display for CompanyIndexProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotYaml(reason) => write!(f, "is not YAML ({reason})"),
            Self::NotAMapping => write!(
                f,
                "is neither a mapping of `{}` and `{}` nor empty",
                CompanyIndexKey::Expanded,
                CompanyIndexKey::Listed
            ),
            Self::KeyNotAllowed(key) => write!(
                f,
                "holds `{key}`, which is not allowed; a company index holds only `{}` and `{}`",
                CompanyIndexKey::Expanded,
                CompanyIndexKey::Listed
            ),
            Self::NotAList { key } => write!(
                f,
                "holds a `{key}` that is not a list of document paths relative to `docs/`"
            ),
            Self::Entry {
                key,
                entry,
                problem,
            } => write!(f, "names `{entry}` in `{key}`, {}", problem.sentence()),
        }
    }
}

/// Reads `text` as a company index. Every shape problem of the text is
/// found in one pass, key by key; an index without one is read as its type.
/// A text holding no document, an explicit `null` and an empty mapping each
/// select nothing.
fn read_index(text: &str) -> Result<CompanyIndex, Vec<CompanyIndexProblem>> {
    let value = serde_yaml_ng::from_str::<Value>(text)
        .map_err(|err| vec![CompanyIndexProblem::NotYaml(err.to_string())])?;
    let mapping = match value {
        Value::Mapping(mapping) => mapping,
        Value::Null => return Ok(CompanyIndex::default()),
        _ => return Err(vec![CompanyIndexProblem::NotAMapping]),
    };
    let mut problems = Vec::new();
    for (key, value) in &mapping {
        let Some(key) = key.as_str().and_then(CompanyIndexKey::named) else {
            let shown = match key {
                Value::String(text) => text.clone(),
                other => serde_yaml_ng::to_string(other)
                    .map_or_else(|_| format!("{other:?}"), |text| text.trim().to_owned()),
            };
            problems.push(CompanyIndexProblem::KeyNotAllowed(shown));
            continue;
        };
        let not_a_list = || CompanyIndexProblem::NotAList { key };
        let Value::Sequence(entries) = value else {
            problems.push(not_a_list());
            continue;
        };
        for entry in entries {
            let Some(entry) = entry.as_str() else {
                problems.push(not_a_list());
                continue;
            };
            if !entry.ends_with(".md") {
                problems.push(CompanyIndexProblem::Entry {
                    key,
                    entry: entry.to_owned(),
                    problem: not_markdown(entry),
                });
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    serde_yaml_ng::from_value(Value::Mapping(mapping))
        .map_err(|err| vec![CompanyIndexProblem::NotYaml(err.to_string())])
}

/// What is wrong with a read or a reference of a company document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanyReadProblem {
    /// It names a snippet.
    Snippet,
    /// It names a domain.
    Domain,
    /// It names a file that does not end in `.md`, which is no company
    /// document.
    NotMarkdown,
    /// It names a file outside the home's `docs/` folder.
    OutsideDocs,
    /// It names a file the home does not hold.
    Missing,
    /// It names a symbolic link, or a file in a linked folder, which is
    /// never read.
    Link,
}

impl CompanyReadProblem {
    /// What the operator reads after the read it names.
    pub(super) fn sentence(self) -> &'static str {
        match self {
            Self::Snippet => {
                "a snippet; a company's knowledge stands alone, so name one of the company's own documents in `docs/` instead"
            }
            Self::Domain => {
                "a domain; a company's knowledge stands alone, so name one of the company's own documents in `docs/` instead"
            }
            Self::NotMarkdown => {
                "which does not end in `.md` and so names no company document; name a Markdown document in `docs/` instead"
            }
            Self::OutsideDocs => {
                "outside the home's `docs/` folder; a company's documents are read only from there"
            }
            Self::Missing => "which the home does not hold; add the document or correct the path",
            Self::Link => {
                "a symbolic link or a file in a linked folder, which is never read; replace the link with what it points to"
            }
        }
    }
}

/// Renders the company home in `tree` into `plan`: its root `AGENTS.md`
/// from `docs/index.yaml`, or the findings that leave that file as it is.
/// Without the index there is nothing to render; with a linked index,
/// nothing is read through it.
pub(super) fn render(tree: &Committed, override_beside_root: bool, plan: &mut RenderPlan) {
    let path = PathBuf::from(AGENTS_FILE);
    let found = |kind| RenderFinding {
        path: path.clone(),
        kind,
    };
    let text = match tree.entry_index(COMPANY_INDEX) {
        EntryIndex::Absent => return,
        EntryIndex::Linked => {
            plan.findings.push(found(RenderFindingKind::LinkedIndex {
                index: COMPANY_INDEX.to_owned(),
            }));
            return;
        }
        EntryIndex::Present(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    };
    if override_beside_root {
        plan.findings.push(RenderFinding {
            path: PathBuf::from(OVERRIDE_FILE),
            kind: RenderFindingKind::OverrideBesideRoot,
        });
    }
    let index = match read_index(&text) {
        Ok(index) => index,
        Err(problems) => {
            plan.findings.extend(problems.into_iter().map(|problem| {
                found(RenderFindingKind::CompanyIndex {
                    index: COMPANY_INDEX.to_owned(),
                    problem,
                })
            }));
            return;
        }
    };
    let mut walk = Walk {
        tree,
        path: &path,
        visiting: Vec::new(),
        done: BTreeSet::new(),
        carrier: BTreeMap::new(),
        order: Vec::new(),
        findings: Vec::new(),
    };
    for entry in &index.expanded {
        let Some(document) = walk.entry(CompanyIndexKey::Expanded, entry) else {
            continue;
        };
        walk.bring(&document, Traversal::Held { carrier: &document });
    }
    let mut lines = Vec::new();
    for entry in &index.listed {
        let Some(document) = walk.entry(CompanyIndexKey::Listed, entry) else {
            continue;
        };
        if let Some(carrier) = walk.carrier.get(&document) {
            let carrier = carrier.clone();
            walk.found(RenderFindingKind::CompanyListedCarried { document, carrier });
            continue;
        }
        let text = String::from_utf8_lossy(tree.file(&document).unwrap_or_default());
        match docs::company_document(&text).frontmatter {
            CompanyFrontmatter::Read {
                description: Some(description),
            } => lines.push(listed_line(&document, &description)),
            CompanyFrontmatter::Read { description: None } => {
                walk.found(RenderFindingKind::CompanyListedWithoutDescription {
                    document: document.clone(),
                });
            }
            CompanyFrontmatter::Invalid(reason) => {
                walk.found(RenderFindingKind::CompanyFrontmatterInvalid {
                    document: document.clone(),
                    reason,
                });
            }
        }
        walk.bring(&document, Traversal::Checked);
    }
    if !walk.findings.is_empty() {
        plan.findings.extend(walk.findings);
        return;
    }
    // The passage every company rendering carries between the generated line
    // and the first document: one fixed text, the same for every company and
    // taking no input, naming the home and no repository host or organization.
    // `<slug>` is literal placeholder text, never substituted.
    let passage = concat!(
        "This is the company's knowledge, rendered from the company's repository: its `docs/index.yaml` and the Markdown documents under `docs/` that index selects. A document named below by one line, its path and what it holds, is read at that path when the work calls for it. The checkout of that repository at `~/.instrukt/companies/<slug>/`, where `<slug>` is the company's key, is the company's home: it serves the company's agents and stays on the repository's default branch.",
        "\n\n",
        "An agent with access to the repository improves this knowledge by editing the Markdown documents (`*.md`) under `docs/`, subfolders included, and the index that selects them. It edits on a branch, in a separate worktree of the company's home or in a clone of the repository (by convention named `company-<slug>`), and opens a pull request. Someone other than the pull request's author reviews it before it merges to the default branch, and through that branch the change reaches the company's agents. The change is made in those documents and that index alone: `AGENTS.md` and any skill published from them are generated output and are left as rendered.",
        "\n\n",
        "Each of those documents carries a non-empty `description` in its frontmatter and states what is true of the company now, in the present tense; what was true before stays in the repository's history.",
    );
    let mut blocks = vec![passage.to_owned()];
    blocks.extend(walk.order.into_iter().map(|body| body.trim().to_owned()));
    if !lines.is_empty() {
        blocks.push(lines.join("\n"));
    }
    let text = finish(&blocks.join("\n\n"));
    plan.writes.push(PlannedFile {
        current: tree.file(AGENTS_FILE).map(<[u8]>::to_vec),
        path,
        text: format!("{GENERATED_LINE}\n{text}"),
    });
}

/// The one line naming a listed document: its path from the repository's
/// root and its description, each run of whitespace read as one space.
fn listed_line(document: &str, description: &str) -> String {
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("- {document}: {description}")
}

/// How a walk brings documents in.
#[derive(Debug, Clone, Copy)]
enum Traversal<'a> {
    /// Under the `expanded` entry whose document is `carrier`: each document
    /// is held in full and recorded as carried by it.
    Held { carrier: &'a str },
    /// Under a `listed` entry: each document is checked and nothing is held.
    Checked,
}

/// The documents a company rendering holds in full, in order, and what it
/// found on the way.
struct Walk<'t> {
    tree: &'t Committed,
    /// The rendering the findings leave as it is.
    path: &'t Path,
    /// The documents being brought in, each requiring the next.
    visiting: Vec<String>,
    /// The documents already brought in.
    done: BTreeSet<String>,
    /// Each document an `expanded` entry brought in, with that entry's
    /// document.
    carrier: BTreeMap<String, String>,
    /// Each document held in full: its text without its required reads, in
    /// traversal order.
    order: Vec<String>,
    findings: Vec<RenderFinding>,
}

impl Walk<'_> {
    /// The home's document the index entry `entry` of `key` names, or the
    /// finding that it names none.
    fn entry(&mut self, key: CompanyIndexKey, entry: &CompanyEntry) -> Option<String> {
        match self.file(COMPANY_INDEX, entry.as_str()) {
            Ok(path) => Some(path),
            Err(problem) => {
                self.found(RenderFindingKind::CompanyIndex {
                    index: COMPANY_INDEX.to_owned(),
                    problem: CompanyIndexProblem::Entry {
                        key,
                        entry: entry.as_str().to_owned(),
                        problem,
                    },
                });
                None
            }
        }
    }

    /// Brings in the document at `path` after its own required reads, once,
    /// each checked and, as `traversal` says, held in full.
    fn bring(&mut self, path: &str, traversal: Traversal<'_>) {
        if let Some(at) = self.visiting.iter().position(|open| open == path) {
            let mut documents = self.visiting[at..].to_vec();
            documents.push(path.to_owned());
            self.found(RenderFindingKind::CompanyCycle { documents });
            return;
        }
        if self.done.contains(path) {
            return;
        }
        let text = String::from_utf8_lossy(self.tree.file(path).unwrap_or_default()).into_owned();
        let document = docs::company_document(&text);
        self.check(path, &text, &document);
        self.visiting.push(path.to_owned());
        for read in &document.reads {
            match self.document(path, read) {
                Ok(read) => self.bring(&read, traversal),
                Err(problem) => self.found(RenderFindingKind::CompanyRead {
                    document: path.to_owned(),
                    read: read.clone(),
                    problem,
                }),
            }
        }
        self.visiting.pop();
        self.done.insert(path.to_owned());
        if let Traversal::Held { carrier } = traversal {
            self.carrier.insert(path.to_owned(), carrier.to_owned());
            self.order.push(document.body);
        }
    }

    /// The findings of the document at `path` itself: its directives and
    /// the references in its text.
    fn check(&mut self, path: &str, text: &str, document: &CompanyDocument) {
        if document
            .directives
            .iter()
            .any(|directive| matches!(directive, Directive::Config { .. }))
        {
            self.found(RenderFindingKind::CompanyConfig {
                document: path.to_owned(),
            });
        }
        for finding in docs::unsupported_directives(Path::new(path), text) {
            self.found(RenderFindingKind::UnsupportedDirective { finding });
        }
        for reference in &document.references {
            let (written, problem) = match reference {
                CompanyReference::Name(name) => (name, named(name)),
                CompanyReference::Path(file) => (file, self.file(path, file).err()),
            };
            if let Some(problem) = problem {
                self.found(RenderFindingKind::CompanyReference {
                    document: path.to_owned(),
                    reference: written.clone(),
                    problem,
                });
            }
        }
    }

    /// The home's document that `read`, written in the document at
    /// `reader`, names: its repository-relative path, or why it names none.
    fn document(&self, reader: &str, read: &str) -> Result<String, CompanyReadProblem> {
        if !read.ends_with(".md") {
            return Err(not_markdown(read));
        }
        self.file(reader, read)
    }

    /// The home's document that the file reference `file`, written in the
    /// document at `reader`, names: its repository-relative path, or why it
    /// names none.
    fn file(&self, reader: &str, file: &str) -> Result<String, CompanyReadProblem> {
        let file = file.strip_prefix('@').unwrap_or(file);
        let path = relative_to(reader, file).ok_or(CompanyReadProblem::OutsideDocs)?;
        if !path.starts_with(DOCS) {
            return Err(CompanyReadProblem::OutsideDocs);
        }
        if self.tree.is_link(&path) {
            return Err(CompanyReadProblem::Link);
        }
        if self.tree.file(&path).is_none() {
            return Err(CompanyReadProblem::Missing);
        }
        Ok(path)
    }

    fn found(&mut self, kind: RenderFindingKind) {
        self.findings.push(RenderFinding {
            path: self.path.to_path_buf(),
            kind,
        });
    }
}

/// What the name `name` (its `@` aside) references outside the home: a
/// snippet or a domain; `None` when it names neither.
fn named(name: &str) -> Option<CompanyReadProblem> {
    let name = name.strip_prefix('@').unwrap_or(name);
    if SnippetId::parse(name).is_ok() {
        Some(CompanyReadProblem::Snippet)
    } else if DomainName::parse(name).is_ok() {
        Some(CompanyReadProblem::Domain)
    } else {
        None
    }
}

/// Why `name`, an index entry or a required read that does not end in
/// `.md`, names no company document: a snippet or a domain it names, or a
/// file that is no Markdown document. A name holding a `.` is a file's.
fn not_markdown(name: &str) -> CompanyReadProblem {
    match named(name) {
        Some(CompanyReadProblem::Domain) if name.contains('.') => CompanyReadProblem::NotMarkdown,
        Some(problem) => problem,
        None => CompanyReadProblem::NotMarkdown,
    }
}

/// `read`, a path relative to the file at `reader`, as a repository-relative
/// path; `None` for an absolute path, a home-relative one or one climbing
/// out of the repository.
fn relative_to(reader: &str, read: &str) -> Option<String> {
    if read.starts_with('/') || read.starts_with('~') || read.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = reader.split('/').collect();
    parts.pop();
    for part in read.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_resolves_relative_to_its_reader() {
        assert_eq!(
            relative_to("docs/index.yaml", "people.md").as_deref(),
            Some("docs/people.md")
        );
        assert_eq!(
            relative_to("docs/people.md", "roles.md").as_deref(),
            Some("docs/roles.md")
        );
        assert_eq!(
            relative_to("docs/teams/a.md", "../roles.md").as_deref(),
            Some("docs/roles.md")
        );
        assert_eq!(relative_to("docs/people.md", "../../x.md"), None);
        assert_eq!(relative_to("docs/people.md", "/etc/x.md"), None);
    }

    fn entries(entries: &[CompanyEntry]) -> Vec<&str> {
        entries.iter().map(CompanyEntry::as_str).collect()
    }

    #[test]
    fn the_index_reads_its_two_keys_and_names_every_other() {
        let index =
            read_index("expanded: [company.md, teams/people.md]\nlisted: [systems.md]\n").unwrap();
        assert_eq!(entries(&index.expanded), ["company.md", "teams/people.md"]);
        assert_eq!(entries(&index.listed), ["systems.md"]);
        let problems = read_index(
            "expanded: [company.md]\ndescription: Acme\npurpose: x\nlisted: systems.md\n",
        )
        .unwrap_err();
        assert_eq!(
            problems,
            [
                CompanyIndexProblem::KeyNotAllowed("description".to_owned()),
                CompanyIndexProblem::KeyNotAllowed("purpose".to_owned()),
                CompanyIndexProblem::NotAList {
                    key: CompanyIndexKey::Listed
                },
            ]
        );
    }

    #[test]
    fn an_index_holding_no_document_null_or_an_empty_mapping_selects_nothing() {
        for text in ["", "# The company's index.\n", "null\n", "~\n", "{}\n"] {
            let index = read_index(text).unwrap();
            assert!(
                index.expanded.is_empty() && index.listed.is_empty(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_list_or_a_text_is_no_index() {
        for text in ["- company.md\n", "company.md\n"] {
            assert_eq!(
                read_index(text).unwrap_err(),
                [CompanyIndexProblem::NotAMapping],
                "{text:?}"
            );
        }
    }

    #[test]
    fn an_entry_that_does_not_end_in_md_names_no_document() {
        let problems =
            read_index("expanded: [index.yaml, general/policy/commits, sdlc, 3]\n").unwrap_err();
        let entry = |entry: &str, problem| CompanyIndexProblem::Entry {
            key: CompanyIndexKey::Expanded,
            entry: entry.to_owned(),
            problem,
        };
        assert_eq!(
            problems,
            [
                entry("index.yaml", CompanyReadProblem::NotMarkdown),
                entry("general/policy/commits", CompanyReadProblem::Snippet),
                entry("sdlc", CompanyReadProblem::Domain),
                CompanyIndexProblem::NotAList {
                    key: CompanyIndexKey::Expanded
                },
            ]
        );
    }

    #[test]
    fn a_listed_line_reads_any_description_as_one_line() {
        let one = "Who works at Acme.";
        let folded = "Who works at Acme,\nand since when.";
        let literal = "Who works at Acme,\n  and\tsince when.\n";
        insta::assert_snapshot!("listed_line_one", listed_line("docs/people.md", one));
        let across = listed_line("docs/people.md", folded);
        insta::assert_snapshot!("listed_line_across_lines", across);
        // Any whitespace run reads as one space: the literal form is the same
        // line as the folded one.
        assert_eq!(listed_line("docs/people.md", literal), across);
    }

    #[test]
    fn a_listed_document_the_expanded_ones_already_bring_is_a_finding_naming_them() {
        let tree = Committed::of(&[
            (
                COMPANY_INDEX,
                "expanded: [company.md]\nlisted: [people.md]\n",
            ),
            (
                "docs/company.md",
                "# Company\n\n## Required reads\n\n- people.md\n",
            ),
            (
                "docs/people.md",
                "---\ndescription: Who.\n---\n\n# People\n",
            ),
        ]);
        let mut plan = RenderPlan::default();
        render(&tree, false, &mut plan);
        assert!(plan.writes.is_empty());
        assert_eq!(
            plan.findings,
            [RenderFinding {
                path: PathBuf::from(AGENTS_FILE),
                kind: RenderFindingKind::CompanyListedCarried {
                    document: "docs/people.md".to_owned(),
                    carrier: "docs/company.md".to_owned(),
                },
            }]
        );
    }

    #[test]
    fn the_company_index_schema_admits_its_two_keys_of_md_paths_and_the_empty_document() {
        let schema = CompanyIndex::editor_schema();
        let variants = schema["anyOf"].as_array().unwrap();
        assert!(variants.contains(&serde_json::json!({"type": "null"})));
        let index = &schema["$defs"]["CompanyIndex"];
        let mut keys: Vec<&String> = index["properties"].as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, CompanyIndexKey::ALL.map(CompanyIndexKey::as_str));
        assert_eq!(index["additionalProperties"], false);
        for key in CompanyIndexKey::ALL {
            assert_eq!(index["properties"][key.as_str()]["type"], "array");
        }
        assert_eq!(
            schema["$defs"]["CompanyEntry"]["pattern"],
            CompanyEntry::PATTERN
        );
    }
}
