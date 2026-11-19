//! The directive checks of validation: the format has no directive that
//! runs a command or includes a file, and what publication renders has no
//! runtime session (`project/instrukt/spec/feature/docs/snippet-validation`,
//! Directives).
//!
//! - An `@exec` or `@include` directive in a snippet, a companion, a
//!   third-party doc or a master is a finding.
//! - Any `@config` directive is a finding in what a committed `AGENTS.md`
//!   holds in full: the masters, the snippets `docs/project/index.yaml`
//!   expands and the masters require, with their closure, as rendering
//!   reads them. A committed rendering resolves no configured value, so it
//!   is the same on every computer.
//! - A `@config` directive with a `runtime.*` path is a finding in what a
//!   global agent file or a domain read renders in full: the snippets the
//!   published positions' indexes render (`expanded`, `general`'s
//!   `agents`), with their closure. Publication has no runtime session.
//! - The repository-root master carrying `## Required reads` is a finding:
//!   `docs/project/index.yaml` selects what the root `AGENTS.md` includes
//!   (`project/instrukt/spec/feature/publication/agent-entrypoints`).

use std::collections::BTreeSet;
use std::path::Path;

use super::_corpus::{Corpus, Lens, Stray, Version, View};
use super::_finding::{Finding, FindingCode};
use super::_position::{INDEX_FILE, Position, PositionIndex};
use super::_refs::{RefTarget, Reference};
use super::_render::{self, RenderContext, Values, is_runtime};
use super::_text::{self, Directive};
use super::DocsError;
use crate::git;

/// The file name of a master.
const MASTER_FILE_NAME: &str = "AGENTS.master.md";

/// Every directive finding of the repository at `repo_root`, whose parsed
/// documentation is `corpus`.
pub(super) fn check(
    corpus: &Corpus,
    repo_root: &Path,
    values: &Values<'_>,
    out: &mut Vec<Finding>,
) -> Result<(), DocsError> {
    for snippet in corpus.snippets() {
        unsupported_in(snippet.current().path(), snippet.current().body(), out);
        for companion in snippet.companions() {
            if let Some(next) = companion.next() {
                unsupported_in(next.path(), next.body(), out);
            }
        }
    }
    for stray in corpus.strays() {
        if let Stray::Companion {
            version: Some(version),
            ..
        } = stray
        {
            unsupported_in(version.path(), version.body(), out);
        }
    }
    for doc in corpus.third_party() {
        unsupported_in(doc.path(), doc.body(), out);
    }

    let cx = RenderContext {
        home: corpus.home(),
        values,
        project: corpus.project(),
    };
    let index_of = |position: &Position| position_index(repo_root, position);
    let masters = compiled_sources(repo_root)?;
    for (rel, text) in &masters {
        let body = _text::split_frontmatter(text).body;
        unsupported_in(Path::new(rel), body, out);
        if rel == MASTER_FILE_NAME && carries_required_reads(body) {
            out.push(Finding::new(
                Path::new(rel),
                FindingCode::MasterRequiredReads,
                Some(format!(
                    "the root master holds only `## Purpose`; `docs/project/{INDEX_FILE}` selects what the root AGENTS.md includes"
                )),
            ));
        }
    }
    committed_config(corpus, &masters, &index_of, &cx, out)?;
    let mut walk = Compiled {
        corpus,
        index_of: &index_of,
        cx: &cx,
        committed: false,
        seen: BTreeSet::new(),
        out,
    };
    for position in corpus.published_domains().iter().map(Position::of_domain) {
        if let Some(index) = position_index(repo_root, &position)? {
            for id in rendered_in_full(&index) {
                walk.reference(&Reference {
                    raw: id.as_str().to_owned(),
                    target: Some(RefTarget::Snippet(id.clone())),
                })?;
            }
        }
    }
    Ok(())
}

/// A finding for each `@config` directive in what the repository's committed
/// `AGENTS.md` files hold in full: each of `masters` (`(path, text)`), the
/// snippets `docs/project/index.yaml` expands, and what they require, with
/// their closure. `index_of` reads a position's `index.yaml`.
fn committed_config(
    corpus: &Corpus,
    masters: &[(String, String)],
    index_of: &IndexOf<'_>,
    cx: &RenderContext<'_>,
    out: &mut Vec<Finding>,
) -> Result<(), DocsError> {
    let mut walk = Compiled {
        corpus,
        index_of,
        cx,
        committed: true,
        seen: BTreeSet::new(),
        out,
    };
    for (rel, text) in masters {
        walk.file(Path::new(rel), _text::split_frontmatter(text).body)?;
    }
    if let Some(index) = index_of(&Position::Project)? {
        for id in index.expanded() {
            walk.reference(&Reference {
                raw: id.as_str().to_owned(),
                target: Some(RefTarget::Snippet(id.clone())),
            })?;
        }
    }
    Ok(())
}

