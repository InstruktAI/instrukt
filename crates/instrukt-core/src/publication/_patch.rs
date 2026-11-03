//! What one project contributes to the global index, and the patch that
//! puts it there: drop what the project owns, refuse a claim on another
//! project's id or domain, add the new entries, keep everything else.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use crate::docs::{Corpus, DocDomain, Scope, Snippet, Visibility};
use crate::ids::{DomainName, ProjectSlug};
use crate::index::{
    FragmentEntry, GlobalIndex, IndexDispatch, IndexEntry, IndexParameter, IndexScope,
    ThirdPartyEntry,
};

/// Why the patch refused a project's contribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// A new entry's id is held by another project's entry.
    DuplicateId { id: String, owner: ProjectSlug },
    /// A domain the project publishes is owned by another project.
    DomainOwned {
        domain: DomainName,
        owner: ProjectSlug,
    },
}

impl fmt::Display for Claim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Claim::DuplicateId { id, owner } => {
                write!(f, "`{id}` is already published by project `{owner}`")
            }
            Claim::DomainOwned { domain, owner } => write!(
                f,
                "domain `{domain}` is owned by project `{owner}`; it is free to claim once `{owner}` stops publishing it"
            ),
        }
    }
}

/// The index entries one project's committed documentation yields.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Contribution {
    pub project: ProjectSlug,
    /// The domains the project publishes.
    pub domains: Vec<DomainName>,
    pub snippets: Vec<IndexEntry>,
    pub third_party: Vec<ThirdPartyEntry>,
    pub fragments: Vec<FragmentEntry>,
}

impl Contribution {
    /// The entries of `corpus`: one global entry per snippet of a domain in
    /// `domains`, one local entry per local snippet (carrying its file's
    /// absolute path in `primary`), one entry per third-party doc and one
    /// per fragment in `fragments` (their names).
    pub(super) fn of(
        project: &ProjectSlug,
        corpus: &Corpus,
        domains: &[DomainName],
        fragments: &[String],
        primary: &Path,
    ) -> Self {
        let mut snippets: Vec<IndexEntry> = corpus
            .snippets()
            .iter()
            .filter_map(|snippet| entry(project, snippet, domains, primary))
            .collect();
        snippets.sort_by(|a, b| a.id.cmp(&b.id));
        let mut third_party: Vec<ThirdPartyEntry> = corpus
            .third_party()
            .iter()
            .map(|doc| {
                let path = doc.path().strip_prefix("docs").unwrap_or(doc.path());
                ThirdPartyEntry {
                    id: doc.id().clone(),
                    description: doc
                        .description()
                        .map(str::to_owned)
                        .unwrap_or_else(|| fallback_description(doc.body(), doc.path())),
                    path: path.to_path_buf(),
                    source_project: project.clone(),
                }
            })
            .collect();
        third_party.sort_by(|a, b| a.id.cmp(&b.id));
        let fragments = fragments
            .iter()
            .map(|name| FragmentEntry {
                name: name.clone(),
                path: Path::new("snippets").join(format!("{name}.md")),
                source_project: project.clone(),
            })
            .collect();
        Self {
            project: project.clone(),
            domains: domains.to_vec(),
            snippets,
            third_party,
            fragments,
        }
    }
}

fn entry(
    project: &ProjectSlug,
    snippet: &Snippet,
    domains: &[DomainName],
    primary: &Path,
) -> Option<IndexEntry> {
    let (scope, path) = match (snippet.scope(), snippet.domain()) {
        (Scope::Global, DocDomain::Published(domain)) if domains.contains(domain) => (
            IndexScope::Global,
            snippet
                .path()
                .strip_prefix("docs/global")
                .ok()?
                .to_path_buf(),
        ),
        (Scope::Project, _) => (IndexScope::Local, primary.join(snippet.path())),
        _ => return None,
    };
    let frontmatter = snippet.current().frontmatter();
    let procedure = &frontmatter.entry;
    Some(IndexEntry {
        id: snippet.id().clone(),
        description: frontmatter.description.clone().unwrap_or_default(),
        taxonomy: snippet.taxonomy().dir().to_owned(),
        scope,
        domain: snippet.domain().to_string(),
        path,
        source_project: project.clone(),
        visibility: Some(
            match frontmatter.visibility {
                Visibility::Public => "public",
                Visibility::Internal => "internal",
            }
            .to_owned(),
        ),
        delivered_by: snippet.delivered_by().to_vec(),
        superseded_by: snippet.superseded_by().into_iter().collect(),
        activation_trigger: procedure.activation_trigger.clone(),
        role: procedure.role.clone(),
        subscriptions: procedure.subscriptions.clone(),
        parameters: procedure
            .parameters
            .iter()
            .map(|p| IndexParameter {
                name: p.name.clone(),
                required: p.required,
                rest: p.rest,
            })
            .collect(),
        dispatch: procedure.dispatch.as_ref().map(|d| IndexDispatch {
            agent: d.agents.clone(),
            thinking_mode: serde_plain(&d.thinking_mode),
        }),
        event_delivery: (!procedure.subscriptions.is_empty())
            .then(|| serde_plain(&procedure.event_delivery)),
        interruptible: procedure.interruptible,
    })
}

