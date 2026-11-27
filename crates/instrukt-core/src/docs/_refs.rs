//! What a reference names.
//!
//! A snippet's references are ids: a `## Required reads` entry is one
//! plain or linked snippet id or one bare domain name ([`entry`]), and a
//! See Also entry opens with an id or carries a linked id
//! (`project/instrukt/spec/domain/docs`, "Reference").
//!
//! The `@` path forms ([`parse`]) remain only so the documentation's own
//! reading can recognise them: in a baseline manifest, and in a snippet,
//! where validation reports them. They are the published-home form (`~/.instrukt/docs/<id>.md` or its
//! home-expanded absolute form) naming a published snippet, the same form
//! naming a domain bundle (`~/.instrukt/docs/<domain>/bundle.md`), and
//! `docs/project/<…>` naming material in the project position. In a
//! snippet, a path reference is a validation finding.

use std::ops::Range;
use std::path::Path;

use serde::Serialize;

use super::_identity::{Placement, place, published_origin};
use super::_position::{GENERAL, Position};
use super::_text;
use crate::ids::{DomainName, ProjectSlug, SnippetId};

/// What a well-formed reference names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "target")]
pub enum RefTarget {
    /// A snippet, by identity (published or local).
    Snippet(SnippetId),
    /// A bare domain name (`general` included): that domain's `expanded`
    /// set, each with its closure.
    Domain(DomainName),
    /// A published domain's generated bundle, named by path.
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
            RefTarget::Domain(_) | RefTarget::Bundle(_) => false,
            RefTarget::LocalFile(_) => true,
        }
    }
}

/// An id, domain or legacy path reference, with what it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reference {
    /// The reference without link markup (or a legacy path's `@`).
    pub raw: String,
    /// What it names; `None` when the reference is not in a valid form.
    pub target: Option<RefTarget>,
}

const HOME_DOCS: &str = ".instrukt/docs/";

/// Reads one `## Required reads` entry of a snippet, as written after its
/// list marker: a plain or linked snippet id, or a bare domain name.
/// Link targets never participate in resolution.
pub(super) fn entry(raw: &str) -> Reference {
    if let Some((id, _)) = link(raw) {
        return Reference {
            raw: id.to_owned(),
            target: SnippetId::parse(id).ok().map(RefTarget::Snippet),
        };
    }
    let target = if let Ok(id) = SnippetId::parse(raw) {
        Some(RefTarget::Snippet(id))
    } else if raw.contains('/') {
        None
    } else {
        DomainName::parse(raw).ok().map(RefTarget::Domain)
    };
    Reference {
        raw: raw.to_owned(),
        target,
    }
}

/// The text and target of a whole inline Markdown link.
pub(super) fn link(raw: &str) -> Option<(&str, &str)> {
    let (text, target) = raw.strip_prefix('[')?.split_once("](")?;
    Some((text, target.strip_suffix(')')?))
}

/// A link occupying a Required reads or See Also list entry. Its span
/// covers only the markup, so rendering preserves the rest of the line.
pub(super) struct ReferenceLink {
    pub line: usize,
    pub span: Range<usize>,
    pub text: String,
    pub target: String,
}

pub(super) fn links(lines: &[_text::Line<'_>]) -> Vec<ReferenceLink> {
    let headings = _text::headings(lines);
    let mut in_references = false;
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some(heading) = headings.iter().find(|h| h.line == index && h.level <= 2) {
            in_references = heading.level == 2
                && matches!(
                    _text::normalize_title(&heading.title).as_str(),
                    "required reads" | "see also"
                );
        }
        if !in_references || line.fenced {
            continue;
        }
        let visible = _text::without_comments(line.text);
        let Some(item) = ["- ", "* ", "+ "]
            .iter()
            .find_map(|bullet| visible.trim().strip_prefix(bullet))
            .map(str::trim)
        else {
            continue;
        };
        let Some((text, target)) = link(item) else {
            continue;
        };
        let Some(offset) = line.text.find(item) else {
            continue;
        };
        let start = line.span.start + offset;
        out.push(ReferenceLink {
            line: index,
            span: start..start + item.len(),
            text: text.to_owned(),
            target: target.to_owned(),
        });
    }
    out
}

