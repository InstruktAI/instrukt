//! Which GitHub repository a project's work lives in, at which API
//! address, and with what credentials (`github-record`, "Writer and
//! convergence": repository and credentials). Resolution only: callers
//! bring their own client.
//!
//! - [`api_url`]: `github.api_url` from the machine's global layer only,
//!   never a repository's, because the token is sent there.
//! - [`repository`]: `github.repository`, else the remote the default
//!   branch tracks when it is a repository on the API's host.
//! - [`code_host`]: whether the project's code is on GitHub, which todos
//!   need; [`todos_need_github`] is the refusal when it is not.
//! - [`credentials`]: `GITHUB_TOKEN`, `GH_TOKEN`, then `gh auth token`;
//!   never stored.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::{ConfigError, Configuration, Env};
use crate::git::{self, GitError};
use crate::home::Home;

/// How long `gh auth token` may take.
const GH_TIMEOUT: Duration = Duration::from_secs(10);

/// The repository could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Git(#[from] GitError),
    /// Neither `github.repository` nor the tracked remote names one.
    #[error(
        "cannot tell which GitHub repository holds this project's work: {reason}; set `github.repository: <owner>/<repo>` in the project's instrukt.yaml"
    )]
    NoRepository { reason: String },
}

/// `github.api_url` of the machine's global layer (default
/// `https://api.github.com`). A project layer never chooses where the
/// token goes.
pub fn api_url(home: &Home) -> Result<String, ConfigError> {
    Ok(Configuration::load_global(home)?
        .github_api_url()?
        .to_owned())
}

/// Where a project's code is hosted, as todos need to know it
/// (`github-record`, "Code on GitHub").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeHost {
    /// The code is in this GitHub repository (`owner/repo`).
    GitHub { repository: String },
    /// The code is not on GitHub: why, naming the remote or
    /// `github.repository`.
    Elsewhere { reason: String },
}

/// The GitHub repository (`owner/repo`) of the project rooted at `root`
/// with effective configuration `config`: `github.repository`, or the
/// repository of the remote its default branch tracks when that remote is
/// on `api_url`'s host.
pub fn repository(
    config: &Configuration,
    api_url: &str,
    root: &Path,
) -> Result<String, ResolveError> {
    match code_host(config, api_url, root)? {
        CodeHost::GitHub { repository } => Ok(repository),
        CodeHost::Elsewhere { reason } => Err(ResolveError::NoRepository { reason }),
    }
}

/// Whether the code of the project rooted at `root` is on GitHub: the
/// remote its default branch tracks is a repository on `api_url`'s host,
/// or `github.repository` names the repository (taken as written).
pub fn code_host(
    config: &Configuration,
    api_url: &str,
    root: &Path,
) -> Result<CodeHost, ResolveError> {
    Ok(resolve_code_host(
        config.github_repository()?,
        api_url,
        root,
    )?)
}

/// [`code_host`] for a `github.repository` value read by the caller.
pub fn resolve_code_host(
    configured: Option<&str>,
    api_url: &str,
    root: &Path,
) -> Result<CodeHost, GitError> {
    let elsewhere = |reason: String| Ok(CodeHost::Elsewhere { reason });
    if let Some(configured) = configured {
        return match owner_repo(configured) {
            Some(repository) => Ok(CodeHost::GitHub { repository }),
            None => elsewhere(format!(
                "`github.repository: {configured}` is not `<owner>/<repo>`"
            )),
        };
    }
    let host = api_host(api_url);
    let branch = default_branch(root)?;
    let Some(remote) = git_value(root, &["config", &format!("branch.{branch}.remote")])? else {
        return elsewhere(format!(
            "the default branch `{branch}` tracks no remote and `github.repository` is not set"
        ));
    };
    let Some(url) = git_value(root, &["remote", "get-url", &remote])? else {
        return elsewhere(format!(
            "the remote `{remote}` has no URL and `github.repository` is not set"
        ));
    };
    match parse_remote_url(&url, &host) {
        Some(repository) => Ok(CodeHost::GitHub { repository }),
        None => elsewhere(format!(
            "the remote `{remote}` ({url}) is not a GitHub repository on {host} and `github.repository` is not set"
        )),
    }
}

/// The plain refusal when todos are on for a project whose code is not on
/// GitHub, for `reason` from [`CodeHost::Elsewhere`].
pub fn todos_need_github(reason: &str) -> String {
    format!(
        "todos need the project's code on GitHub: {reason}; when the remote reaches a GitHub repository through an address that does not read as one, name it with `github.repository: <owner>/<repo>` in the project's instrukt.yaml"
    )
}

/// The token to authenticate with: `GITHUB_TOKEN`, `GH_TOKEN`, then
/// `gh auth token`. `None` when none is available. Never stored.
pub fn credentials(env: &Env) -> Option<String> {
    for name in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Some(value) = env.get(name).map(str::trim).filter(|v| !v.is_empty()) {
            return Some(value.to_owned());
        }
    }
    gh_auth_token()
}

/// The project's default branch: the remote's `HEAD`, then
/// `init.defaultBranch`, then `main`.
pub fn default_branch(root: &Path) -> Result<String, GitError> {
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

/// A git query whose "no answer" is a non-zero exit with no output.
fn git_value(root: &Path, args: &[&str]) -> Result<Option<String>, GitError> {
    match git::run(root, args) {
        Ok(text) => {
            let text = text.trim();
            Ok((!text.is_empty()).then(|| text.to_owned()))
        }
        Err(GitError::Failed { stderr, .. }) if stderr.trim().is_empty() => Ok(None),
        Err(err) => Err(err),
    }
}

/// The web host whose repositories the API at `api_url` serves:
/// `api.github.com` serves `github.com`; any other address serves its own
/// host.
pub fn api_host(api_url: &str) -> String {
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
fn parse_remote_url(url: &str, host: &str) -> Option<String> {
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

    #[test]
    fn the_first_token_variable_set_is_the_credential() {
        let env = Env::from_pairs([("GH_TOKEN", "second"), ("GITHUB_TOKEN", "first")]);
        assert_eq!(credentials(&env).as_deref(), Some("first"));
        let env = Env::from_pairs([("GITHUB_TOKEN", " "), ("GH_TOKEN", "second")]);
        assert_eq!(credentials(&env).as_deref(), Some("second"));
    }
}
