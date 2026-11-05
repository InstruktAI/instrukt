//! Where a documentation file sits, and the identity that follows from it.
//!
//! Identity is derived, never authored (`general/spec/snippet-authoring-schema`):
//! `general/<taxonomy>/<rest>` under `docs/general/`,
//! `<domain>/<taxonomy>/<rest>` under `docs/domain/<domain>/` and
//! `project/<slug>/<taxonomy>/<rest>` under `docs/project/`.

use std::fmt;

use serde::Serialize;

use super::_position::{GENERAL, Position, is_reserved_domain_folder};
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
}

impl fmt::Display for Taxonomy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.dir())
    }
}

/// The scope a snippet belongs to, from its position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// A published position: `docs/general/` or a `docs/domain/<x>/`.
    Global,
    /// `docs/project/`: a project's own snippet.
    Project,
}

/// A snippet's derived domain: a published domain (`general` among them),
/// or the structural marker `project` of a project snippet.
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
    /// `docs/general/baseline.md`, `docs/domain/<domain>/baseline.md` or
    /// their `baseline-progressive.md`.
    Domain(DomainName),
    /// `docs/project/baseline.md`.
    Project,
}

/// Why a Markdown file under `docs/` yields no identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Unplaced {
    /// A taxonomy directory that is not one of the seven.
    UnknownTaxonomy(String),
    /// A file directly under `docs/domain/`, or a domain folder whose name
    /// is not a domain name.
    MissingDomain(String),
    /// A domain folder carrying a reserved name (`general`, `project`,
    /// `third-party`).
    ReservedDomain(String),
    /// A project snippet in a repository whose `instrukt.yaml` declares no
    /// `project`.
    UndeclaredProject,
    /// Not below a position or the third-party root — the pre-release
    /// layout's `docs/global/` and `docs/local/` among them.
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
    /// A third-party doc, identified by its path below `docs/third-party/`
    /// prefixed with `third-party/`.
    ThirdParty(ThirdPartyId),
    /// Under a published position's `workflow/`: not a snippet.
    Workflow,
    Unplaced(Unplaced),
}

/// Places the Markdown file at `rel` (`/`-separated, relative to the
/// repository root, beginning `docs/`). `None` for a path that is not a
/// documentation Markdown file: not below `docs/`, not `.md`, or inside a
/// directory whose name starts with `_`. A file of the pre-release layout
/// (`docs/global/…`, `docs/local/…`) yields no identity.
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
        [GENERAL, tail @ ..] => match DomainName::parse(GENERAL) {
            Ok(general) => place_published(general, tail),
            Err(_) => Placement::Unplaced(Unplaced::Outside),
        },
        ["domain", domain, tail @ ..] if !tail.is_empty() => {
            if is_reserved_domain_folder(domain) {
                return Some(Placement::Unplaced(Unplaced::ReservedDomain(
                    (*domain).to_owned(),
                )));
            }
            match DomainName::parse(domain) {
                Ok(name) => place_published(name, tail),
                Err(_) => Placement::Unplaced(Unplaced::MissingDomain((*domain).to_owned())),
            }
        }
        ["domain", file] => Placement::Unplaced(Unplaced::MissingDomain((*file).to_owned())),
        ["project", tail @ ..] => place_project(tail, project),
        ["third-party", tail @ ..] if !tail.is_empty() => {
            match ThirdPartyId::parse(&format!("third-party/{}", tail.join("/"))) {
                Ok(id) => Placement::ThirdParty(id),
                Err(_) => Placement::Unplaced(Unplaced::Outside),
            }
        }
        _ => Placement::Unplaced(Unplaced::Outside),
    })
}

/// The repository path of a published snippet's file: `general/…` below
/// `docs/general/`, `<x>/…` below `docs/domain/<x>/`. `None` for a project
/// snippet, whose file is found through its project.
pub fn published_source(id: &SnippetId) -> Option<String> {
    let domain = id.domain()?;
    let rest = id.as_str().strip_prefix(domain.as_str())?;
    Some(format!("{}{rest}.md", Position::of_domain(&domain).dir()))
}

/// The repository path a path below the home's published tree
/// (`<x>/<rest>`, relative to `~/.instrukt/docs/`) is published from:
/// `docs/general/<rest>` or `docs/domain/<x>/<rest>`. `None` for a path
/// with no domain segment or one that names no domain.
pub fn published_origin(home_rel: &str) -> Option<String> {
    let (domain, rest) = home_rel.split_once('/')?;
    let domain = DomainName::parse(domain).ok()?;
    Some(format!("{}/{rest}", Position::of_domain(&domain).dir()))
}

/// Where a file below a published position (`docs/general/…`,
/// `docs/domain/<x>/…`) is published, relative to `~/.instrukt/docs/`:
/// `general/…` or `<x>/…`. `None` for a path outside every published
/// position.
pub fn published_destination(repo_rel: &str) -> Option<String> {
    if let Some(rest) = repo_rel.strip_prefix(&format!("docs/{GENERAL}/")) {
        return Some(format!("{GENERAL}/{rest}"));
    }
    let rest = repo_rel.strip_prefix("docs/domain/")?;
    let (domain, _) = rest.split_once('/')?;
    (!is_reserved_domain_folder(domain) && DomainName::parse(domain).is_ok())
        .then(|| rest.to_owned())
}