/// How a See Also entry names its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SeeAlsoForm {
    /// The entry opens with the plain id.
    Plain,
    /// The entry opens with a link whose text is the id; the link's
    /// target is checked as a reference link.
    Linked,
}

/// The id a `See Also` entry names, and how it names it.
pub(super) struct SeeAlso {
    pub id: SnippetId,
    pub form: SeeAlsoForm,
}

/// The ids the `See Also` entries of `lines` open with, plain or as the
/// text of a link.
pub(super) fn see_also(lines: &[_text::Line<'_>]) -> Vec<SeeAlso> {
    let headings = _text::headings(lines);
    let Some(start) = headings
        .iter()
        .position(|h| h.level == 2 && _text::normalize_title(&h.title) == "see also")
    else {
        return Vec::new();
    };
    let from = headings[start].line + 1;
    let to = headings
        .iter()
        .skip(start + 1)
        .find(|h| h.level <= 2)
        .map_or(lines.len(), |h| h.line);
    lines[from..to]
        .iter()
        .filter(|line| !line.fenced)
        .filter_map(|line| {
            let text = _text::without_comments(line.text);
            let item = ["- ", "* ", "+ "]
                .iter()
                .find_map(|bullet| text.trim().strip_prefix(bullet))
                .map(str::to_owned)?;
            let first = item.split_whitespace().next()?.trim_matches('`');
            let (raw, form) = match link(first) {
                Some((text, _)) => (text, SeeAlsoForm::Linked),
                None => (first, SeeAlsoForm::Plain),
            };
            let id = SnippetId::parse(raw).ok()?;
            Some(SeeAlso { id, form })
        })
        .collect()
}

/// Whether a reference standing in `from` may name `target`. References
/// run upward only, from a specific body toward the generic one
/// (`general/policy/referencing-doc-snippets`): `general` names only
/// `general`; a domain names only itself and `general`; the project names
/// itself, `general` and any domain. `project` is the slug the validated
/// repository declares, the only project its snippets name.
pub(super) fn runs_upward(
    from: &Position,
    target: &RefTarget,
    project: Option<&ProjectSlug>,
) -> bool {
    let domain = match target {
        RefTarget::Snippet(id) if id.is_project() => {
            return *from == Position::Project
                && project.is_some_and(|own| project_of(id) == Some(own.as_str()));
        }
        RefTarget::LocalFile(_) => return *from == Position::Project,
        RefTarget::Snippet(id) => match id.domain() {
            Some(domain) => domain,
            None => return false,
        },
        RefTarget::Domain(domain) | RefTarget::Bundle(domain) => domain.clone(),
    };
    match from {
        Position::General => domain.as_str() == GENERAL,
        Position::Domain(own) => domain.as_str() == GENERAL || domain == *own,
        Position::Project => true,
    }
}

/// What a reference standing in `from` may name, as a direction finding
/// states it after the reference.
pub(super) fn upward_from(from: &Position) -> String {
    match from {
        Position::General => format!("{GENERAL} references only {GENERAL}"),
        Position::Domain(domain) => format!("{domain} references only {domain} and {GENERAL}"),
        Position::Project => {
            format!("project references only its own project, {GENERAL} and domains")
        }
    }
}

/// The slug a project snippet's id names (`project/<slug>/…`).
fn project_of(id: &SnippetId) -> Option<&str> {
    id.as_str().strip_prefix("project/")?.split('/').next()
}

/// Whether `token` names a snippet file by its path: an `@` reference, or
/// a path to a Markdown file below a position or the home's published
/// tree.
pub(super) fn is_snippet_path(token: &str) -> bool {
    let path = token.strip_prefix('@').unwrap_or(token);
    path.ends_with(".md")
        && (path.starts_with("docs/")
            || path.starts_with("~/")
            || path.contains(HOME_DOCS)
            || token.starts_with('@'))
}

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
    match place(&published_origin(published)?, project)? {
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
            target("~/.instrukt/docs/general/policy/x.md"),
            snippet("general/policy/x")
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
    fn an_entry_names_a_snippet_or_a_domain() {
        assert_eq!(entry("sdlc/policy/x").target, snippet("sdlc/policy/x"));
        assert_eq!(
            entry("project/demo/spec/a").target,
            snippet("project/demo/spec/a")
        );
        for name in ["sdlc", "general"] {
            assert_eq!(
                entry(name).target,
                Some(RefTarget::Domain(DomainName::parse(name).unwrap()))
            );
        }
        for raw in [
            "sdlc/notes",
            "project",
            "@~/.instrukt/docs/sdlc/policy/x.md",
            "a b",
        ] {
            assert_eq!(entry(raw).target, None, "{raw}");
        }
    }

    #[test]
    fn a_link_names_its_id_independently_of_its_path() {
        let reference = entry("[sdlc/policy/x](a-stale-path.md)");
        assert_eq!(reference.target, snippet("sdlc/policy/x"));
        assert_eq!(reference.raw, "sdlc/policy/x");
    }

    #[test]
    fn references_run_upward_only() {
        let project = ProjectSlug::parse("demo").unwrap();
        let acme = Position::of_domain(&DomainName::parse("acme").unwrap());
        let named = |from: &Position, raw: &str| {
            let target = entry(raw).target.unwrap();
            runs_upward(from, &target, Some(&project))
        };
        let general = Position::General;
        assert!(named(&general, "general/policy/x"));
        assert!(named(&general, "general"));
        for raw in ["acme/policy/x", "acme", "project/demo/spec/a"] {
            assert!(!named(&general, raw), "general -> {raw}");
        }
        for raw in ["acme/policy/x", "acme", "general/policy/x", "general"] {
            assert!(named(&acme, raw), "acme -> {raw}");
        }
        for raw in ["ops/policy/x", "ops", "project/demo/spec/a"] {
            assert!(!named(&acme, raw), "acme -> {raw}");
        }
        for raw in [
            "project/demo/spec/a",
            "general/policy/x",
            "acme",
            "ops/policy/x",
        ] {
            assert!(named(&Position::Project, raw), "project -> {raw}");
        }
        assert!(!named(&Position::Project, "project/other/spec/a"));
        let manifest = RefTarget::LocalFile("docs/project/baseline.md".to_owned());
        assert!(runs_upward(&Position::Project, &manifest, Some(&project)));
        assert!(!runs_upward(&acme, &manifest, Some(&project)));
    }

    #[test]
    fn a_see_also_entry_names_its_id_plain_or_linked() {
        let body =
            "# T\n\n## See Also\n\n- sdlc/policy/x — why\n- [ops/policy/y](../y.md)\n- prose\n";
        let lines = _text::lines(body);
        let entries: Vec<_> = see_also(&lines)
            .into_iter()
            .map(|e| (e.id.to_string(), e.form))
            .collect();
        assert_eq!(
            entries,
            [
                ("sdlc/policy/x".to_owned(), SeeAlsoForm::Plain),
                ("ops/policy/y".to_owned(), SeeAlsoForm::Linked),
            ]
        );
    }

    #[test]
    fn a_path_to_a_snippet_file_is_recognized() {
        for token in [
            "@~/.instrukt/docs/sdlc/policy/x.md",
            "~/.instrukt/docs/sdlc/policy/x.md",
            "docs/domain/sdlc/policy/x.md",
            "@x.md",
        ] {
            assert!(is_snippet_path(token), "{token}");
        }
        for token in ["sdlc/policy/x", "README", "notes.md"] {
            assert!(!is_snippet_path(token), "{token}");
        }
    }

    #[test]
    fn other_forms_name_nothing() {
        for raw in [
            "docs/domain/sdlc/policy/x.md",
            "~/.elsewhere/docs/sdlc/policy/x.md",
            "/elsewhere/.instrukt/docs/sdlc/policy/x.md",
            "~/.instrukt/docs/third-party/x.md",
            "~/.instrukt/docs/sdlc/notes/x.md",
            "./x.md",
        ] {
            assert_eq!(target(raw), None, "{raw}");
        }
    }
}
