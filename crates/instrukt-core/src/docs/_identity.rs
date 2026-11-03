//! Where a documentation file sits, and the identity that follows from it.
//!
//! Identity is derived, never authored (`general/spec/snippet-authoring-schema`):
//! `<domain>/<taxonomy>/<rest>` in the published half,
//! `local/projects/<slug>/<taxonomy>/<rest>` in the local half.

use std::fmt;

use serde::Serialize;

use crate::ids::{DomainName, ProjectSlug, SnippetId, ThirdPartyId};

/// A snippet's type, from its taxonomy directory. Ordered as listings order
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Taxonomy {
    Principle,
    Concept,
    Policy,
    Procedure,
    Design,
    Spec,
    Adr,
}

impl Taxonomy {
    pub const ALL: [Taxonomy; 7] = [
        Taxonomy::Principle,
        Taxonomy::Concept,
        Taxonomy::Policy,
        Taxonomy::Procedure,
        Taxonomy::Design,
        Taxonomy::Spec,
        Taxonomy::Adr,
    ];

    /// The taxonomy a directory name names.
    pub fn from_dir(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.dir() == name)
    }

    /// The directory name, which is also the `type` value.
    pub fn dir(self) -> &'static str {
        match self {
            Taxonomy::Principle => "principle",
            Taxonomy::Concept => "concept",
            Taxonomy::Policy => "policy",
            Taxonomy::Procedure => "procedure",
            Taxonomy::Design => "design",
            Taxonomy::Spec => "spec",
            Taxonomy::Adr => "adr",
        }
    }

    /// The suffix the H1 ends with: `— <Type>`.
    pub fn title_suffix(self) -> &'static str {
        match self {
            Taxonomy::Principle => "Principle",
            Taxonomy::Concept => "Concept",
            Taxonomy::Policy => "Policy",
            Taxonomy::Procedure => "Procedure",
            Taxonomy::Design => "Design",
            Taxonomy::Spec => "Spec",
            Taxonomy::Adr => "ADR",
        }
    }

    /// The H2 sections the taxonomy requires.
    pub fn required_sections(self) -> &'static [&'static str] {
        match self {
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

    /// The H2 sections the taxonomy allows besides its required ones and
    /// the sections every snippet may carry (`Required reads`, `See also`).
    pub fn optional_sections(self) -> &'static [&'static str] {
        match self {
            Taxonomy::Procedure => &["Discipline"],
            Taxonomy::Spec => &["Allowed values", "Known caveats"],
            Taxonomy::Adr => &["Alternatives Considered", "Status"],
            _ => &[],
        }
    }
}

impl fmt::Display for Taxonomy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.dir())
    }
}

/// The scope a snippet belongs to, from its half.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// The published half: a domain snippet.
    Global,
    /// The local half: a project's own snippet.
    Project,
}

/// A snippet's derived domain: a published domain, or the structural
/// marker `project` of a local snippet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DocDomain {
    Published(DomainName),
    Project,
}

impl fmt::Display for DocDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocDomain::Published(domain) => f.write_str(domain.as_str()),
            DocDomain::Project => f.write_str("project"),
        }
    }
}

impl Serialize for DocDomain {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// The derived classification of a snippet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Classification {
    pub id: SnippetId,
    pub taxonomy: Taxonomy,
    pub scope: Scope,
    pub domain: DocDomain,
}

/// Who a baseline manifest belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BaselineOwner {
    /// `docs/global/<domain>/baseline.md` or `baseline-progressive.md`.
    Domain(DomainName),
    /// `docs/local/baseline.md`.
    Project,
}

/// Why a Markdown file under `docs/` yields no identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Unplaced {
    /// A taxonomy directory that is not one of the seven.
    UnknownTaxonomy(String),
    /// A published file with no domain segment, or a domain segment that is
    /// not a domain name.
    MissingDomain(String),
    /// A local snippet in a repository whose `instrukt.yaml` declares no
    /// `project`.
    UndeclaredProject,
    /// Not below a half, the third-party root or the fragment root.
    Outside,
}

/// What a Markdown file under `docs/` is, by its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Placement {
    Snippet(Classification),
    Baseline {
        owner: BaselineOwner,
        progressive: bool,
    },
    /// A fragment under `docs/global/snippets/`, named by its path below
    /// that root.
    Fragment(String),
    /// A third-party doc, identified by its path below `docs/third-party/`.
    ThirdParty(ThirdPartyId),
    /// Under `docs/global/<domain>/workflow/`: not a snippet.
    Workflow,
    Unplaced(Unplaced),
}

