//! The directive checks of validation: the format has no directive that
//! runs a command, and what publication compiles has no runtime session
//! (`local/projects/instrukt/spec/feature/docs/snippet-validation`,
//! Directives).
//!
//! - An `@exec` directive in a snippet, a companion, a fragment, a
//!   third-party doc or a master is a finding.
//! - A `@config` directive with a `runtime.*` path is a finding in every
//!   file compiled into a bundle, an `AGENTS.md` or a global entrypoint: the
//!   masters and the global agent files, the snippets the published
//!   domains' baselines and the masters require (transitively, as
//!   compilation reads them), and the fragments those include.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::_corpus::{Corpus, Lens, Stray, Version, View};
use super::_finding::{Finding, FindingCode};
use super::_identity::BaselineOwner;
use super::_refs::{RefTarget, Reference};
use super::_render::{self, Fragments, RenderContext, Values, is_runtime};
use super::_text::{self, Directive};
use super::DocsError;
use crate::git;

/// The file name of a master.
const MASTER_FILE_NAME: &str = "AGENTS.master.md";
/// The directory of the global master and the agent addenda.
const AGENTS_DIR: &str = "agents/";

/// Every directive finding of the repository at `repo_root`, whose parsed
/// documentation is `corpus`.
pub(super) fn check(
    corpus: &Corpus,
    repo_root: &Path,
    values: &Values<'_>,
    out: &mut Vec<Finding>,
) -> Result<(), DocsError> {
    for snippet in corpus.snippets() {
        exec_in(snippet.current().path(), snippet.current().body(), out);
        for companion in snippet.companions() {
            if let Some(next) = companion.next() {
                exec_in(next.path(), next.body(), out);
            }
        }
    }
    for stray in corpus.strays() {
        if let Stray::Companion {
            version: Some(version),
            ..
        } = stray
        {
            exec_in(version.path(), version.body(), out);
        }
    }
    for doc in corpus.third_party() {
        exec_in(doc.path(), doc.body(), out);
    }
    for (path, body) in corpus.fragment_files() {
        exec_in(Path::new(&path), body, out);
    }

    let cx = RenderContext {
        fragments: Fragments::CorpusFirst(corpus),
        values,
        repo_root: Some(repo_root),
        project: corpus.project(),
    };
    let mut walk = Compiled {
        corpus,
        cx: &cx,
        seen: BTreeSet::new(),
        out,
    };
    for (rel, text) in compiled_sources(repo_root)? {
        let body = _text::split_frontmatter(&text).body.to_owned();
        exec_in(Path::new(&rel), &body, walk.out);
        walk.file(Path::new(&rel), &body)?;
    }
    let owned = corpus.published_domains();
    for manifest in corpus.baselines() {
        let compiled = !manifest.is_progressive()
            && matches!(manifest.owner(), BaselineOwner::Domain(d) if owned.contains(d));
        if compiled {
            for entry in manifest.entries() {
                walk.reference(entry)?;
            }
        }
    }
    Ok(())
}

/// An `@exec` finding for each such directive in `body`.
fn exec_in(path: &Path, body: &str, out: &mut Vec<Finding>) {
    for directive in _text::directives(&_text::lines(body)) {
        if let Directive::Exec { command, .. } = directive {
            out.push(Finding::new(
                path,
                FindingCode::DirectiveExec,
                Some(command),
            ));
        }
    }
}

/// The walk over what publication compiles, each target once.
struct Compiled<'w, 'c> {
    corpus: &'c Corpus,
    cx: &'w RenderContext<'w>,
    seen: BTreeSet<RefTarget>,
    out: &'w mut Vec<Finding>,
}

impl Compiled<'_, '_> {
    /// Checks one compiled file, its fragments, and then what it requires.
    fn file(&mut self, path: &Path, body: &str) -> Result<(), DocsError> {
        let lensed = current(body);
        self.runtime_in(path, &lensed);
        for directive in _text::directives(&_text::lines(&lensed)) {
            if let Directive::Include { path: include, .. } = directive
                && let Some(fragment) = self.corpus.fragment(&include)
            {
                let fragment_path = include.replacen("docs/snippets/", "docs/global/snippets/", 1);
                self.runtime_in(&PathBuf::from(fragment_path), &current(fragment));
            }
        }
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
            RefTarget::Bundle(domain) => {
                let entries: Vec<Reference> = self
                    .corpus
                    .baselines()
                    .iter()
                    .filter(|m| {
                        !m.is_progressive() && m.owner() == &BaselineOwner::Domain(domain.clone())
                    })
                    .flat_map(|m| m.entries().into_iter().cloned())
                    .collect();
                for entry in &entries {
                    self.reference(entry)?;
                }
            }
            RefTarget::LocalFile(path) => {
                let entries: Vec<Reference> = self
                    .corpus
                    .baselines()
                    .iter()
                    .filter(|m| m.path() == Path::new(path))
                    .flat_map(|m| m.entries().into_iter().cloned())
                    .collect();
                for entry in &entries {
                    self.reference(entry)?;
                }
            }
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

/// The masters and global agent files of the repository at `repo_root`,
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
        let is_source = rel.rsplit('/').next() == Some(MASTER_FILE_NAME)
            || (rel.starts_with(AGENTS_DIR) && rel.ends_with(".md"));
        if !is_source {
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
