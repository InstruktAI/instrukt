//! Validation of the position indexes
//! (`project/instrukt/spec/feature/docs/position-index`, Findings and
//! Notices).
//!
//! Every `index.yaml` below the repository's `docs/` is read as YAML and
//! checked key by key, so one run reports every finding of every index
//! rather than the first parse error: the keys its position allows, the
//! shape of each value, the ids it names (delivered, of its own position
//! unless it is `general`'s), the agent CLIs `agents` maps, and the listed
//! ids the expanded ones already bring in. An `index.yaml` anywhere but a
//! position's root is a finding; a domain without one is a notice.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_yaml_ng::Value;

use super::_corpus::{Corpus, relative};
use super::_finding::{Finding, FindingCode, Notice, NoticeCode};
use super::_position::{AgentCli, GENERAL, INDEX_FILE, Position, PositionIndex};
use super::DocsError;
use crate::ids::{DomainName, SnippetId};
use crate::index::GlobalIndex;

/// The supported agent CLIs, as `agents` names them.
const AGENT_CLIS: [(&str, AgentCli); 4] = [
    ("claude", AgentCli::Claude),
    ("codex", AgentCli::Codex),
    ("gemini", AgentCli::Gemini),
    ("deepseek", AgentCli::Deepseek),
];

/// What one position's `index.yaml` may hold.
fn allowed_keys(position: &Position) -> &'static [&'static str] {
    match position {
        Position::General => &["expanded", "listed", "agents"],
        Position::Domain(_) => &["description", "expanded", "listed"],
        Position::Project => &["expanded", "listed"],
    }
}

/// The position whose root `rel` (`docs/…/index.yaml`) is the index of;
/// `None` when it sits anywhere else.
fn position_of(rel: &str) -> Option<Position> {
    let dir = rel.strip_suffix(&format!("/{INDEX_FILE}"))?;
    match dir.strip_prefix("docs/")? {
        GENERAL => Some(Position::General),
        "project" => Some(Position::Project),
        rest => {
            let name = rest.strip_prefix("domain/")?;
            (!name.contains('/') && !super::is_reserved_domain_folder(name))
                .then(|| DomainName::parse(name).ok().map(Position::Domain))
                .flatten()
        }
    }
}

/// Whether the repository at `repo_root` holds the position `domain`
/// addresses.
pub(super) fn holds(repo_root: &Path, domain: &DomainName) -> bool {
    repo_root.join(Position::of_domain(domain).dir()).is_dir()
}

/// The `index.yaml` of the published position `domain` addresses, as
/// validation sees it: the repository's own when it holds the position,
/// the home's otherwise; `None` without one.
pub(super) fn domain_index_text(
    corpus: &Corpus,
    repo_root: &Path,
    domain: &DomainName,
) -> Result<Option<String>, DocsError> {
    let path = if holds(repo_root, domain) {
        repo_root
            .join(Position::of_domain(domain).dir())
            .join(INDEX_FILE)
    } else {
        corpus.home().domain_dir(domain).join(INDEX_FILE)
    };
    read_optional(&path)
}

/// The `expanded` ids of the index of `domain`, as validation sees it;
/// empty without a readable index.
pub(super) fn domain_expanded(
    corpus: &Corpus,
    repo_root: &Path,
    domain: &DomainName,
) -> Result<Vec<SnippetId>, DocsError> {
    let Some(text) = domain_index_text(corpus, repo_root, domain)? else {
        return Ok(Vec::new());
    };
    Ok(PositionIndex::parse(&Position::of_domain(domain), &text)
        .map(|index| index.expanded().to_vec())
        .unwrap_or_default())
}

