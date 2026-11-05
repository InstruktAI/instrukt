//! Snippet shapes (`project/instrukt/spec/feature/docs/snippet-schema`):
//! the root rules built into the binary, the sections a position's
//! `schema.yaml` adds per folder, and the effective schema a snippet is
//! held to.
//!
//! The effective schema of a folder is the root rules for a type, then
//! every key of that one position's `schema.yaml` whose path is the folder
//! or an ancestor of it, shortest first. Nothing crosses positions or
//! repositories, so a folder's schema is the same on every computer.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use super::_finding::{Finding, FindingCode};
use super::_identity::Taxonomy;
use super::_position::{GENERAL, Position, PositionSchema, SCHEMA_FILE, is_reserved_domain_folder};
use super::_text::normalize_title;
use super::DocsError;
use crate::ids::DomainName;

/// The optional first H2 every snippet may carry.
pub const REQUIRED_READS: &str = "Required reads";

/// The optional last H2 every snippet may carry.
pub const SEE_ALSO: &str = "See Also";

/// The sections the root rules require of `taxonomy`, in order.
fn root_required(taxonomy: Taxonomy) -> &'static [&'static str] {
    match taxonomy {
        Taxonomy::Principle => &["Principle", "Rationale", "Implications", "Tensions"],
        Taxonomy::Concept => &["What", "Why"],
        Taxonomy::Policy => &["Rules", "Rationale", "Scope", "Enforcement", "Exceptions"],
        Taxonomy::Procedure => &["Goal", "Preconditions", "Steps", "Outputs", "Recovery"],
        Taxonomy::Design => &[
            "Purpose",
            "Inputs/Outputs",
            "Invariants",
            "Primary flows",
            "Failure modes",
        ],
        Taxonomy::Spec => &["What it is", "Canonical fields"],
        Taxonomy::Adr => &["Context", "Decision", "Consequences"],
    }
}

/// The sections the root rules allow `taxonomy` besides its required ones
/// and the two every snippet may carry.
fn root_optional(taxonomy: Taxonomy) -> &'static [&'static str] {
    match taxonomy {
        Taxonomy::Procedure => &["Discipline"],
        Taxonomy::Spec => &["Allowed values", "Known caveats"],
        Taxonomy::Adr => &["Alternatives Considered", "Status"],
        _ => &[],
    }
}

/// What imposes a section rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Imposer {
    /// The root rules built into the binary.
    Root,
    /// One path key of a position's `schema.yaml`.
    Key {
        /// The `schema.yaml`, relative to the repository root.
        file: String,
        key: String,
    },
}

impl fmt::Display for Imposer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Imposer::Root => f.write_str("instrukt root rules"),
            Imposer::Key { file, key } => write!(f, "{file} {key}"),
        }
    }
}

/// One H2 section of an effective schema and what imposes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaSection {
    pub title: String,
    pub imposed_by: Imposer,
}

/// The sections a snippet of one type in one folder is held to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSchema {
    pub taxonomy: Taxonomy,
    /// Required, in the order the steps impose them.
    pub required: Vec<SchemaSection>,
    /// Every other allowed section: `Required reads` first, `See Also`
    /// last.
    pub optional: Vec<SchemaSection>,
}

impl EffectiveSchema {
    /// The schema imposing a section titled `title`, if the schema allows
    /// it; titles match as headings do (`See Also` is `See also`).
    pub(super) fn allows(&self, title: &str) -> bool {
        let wanted = normalize_title(title);
        self.required
            .iter()
            .chain(&self.optional)
            .any(|s| normalize_title(&s.title) == wanted)
    }
}

/// A position's `schema.yaml` as read from the working tree.
#[derive(Debug, Clone)]
pub(super) struct LocatedSchema {
    /// Relative to the repository root.
    file: String,
    /// `None` when the file cannot be parsed.
    schema: Option<PositionSchema>,
    /// Why it cannot be parsed.
    error: Option<String>,
}

/// Every `schema.yaml` below a repository's `docs/`.
#[derive(Debug, Clone, Default)]
pub(super) struct Schemas {
    by_position: BTreeMap<Position, LocatedSchema>,
    /// `schema.yaml` files anywhere but at a position root.
    misplaced: Vec<String>,
}

