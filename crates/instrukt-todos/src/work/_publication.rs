//! Live publication of a todo's work branch (`github-record`, Issues and
//! visible work): on runner ticks, the commits builders land on the
//! branch are pushed to the remote the project's default branch
//! tracks, fast-forward only, so the todo's pull request shows them while
//! the work is built and reviewed. Agent commands never push.

use std::path::Path;
use std::time::Duration;

use instrukt_core::process::{Invocation, ProcessError};

/// How long one push may take before it is stopped and its process group
/// reaped; the next tick tries again.
const PUSH_TIMEOUT: Duration = Duration::from_secs(120);

/// What one push of a work branch came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Pushed {
    /// The remote branch holds the local head.
    Published,
    /// The remote branch moved where the local one cannot fast-forward it;
    /// the remote is left as it is.
    Diverged,
    /// The remote could not be reached, or the push did not finish: why.
    Unreached(String),
}

/// Pushes `refs/heads/<branch>` of the repository at `root` to the same
/// branch of `remote`, without force and without writing any upstream
/// configuration, through the operator's own git transport.
pub(crate) fn push(root: &Path, remote: &str, branch: &str) -> Pushed {
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    // git has no in-process binding here; the push runs as a bounded
    // child process whose process group is reaped on timeout.
    let done = Invocation::program("git", &["push", "--porcelain", "--", remote, &refspec])
        .env("GIT_TERMINAL_PROMPT", "0")
        .cwd(root)
        .run(PUSH_TIMEOUT);
    match done {
        Ok(done) if done.success => Pushed::Published,
        Ok(done) if rejected_as_diverged(&done.stdout) => Pushed::Diverged,
        Ok(done) => Pushed::Unreached(done.stderr.trim().to_owned()),
        Err(error @ (ProcessError::Timeout { .. } | ProcessError::Io { .. })) => {
            Pushed::Unreached(error.to_string())
        }
        Err(error @ ProcessError::Spawn { .. }) => Pushed::Unreached(error.to_string()),
    }
}

/// Whether `git push --porcelain` output reports a ref rejected because the
/// remote holds commits the pushed one does not: a `!` line naming
/// `non-fast-forward` or `fetch first`.
pub(crate) fn rejected_as_diverged(porcelain: &str) -> bool {
    porcelain.lines().any(|line| {
        line.starts_with('!')
            && (line.contains("(non-fast-forward)") || line.contains("(fetch first)"))
    })
}

#[cfg(test)]
mod tests {
    use super::rejected_as_diverged;

    #[test]
    fn a_non_fast_forward_rejection_is_a_diverged_branch() {
        let out = "To /srv/demo.git\n!\trefs/heads/a/work:refs/heads/a/work\t[rejected] (non-fast-forward)\nDone\n";
        assert!(rejected_as_diverged(out));
        let fetch = "To /srv/demo.git\n!\trefs/heads/a/work:refs/heads/a/work\t[rejected] (fetch first)\nDone\n";
        assert!(rejected_as_diverged(fetch));
    }

    #[test]
    fn other_failures_are_not_a_diverged_branch() {
        let hook = "To /srv/demo.git\n!\trefs/heads/a/work:refs/heads/a/work\t[remote rejected] (pre-receive hook declined)\nDone\n";
        assert!(!rejected_as_diverged(hook));
        assert!(!rejected_as_diverged(""));
    }
}
