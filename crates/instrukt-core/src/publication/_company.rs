//! A company home's committed `AGENTS.md`
//! (`project/instrukt/spec/feature/publication/agent-entrypoints`, "Company
//! home entrypoint"): the generated line, the company passage, the root
//! master's own content, then its required reads in full, each before the
//! document that requires it and each once.
//!
//! A company's knowledge stands alone. The master and the company's
//! documents read only the home's own documents, each named by its path
//! relative to the file that reads it and confined to the home's `docs/`
//! folder. A read or a reference to a snippet, a domain or a file outside
//! `docs/`, a read of a file the home does not hold or of a symbolic link,
//! and an `@config`, `@exec` or `@include` are findings, and a home with a
//! finding is not rendered.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::_render::{
    AGENTS_FILE, GENERATED_LINE, OVERRIDE_FILE, PlannedFile, RenderFinding, RenderFindingKind,
    RenderPlan, finish,
};
use super::_source::{Committed, MASTER_FILE_NAME};
use crate::docs::{self, CompanyDocument, Directive};
use crate::ids::{DomainName, SnippetId};

/// The passage every company rendering carries between the generated line
/// and the master's content: the same for every company, naming the home
/// and no repository host or organization.
pub(super) const COMPANY_PASSAGE: &str = "This folder is the company's home. An agent working here may change the company's documents — `AGENTS.master.md` and the files in `docs/` — and commit them; the change reaches the company's agents through the company's repository.";

/// The home's documents are below this folder.
const DOCS: &str = "docs/";

/// What is wrong with a read or a reference of a company document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanyReadProblem {
    /// It names a snippet.
    Snippet,
    /// It names a domain.
    Domain,
    /// It names a file outside the home's `docs/` folder.
    OutsideDocs,
    /// It names a file the home does not hold.
    Missing,
    /// It names a symbolic link, which is never read.
    Link,
}

impl CompanyReadProblem {
    /// What the operator reads after the read it names.
    pub(super) fn sentence(self) -> &'static str {
        match self {
            Self::Snippet => {
                "a snippet; a company's knowledge stands alone, so name one of the company's own documents in `docs/` instead"
            }
            Self::Domain => {
                "a domain; a company's knowledge stands alone, so name one of the company's own documents in `docs/` instead"
            }
            Self::OutsideDocs => {
                "outside the home's `docs/` folder; a company's documents are read only from there"
            }
            Self::Missing => "which the home does not hold; add the document or correct the path",
            Self::Link => {
                "a symbolic link, which is never read; replace the link with the file, or read its target directly"
            }
        }
    }
}

/// Renders the company home in `tree` into `plan`: its root `AGENTS.md`,
/// or the findings that leave that file as it is. Without a root master
/// there is nothing to render.
pub(super) fn render(tree: &Committed, override_beside_root: bool, plan: &mut RenderPlan) {
    let Some(master) = tree.file(MASTER_FILE_NAME) else {
        return;
    };
    let path = PathBuf::from(AGENTS_FILE);
    if override_beside_root {
        plan.findings.push(RenderFinding {
            path: PathBuf::from(OVERRIDE_FILE),
            kind: RenderFindingKind::OverrideBesideMaster,
        });
    }
    let master_text = String::from_utf8_lossy(master).into_owned();
    let mut walk = Walk {
        tree,
        path: &path,
        seen: BTreeSet::new(),
        order: Vec::new(),
        findings: Vec::new(),
    };
    let document = docs::company_document(&master_text);
    walk.check(MASTER_FILE_NAME, &master_text, &document);
    walk.follow(MASTER_FILE_NAME, &document.reads);
    if !walk.findings.is_empty() {
        plan.findings.extend(walk.findings);
        return;
    }
    let mut blocks = vec![COMPANY_PASSAGE.to_owned(), document.body.trim().to_owned()];
    blocks.extend(walk.order.into_iter().map(|body| body.trim().to_owned()));
    let text = finish(&blocks.join("\n\n"));
    plan.writes.push(PlannedFile {
        current: tree.file(AGENTS_FILE).map(<[u8]>::to_vec),
        path,
        text: format!("{GENERATED_LINE}\n{text}"),
    });
}