impl Schemas {
    /// Reads every `schema.yaml` below `<repo_root>/docs/`, skipping
    /// directories whose name starts with `_`.
    pub(super) fn load(repo_root: &Path) -> Result<Self, DocsError> {
        let docs = super::_corpus::confined_docs(repo_root)?;
        let mut schemas = Schemas::default();
        if !docs.is_dir() {
            return Ok(schemas);
        }
        let walker = walkdir::WalkDir::new(&docs)
            .follow_links(false)
            .follow_root_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0
                    || entry.file_type().is_file()
                    || !entry.file_name().to_string_lossy().starts_with('_')
            });
        for entry in walker {
            let entry = entry.map_err(|err| DocsError::Walk {
                path: err.path().unwrap_or(&docs).to_path_buf(),
                message: err.to_string(),
            })?;
            if !entry.file_type().is_file() || entry.file_name() != SCHEMA_FILE {
                continue;
            }
            let rel = super::_corpus::relative(repo_root, entry.path());
            let text = std::fs::read_to_string(entry.path()).map_err(|source| DocsError::Read {
                path: entry.path().to_path_buf(),
                source,
            })?;
            schemas.add(&rel, &text);
        }
        Ok(schemas)
    }

    /// Records the `schema.yaml` at `rel` holding `text`.
    pub(super) fn add(&mut self, rel: &str, text: &str) {
        let Some(position) = rel
            .strip_suffix(&format!("/{SCHEMA_FILE}"))
            .and_then(position_at_root)
        else {
            self.misplaced.push(rel.to_owned());
            return;
        };
        let (schema, error) = match PositionSchema::parse(text) {
            Ok(schema) => (Some(schema), None),
            Err(err) => (None, Some(one_line(&err.to_string()))),
        };
        self.by_position.insert(
            position,
            LocatedSchema {
                file: rel.to_owned(),
                schema,
                error,
            },
        );
    }

    /// The effective schema of a snippet of `taxonomy` in `folder` (relative
    /// to the root of `position`).
    pub(super) fn effective(
        &self,
        position: &Position,
        taxonomy: Taxonomy,
        folder: &str,
    ) -> EffectiveSchema {
        effective(self.by_position.get(position), taxonomy, folder)
    }

    /// Every finding on the schemas themselves.
    pub(super) fn findings(&self) -> Vec<Finding> {
        let mut out: Vec<Finding> = self
            .misplaced
            .iter()
            .map(|file| Finding::new(file, FindingCode::SchemaMisplaced, None))
            .collect();
        for located in self.by_position.values() {
            out.extend(located_findings(located));
        }
        out
    }

    /// The findings on `position`'s own `schema.yaml`.
    fn findings_of(&self, position: &Position) -> Vec<Finding> {
        self.by_position
            .get(position)
            .map(located_findings)
            .unwrap_or_default()
    }
}