fn place_published(domain_name: DomainName, after: &[&str]) -> Placement {
    let domain = domain_name.as_str().to_owned();
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

fn place_project(tail: &[&str], project: Option<&ProjectSlug>) -> Placement {
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
                format!("project/{project}/{}/{}", taxonomy.dir(), rest.join("/")),
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

    fn id_of(rel: &str, project: Option<&ProjectSlug>) -> Option<SnippetId> {
        match place(rel, project)? {
            Placement::Snippet(c) => Some(c.id),
            _ => None,
        }
    }

    fn id(text: &str) -> Option<SnippetId> {
        SnippetId::parse(text).ok()
    }

    #[test]
    fn each_position_derives_its_identities() {
        assert_eq!(
            id_of("docs/general/policy/x.md", None),
            id("general/policy/x")
        );
        assert_eq!(
            id_of("docs/domain/sdlc/policy/rust/core.md", None),
            id("sdlc/policy/rust/core")
        );
        assert_eq!(
            id_of("docs/project/spec/feature/docs/x.md", Some(&slug())),
            id("project/demo/spec/feature/docs/x")
        );
    }

    #[test]
    fn third_party_ids_keep_their_prefix() {
        assert_eq!(
            place("docs/third-party/tool/usage.md", None),
            Some(Placement::ThirdParty(
                ThirdPartyId::parse("third-party/tool/usage").unwrap()
            ))
        );
    }

    #[test]
    fn paths_without_an_identity_say_why() {
        let cases = [
            (
                "docs/domain/sdlc/notes/x.md",
                Unplaced::UnknownTaxonomy("notes".into()),
            ),
            ("docs/domain/x.md", Unplaced::MissingDomain("x".into())),
            (
                "docs/domain/Bad/policy/x.md",
                Unplaced::MissingDomain("Bad".into()),
            ),
            ("docs/project/spec/x.md", Unplaced::UndeclaredProject),
            ("docs/x.md", Unplaced::Outside),
        ];
        for (rel, why) in cases {
            assert_eq!(place(rel, None), Some(Placement::Unplaced(why)), "{rel}");
        }
    }

    #[test]
    fn a_domain_folder_with_a_reserved_name_yields_no_identity() {
        for name in ["general", "project", "third-party"] {
            assert_eq!(
                place(&format!("docs/domain/{name}/policy/x.md"), None),
                Some(Placement::Unplaced(Unplaced::ReservedDomain(name.into()))),
                "{name}"
            );
        }
    }

    #[test]
    fn the_pre_release_layout_yields_no_identity() {
        let project = slug();
        for rel in [
            "docs/global/sdlc/policy/x.md",
            "docs/global/snippets/step.md",
            "docs/local/spec/x.md",
        ] {
            assert_eq!(
                place(rel, Some(&project)),
                Some(Placement::Unplaced(Unplaced::Outside)),
                "{rel}"
            );
        }
    }

    #[test]
    fn manifests_workflow_and_skipped_paths_are_recognised() {
        assert!(matches!(
            place("docs/domain/sdlc/baseline-progressive.md", None),
            Some(Placement::Baseline {
                progressive: true,
                ..
            })
        ));
        assert!(matches!(
            place("docs/project/baseline.md", None),
            Some(Placement::Baseline {
                owner: BaselineOwner::Project,
                ..
            })
        ));
        assert_eq!(
            place("docs/domain/sdlc/workflow/templates/a.md", None),
            Some(Placement::Workflow)
        );
        assert_eq!(place("docs/domain/sdlc/_drafts/policy/a.md", None), None);
        assert_eq!(place("docs/domain/sdlc/policy/a.yaml", None), None);
    }

    #[test]
    fn a_published_snippet_maps_between_its_source_and_its_home_path() {
        let general = SnippetId::parse("general/policy/x").unwrap();
        let sdlc = SnippetId::parse("sdlc/policy/x").unwrap();
        assert_eq!(
            published_source(&general).as_deref(),
            Some("docs/general/policy/x.md")
        );
        assert_eq!(
            published_source(&sdlc).as_deref(),
            Some("docs/domain/sdlc/policy/x.md")
        );
        assert_eq!(
            published_origin("sdlc/workflow/work.yaml").as_deref(),
            Some("docs/domain/sdlc/workflow/work.yaml")
        );
        assert_eq!(
            published_destination("docs/domain/sdlc/policy/x.md").as_deref(),
            Some("sdlc/policy/x.md")
        );
        assert_eq!(
            published_destination("docs/general/policy/x.md").as_deref(),
            Some("general/policy/x.md")
        );
        assert_eq!(published_destination("docs/project/spec/x.md"), None);
    }

    #[test]
    fn companion_names_split_into_parent_and_slug() {
        assert_eq!(
            companion_shape("docs/project/spec/a.my-todo.md"),
            Some(("docs/project/spec/a.md".into(), "my-todo".into()))
        );
        assert_eq!(companion_shape("docs/project/spec/a.md"), None);
    }

    proptest! {
        /// Every taxonomy path below a position yields exactly the identity
        /// its segments spell, a domain's without a `domain/` prefix.
        #[test]
        fn identity_follows_the_path(
            domain in "[a-z][a-z0-9-]{0,8}",
            tax in prop::sample::select(Taxonomy::ALL.to_vec()),
            rest in prop::collection::vec("[a-z0-9][a-z0-9-]{0,8}", 1..4),
        ) {
            prop_assume!(!is_reserved_domain_folder(&domain) && DomainName::parse(&domain).is_ok());
            let rest = rest.join("/");
            let published = id_of(&format!("docs/domain/{domain}/{}/{rest}.md", tax.dir()), None);
            prop_assert_eq!(published, id(&format!("{domain}/{}/{rest}", tax.dir())));
            let local = id_of(&format!("docs/project/{}/{rest}.md", tax.dir()), Some(&slug()));
            prop_assert_eq!(local, id(&format!("project/demo/{}/{rest}", tax.dir())));
        }
    }
}
