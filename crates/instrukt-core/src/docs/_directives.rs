//! The directive checks of validation: the format has no directive that
//! runs a command or includes a file, and what publication renders has no
//! runtime session (`project/instrukt/spec/feature/docs/snippet-validation`,
//! Directives).
//!
//! - An `@exec` or `@include` directive in a snippet, a companion, a
//!   third-party doc or a master is a finding.
//! - A `@config` directive with a `runtime.*` path is a finding in every
//!   file rendered into an `AGENTS.md` or a global agent file: the masters,
//!   and the snippets the repository's position indexes render in full
//!   (`expanded`, `general`'s `agents`) and the masters require, with their
//!   closure, as rendering reads them.
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
    let mut walk = Compiled {
        corpus,
        repo_root,
        cx: &cx,
        seen: BTreeSet::new(),
        out,
    };
    for (rel, text) in compiled_sources(repo_root)? {
        let body = _text::split_frontmatter(&text).body.to_owned();
        unsupported_in(Path::new(&rel), &body, walk.out);
        if rel == MASTER_FILE_NAME && carries_required_reads(&body) {
            walk.out.push(Finding::new(
                Path::new(&rel),
                FindingCode::MasterRequiredReads,
                Some(format!(
                    "the root master holds only `## Purpose`; `docs/project/{INDEX_FILE}` selects what the root AGENTS.md includes"
                )),
            ));
        }
        walk.file(Path::new(&rel), &body)?;
    }
    let mut positions = vec![Position::Project];
    positions.extend(corpus.published_domains().iter().map(Position::of_domain));
    for position in positions {
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

/// Whether a master's body carries a `## Required reads` heading outside
/// fenced code.
fn carries_required_reads(body: &str) -> bool {
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

/// The walk over what publication renders, each target once.
struct Compiled<'w, 'c> {
    corpus: &'c Corpus,
    repo_root: &'w Path,
    cx: &'w RenderContext<'w>,
    seen: BTreeSet<RefTarget>,
    out: &'w mut Vec<Finding>,
}

impl Compiled<'_, '_> {
    /// Checks one compiled file, and then what it requires.
    fn file(&mut self, path: &Path, body: &str) -> Result<(), DocsError> {
        self.runtime_in(path, &current(body));
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
                if let Some(index) = position_index(self.repo_root, &position)? {
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

    /// A finding for each `runtime.*` `@config` directive in `body`.
    fn runtime_in(&mut self, path: &Path, body: &str) {
        for directive in _text::directives(&_text::lines(body)) {
            if let Directive::Config { path: value, .. } = directive
                && is_runtime(&value)
            {
                self.out.push(Finding::new(
                    path,
                    FindingCode::ConfigRuntimeCompiled,
                    Some(value),
                ));
            }
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
fn compiled_sources(repo_root: &Path) -> Result<Vec<(String, String)>, DocsError> {
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