fn read_optional(path: &Path) -> Result<Option<String>, DocsError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(DocsError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Every `index.yaml` below `<repo_root>/docs/`, skipping `_` directories,
/// as its repository path and text.
fn index_files(repo_root: &Path) -> Result<Vec<(String, String)>, DocsError> {
    let docs = super::_corpus::confined_docs(repo_root)?;
    if !docs.is_dir() {
        return Ok(Vec::new());
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
    let mut out = Vec::new();
    for entry in walker {
        let entry = entry.map_err(|err| DocsError::Walk {
            path: err.path().unwrap_or(&docs).to_path_buf(),
            message: err.to_string(),
        })?;
        if !entry.file_type().is_file() || entry.file_name() != INDEX_FILE {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).map_err(|source| DocsError::Read {
            path: entry.path().to_path_buf(),
            source,
        })?;
        out.push((relative(repo_root, entry.path()), text));
    }
    Ok(out)
}

/// Which snippets an id may name, and what each id is.
pub(super) trait Snippets {
    /// Whether `id` names a snippet, and whether that snippet is pending;
    /// `None` when it names none.
    fn pending(&self, id: &SnippetId) -> Option<bool>;
    /// The ids `id`'s closure carries beyond itself: its transitive
    /// required reads, each bare-domain read standing for that domain's
    /// `expanded` set with its closure.
    fn closure(&mut self, id: &SnippetId) -> Result<BTreeSet<SnippetId>, DocsError>;
}

/// The findings of every `index.yaml` in the repository at `repo_root`,
/// and the notices of the domains it holds without one.
pub(super) fn check(
    corpus: &Corpus,
    repo_root: &Path,
    snippets: &mut dyn Snippets,
    out: &mut Vec<Finding>,
    notices: &mut Vec<Notice>,
) -> Result<(), DocsError> {
    let mut indexed = BTreeSet::new();
    for (rel, text) in index_files(repo_root)? {
        let Some(position) = position_of(&rel) else {
            out.push(Finding::new(&rel, FindingCode::IndexNested, None));
            continue;
        };
        indexed.insert(position.clone());
        Index {
            corpus,
            position: &position,
            path: PathBuf::from(&rel),
            out: &mut *out,
        }
        .check(&text, snippets)?;
    }
    for domain in held_domains(repo_root)? {
        let position = Position::Domain(domain.clone());
        if !indexed.contains(&position) {
            notices.push(Notice {
                path: PathBuf::from(format!("{}/", position.dir())),
                code: NoticeCode::DomainWithoutIndex,
                value: Some(format!(
                    "`{domain}` has no {INDEX_FILE}: it is absent from the global agent files and found only through `instrukt docs index --domain {domain}`"
                )),
            });
        }
    }
    Ok(())
}

/// The domains below the repository's `docs/domain/`, by folder name.
fn held_domains(repo_root: &Path) -> Result<Vec<DomainName>, DocsError> {
    let dir = repo_root.join("docs/domain");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(DocsError::Read { path: dir, source }),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| DocsError::Read {
            path: dir.clone(),
            source,
        })?;
        let is_dir = entry
            .file_type()
            .map_err(|source| DocsError::Read {
                path: entry.path(),
                source,
            })?
            .is_dir();
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_dir
            && !name.starts_with('_')
            && !super::is_reserved_domain_folder(&name)
            && let Ok(domain) = DomainName::parse(&name)
        {
            out.push(domain);
        }
    }
    out.sort();
    Ok(out)
}

/// One position index being checked.
struct Index<'c, 'o> {
    corpus: &'c Corpus,
    position: &'c Position,
    path: PathBuf,
    out: &'o mut Vec<Finding>,
}

impl Index<'_, '_> {
    fn finding(&mut self, code: FindingCode, value: String) {
        self.out.push(Finding::new(&self.path, code, Some(value)));
    }

    fn check(&mut self, text: &str, snippets: &mut dyn Snippets) -> Result<(), DocsError> {
        let mapping = match serde_yaml_ng::from_str::<Value>(text) {
            Ok(Value::Mapping(mapping)) => mapping,
            Ok(Value::Null) => serde_yaml_ng::Mapping::new(),
            Ok(_) => {
                self.finding(
                    FindingCode::IndexInvalid,
                    "not a mapping of keys".to_owned(),
                );
                return Ok(());
            }
            Err(err) => {
                self.finding(FindingCode::IndexInvalid, err.to_string());
                return Ok(());
            }
        };
        let allowed = allowed_keys(self.position);
        let mut expanded = Vec::new();
        let mut listed = Vec::new();
        let mut described = false;
        for (key, value) in &mapping {
            let Some(name) = key.as_str().filter(|k| allowed.contains(k)) else {
                let shown = key
                    .as_str()
                    .map_or_else(|| format!("{key:?}"), str::to_owned);
                self.finding(FindingCode::IndexKeyNotAllowed, shown);
                continue;
            };
            match name {
                "description" => {
                    described = true;
                    let line = value.as_str().map(str::trim).unwrap_or_default();
                    if line.is_empty() || line.contains('\n') {
                        self.finding(
                            FindingCode::IndexShapeInvalid,
                            "description: not one non-empty line".to_owned(),
                        );
                    }
                }
                "expanded" => expanded = self.ids("expanded", value, snippets),
                "listed" => listed = self.ids("listed", value, snippets),
                _ => self.agents(value, snippets),
            }
        }
        if matches!(self.position, Position::Domain(_)) && !described {
            self.finding(
                FindingCode::IndexDescriptionMissing,
                "description".to_owned(),
            );
        }
        for id in &listed {
            for root in &expanded {
                if root == id || snippets.closure(root)?.contains(id) {
                    self.finding(
                        FindingCode::IndexListedInExpanded,
                        format!("listed: {id} is already in the closure of expanded {root}"),
                    );
                    break;
                }
            }
        }
        Ok(())
    }