/// The position whose root is `dir` (relative to the repository root).
fn position_at_root(dir: &str) -> Option<Position> {
    match dir.split('/').collect::<Vec<_>>().as_slice() {
        ["docs", GENERAL] => Some(Position::General),
        ["docs", "project"] => Some(Position::Project),
        ["docs", "domain", name] if !is_reserved_domain_folder(name) => {
            DomainName::parse(name).ok().map(Position::Domain)
        }
        _ => None,
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `key` is a valid path key: whole segments, the first a
/// taxonomy directory.
fn key_taxonomy(key: &str) -> Option<Taxonomy> {
    let segments: Vec<&str> = key.split('/').collect();
    if segments
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == "..")
    {
        return None;
    }
    Taxonomy::from_dir(segments[0])
}

/// Whether the key `key` applies to `folder`: the same path or an
/// ancestor of it.
fn applies(key: &str, folder: &str) -> bool {
    folder == key
        || folder
            .strip_prefix(key)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The valid keys of `schema` that apply to `folder`, shortest path first.
fn chain<'s>(
    schema: &'s PositionSchema,
    folder: &str,
) -> Vec<(&'s String, &'s super::FolderSections)> {
    let mut keys: Vec<_> = schema
        .folders
        .iter()
        .filter(|(key, _)| key_taxonomy(key).is_some() && applies(key, folder))
        .collect();
    keys.sort_by_key(|(key, _)| key.split('/').count());
    keys
}

fn effective(located: Option<&LocatedSchema>, taxonomy: Taxonomy, folder: &str) -> EffectiveSchema {
    let mut required: Vec<SchemaSection> = root_required(taxonomy)
        .iter()
        .map(|title| section(title, Imposer::Root))
        .collect();
    let mut optional: Vec<SchemaSection> = std::iter::once(&REQUIRED_READS)
        .chain(root_optional(taxonomy))
        .map(|title| section(title, Imposer::Root))
        .collect();
    let (file, keys) = match located {
        Some(LocatedSchema {
            file,
            schema: Some(schema),
            ..
        }) => (file.as_str(), chain(schema, folder)),
        _ => ("", Vec::new()),
    };
    for (key, sections) in &keys {
        let imposer = Imposer::Key {
            file: file.to_owned(),
            key: (*key).clone(),
        };
        for title in &sections.required {
            if !contains(&required, title) {
                required.push(section(title, imposer.clone()));
            }
        }
        for title in &sections.optional {
            if !contains(&optional, title) {
                optional.push(section(title, imposer.clone()));
            }
        }
    }
    optional.retain(|s| !contains(&required, &s.title));
    if !contains(&required, SEE_ALSO) && !contains(&optional, SEE_ALSO) {
        optional.push(section(SEE_ALSO, Imposer::Root));
    }
    EffectiveSchema {
        taxonomy,
        required,
        optional,
    }
}

fn section(title: &str, imposed_by: Imposer) -> SchemaSection {
    SchemaSection {
        title: title.to_owned(),
        imposed_by,
    }
}

fn contains(sections: &[SchemaSection], title: &str) -> bool {
    let wanted = normalize_title(title);
    sections.iter().any(|s| normalize_title(&s.title) == wanted)
}

/// The findings on one `schema.yaml`: unparseable, a malformed key, a
/// section a key both requires and makes optional, a section a key drops.
fn located_findings(located: &LocatedSchema) -> Vec<Finding> {
    let file = &located.file;
    let Some(schema) = &located.schema else {
        return vec![Finding::new(
            file,
            FindingCode::SchemaInvalid,
            located.error.clone(),
        )];
    };
    let mut out = Vec::new();
    for (key, sections) in &schema.folders {
        let Some(taxonomy) = key_taxonomy(key) else {
            out.push(Finding::new(
                file,
                FindingCode::SchemaKeyInvalid,
                Some(key.clone()),
            ));
            continue;
        };
        for title in &sections.optional {
            let both = sections
                .required
                .iter()
                .any(|r| normalize_title(r) == normalize_title(title));
            if both {
                out.push(Finding::new(
                    file,
                    FindingCode::SchemaSectionConflict,
                    Some(format!("{key}: {title}")),
                ));
                continue;
            }
            // What the root rules and every shorter path require here.
            let parent = key.rsplit_once('/').map(|(p, _)| p).unwrap_or_default();
            let inherited = if parent.is_empty() {
                effective(None, taxonomy, key)
            } else {
                effective(Some(located), taxonomy, parent)
            };
            if let Some(required) = inherited
                .required
                .iter()
                .find(|r| normalize_title(&r.title) == normalize_title(title))
            {
                out.push(Finding::new(
                    file,
                    FindingCode::SchemaSectionDropped,
                    Some(format!(
                        "{key}: optional {title}, required by {}",
                        required.imposed_by
                    )),
                ));
            }
        }
    }
    out
}

/// The effective schema of a folder, as `instrukt docs schema <folder>`
/// prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSchema {
    /// The folder, relative to the repository root.
    pub folder: String,
    /// One per type the folder can hold, in listing order.
    pub types: Vec<EffectiveSchema>,
    /// Findings on the position's `schema.yaml`; while any remains the
    /// schema is not to be trusted.
    pub findings: Vec<Finding>,
}

