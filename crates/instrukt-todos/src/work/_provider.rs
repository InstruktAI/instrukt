//! Where a project's work is reflected and with what credentials
//! (`github-record`, "Writer and convergence": repository and
//! credentials).

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use instrukt_core::config::{Configuration, Env};
use instrukt_core::git;

use super::_error::WorkError;

/// How long `gh auth token` may take.
const GH_TIMEOUT: Duration = Duration::from_secs(10);

/// The GitHub repository (`owner/repo`) of the project rooted at `root`:
/// `github.repository`, or the repository of the remote its default
/// branch tracks when that remote is on `api_url`'s host.
pub(crate) fn repository(config: &Configuration, root: &Path) -> Result<String, WorkError> {
    if let Some(configured) = config.github_repository()? {
        return owner_repo(configured).ok_or_else(|| WorkError::NoRepository {
            reason: format!("`github.repository: {configured}` is not `<owner>/<repo>`"),
        });
    }
    let host = api_host(config.github_api_url()?);
    let branch = default_branch(root)?;
    let remote =
        git_value(root, &["config", &format!("branch.{branch}.remote")])?.ok_or_else(|| {
            WorkError::NoRepository {
                reason: format!("the default branch `{branch}` tracks no remote"),
            }
        })?;
    let url = git_value(root, &["remote", "get-url", &remote])?.ok_or_else(|| {
        WorkError::NoRepository {
            reason: format!("the remote `{remote}` has no URL"),
        }
    })?;
    parse_remote_url(&url, &host).ok_or_else(|| WorkError::NoRepository {
        reason: format!("the remote `{remote}` is not a GitHub repository on {host}"),
    })
}

/// The token reflection authenticates with: `GITHUB_TOKEN`, `GH_TOKEN`,
/// then `gh auth token`. Never stored.
pub(crate) fn token(env: &Env) -> Option<String> {
    for name in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Some(value) = env.get(name).map(str::trim).filter(|v| !v.is_empty()) {
            return Some(value.to_owned());
        }
    }
    gh_auth_token()
}

/// `gh auth token`, bounded; absent when `gh` is missing, signed out or
/// slow.
fn gh_auth_token() -> Option<String> {
    let mut child = Command::new("gh")
        .args(["auth", "token"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut text = String::new();
                child.stdout.take()?.read_to_string(&mut text).ok()?;
                let token = text.trim();
                return (!token.is_empty()).then(|| token.to_owned());
            }
            Ok(None) if started.elapsed() < GH_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                if let Err(err) = child.kill() {
                    tracing::warn!(event = "gh_token_kill_failed", error = %err, "cannot stop `gh auth token`");
                }
                if let Err(err) = child.wait() {
                    tracing::warn!(event = "gh_token_wait_failed", error = %err, "cannot reap `gh auth token`");
                }
                tracing::warn!(
                    event = "gh_token_timeout",
                    "`gh auth token` did not answer in time"
                );
                return None;
            }
        }
    }
}

/// The default branch: the remote's `HEAD`, then `init.defaultBranch`,
/// then `main`.
pub(crate) fn default_branch(root: &Path) -> Result<String, WorkError> {
    let remotes = git::run(root, ["remote"])?;
    for remote in remotes.lines().map(str::trim).filter(|r| !r.is_empty()) {
        let head = format!("refs/remotes/{remote}/HEAD");
        if let Some(target) = git_value(root, &["symbolic-ref", "--quiet", &head])? {
            let prefix = format!("refs/remotes/{remote}/");
            if let Some(branch) = target.strip_prefix(&prefix) {
                return Ok(branch.to_owned());
            }
        }
    }
    Ok(git_value(root, &["config", "init.defaultBranch"])?.unwrap_or_else(|| "main".to_owned()))
}

/// A git query whose "no answer" is a non-zero exit with no output.
fn git_value(root: &Path, args: &[&str]) -> Result<Option<String>, WorkError> {
    match git::run(root, args) {
        Ok(text) => {
            let text = text.trim();
            Ok((!text.is_empty()).then(|| text.to_owned()))
        }
        Err(git::GitError::Failed { stderr, .. }) if stderr.trim().is_empty() => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// The web host whose repositories the API at `api_url` serves:
/// `api.github.com` serves `github.com`; any other address serves its own
/// host.
pub(crate) fn api_host(api_url: &str) -> String {
    let rest = api_url.split_once("://").map_or(api_url, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    match host.as_str() {
        "api.github.com" => "github.com".to_owned(),
        _ => host,
    }
}

/// `owner/repo` of a remote URL on `host`: `https://host/owner/repo(.git)`,
/// `ssh://git@host/owner/repo(.git)` or `git@host:owner/repo(.git)`.
pub(crate) fn parse_remote_url(url: &str, host: &str) -> Option<String> {
    let path = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let authority = authority.rsplit('@').next()?;
        let remote_host = authority.split(':').next()?;
        if !remote_host.eq_ignore_ascii_case(host) {
            return None;
        }
        path
    } else {
        let (authority, path) = url.split_once(':')?;
        let remote_host = authority.rsplit('@').next()?;
        if !remote_host.eq_ignore_ascii_case(host) {
            return None;
        }
        path
    };
    owner_repo(path.trim_end_matches('/').trim_end_matches(".git"))
}

fn owner_repo(text: &str) -> Option<String> {
    let (owner, repo) = text.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(repo)).then(|| format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_api_serves_github_com_and_others_serve_their_host() {
        assert_eq!(api_host("https://api.github.com"), "github.com");
        assert_eq!(api_host("https://ghe.example/api/v3"), "ghe.example");
        assert_eq!(api_host("http://127.0.0.1:4000"), "127.0.0.1");
    }

    #[test]
    fn remote_urls_of_the_host_name_their_repository() {
        for url in [
            "https://github.com/acme/demo.git",
            "https://github.com/acme/demo",
            "git@github.com:acme/demo.git",
            "ssh://git@github.com/acme/demo.git",
        ] {
            assert_eq!(
                parse_remote_url(url, "github.com").as_deref(),
                Some("acme/demo"),
                "{url}"
            );
        }
        assert_eq!(
            parse_remote_url("https://gitlab.com/acme/demo.git", "github.com"),
            None
        );
        assert_eq!(parse_remote_url("/srv/demo.git", "github.com"), None);
        assert_eq!(
            parse_remote_url("https://github.com/acme", "github.com"),
            None
        );
    }

    #[test]
    fn an_owner_repo_is_two_path_segments_without_traversal() {
        assert_eq!(owner_repo("acme/demo").as_deref(), Some("acme/demo"));
        for bad in ["acme", "acme/", "../demo", "acme/de mo", "a/b/c"] {
            assert_eq!(owner_repo(bad), None, "{bad}");
        }
    }
}