    /// The well-formed ids of the list `key` holds, each checked.
    fn ids(&mut self, key: &str, value: &Value, snippets: &mut dyn Snippets) -> Vec<SnippetId> {
        let Value::Sequence(items) = value else {
            self.finding(
                FindingCode::IndexShapeInvalid,
                format!("{key}: not a list of ids"),
            );
            return Vec::new();
        };
        let mut ids = Vec::new();
        for item in items {
            let Some(id) = item.as_str().and_then(|raw| SnippetId::parse(raw).ok()) else {
                let shown = item
                    .as_str()
                    .map_or_else(|| format!("{item:?}"), str::to_owned);
                self.finding(
                    FindingCode::IndexShapeInvalid,
                    format!("{key}: {shown} is not a snippet id"),
                );
                continue;
            };
            if !self.own(&id) {
                self.finding(
                    FindingCode::IndexIdForeign,
                    format!("{key}: {id} is not of {}", self.position),
                );
                continue;
            }
            match snippets.pending(&id) {
                None => self.finding(FindingCode::IndexIdUnresolved, format!("{key}: {id}")),
                Some(true) => self.finding(FindingCode::IndexIdPending, format!("{key}: {id}")),
                Some(false) => ids.push(id),
            }
        }
        ids
    }

    /// Whether this position's index may name `id`: `general`'s any
    /// domain's, every other one only its own position's.
    fn own(&self, id: &SnippetId) -> bool {
        match self.position {
            Position::General => !id.is_project(),
            Position::Domain(domain) => id.domain().as_ref() == Some(domain),
            Position::Project => {
                let prefix = self.corpus.project().map(|slug| format!("project/{slug}/"));
                prefix.is_some_and(|prefix| id.as_str().starts_with(&prefix))
            }
        }
    }

    fn agents(&mut self, value: &Value, snippets: &mut dyn Snippets) {
        let Value::Mapping(map) = value else {
            self.finding(
                FindingCode::IndexShapeInvalid,
                "agents: not a map from agent CLI to ids".to_owned(),
            );
            return;
        };
        let supported: Vec<&str> = AGENT_CLIS.iter().map(|(name, _)| *name).collect();
        for (key, ids) in map {
            let name = key
                .as_str()
                .map_or_else(|| format!("{key:?}"), str::to_owned);
            if !supported.contains(&name.as_str()) {
                self.finding(
                    FindingCode::IndexAgentUnsupported,
                    format!(
                        "agents: {name} (the supported agent CLIs are {})",
                        supported.join(", ")
                    ),
                );
                continue;
            }
            self.ids(&format!("agents.{name}"), ids, snippets);
        }
    }
}

/// Which snippets exist and are pending, by what the repository holds and,
/// for the positions it does not hold, by the home's global index.
pub(super) fn pending_in(
    corpus: &Corpus,
    repo_root: &Path,
    index: Option<&GlobalIndex>,
    id: &SnippetId,
) -> Option<bool> {
    let own = match id.domain() {
        Some(domain) => holds(repo_root, &domain) || corpus.published_domains().contains(&domain),
        None => true,
    };
    if own {
        return corpus.snippet(id).map(super::Snippet::is_pending);
    }
    index?
        .snippets
        .iter()
        .find(|entry| &entry.id == id)
        .map(|entry| !entry.delivered_by.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_belongs_to_the_position_at_whose_root_it_sits() {
        assert_eq!(
            position_of("docs/general/index.yaml"),
            Some(Position::General)
        );
        assert_eq!(
            position_of("docs/project/index.yaml"),
            Some(Position::Project)
        );
        assert_eq!(
            position_of("docs/domain/sdlc/index.yaml"),
            Some(Position::Domain(DomainName::parse("sdlc").unwrap()))
        );
        for rel in [
            "docs/index.yaml",
            "docs/general/policy/index.yaml",
            "docs/domain/sdlc/spec/index.yaml",
            "docs/domain/third-party/index.yaml",
            "docs/third-party/index.yaml",
        ] {
            assert_eq!(position_of(rel), None, "{rel}");
        }
    }

    #[test]
    fn each_position_allows_its_own_keys() {
        assert!(allowed_keys(&Position::General).contains(&"agents"));
        assert!(!allowed_keys(&Position::Project).contains(&"agents"));
        assert!(
            allowed_keys(&Position::Domain(DomainName::parse("x").unwrap()))
                .contains(&"description")
        );
        assert!(!allowed_keys(&Position::General).contains(&"description"));
    }
}