/// The effective schema of `folder` (absolute, or relative to `cwd`), read
/// from `repo_root`'s working tree. The folder need not exist; it must lie
/// inside a position and, below a position's root, inside a taxonomy
/// directory. Errors name `folder` as given.
pub fn schema(repo_root: &Path, cwd: &Path, folder: &Path) -> Result<FolderSchema, DocsError> {
    let root = repo_root.canonicalize().map_err(|source| DocsError::Read {
        path: repo_root.to_path_buf(),
        source,
    })?;
    let absolute = cwd.join(folder);
    let resolved = resolve(&absolute)?;
    let Ok(rel) = resolved.strip_prefix(&root) else {
        return Err(DocsError::OutsideRepository {
            path: folder.to_path_buf(),
        });
    };
    let rel = super::_corpus::relative(Path::new(""), rel);
    let (position, inner) = split_position(&rel).ok_or_else(|| DocsError::OutsidePositions {
        path: folder.to_path_buf(),
    })?;
    let types: Vec<Taxonomy> = match inner.split('/').next().filter(|s| !s.is_empty()) {
        None => Taxonomy::ALL.to_vec(),
        Some(dir) => {
            vec![
                Taxonomy::from_dir(dir).ok_or_else(|| DocsError::NotATaxonomyFolder {
                    path: folder.to_path_buf(),
                    dir: dir.to_owned(),
                })?,
            ]
        }
    };
    let mut schemas = Schemas::default();
    let file = format!("{}/{SCHEMA_FILE}", position.dir());
    match std::fs::read_to_string(root.join(&file)) {
        Ok(text) => schemas.add(&file, &text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(DocsError::Read {
                path: root.join(&file),
                source,
            });
        }
    }
    let types = types
        .into_iter()
        .map(|taxonomy| schemas.effective(&position, taxonomy, &inner))
        .collect();
    Ok(FolderSchema {
        folder: rel,
        types,
        findings: schemas.findings_of(&position),
    })
}

/// `path` with `.` and `..` resolved and its longest existing ancestor
/// canonicalized, so a folder that does not exist yet still resolves
/// through symbolic links above it.
fn resolve(path: &Path) -> Result<PathBuf, DocsError> {
    let mut lexical = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other.as_os_str()),
        }
    }
    let mut existing = lexical.clone();
    let mut rest = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => rest.push(name.to_owned()),
            None => break,
        }
        existing.pop();
    }
    let mut out = existing.canonicalize().map_err(|source| DocsError::Read {
        path: existing.clone(),
        source,
    })?;
    out.extend(rest.iter().rev());
    Ok(out)
}

/// The position a repository-relative path lies in and the path relative
/// to that position's root (empty at the root).
fn split_position(rel: &str) -> Option<(Position, String)> {
    let parts: Vec<&str> = rel.split('/').collect();
    let (position, depth) = match parts.as_slice() {
        ["docs", GENERAL, ..] => (Position::General, 2),
        ["docs", "project", ..] => (Position::Project, 2),
        ["docs", "domain", name, ..] if !is_reserved_domain_folder(name) => {
            (Position::Domain(DomainName::parse(name).ok()?), 3)
        }
        _ => return None,
    };
    Some((position, parts[depth..].join("/")))
}

