//! Where a sync reads from: the project the caller stands in, its primary
//! worktree, its canonical main and the committed files of that commit.
//!
//! A sync never reads the working tree's documentation: every file it
//! publishes comes from the commit canonical main points at, read through
//! git's object store. The repository's committed entrypoints are rendered
//! from one view of the repository — a commit, or the working tree — read
//! as the same [`Committed`] file map.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use super::PublicationError;
use crate::config::{self, CONFIG_FILE_NAME};
use crate::docs::SourceFile;
use crate::fsx::atomic_write;
use crate::git;
use crate::ids::{CommitSha, ProjectSlug};

/// The file name of a project master.
pub(super) const MASTER_FILE_NAME: &str = "AGENTS.master.md";

/// The file a master renders into, and the file Codex reads in its place.
const ENTRYPOINT_FILE_NAMES: [&str; 2] = ["AGENTS.md", "AGENTS.override.md"];

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

/// The files of one view of a repository that publication reads: the
/// documentation, the root `instrukt.yaml`, every master and every
/// `AGENTS.md` and `AGENTS.override.md`. Regular files only; symbolic links
/// and submodules are never followed.
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
        Self::of_objects(repo, wanted)
    }

    /// Reads the files of the index a commit is being made from in the
    /// repository at `repo`: `index` (git's `GIT_INDEX_FILE` for the run),
    /// or the repository's own index when `None`. Stage-0 regular files
    /// only, as [`Committed::read`] reads a commit.
    pub(super) fn read_index(repo: &Path, index: Option<&Path>) -> Result<Self, PublicationError> {
        let listing = git::run_on_index(repo, index, ["ls-files", "--stage", "-z"], b"")?;
        let mut wanted: Vec<(String, String)> = Vec::new();
        for record in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let record = String::from_utf8_lossy(record);
            // `<mode> <object> <stage>\t<path>`.
            let Some((meta, path)) = record.split_once('\t') else {
                continue;
            };
            let mut meta = meta.split(' ');
            let (Some(mode), Some(object), Some(stage)) = (meta.next(), meta.next(), meta.next())
            else {
                continue;
            };
            if stage != "0" || !matches!(mode, "100644" | "100755") || !is_read(path) {
                continue;
            }
            wanted.push((object.to_owned(), path.to_owned()));
        }
        Self::of_objects(repo, wanted)
    }

    /// The blobs `wanted` names, `(object, path)`, read through one
    /// `cat-file --batch` in the repository at `repo`.
    fn of_objects(repo: &Path, wanted: Vec<(String, String)>) -> Result<Self, PublicationError> {
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

    /// The files of the working tree whose root is `root`: in a git
    /// working tree, the tracked files and the untracked ones git does not
    /// ignore, with the root `instrukt.yaml` and every `AGENTS.md` and
    /// `AGENTS.override.md` read whether ignored or not; elsewhere every
    /// file below `root`.
    pub(super) fn working_tree(root: &Path) -> Result<Self, PublicationError> {
        let mut paths: BTreeSet<String> = BTreeSet::new();
        if git::worktree_root(root)?.is_some() {
            let listed = |args: &[&str]| -> Result<Vec<String>, PublicationError> {
                let mut all = vec!["ls-files", "-z", "--cached", "--others"];
                all.extend_from_slice(args);
                let bytes = git::run_bytes(root, all)?;
                Ok(bytes
                    .split(|b| *b == 0)
                    .filter(|p| !p.is_empty())
                    .map(|p| String::from_utf8_lossy(p).into_owned())
                    .collect())
            };
            paths.extend(listed(&["--exclude-standard"])?);
            let mut entrypoints = vec!["--"];
            let patterns: Vec<String> = ENTRYPOINT_FILE_NAMES
                .iter()
                .flat_map(|name| [(*name).to_owned(), format!("*/{name}")])
                .collect();
            entrypoints.extend(patterns.iter().map(String::as_str));
            paths.extend(listed(&entrypoints)?);
            paths.insert(CONFIG_FILE_NAME.to_owned());
        } else {
            for entry in walkdir::WalkDir::new(root)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| e.file_name() != ".git")
            {
                let entry = entry.map_err(|err| PublicationError::Io {
                    path: err.path().unwrap_or(root).to_path_buf(),
                    source: err.into(),
                })?;
                if let Ok(rel) = entry.path().strip_prefix(root)
                    && let Some(rel) = rel.to_str()
                {
                    paths.insert(rel.replace(std::path::MAIN_SEPARATOR, "/"));
                }
            }
        }
        let mut files = BTreeMap::new();
        for path in paths.into_iter().filter(|path| is_read(path)) {
            let full = root.join(safe_relative(&path)?);
            match std::fs::symlink_metadata(&full) {
                Ok(meta) if meta.is_file() => {}
                Ok(_) => continue,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => return Err(PublicationError::Io { path: full, source }),
            }
            let bytes = std::fs::read(&full)
                .map_err(|source| PublicationError::Io { path: full, source })?;
            files.insert(path, bytes);
        }
        Ok(Self { files })
    }

    /// A view holding exactly `files`, by repository-relative path.
    #[cfg(test)]
    pub(super) fn of(files: &[(&str, &str)]) -> Self {
        Self {
            files: files
                .iter()
                .map(|(path, text)| ((*path).to_owned(), text.as_bytes().to_vec()))
                .collect(),
        }
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

    /// Every `AGENTS.md` of the view, by repository-relative path, with
    /// its bytes.
    pub(super) fn entrypoints(&self) -> impl Iterator<Item = (&str, &[u8])> + '_ {
        self.files
            .iter()
            .filter(|(path, _)| path.rsplit('/').next() == Some(ENTRYPOINT_FILE_NAMES[0]))
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
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
        || is_master(path)
        || path
            .rsplit('/')
            .next()
            .is_some_and(|name| ENTRYPOINT_FILE_NAMES.contains(&name))
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
        assert!(safe_relative("docs/general/a.md").is_ok());
    }
}