/// The lowercase name a unit enum serializes to.
fn serde_plain<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        _ => String::new(),
    }
}

/// A third-party doc without a `description`: its first H1, else its file
/// name.
fn fallback_description(body: &str, path: &Path) -> String {
    body.lines()
        .find_map(|line| line.strip_prefix("# "))
        .map(|title| title.trim().to_owned())
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

/// Applies `contribution` to `index`: every entry its project owns is
/// dropped, the new entries are checked against those that remain and
/// added; other projects' entries are carried over unchanged. `publishers`
/// names each published domain's recorded owner.
pub(super) fn patch(
    index: &GlobalIndex,
    publishers: &BTreeMap<DomainName, ProjectSlug>,
    contribution: &Contribution,
) -> Result<GlobalIndex, Claim> {
    let project = &contribution.project;
    for domain in &contribution.domains {
        if let Some(owner) = publishers.get(domain).filter(|owner| *owner != project) {
            return Err(Claim::DomainOwned {
                domain: domain.clone(),
                owner: owner.clone(),
            });
        }
        if let Some(other) = index.snippets.iter().find(|e| {
            e.scope == IndexScope::Global
                && e.domain == domain.as_str()
                && &e.source_project != project
        }) {
            return Err(Claim::DomainOwned {
                domain: domain.clone(),
                owner: other.source_project.clone(),
            });
        }
    }
    let mut snippets: Vec<IndexEntry> = index
        .snippets
        .iter()
        .filter(|e| &e.source_project != project)
        .cloned()
        .collect();
    let mut third_party: Vec<ThirdPartyEntry> = index
        .third_party
        .iter()
        .filter(|e| &e.source_project != project)
        .cloned()
        .collect();
    let mut fragments: Vec<FragmentEntry> = index
        .fragments
        .iter()
        .filter(|e| &e.source_project != project)
        .cloned()
        .collect();
    let held: BTreeMap<&str, &ProjectSlug> = snippets
        .iter()
        .map(|e| (e.id.as_str(), &e.source_project))
        .chain(
            third_party
                .iter()
                .map(|e| (e.id.as_str(), &e.source_project)),
        )
        .chain(
            fragments
                .iter()
                .map(|e| (e.name.as_str(), &e.source_project)),
        )
        .collect();
    let new_ids = contribution
        .snippets
        .iter()
        .map(|e| e.id.as_str())
        .chain(contribution.third_party.iter().map(|e| e.id.as_str()));
    for id in new_ids {
        if let Some(owner) = held.get(id) {
            return Err(Claim::DuplicateId {
                id: id.to_owned(),
                owner: (*owner).clone(),
            });
        }
    }
    for fragment in &contribution.fragments {
        if let Some(other) = fragments.iter().find(|e| e.name == fragment.name) {
            return Err(Claim::DuplicateId {
                id: other.path.display().to_string(),
                owner: other.source_project.clone(),
            });
        }
    }
    snippets.extend(contribution.snippets.iter().cloned());
    third_party.extend(contribution.third_party.iter().cloned());
    fragments.extend(contribution.fragments.iter().cloned());
    snippets.sort_by(|a, b| a.id.cmp(&b.id));
    third_party.sort_by(|a, b| a.id.cmp(&b.id));
    fragments.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(GlobalIndex {
        snippets,
        third_party,
        fragments,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use proptest::prelude::*;

    use super::*;
    use crate::ids::SnippetId;

    fn slug(s: &str) -> ProjectSlug {
        ProjectSlug::parse(s).unwrap()
    }

    fn domain(s: &str) -> DomainName {
        DomainName::parse(s).unwrap()
    }

    fn entry(id: &str, owner: &str) -> IndexEntry {
        let global = !id.starts_with("local/");
        IndexEntry {
            id: SnippetId::parse(id).unwrap(),
            description: format!("about {id}"),
            taxonomy: "policy".into(),
            scope: if global {
                IndexScope::Global
            } else {
                IndexScope::Local
            },
            domain: if global {
                id.split('/').next().unwrap().into()
            } else {
                "project".into()
            },
            path: PathBuf::from(format!("{id}.md")),
            source_project: slug(owner),
            visibility: None,
            delivered_by: Vec::new(),
            superseded_by: Vec::new(),
            activation_trigger: None,
            role: None,
            subscriptions: Vec::new(),
            parameters: Vec::new(),
            dispatch: None,
            event_delivery: None,
            interruptible: false,
        }
    }

    fn contribution(project: &str, domains: &[&str], ids: &[&str]) -> Contribution {
        Contribution {
            project: slug(project),
            domains: domains.iter().map(|d| domain(d)).collect(),
            snippets: ids.iter().map(|id| entry(id, project)).collect(),
            third_party: Vec::new(),
            fragments: Vec::new(),
        }
    }

    fn index(entries: Vec<IndexEntry>) -> GlobalIndex {
        GlobalIndex {
            snippets: entries,
            ..GlobalIndex::default()
        }
    }

    #[test]
    fn a_patch_replaces_only_the_projects_own_entries() {
        let other = entry("beta/policy/b", "beta-project");
        let before = index(vec![
            entry("alpha/policy/old", "alpha-project"),
            other.clone(),
        ]);
        let new = contribution("alpha-project", &["alpha"], &["alpha/policy/new"]);
        let after = patch(&before, &BTreeMap::new(), &new).unwrap();
        let mut expected = vec![other, new.snippets[0].clone()];
        expected.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(after.snippets, expected);
    }

    #[test]
    fn an_id_held_by_another_project_is_refused_naming_its_owner() {
        let before = index(vec![entry("local/projects/same/spec/a", "first")]);
        let new = contribution("second", &[], &["local/projects/same/spec/a"]);
        assert_eq!(
            patch(&before, &BTreeMap::new(), &new),
            Err(Claim::DuplicateId {
                id: "local/projects/same/spec/a".into(),
                owner: slug("first"),
            })
        );
    }

    #[test]
    fn a_domain_recorded_for_another_project_is_refused() {
        let publishers = BTreeMap::from([(domain("alpha"), slug("first"))]);
        let new = contribution("second", &["alpha"], &[]);
        assert_eq!(
            patch(&GlobalIndex::default(), &publishers, &new),
            Err(Claim::DomainOwned {
                domain: domain("alpha"),
                owner: slug("first"),
            })
        );
    }

    #[test]
    fn a_domain_carrying_another_projects_entries_is_refused() {
        let before = index(vec![entry("alpha/policy/a", "first")]);
        let new = contribution("second", &["alpha"], &[]);
        assert_eq!(
            patch(&before, &BTreeMap::new(), &new),
            Err(Claim::DomainOwned {
                domain: domain("alpha"),
                owner: slug("first"),
            })
        );
    }

    fn ids() -> impl Strategy<Value = Vec<String>> {
        prop::collection::btree_set("[a-c]{1,2}", 0..4)
            .prop_map(|set| set.into_iter().collect::<Vec<_>>())
    }

    proptest! {
        #[test]
        fn patching_is_idempotent_and_keeps_other_owners(mine in ids(), theirs in ids()) {
            let others: Vec<IndexEntry> = theirs
                .iter()
                .map(|n| entry(&format!("beta/policy/{n}"), "beta-project"))
                .collect();
            let start = index(others.clone());
            let ids: Vec<String> = mine.iter().map(|n| format!("alpha/policy/{n}")).collect();
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            let new = contribution("alpha-project", &["alpha"], &refs);
            let once = patch(&start, &BTreeMap::new(), &new).unwrap();
            let twice = patch(&once, &BTreeMap::new(), &new).unwrap();
            prop_assert_eq!(&once, &twice);
            let kept: Vec<&IndexEntry> = once
                .snippets
                .iter()
                .filter(|e| e.source_project.as_str() == "beta-project")
                .collect();
            prop_assert_eq!(kept, others.iter().collect::<Vec<_>>());
        }
    }
}
