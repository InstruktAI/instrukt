//! Where a sync reads from: the project the caller stands in, its primary
//! worktree, its canonical main and the committed files of that commit.
//!
//! Publication never reads the working tree's documentation: every file it
//! publishes or compiles comes from the commit canonical main points at,
//! read through git's object store.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use super::PublicationError;
use crate::config::{self, CONFIG_FILE_NAME};
use crate::docs::SourceFile;
use crate::fsx::atomic_write;
use crate::git;
use crate::ids::{CommitSha, ProjectSlug};

/// The file name of a project master.
pub(super) const MASTER_FILE_NAME: &str = "AGENTS.master.md";

/// The project a sync publishes.
#[derive(Debug, Clone)]
pub(super) struct Project {
    pub slug: ProjectSlug,
    /// The root of the worktree the caller stands in.
    pub worktree: PathBuf,
    /// The project's primary worktree, canonical.
    pub primary: PathBuf,
}

/// Finds the project the caller in `cwd` stands in. A directory outside a
/// repository, without a project layer or with a layer declaring no slug,
/// is refused.
pub(super) fn locate(cwd: &Path) -> Result<Project, PublicationError> {
    let worktree = git::worktree_root(cwd)?.ok_or_else(|| PublicationError::NotInRepository {
        dir: cwd.to_path_buf(),
    })?;
    let layer = worktree.join(CONFIG_FILE_NAME);
    let slug = match config::project_slug(&worktree)? {
        Some(slug) => slug,
        None if layer.is_file() => return Err(PublicationError::NoProjectSlug { path: layer }),
        None => return Err(PublicationError::NoProjectLayer { path: layer }),
    };
    let primary = primary_worktree(&worktree)?;
    Ok(Project {
        slug,
        worktree,
        primary,
    })
}

/// The first worktree git lists: the repository's primary worktree.
fn primary_worktree(worktree: &Path) -> Result<PathBuf, PublicationError> {
    let listing = git::run(worktree, ["worktree", "list", "--porcelain"])?;
    let path = listing
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .ok_or_else(|| PublicationError::NoPrimaryWorktree {
            dir: worktree.to_path_buf(),
        })?;
    PathBuf::from(path)
        .canonicalize()
        .map_err(|source| PublicationError::Io {
            path: PathBuf::from(path),
            source,
        })
}

/// The default branch and the commit it stands at in the primary worktree.
/// The branch is `git::default_branch` through `origin`, the one rule for
/// "main" every context shares.
pub(super) fn canonical_main(primary: &Path) -> Result<(String, CommitSha), PublicationError> {
    let branch = git::default_branch(primary, "origin")?;
    if branch.is_empty() {
        return Err(PublicationError::NoDefaultBranch {
            dir: primary.to_path_buf(),
        });
    }
    let reference = format!("refs/heads/{branch}");
    let sha = git::run(
        primary,
        ["for-each-ref", "--format=%(objectname)", reference.as_str()],
    )?;
    let sha = sha.trim();
    if sha.is_empty() {
        return Err(PublicationError::MainHasNoCommit {
            dir: primary.to_path_buf(),
            branch,
        });
    }
    let commit = CommitSha::parse(sha).map_err(|_| PublicationError::MainHasNoCommit {
        dir: primary.to_path_buf(),
        branch: branch.clone(),
    })?;
    Ok((branch, commit))
}

/// The committed files of one commit that publication reads: the
/// documentation, the project layer, the global master and addenda under
/// `agents/`, and every master. Regular files only; symbolic links and
/// submodules are never followed.
#[derive(Debug, Clone, Default)]
pub(super) struct Committed {
    files: BTreeMap<String, Vec<u8>>,
}