/// How a walk reads a position's `index.yaml`; `None` without one.
type IndexOf<'a> = dyn Fn(&Position) -> Result<Option<PositionIndex>, DocsError> + 'a;

/// Whether a master's body carries a `## Required reads` heading outside
/// fenced code.
pub(crate) fn carries_required_reads(body: &str) -> bool {
    _text::headings(&_text::lines(body))
        .iter()
        .any(|h| h.level == 2 && _text::normalize_title(&h.title) == "required reads")
}

/// The `index.yaml` of `position` in the repository at `repo_root`; `None`
/// without one or when it does not parse (the index check reports that).
fn position_index(
    repo_root: &Path,
    position: &Position,
) -> Result<Option<PositionIndex>, DocsError> {
    let path = repo_root.join(position.dir()).join(INDEX_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(PositionIndex::parse(position, &text).ok()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(DocsError::Read { path, source }),
    }
}

/// The ids an index has rendered in full: `expanded`, and for `general`
/// every agent CLI's `agents` snippets.
fn rendered_in_full(index: &PositionIndex) -> Vec<&crate::ids::SnippetId> {
    let mut ids: Vec<_> = index.expanded().iter().collect();
    if let PositionIndex::General(general) = index {
        ids.extend(general.agents.values().flatten());
    }
    ids
}

/// A finding for each `@exec` and `@include` directive of the source text
/// `text` at `path` (its frontmatter, if any, aside): the rule validation
/// applies, for a rendering that holds the text.
pub(crate) fn unsupported_directives(path: &Path, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    unsupported_in(path, _text::split_frontmatter(text).body, &mut out);
    out
}

/// A finding for each `@exec` and `@include` directive in `body`, naming
/// its command or path as written.
fn unsupported_in(path: &Path, body: &str, out: &mut Vec<Finding>) {
    for directive in _text::directives(&_text::lines(body)) {
        let (code, value) = match directive {
            Directive::Exec { command, .. } => (FindingCode::DirectiveExec, command),
            Directive::Include { path: include, .. } => (FindingCode::DirectiveInclude, include),
            Directive::Config { .. } => continue,
        };
        out.push(Finding::new(path, code, Some(value)));
    }
}

/// The walk over what a rendering holds in full, each target once: a
/// committed `AGENTS.md`, where any `@config` is a finding, or a global file
/// or domain read, where a `runtime.*` one is.
struct Compiled<'w, 'c> {
    corpus: &'c Corpus,
    index_of: &'w IndexOf<'w>,
    cx: &'w RenderContext<'w>,
    committed: bool,
    seen: BTreeSet<RefTarget>,
    out: &'w mut Vec<Finding>,
}

impl Compiled<'_, '_> {
    /// Checks one compiled file, and then what it requires.
    fn file(&mut self, path: &Path, body: &str) -> Result<(), DocsError> {
        self.config_in(path, &current(body));
        for read in _render::dependency_reads(body, Lens::Current, self.cx)? {
            self.reference(&read)?;
        }
        Ok(())
    }

    fn reference(&mut self, reference: &Reference) -> Result<(), DocsError> {
        let Some(target) = &reference.target else {
            return Ok(());
        };
        if !self.seen.insert(target.clone()) {
            return Ok(());
        }
        match target {
            RefTarget::Snippet(id) => {
                // Only this repository's snippets are validated here; a
                // pending one is never compiled.
                let Some(snippet) = self.corpus.snippet(id) else {
                    return Ok(());
                };
                if let View::Visible { version, .. } = snippet.view(Lens::Current) {
                    let path = version.path().to_path_buf();
                    let body = version.body().to_owned();
                    self.file(&path, &body)?;
                }
            }
            // A bare-domain read renders that domain's `expanded` set.
            RefTarget::Domain(domain) => {
                let position = Position::of_domain(domain);
                if let Some(index) = (self.index_of)(&position)? {
                    for id in index.expanded() {
                        self.reference(&Reference {
                            raw: id.as_str().to_owned(),
                            target: Some(RefTarget::Snippet(id.clone())),
                        })?;
                    }
                }
            }
            // A path reference is validation's finding elsewhere; it names
            // nothing rendering reads.
            RefTarget::Bundle(_) | RefTarget::LocalFile(_) => {}
        }
        Ok(())
    }

    /// A finding for each `@config` directive in `body` this walk refuses:
    /// any in a committed rendering, a `runtime.*` one elsewhere.
    fn config_in(&mut self, path: &Path, body: &str) {
        for directive in _text::directives(&_text::lines(body)) {
            let Directive::Config { path: value, .. } = directive else {
                continue;
            };
            let code = if self.committed {
                FindingCode::ConfigCommitted
            } else if is_runtime(&value) {
                FindingCode::ConfigRuntimeCompiled
            } else {
                continue;
            };
            self.out.push(Finding::new(path, code, Some(value)));
        }
    }
}

/// `body` as compilation reads it: the current contract, planned material
/// resolved away.
fn current(body: &str) -> String {
    Version::parse("", body, None, None, Path::new("")).lens_body(Lens::Current)
}

/// The masters of the repository at `repo_root`,
/// with their text: tracked and untracked files git does not ignore in a
/// working tree, every file of a directory that is none.
pub(super) fn compiled_sources(repo_root: &Path) -> Result<Vec<(String, String)>, DocsError> {
    let listed: Vec<String> = match git::worktree_root(repo_root)? {
        Some(root) if same_dir(&root, repo_root)? => {
            let listing = git::run_bytes(
                repo_root,
                [
                    "ls-files",
                    "-z",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                ],
            )?;
            listing
                .split(|b| *b == 0)
                .filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .collect()
        }
        _ => walk(repo_root)?,
    };
    let mut out = Vec::new();
    for rel in listed {
        if rel.rsplit('/').next() != Some(MASTER_FILE_NAME) {
            continue;
        }
        let path = repo_root.join(&rel);
        match std::fs::read_to_string(&path) {
            Ok(text) => out.push((rel, text)),
            // Listed but gone (deleted in the working tree): nothing to check.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(DocsError::Read { path, source }),
        }
    }
    out.sort();
    Ok(out)
}

fn same_dir(a: &Path, b: &Path) -> Result<bool, DocsError> {
    let canonical = |p: &Path| {
        p.canonicalize().map_err(|source| DocsError::Read {
            path: p.to_path_buf(),
            source,
        })
    };
    Ok(canonical(a)? == canonical(b)?)
}

/// Every regular file below `root`, relative and `/`-separated, without
/// following symbolic links.
fn walk(root: &Path) -> Result<Vec<String>, DocsError> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|err| DocsError::Walk {
            path: err.path().unwrap_or(root).to_path_buf(),
            message: err.to_string(),
        })?;
        if entry.file_type().is_file() {
            out.push(super::_corpus::relative(root, entry.path()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::_corpus::SourceFile;
    use crate::ids::ProjectSlug;

    const STYLE: &str = "---\ndescription: Style.\n---\n\n# Style — Policy\n\n## Rules\n\n<!-- @config: project -->\n";

    #[test]
    fn a_configured_value_in_a_master_is_a_finding_and_in_a_listed_snippet_none() {
        let project = ProjectSlug::parse("demo").unwrap();
        let corpus = Corpus::from_sources(
            Some(project.clone()),
            None,
            [SourceFile {
                path: "docs/project/policy/style.md".to_owned(),
                text: STYLE.to_owned(),
            }],
        );
        let index = "listed:\n  - project/demo/policy/style\n";
        let index_of = |position: &Position| -> Result<Option<PositionIndex>, DocsError> {
            Ok((*position == Position::Project)
                .then(|| PositionIndex::parse(position, index).unwrap()))
        };
        let values = Values::committed();
        let cx = RenderContext {
            home: None,
            values: &values,
            project: Some(&project),
        };
        let masters = [(
            MASTER_FILE_NAME.to_owned(),
            "## Purpose\n\n<!-- @config: project -->\n".to_owned(),
        )];
        let mut out = Vec::new();
        committed_config(&corpus, &masters, &index_of, &cx, &mut out).unwrap();
        assert_eq!(
            out,
            [Finding::new(
                Path::new(MASTER_FILE_NAME),
                FindingCode::ConfigCommitted,
                Some("project".to_owned()),
            )]
        );
    }
}