/// Domain names an identity grammar reserves.
const RESERVED_DOMAINS: [&str; 4] = ["local", "projects", "snippets", "third-party"];

/// Places the Markdown file at `rel` (`/`-separated, relative to the
/// repository root, beginning `docs/`). `None` for a path that is not a
/// documentation Markdown file: not below `docs/`, not `.md`, or inside a
/// directory whose name starts with `_`.
pub(super) fn place(rel: &str, project: Option<&ProjectSlug>) -> Option<Placement> {
    let stem = rel.strip_suffix(".md")?;
    let parts: Vec<&str> = stem.split('/').collect();
    let (first, rest) = parts.split_first()?;
    if *first != "docs" || rest.is_empty() {
        return None;
    }
    let dirs = &rest[..rest.len() - 1];
    if dirs.iter().any(|dir| dir.starts_with('_')) {
        return None;
    }
    Some(match rest {
        ["global", "snippets", tail @ ..] if !tail.is_empty() => {
            Placement::Fragment(tail.join("/"))
        }
        ["global", tail @ ..] => place_published(tail),
        ["local", tail @ ..] => place_local(tail, project),
        ["third-party", tail @ ..] if !tail.is_empty() => {
            match ThirdPartyId::parse(&tail.join("/")) {
                Ok(id) => Placement::ThirdParty(id),
                Err(_) => Placement::Unplaced(Unplaced::Outside),
            }
        }
        _ => Placement::Unplaced(Unplaced::Outside),
    })
}

fn place_published(tail: &[&str]) -> Placement {
    let [domain, after @ ..] = tail else {
        return Placement::Unplaced(Unplaced::Outside);
    };
    if after.is_empty() {
        return Placement::Unplaced(Unplaced::MissingDomain((*domain).to_owned()));
    }
    let domain_name = match DomainName::parse(domain) {
        Ok(name) if !RESERVED_DOMAINS.contains(domain) => name,
        _ => return Placement::Unplaced(Unplaced::MissingDomain((*domain).to_owned())),
    };
    match after {
        ["workflow", ..] => Placement::Workflow,
        [name @ ("baseline" | "baseline-progressive")] => Placement::Baseline {
            owner: BaselineOwner::Domain(domain_name),
            progressive: *name == "baseline-progressive",
        },
        [taxonomy, rest @ ..] if !rest.is_empty() => match Taxonomy::from_dir(taxonomy) {
            Some(taxonomy) => classify(
                format!("{domain}/{}/{}", taxonomy.dir(), rest.join("/")),
                taxonomy,
                Scope::Global,
                DocDomain::Published(domain_name),
            ),
            None => Placement::Unplaced(Unplaced::UnknownTaxonomy((*taxonomy).to_owned())),
        },
        [other, ..] => Placement::Unplaced(Unplaced::UnknownTaxonomy((*other).to_owned())),
        [] => Placement::Unplaced(Unplaced::Outside),
    }
}

fn place_local(tail: &[&str], project: Option<&ProjectSlug>) -> Placement {
    match tail {
        ["baseline"] => Placement::Baseline {
            owner: BaselineOwner::Project,
            progressive: false,
        },
        [taxonomy, rest @ ..] if !rest.is_empty() => {
            let Some(taxonomy) = Taxonomy::from_dir(taxonomy) else {
                return Placement::Unplaced(Unplaced::UnknownTaxonomy((*taxonomy).to_owned()));
            };
            let Some(project) = project else {
                return Placement::Unplaced(Unplaced::UndeclaredProject);
            };
            classify(
                format!(
                    "local/projects/{project}/{}/{}",
                    taxonomy.dir(),
                    rest.join("/")
                ),
                taxonomy,
                Scope::Project,
                DocDomain::Project,
            )
        }
        [other] => Placement::Unplaced(Unplaced::UnknownTaxonomy((*other).to_owned())),
        _ => Placement::Unplaced(Unplaced::Outside),
    }
}

fn classify(id: String, taxonomy: Taxonomy, scope: Scope, domain: DocDomain) -> Placement {
    match SnippetId::parse(&id) {
        Ok(id) => Placement::Snippet(Classification {
            id,
            taxonomy,
            scope,
            domain,
        }),
        Err(_) => Placement::Unplaced(Unplaced::Outside),
    }
}