impl Committed {
    /// Reads the files of `commit` in the repository at `repo`.
    pub(super) fn read(repo: &Path, commit: &CommitSha) -> Result<Self, PublicationError> {
        let listing = git::run_bytes(
            repo,
            ["ls-tree", "-r", "-z", "--full-tree", commit.as_str()],
        )?;
        let mut wanted: Vec<(String, String)> = Vec::new();
        for record in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let record = String::from_utf8_lossy(record);
            let Some((meta, path)) = record.split_once('\t') else {
                continue;
            };
            let mut meta = meta.split(' ');
            let (Some(mode), Some(kind), Some(object)) = (meta.next(), meta.next(), meta.next())
            else {
                continue;
            };
            if kind != "blob" || !matches!(mode, "100644" | "100755") || !is_read(path) {
                continue;
            }
            wanted.push((object.to_owned(), path.to_owned()));
        }
        let input: String = wanted
            .iter()
            .map(|(object, _)| format!("{object}\n"))
            .collect();
        let output = if wanted.is_empty() {
            Vec::new()
        } else {
            git::run_with_input(repo, ["cat-file", "--batch"], input.as_bytes())?
        };
        let mut files = BTreeMap::new();
        let mut rest = output.as_slice();
        for (object, path) in wanted {
            let (contents, after) =
                next_blob(rest).ok_or_else(|| PublicationError::UnreadableObject {
                    object: object.clone(),
                    path: path.clone(),
                })?;
            files.insert(path, contents.to_vec());
            rest = after;
        }
        Ok(Self { files })
    }

    pub(super) fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// The files below `prefix` (which ends with `/`), with their paths
    /// relative to it.
    pub(super) fn below<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a [u8])> + 'a {
        self.files
            .range(prefix.to_owned()..)
            .take_while(move |(path, _)| path.starts_with(prefix))
            .filter_map(move |(path, bytes)| Some((path.strip_prefix(prefix)?, bytes.as_slice())))
    }

    /// The Markdown files below `docs/`, as the docs model reads them.
    pub(super) fn doc_sources(&self) -> Vec<SourceFile> {
        self.below("docs/")
            .filter(|(rel, _)| rel.ends_with(".md"))
            .map(|(rel, bytes)| SourceFile {
                path: format!("docs/{rel}"),
                text: String::from_utf8_lossy(bytes).into_owned(),
            })
            .collect()
    }

    /// The repository-relative paths of every committed master.
    pub(super) fn masters(&self) -> Vec<&str> {
        self.files
            .keys()
            .filter(|path| is_master(path))
            .map(String::as_str)
            .collect()
    }

    /// Writes every file below `dir`, which is empty and owned by the
    /// caller.
    pub(super) fn materialize(&self, dir: &Path) -> Result<(), PublicationError> {
        for (path, bytes) in &self.files {
            let target = dir.join(safe_relative(path)?);
            atomic_write(&target, bytes)?;
        }
        Ok(())
    }
}

/// `path` as a relative path of normal components; anything else is
/// refused rather than written outside its root.
pub(super) fn safe_relative(path: &str) -> Result<PathBuf, PublicationError> {
    let rel = PathBuf::from(path);
    if path.is_empty()
        || !rel
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(PublicationError::UnsafePath {
            path: path.to_owned(),
        });
    }
    Ok(rel)
}

fn is_master(path: &str) -> bool {
    path.rsplit('/').next() == Some(MASTER_FILE_NAME)
}

fn is_read(path: &str) -> bool {
    path == CONFIG_FILE_NAME
        || path.starts_with("docs/")
        || path.starts_with("agents/")
        || is_master(path)
}

/// Splits one `cat-file --batch` record off `output`: its contents and
/// what follows it.
fn next_blob(output: &[u8]) -> Option<(&[u8], &[u8])> {
    let newline = output.iter().position(|b| *b == b'\n')?;
    let header = std::str::from_utf8(output.get(..newline)?).ok()?;
    let size: usize = header.rsplit(' ').next()?.parse().ok()?;
    let start = newline + 1;
    let contents = output.get(start..start + size)?;
    let after = output.get(start + size + 1..)?;
    Some((contents, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_output_splits_into_blobs() {
        let output = b"aaa blob 3\nabc\nbbb blob 0\n\n";
        let (first, rest) = next_blob(output).unwrap();
        assert_eq!(first, b"abc");
        let (second, rest) = next_blob(rest).unwrap();
        assert_eq!(second, b"");
        assert!(rest.is_empty());
    }

    #[test]
    fn paths_leaving_their_root_are_refused() {
        for path in ["../x", "/abs", "a/../b", "", "./a"] {
            assert!(safe_relative(path).is_err(), "{path}");
        }
        assert!(safe_relative("docs/global/a.md").is_ok());
    }
}
