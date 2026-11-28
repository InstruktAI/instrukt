//! Work's GitHub boundary for company knowledge proposals
//! (`project/instrukt/spec/domain/work`, knowledge proposals): whether a
//! commit heads any pull request, and opening a proposal's pull request
//! under the proposal credential, asking first so that a repeated tick or a
//! crash between the two calls never opens a second one. The runner decides
//! when.

use instrukt_core::git;
use instrukt_core::ids::CommitSha;
use instrukt_runner::interfaces::{ProposalOpening, ProposalPull};
use jiff::Timestamp;
use serde_json::json;

use super::GitHubAccess;

/// Whether any pull request of `repository`, open or closed, has `head` as
/// its head commit.
pub(super) fn heads_pull(
    access: &GitHubAccess,
    repository: &str,
    head: &CommitSha,
    clock: &dyn Fn() -> Timestamp,
) -> Result<bool, String> {
    let gh = access
        .proposal_client(repository, clock)
        .map_err(|error| error.to_string())?;
    let pulls = gh
        .list(&format!("/commits/{head}/pulls"))
        .map_err(|error| error.to_string())?;
    Ok(pulls
        .iter()
        .any(|pull| pull["head"]["sha"].as_str() == Some(head.as_str())))
}

/// Opens `proposal`'s pull request, unless one with its branch as head is
/// open: a fixed-wording title naming the branch, and a body of fixed words
/// followed by the commits' messages in a fenced block.
pub(super) fn open(
    access: &GitHubAccess,
    proposal: &ProposalPull<'_>,
    clock: &dyn Fn() -> Timestamp,
) -> Result<ProposalOpening, String> {
    let gh = access
        .proposal_client(proposal.repository, clock)
        .map_err(|error| error.to_string())?;
    let owner = proposal.repository.split('/').next().unwrap_or_default();
    let head = super::_reflect::encode(&format!("{owner}:{}", proposal.branch));
    let open = gh
        .list(&format!("/pulls?head={head}&state=open"))
        .map_err(|error| error.to_string())?;
    if !open.is_empty() {
        return Ok(ProposalOpening::AlreadyOpen);
    }
    let range = format!("{}..{}", proposal.canonical, proposal.head);
    let messages = git::run(proposal.root, ["log", "--format=%B", range.as_str()])
        .map_err(|error| error.to_string())?;
    let body = format!(
        "This change to the company's knowledge was asked of the company's agent by an administrator of the company. Its commits:\n\n{}\n",
        instrukt_core::docs::fenced(&messages)
    );
    let created = gh
        .post(
            "/pulls",
            &json!({
                "title": format!("Knowledge proposal: {}", proposal.branch),
                "head": proposal.branch,
                "base": proposal.base,
                "body": body,
            }),
        )
        .map_err(|error| error.to_string())?;
    created["number"]
        .as_u64()
        .map(ProposalOpening::Opened)
        .ok_or_else(|| "GitHub answered the opening without a pull request number".to_owned())
}
