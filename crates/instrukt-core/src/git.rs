//! The `git` CLI as every context reaches it.
//!
//! instrukt talks to git only through the `git` executable (architecture
//! overview, "External surfaces"). This module runs it in a given directory
//! and returns its output; what a context asks of git stays in that context.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The variables through which git locates a repository (`git rev-parse
/// --local-env-vars`). Git exports them to hooks; every call here names its
/// directory instead, so inherited values would aim it at another
/// repository.
const REPOSITORY_ENV: [&str; 15] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// A git invocation failed to start or exited non-zero.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("cannot run git in {dir}: {source}")]
    Spawn {
        dir: PathBuf,
        source: std::io::Error,
    },
    #[error("`git {args}` failed in {dir} ({status}): {stderr}")]
    Failed {
        dir: PathBuf,
        args: String,
        status: String,
        stderr: String,
    },
    #[error("`git {args}` printed non-UTF-8 output in {dir}")]
    NotUtf8 { dir: PathBuf, args: String },
}

/// Runs `git <args>` in `dir`; returns stdout with one trailing newline
/// removed. A non-zero exit is an error carrying stderr.
pub fn run<I, S>(dir: &Path, args: I) -> Result<String, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let (args_text, output) = raw(dir, args)?;
    if !output.status.success() {
        return Err(failed(dir, args_text, &output));
    }
    let mut text = String::from_utf8(output.stdout).map_err(|_| GitError::NotUtf8 {
        dir: dir.to_path_buf(),
        args: args_text,
    })?;
    if text.ends_with('\n') {
        text.pop();
    }
    Ok(text)
}

/// Runs one of git's predicate commands (`merge-base --is-ancestor`,
/// `diff --quiet`) in `dir`: exit 0 is `true`, exit 1 is `false`, any other
/// status is an error carrying stderr.
pub fn succeeds<I, S>(dir: &Path, args: I) -> Result<bool, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let (args_text, output) = raw(dir, args)?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(failed(dir, args_text, &output)),
    }
}

/// Runs `git <args>` in `dir` and returns stdout as bytes, for blobs that
/// need not be text.
pub fn run_bytes<I, S>(dir: &Path, args: I) -> Result<Vec<u8>, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let (args_text, output) = raw(dir, args)?;
    if !output.status.success() {
        return Err(failed(dir, args_text, &output));
    }
    Ok(output.stdout)
}

/// The top-level directory of the working tree containing `dir`, or `None`
/// when git positively reports that `dir` is not inside a working tree.
/// Any other failure (a corrupt configuration, an unreadable repository) is
/// an error.
pub fn worktree_root(dir: &Path) -> Result<Option<PathBuf>, GitError> {
    let args = ["rev-parse", "--show-toplevel"];
    let (args_text, output) = raw(dir, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not a git repository")
            || stderr.contains("this operation must be run in a work tree")
        {
            return Ok(None);
        }
        return Err(failed(dir, args_text, &output));
    }
    let text = String::from_utf8(output.stdout).map_err(|_| GitError::NotUtf8 {
        dir: dir.to_path_buf(),
        args: args_text,
    })?;
    Ok(Some(PathBuf::from(text.trim_end_matches('\n'))))
}

fn failed(dir: &Path, args: String, output: &Output) -> GitError {
    GitError::Failed {
        dir: dir.to_path_buf(),
        args,
        status: output.status.to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn raw<I, S>(dir: &Path, args: I) -> Result<(String, Output), GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<_> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
    let args_text = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let mut command = Command::new("git");
    for var in REPOSITORY_ENV {
        command.env_remove(var);
    }
    let output = command
        .args(&args)
        .current_dir(dir)
        .output()
        .map_err(|source| GitError::Spawn {
            dir: dir.to_path_buf(),
            source,
        })?;
    Ok((args_text, output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), ["init", "--quiet"]).unwrap();
        dir
    }

    #[test]
    fn a_predicate_answers_true_false_and_errs_on_anything_else() {
        let dir = repo();
        assert!(succeeds(dir.path(), ["diff", "--quiet"]).unwrap());
        std::fs::write(dir.path().join("f"), "x").unwrap();
        run(dir.path(), ["add", "f"]).unwrap();
        assert!(!succeeds(dir.path(), ["diff", "--cached", "--quiet"]).unwrap());
        let err = succeeds(
            dir.path(),
            ["merge-base", "--is-ancestor", "no-such-rev", "HEAD"],
        );
        assert!(matches!(err, Err(GitError::Failed { .. })), "{err:?}");
    }

    #[test]
    fn worktree_root_is_none_only_outside_a_repository() {
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(worktree_root(outside.path()).unwrap(), None);
        let dir = repo();
        let root = worktree_root(dir.path()).unwrap().unwrap();
        assert_eq!(
            root.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
        std::fs::write(dir.path().join(".git/config"), "[broken").unwrap();
        assert!(worktree_root(dir.path()).is_err());
    }
}