/// The position and in-position folder of a snippet file at `rel`
/// (repository-relative).
pub(super) fn folder_of(rel: &Path) -> Option<(Position, String)> {
    let parent = rel.parent()?;
    split_position(&super::_corpus::relative(Path::new(""), parent))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schemas(text: &str) -> Schemas {
        let mut schemas = Schemas::default();
        schemas.add("docs/project/schema.yaml", text);
        schemas
    }

    fn titles(sections: &[SchemaSection]) -> Vec<&str> {
        sections.iter().map(|s| s.title.as_str()).collect()
    }

    fn codes(findings: &[Finding]) -> Vec<FindingCode> {
        findings.iter().map(|f| f.code).collect()
    }

    const FEATURE: &str = "spec:\n  optional: [Notes]\nspec/feature:\n  required: [Use cases]\n";

    #[test]
    fn without_a_schema_a_folder_follows_the_root_rules() {
        let effective = Schemas::default().effective(&Position::Project, Taxonomy::Spec, "spec/x");
        assert_eq!(
            titles(&effective.required),
            ["What it is", "Canonical fields"]
        );
        assert_eq!(
            titles(&effective.optional),
            [
                "Required reads",
                "Allowed values",
                "Known caveats",
                "See Also"
            ]
        );
        assert!(
            effective
                .required
                .iter()
                .all(|s| s.imposed_by == Imposer::Root)
        );
    }

    #[test]
    fn keys_add_their_sections_after_the_root_rules_shortest_path_first() {
        let effective =
            schemas(FEATURE).effective(&Position::Project, Taxonomy::Spec, "spec/feature/docs");
        assert_eq!(
            titles(&effective.required),
            ["What it is", "Canonical fields", "Use cases"]
        );
        assert_eq!(
            effective.required[2].imposed_by,
            Imposer::Key {
                file: "docs/project/schema.yaml".into(),
                key: "spec/feature".into()
            }
        );
        assert!(titles(&effective.optional).contains(&"Notes"));
    }

    #[test]
    fn a_key_holds_only_the_snippets_under_its_path() {
        let schemas = schemas(FEATURE);
        for folder in ["spec/domain", "spec/features"] {
            let effective = schemas.effective(&Position::Project, Taxonomy::Spec, folder);
            assert!(!effective.allows("Use cases"), "{folder}");
        }
    }

    #[test]
    fn a_schema_never_reaches_another_position() {
        let effective =
            schemas(FEATURE).effective(&Position::General, Taxonomy::Spec, "spec/feature");
        assert!(!effective.allows("Use cases"));
    }

    #[test]
    fn a_key_making_optional_what_the_root_rules_require_is_a_finding() {
        let findings = schemas("spec/feature:\n  optional: [What it is]\n").findings();
        assert_eq!(codes(&findings), [FindingCode::SchemaSectionDropped]);
    }

    #[test]
    fn a_key_making_optional_what_a_shorter_path_requires_is_a_finding() {
        let findings = schemas(
            "spec/feature:\n  required: [Use cases]\nspec/feature/docs:\n  optional: [Use cases]\n",
        )
        .findings();
        assert_eq!(codes(&findings), [FindingCode::SchemaSectionDropped]);
    }

    #[test]
    fn a_dropped_section_stays_required() {
        let effective = schemas("spec/feature:\n  optional: [What it is]\n").effective(
            &Position::Project,
            Taxonomy::Spec,
            "spec/feature",
        );
        assert!(titles(&effective.required).contains(&"What it is"));
        assert!(!titles(&effective.optional).contains(&"What it is"));
    }

    #[test]
    fn a_section_both_required_and_optional_in_one_key_is_a_finding() {
        let findings =
            schemas("spec/feature:\n  required: [Notes]\n  optional: [Notes]\n").findings();
        assert_eq!(codes(&findings), [FindingCode::SchemaSectionConflict]);
    }

    #[test]
    fn a_key_outside_the_taxonomy_or_not_in_whole_segments_is_a_finding() {
        for key in ["feature", "spec/", "spec//feature", "/spec", "spec/../x"] {
            let findings = schemas(&format!("'{key}':\n  required: [X]\n")).findings();
            assert_eq!(codes(&findings), [FindingCode::SchemaKeyInvalid], "{key}");
        }
    }

    #[test]
    fn an_unparseable_schema_is_a_finding_and_adds_nothing() {
        let schemas = schemas("spec/feature:\n  dropped: [What it is]\n");
        assert_eq!(codes(&schemas.findings()), [FindingCode::SchemaInvalid]);
        let effective = schemas.effective(&Position::Project, Taxonomy::Spec, "spec/feature");
        assert_eq!(
            titles(&effective.required),
            ["What it is", "Canonical fields"]
        );
    }

    #[test]
    fn a_schema_away_from_a_position_root_is_a_finding() {
        let mut schemas = Schemas::default();
        for rel in [
            "docs/project/spec/schema.yaml",
            "docs/schema.yaml",
            "docs/domain/schema.yaml",
        ] {
            schemas.add(rel, "");
        }
        assert_eq!(
            codes(&schemas.findings()),
            [FindingCode::SchemaMisplaced; 3]
        );
    }

    #[test]
    fn a_snippet_folder_is_relative_to_its_position_root() {
        assert_eq!(
            folder_of(Path::new("docs/domain/sdlc/procedure/lifecycle/x.md")),
            Some((
                Position::Domain(DomainName::parse("sdlc").unwrap()),
                "procedure/lifecycle".to_owned()
            ))
        );
        assert_eq!(
            folder_of(Path::new("docs/project/spec/x.md")),
            Some((Position::Project, "spec".to_owned()))
        );
    }
}