/// The parent `<stem>.md` and slug of a companion-shaped file name
/// `<stem>.<slug>.md`, without checking that the parent exists.
pub(super) fn companion_shape(rel: &str) -> Option<(String, String)> {
    let stem = rel.strip_suffix(".md")?;
    let (dir, name) = match stem.rsplit_once('/') {
        Some((dir, name)) => (Some(dir), name),
        None => (None, stem),
    };
    let (parent, slug) = name.rsplit_once('.')?;
    if parent.is_empty() || slug.is_empty() {
        return None;
    }
    let parent_rel = match dir {
        Some(dir) => format!("{dir}/{parent}.md"),
        None => format!("{parent}.md"),
    };
    Some((parent_rel, slug.to_owned()))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn slug() -> ProjectSlug {
        ProjectSlug::parse("demo").unwrap()
    }

    fn id_of(rel: &str, project: Option<&ProjectSlug>) -> Option<String> {
        match place(rel, project)? {
            Placement::Snippet(c) => Some(c.id.to_string()),
            _ => None,
        }
    }

    #[test]
    fn published_and_local_halves_derive_their_identities() {
        assert_eq!(
            id_of("docs/global/sdlc/policy/rust/core.md", None).as_deref(),
            Some("sdlc/policy/rust/core")
        );
        assert_eq!(
            id_of("docs/local/spec/feature/docs/x.md", Some(&slug())).as_deref(),
            Some("local/projects/demo/spec/feature/docs/x")
        );
    }

    #[test]
    fn paths_without_an_identity_say_why() {
        let cases = [
            (
                "docs/global/sdlc/notes/x.md",
                Unplaced::UnknownTaxonomy("notes".into()),
            ),
            ("docs/global/x.md", Unplaced::MissingDomain("x".into())),
            (
                "docs/global/projects/policy/x.md",
                Unplaced::MissingDomain("projects".into()),
            ),
            ("docs/local/spec/x.md", Unplaced::UndeclaredProject),
            ("docs/x.md", Unplaced::Outside),
        ];
        for (rel, why) in cases {
            assert_eq!(place(rel, None), Some(Placement::Unplaced(why)), "{rel}");
        }
    }

    #[test]
    fn manifests_fragments_workflow_and_skipped_paths_are_recognised() {
        assert!(matches!(
            place("docs/global/sdlc/baseline-progressive.md", None),
            Some(Placement::Baseline {
                progressive: true,
                ..
            })
        ));
        assert!(matches!(
            place("docs/local/baseline.md", None),
            Some(Placement::Baseline {
                owner: BaselineOwner::Project,
                ..
            })
        ));
        assert_eq!(
            place("docs/global/snippets/step.md", None),
            Some(Placement::Fragment("step".into()))
        );
        assert_eq!(
            place("docs/global/sdlc/workflow/templates/a.md", None),
            Some(Placement::Workflow)
        );
        assert_eq!(place("docs/global/sdlc/_drafts/policy/a.md", None), None);
        assert_eq!(place("docs/global/sdlc/policy/a.yaml", None), None);
    }

    #[test]
    fn companion_names_split_into_parent_and_slug() {
        assert_eq!(
            companion_shape("docs/local/spec/a.my-todo.md"),
            Some(("docs/local/spec/a.md".into(), "my-todo".into()))
        );
        assert_eq!(companion_shape("docs/local/spec/a.md"), None);
    }

    proptest! {
        /// Every taxonomy path below a half yields exactly the identity its
        /// segments spell, and never one beginning with the other half's
        /// prefix.
        #[test]
        fn identity_follows_the_path(
            domain in "[a-z][a-z0-9-]{0,8}",
            tax in prop::sample::select(Taxonomy::ALL.to_vec()),
            rest in prop::collection::vec("[a-z0-9][a-z0-9-]{0,8}", 1..4),
        ) {
            prop_assume!(!RESERVED_DOMAINS.contains(&domain.as_str()));
            let rest = rest.join("/");
            let published = id_of(&format!("docs/global/{domain}/{}/{rest}.md", tax.dir()), None);
            prop_assert_eq!(published, Some(format!("{domain}/{}/{rest}", tax.dir())));
            let local = id_of(&format!("docs/local/{}/{rest}.md", tax.dir()), Some(&slug()));
            prop_assert_eq!(local, Some(format!("local/projects/demo/{}/{rest}", tax.dir())));
        }
    }
}