/// The documents a company rendering holds in full, in order, and what it
/// found on the way.
struct Walk<'t> {
    tree: &'t Committed,
    /// The rendering the findings leave as it is.
    path: &'t Path,
    seen: BTreeSet<String>,
    /// Each document's text without its required reads, in traversal order.
    order: Vec<String>,
    findings: Vec<RenderFinding>,
}

impl Walk<'_> {
    /// Brings in each of `reads`, read by the document at `reader`: each
    /// after its own required reads, each once.
    fn follow(&mut self, reader: &str, reads: &[String]) {
        for read in reads {
            let path = match self.document(reader, read) {
                Ok(path) => path,
                Err(problem) => {
                    self.found(RenderFindingKind::CompanyRead {
                        document: reader.to_owned(),
                        read: read.clone(),
                        problem,
                    });
                    continue;
                }
            };
            if !self.seen.insert(path.clone()) {
                continue;
            }
            let text =
                String::from_utf8_lossy(self.tree.file(&path).unwrap_or_default()).into_owned();
            let document = docs::company_document(&text);
            self.check(&path, &text, &document);
            self.follow(&path, &document.reads);
            self.order.push(document.body);
        }
    }

    /// The findings of the document at `path` itself: its directives and
    /// the references in its text.
    fn check(&mut self, path: &str, text: &str, document: &CompanyDocument) {
        if document
            .directives
            .iter()
            .any(|directive| matches!(directive, Directive::Config { .. }))
        {
            self.found(RenderFindingKind::CompanyConfig {
                document: path.to_owned(),
            });
        }
        for finding in docs::unsupported_directives(Path::new(path), text) {
            self.found(RenderFindingKind::UnsupportedDirective { finding });
        }
        for reference in &document.references {
            if let Err(problem) = self.document(path, reference) {
                self.found(RenderFindingKind::CompanyReference {
                    document: path.to_owned(),
                    reference: reference.clone(),
                    problem,
                });
            }
        }
    }

    /// The home's document that `read`, written in the document at
    /// `reader`, names: its repository-relative path, or why it names none.
    fn document(&self, reader: &str, read: &str) -> Result<String, CompanyReadProblem> {
        let read = read.strip_prefix('@').unwrap_or(read);
        if !read.ends_with(".md") {
            if SnippetId::parse(read).is_ok() {
                return Err(CompanyReadProblem::Snippet);
            }
            if DomainName::parse(read).is_ok() {
                return Err(CompanyReadProblem::Domain);
            }
        }
        let path = relative_to(reader, read).ok_or(CompanyReadProblem::OutsideDocs)?;
        if !path.starts_with(DOCS) {
            return Err(CompanyReadProblem::OutsideDocs);
        }
        if self.tree.is_link(&path) {
            return Err(CompanyReadProblem::Link);
        }
        if self.tree.file(&path).is_none() {
            return Err(CompanyReadProblem::Missing);
        }
        Ok(path)
    }

    fn found(&mut self, kind: RenderFindingKind) {
        self.findings.push(RenderFinding {
            path: self.path.to_path_buf(),
            kind,
        });
    }
}

/// `read`, a path relative to the file at `reader`, as a repository-relative
/// path; `None` for an absolute path, a home-relative one or one climbing
/// out of the repository.
fn relative_to(reader: &str, read: &str) -> Option<String> {
    if read.starts_with('/') || read.starts_with('~') || read.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = reader.split('/').collect();
    parts.pop();
    for part in read.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_resolves_relative_to_its_reader() {
        assert_eq!(
            relative_to("AGENTS.master.md", "docs/people.md").as_deref(),
            Some("docs/people.md")
        );
        assert_eq!(
            relative_to("docs/people.md", "roles.md").as_deref(),
            Some("docs/roles.md")
        );
        assert_eq!(
            relative_to("docs/teams/a.md", "../roles.md").as_deref(),
            Some("docs/roles.md")
        );
        assert_eq!(relative_to("docs/people.md", "../../x.md"), None);
        assert_eq!(relative_to("docs/people.md", "/etc/x.md"), None);
    }
}
