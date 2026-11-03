//! The `git` CLI as every context reaches it.
//!
//! instrukt talks to git only through the `git` executable (architecture
//! overview, "External surfaces"). This module runs it in a given directory
//! and returns its output; what a context asks of git stays in that context.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
        return Err(GitError::Failed {
            dir: dir.to_path_buf(),
            args: args_text,
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
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

/// Runs `git <args>` in `dir` and reports only whether it exited zero —
/// for git's predicate commands (`merge-base --is-ancestor`, `diff --quiet`).
pub fn succeeds<I, S>(dir: &Path, args: I) -> Result<bool, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let (_, output) = raw(dir, args)?;
    Ok(output.status.success())
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
        return Err(GitError::Failed {
            dir: dir.to_path_buf(),
            args: args_text,
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output.stdout)
}

/// The top-level directory of the working tree containing `dir`, or `None`
/// outside any git working tree.
pub fn worktree_root(dir: &Path) -> Result<Option<PathBuf>, GitError> {
    let (_, output) = raw(dir, ["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8(output.stdout).map_err(|_| GitError::NotUtf8 {
        dir: dir.to_path_buf(),
        args: "rev-parse --show-toplevel".to_owned(),
    })?;
    Ok(Some(PathBuf::from(text.trim_end_matches('\n'))))
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
    let output = Command::new("git")
        .args(&args)
        .current_dir(dir)
        .output()
        .map_err(|source| GitError::Spawn {
            dir: dir.to_path_buf(),
            source,
        })?;
    Ok((args_text, output))
}
