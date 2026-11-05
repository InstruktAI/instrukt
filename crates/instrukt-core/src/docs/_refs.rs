//! The forms an `@` reference may take and what each names.
//!
//! Three forms are valid: the published-home form
//! (`~/.instrukt/docs/<id>.md` or its home-expanded absolute form) naming a
//! published snippet, the same form naming a domain bundle
//! (`~/.instrukt/docs/<domain>/bundle.md`), and `docs/project/<…>` naming
//! material in the project's local half.

use std::path::Path;

use serde::Serialize;

use super::_identity::{Placement, place};
use crate::ids::{DomainName, ProjectSlug, SnippetId};

/// What a well-formed reference names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "target")]
pub enum RefTarget {
    /// A snippet, by identity (published or local).
    Snippet(SnippetId),
    /// A published domain's generated bundle.
    Bundle(DomainName),
    /// A local-half file that is not a snippet (e.g.
    /// `docs/project/baseline.md`), repository-relative.
    LocalFile(String),
}

impl RefTarget {
    /// Whether the target lies in the local half.
    pub fn is_local(&self) -> bool {
        match self {
            RefTarget::Snippet(id) => id.is_project(),
            RefTarget::Bundle(_) => false,
            RefTarget::LocalFile(_) => true,
        }
    }
}

/// An `@` reference as written, with what it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reference {
    /// The reference as written, without its `@`.
    pub raw: String,
    /// What it names; `None` when the reference is not in a valid form.
    pub target: Option<RefTarget>,
}

const HOME_DOCS: &str = ".instrukt/docs/";

/// Reads `raw` (a reference without its `@`) for a repository declaring
/// `project`, whose readers' home directory is `user_home`.
pub(super) fn parse(raw: &str, project: Option<&ProjectSlug>, user_home: &Path) -> Reference {
    Reference {
        raw: raw.to_owned(),
        target: target_of(raw, project, user_home),
    }
}

fn target_of(raw: &str, project: Option<&ProjectSlug>, user_home: &Path) -> Option<RefTarget> {
    if let Some(local) = raw.strip_prefix("docs/project/") {
        let rel = format!("docs/project/{local}");
        return match place(&rel, project)? {
            Placement::Snippet(class) => Some(RefTarget::Snippet(class.id)),
            Placement::Baseline { .. } => Some(RefTarget::LocalFile(rel)),
            _ => None,
        };
    }
    let published = raw
        .strip_prefix("~/")
        .and_then(|rest| rest.strip_prefix(HOME_DOCS))
        .or_else(|| {
            let home = user_home.to_str()?;
            raw.strip_prefix(home)?
                .strip_prefix('/')?
                .strip_prefix(HOME_DOCS)
        })?;
    if let Some((domain, "bundle.md")) = published.split_once('/') {
        return DomainName::parse(domain).ok().map(RefTarget::Bundle);
    }
    match place(&format!("docs/global/{published}"), project)? {
        Placement::Snippet(class) => Some(RefTarget::Snippet(class.id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn target(raw: &str) -> Option<RefTarget> {
        let project = ProjectSlug::parse("demo").unwrap();
        parse(raw, Some(&project), &PathBuf::from("/u")).target
    }

    fn snippet(id: &str) -> Option<RefTarget> {
        Some(RefTarget::Snippet(SnippetId::parse(id).unwrap()))
    }

    #[test]
    fn valid_forms_name_their_targets() {
        assert_eq!(
            target("~/.instrukt/docs/sdlc/policy/x.md"),
            snippet("sdlc/policy/x")
        );
        assert_eq!(
            target("/u/.instrukt/docs/sdlc/policy/x.md"),
            snippet("sdlc/policy/x")
        );
        assert_eq!(
            target("~/.instrukt/docs/sdlc/bundle.md"),
            Some(RefTarget::Bundle(DomainName::parse("sdlc").unwrap()))
        );
        assert_eq!(
            target("docs/project/spec/a.md"),
            snippet("project/demo/spec/a")
        );
        let manifest = "docs/project/baseline.md";
        assert_eq!(
            target(manifest),
            Some(RefTarget::LocalFile(manifest.to_owned()))
        );
    }

    #[test]
    fn other_forms_name_nothing() {
        for raw in [
            "docs/global/sdlc/policy/x.md",
            "~/.teleclaude/docs/sdlc/policy/x.md",
            "/elsewhere/.instrukt/docs/sdlc/policy/x.md",
            "~/.instrukt/docs/third-party/x.md",
            "~/.instrukt/docs/sdlc/notes/x.md",
            "./x.md",
        ] {
            assert_eq!(target(raw), None, "{raw}");
        }
    }
}
