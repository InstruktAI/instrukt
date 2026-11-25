//! Reflection of local items to GitHub (`github-record`): the local
//! record is the one authority, and GitHub shows it one way — the issue,
//! its output labels, the native artifacts, the pull request and its
//! reviews, and the board. Nothing read from GitHub changes the record
//! except people's controls on the issue (`/instrukt` commands and the
//! `hold` label), which the session applies once each. Retries recognize
//! writes that already landed: an issue by the identity its body names, an
//! artifact version by its digest, a review by its body and commit.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use instrukt_core::config::BoardRef;

use super::_envelope::{self, TODO_LABEL};
use super::_github::{Fetched, GitHub, GitHubError};
use super::_model::{TodoRecord, digest_of};
use super::_store::{IssueRef, KeptRefusal, LocalItem, ReflectionState, RefusedTarget};
use crate::evidence::{StampFact, TodoStatus};

/// The `/instrukt` command comments not yet answered.
fn commands(comments: &[Value], seen: &[u64]) -> Vec<Value> {
    comments
        .iter()
        .filter(|c| {
            c["body"]
                .as_str()
                .is_some_and(|b| b.trim_start().starts_with(super::_inbound::COMMAND))
                && c["id"].as_u64().is_some_and(|id| !seen.contains(&id))
        })
        .cloned()
        .collect()
}

/// Answers on the todo's issue. A reply already posted — by an author who
/// can write to the repository or by the configured bot, as its verified
/// identity — is recognized by its text among the comments updated since
/// `since`, the command's time (the whole conversation without it), and
/// not posted twice.
pub(crate) fn reply(
    gh: &GitHub,
    issue: IssueRef,
    source: u64,
    since: Option<&str>,
    text: &str,
) -> Result<(), GitHubError> {
    let text = format!("{text}\n\n<!-- instrukt:command-reply:{source} -->");
    let comments = gh.list(&comments_path(issue, since))?;
    let bot = gh.bot().ok();
    if comments.iter().any(|comment| {
        (has_write_access(comment)
            || bot
                .as_ref()
                .is_some_and(|bot| bot.matches(&comment["user"])))
            && comment["body"].as_str() == Some(&text)
    }) {
        return Ok(());
    }
    gh.post(
        &format!("/issues/{}/comments", issue.number),
        &json!({ "body": text }),
    )
    .map(|_| ())
}

/// The path of the comments of `issue` updated since `since`, all of them
/// without it.
fn comments_path(issue: IssueRef, since: Option<&str>) -> String {
    match since {
        Some(since) => format!("/issues/{}/comments?since={}", issue.number, encode(since)),
        None => format!("/issues/{}/comments", issue.number),
    }
}

/// What a read of a changed issue found: the issue, and the comments
/// updated since the stored position as (identity, update time), in the
/// order GitHub lists them. Its `/instrukt` commands not yet answered are
/// `inbound` on the reflection read; its `ETag` is the reflection's
/// `etag`, kept only once every control read is applied or answered.
pub(crate) struct IssueRead {
    pub issue: Value,
    pub comments: Vec<(u64, jiff::Timestamp)>,
}

/// Reads the item's issue for people's controls: a conditional request
/// (an unchanged issue is not read further, `None`), then the comments
/// updated since the stored position. A gone issue leaves the item to be
/// reflected again by the next flush. Nothing read changes the record.
pub(crate) fn read_issue(
    gh: &GitHub,
    reflection: &mut ReflectionState,
) -> Result<Option<IssueRead>, GitHubError> {
    let Some(issue) = reflection.issue else {
        return Ok(None);
    };
    let (body, etag) = match gh.get(
        &format!("/issues/{}", issue.number),
        reflection.etag.as_deref(),
    ) {
        Ok(Fetched::NotModified) => return Ok(None),
        Ok(Fetched::Fresh { body, etag }) => (body, etag),
        Err(GitHubError::NotFound { .. }) => {
            // The issue is gone; the next flush reflects the item again.
            *reflection = ReflectionState {
                pending: true,
                ..ReflectionState::default()
            };
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    let path = comments_path(issue, reflection.comments_since.as_deref());
    let listed = gh.list(&path)?;
    let mut comments = Vec::with_capacity(listed.len());
    for comment in &listed {
        let (Some(id), Some(at)) = (
            comment["id"].as_u64(),
            comment["updated_at"]
                .as_str()
                .and_then(|at| at.parse::<jiff::Timestamp>().ok()),
        ) else {
            return Err(GitHubError::Decode {
                path,
                message: "a comment without an id or update time".to_owned(),
            });
        };
        comments.push((id, at));
    }
    reflection.inbound = commands(&listed, &reflection.commands_seen);
    reflection.etag = etag;
    Ok(Some(IssueRead {
        issue: body,
        comments,
    }))
}

/// The events of `issue` from the stored position on, as (page, event)
/// in order: from the stored page while it still holds the last event
/// read, else once in full from the first page.
pub(crate) fn read_events(
    gh: &GitHub,
    issue: IssueRef,
    reflection: &ReflectionState,
) -> Result<Vec<(u64, Value)>, GitHubError> {
    use super::_positions::{EVENTS_PAGE, resumes_at};
    let read_page = |page: u64| -> Result<Vec<Value>, GitHubError> {
        let path = format!(
            "/issues/{}/events?per_page={EVENTS_PAGE}&page={page}",
            issue.number
        );
        match gh.read(&path)? {
            Value::Array(events) => Ok(events),
            _ => Err(GitHubError::Decode {
                path,
                message: "expected a list".to_owned(),
            }),
        }
    };
    let mut page = reflection.events_page.unwrap_or(1).max(1);
    let mut events = read_page(page)?;
    let ids: Vec<u64> = events.iter().filter_map(|e| e["id"].as_u64()).collect();
    if !resumes_at(page, &ids, reflection.events_last) {
        page = 1;
        events = read_page(page)?;
    }
    let mut read = Vec::new();
    loop {
        let full = events.len() as u64 >= EVENTS_PAGE;
        read.extend(events.into_iter().map(|event| (page, event)));
        if !full {
            return Ok(read);
        }
        page += 1;
        events = read_page(page)?;
    }
}

/// Writes the item's pending changes to GitHub: its issue, any artifact
/// versions not yet uploaded and the issue's visible fields; then the
/// linked pull request and the board. No comment is read or written: the
/// record lives on this computer only.
pub(crate) fn flush(
    gh: &GitHub,
    item: &mut LocalItem,
    root: &std::path::Path,
    base: &str,
    board: Option<&BoardRef>,
    needs_you: bool,
) -> Result<(), GitHubError> {
    if item.reflection.pending {
        let Some(issue) = ensure_issue(gh, item)? else {
            // GitHub refused the issue itself: kept with its reason, and
            // not sent again until the record changes it.
            item.reflection.pending = false;
            return Ok(());
        };
        upload_artifacts(gh, item, issue)?;
        let mut fields = _envelope::issue(&item.record);
        let snapshot = gh.read(&format!("/issues/{}", issue.number))?;
        if let Some(labels) = snapshot["labels"].as_array() {
            fields.labels.extend(
                labels
                    .iter()
                    .filter_map(|l| l["name"].as_str())
                    .filter(|l| !output_label(l))
                    .map(str::to_owned),
            );
        }
        let path = format!("/issues/{}", issue.number);
        let body = fields.update_json();
        send(
            &mut item.reflection,
            RefusedTarget::Issue,
            request_identity("PATCH", &path, &body, None),
            || gh.patch(&path, &body),
        )?;
        item.reflection.etag = None;
        item.reflection.pending = false;
        item.reflection.failure = None;
        item.reflection.remote_revision = Some(item.record.revision);
        tracing::debug!(
            event = "work_reflected",
            todo = %item.record.slug,
            issue = issue.number,
            revision = item.record.revision,
            "work record reflected"
        );
    }
    if pull_request_owed(item)
        && let Some(issue) = item.reflection.issue
    {
        reflect_pull_request(gh, item, issue, root, base)?;
    }
    if let (Some(board), Some(issue)) = (board, item.reflection.issue) {
        reflect_board(gh, board, item, issue, needs_you)?;
    }
    Ok(())
}

/// A board's identity as membership records it: `<owner>/<number>`.
fn board_key(board: &BoardRef) -> String {
    format!("{}/{}", board.owner, board.number)
}

/// Puts the issue on the board with the record's status. A failure is kept
/// for the next attempt and never blocks the issue or the record; only a
/// GitHub that stopped answering ends the command's reflection.
fn reflect_board(
    gh: &GitHub,
    board: &BoardRef,
    item: &mut LocalItem,
    issue: IssueRef,
    needs_you: bool,
) -> Result<(), GitHubError> {
    let status = _envelope::record_status(&item.record).as_str();
    let attempt = (|| {
        let content = gh.read(&format!("/issues/{}", issue.number))?["node_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| GitHubError::Decode {
                path: format!("/issues/{}", issue.number),
                message: "an issue without a node id".to_owned(),
            })?;
        let account = gh.for_role(None, instrukt_core::github::Operation::Board);
        super::_board::reflect(&account, board, &content, status, needs_you)
    })();
    match attempt {
        Ok(board_item) => {
            item.reflection.board_item = Some(board_item);
            item.reflection.board = Some(board_key(board));
            item.reflection.board_status = Some(status.to_owned());
            item.reflection.board_failure = None;
            Ok(())
        }
        Err(
            err @ (GitHubError::RateLimited { .. }
            | GitHubError::Unreachable(_)
            | GitHubError::App(
                instrukt_core::github::AppError::RateLimited { .. }
                | instrukt_core::github::AppError::Unreachable,
            )),
        ) => {
            item.reflection.board_failure = Some(err.to_string());
            Err(err)
        }
        Err(err) => {
            tracing::warn!(
                event = "work_board_pending",
                todo = %item.record.slug,
                error = %err,
                "board reflection failed; it is retried on its own"
            );
            item.reflection.board_failure = Some(err.to_string());
            Ok(())
        }
    }
}

/// Whether the linked pull request still owes creation, a review or its
/// closing (`github-record`: "If the branch is not available on GitHub,
/// creation remains pending").
pub(crate) fn pull_request_owed(item: &LocalItem) -> bool {
    let record = &item.record;
    if !_envelope::is_repository_work(record.kind)
        || record.branch.is_none()
        || record.status == TodoStatus::Removed
    {
        return false;
    }
    match item.reflection.pull {
        None => true,
        Some(_) => {
            build_verdicts(record)
                .iter()
                .any(|v| !item.reflection.reviews.contains(&v.sequence))
                || (record.delivery.is_some() && !item.reflection.pull_closed)
        }
    }
}

/// The item's issue, created when GitHub holds none yet; `None` while
/// GitHub refuses its creation.
fn ensure_issue(gh: &GitHub, item: &mut LocalItem) -> Result<Option<IssueRef>, GitHubError> {
    if let Some(issue) = item.reflection.issue {
        return Ok(Some(issue));
    }
    let issue = match find_issue(gh, &item.record)? {
        Some(issue) => issue,
        None => {
            let body = _envelope::issue(&item.record).create_json();
            let sent = send(
                &mut item.reflection,
                RefusedTarget::Issue,
                request_identity("POST", "/issues", &body, None),
                || gh.post("/issues", &body),
            )?;
            let Sent::Accepted(created) = sent else {
                return Ok(None);
            };
            issue_ref(&created, "/issues")?
        }
    };
    item.reflection.issue = Some(issue);
    Ok(Some(issue))
}

/// What sending one request to a reflected target came to.
enum Sent<T> {
    /// GitHub accepted it, with its answer.
    Accepted(T),
    /// GitHub refused it as invalid, now or before: kept with its reason.
    Refused,
}

/// Sends a request to a reflected `target` through `request`, unless the
/// target's kept refusal has the same `identity`: content GitHub refused is
/// not sent again until it changes (SC-12). A refused answer is kept for
/// the target with GitHub's detail, and an accepted one drops the target's
/// refusal. Any other failure is the request's and returned.
fn send<T>(
    reflection: &mut ReflectionState,
    target: RefusedTarget,
    identity: String,
    request: impl FnOnce() -> Result<T, GitHubError>,
) -> Result<Sent<T>, GitHubError> {
    if refused_before(&reflection.refusals, &target, &identity) {
        return Ok(Sent::Refused);
    }
    match request() {
        Ok(answer) => {
            reflection.refusals.retain(|kept| kept.target != target);
            Ok(Sent::Accepted(answer))
        }
        Err(GitHubError::Invalid { detail, path, .. }) => {
            tracing::warn!(
                event = "work_reflection_refused",
                path,
                detail,
                "GitHub refused reflected content; it is not sent again until it changes"
            );
            reflection.refusals.retain(|kept| kept.target != target);
            reflection.refusals.push(KeptRefusal {
                target,
                identity,
                detail,
            });
            Ok(Sent::Refused)
        }
        Err(err) => Err(err),
    }
}

/// Whether `refusals` keeps a refusal of `target` for a request with
/// `identity`.
fn refused_before(refusals: &[KeptRefusal], target: &RefusedTarget, identity: &str) -> bool {
    refusals
        .iter()
        .any(|kept| &kept.target == target && kept.identity == identity)
}

/// The identity of a request: a digest of its method, path and body, and of
/// the branch head it concerns when it concerns one.
fn request_identity(method: &str, path: &str, body: &Value, head: Option<&str>) -> String {
    digest_of(&format!(
        "{method} {path}\n{}\n{}",
        head.unwrap_or_default(),
        body
    ))
    .as_str()
    .to_owned()
}

/// The issue already reflecting `record` (a retry after an answer that
/// never arrived), found by the identity its body names.
fn find_issue(gh: &GitHub, record: &TodoRecord) -> Result<Option<IssueRef>, GitHubError> {
    let issues = gh.list(&format!("/issues?labels={TODO_LABEL}&state=all"))?;
    for issue in issues {
        if issue["body"].as_str().and_then(_envelope::issue_identity) == Some(record.id.clone()) {
            return issue_ref(&issue, "/issues").map(Some);
        }
    }
    Ok(None)
}

fn issue_ref(value: &Value, path: &str) -> Result<IssueRef, GitHubError> {
    match (value["number"].as_u64(), value["id"].as_u64()) {
        (Some(number), Some(id)) => Ok(IssueRef { number, id }),
        _ => Err(GitHubError::Decode {
            path: path.to_owned(),
            message: "an issue without number or id".to_owned(),
        }),
    }
}

/// `author_association` values of authors who can write to the
/// repository: a command reply already posted by one of them is recognized
/// as posted. Who may command is decided apart, by `github.approvers` and
/// GitHub's answer on write access.
pub(crate) const TRUSTED_ASSOCIATIONS: [&str; 3] = ["OWNER", "MEMBER", "COLLABORATOR"];

/// Whether the comment's author can write to the repository.
pub(crate) fn has_write_access(comment: &Value) -> bool {
    TRUSTED_ASSOCIATIONS.contains(&comment["author_association"].as_str().unwrap_or_default())
}

/// Uploads every artifact version not yet on GitHub and learns the
/// provider's version numbers by digest. An upload to an artifact whose
/// address the record holds reads no earlier version: it is confirmed by
/// GitHub's newest version alone, so the cost does not grow with the
/// artifact's history (SC-11). The versions newer than the last one
/// confirmed are read only to recover: for an address just found on the
/// issue (a creation whose answer was lost), whose versions a retry then
/// reuses instead of uploading them twice, and for a confirmation that
/// does not match.
fn upload_artifacts(gh: &GitHub, item: &mut LocalItem, issue: IssueRef) -> Result<(), GitHubError> {
    let number = issue.number;
    let mut listed: Option<Vec<Value>> = None;
    for artifact in &mut item.record.artifacts {
        if artifact.versions.iter().all(|v| v.remote.is_some()) {
            continue;
        }
        let recovered = artifact.address.is_none();
        if recovered {
            let existing = match &listed {
                Some(list) => list.clone(),
                None => {
                    let list = gh.list(&format!("/issues/{number}/artifacts"))?;
                    listed = Some(list.clone());
                    list
                }
            };
            artifact.address = existing.iter().find_map(|a| {
                (a["name"].as_str() == Some(artifact.name.as_str()))
                    .then(|| a["number"].as_u64())
                    .flatten()
            });
        }
        let confirmed = artifact
            .versions
            .iter()
            .filter_map(|v| v.remote)
            .max()
            .unwrap_or(0);
        let mut remote = match artifact.address {
            Some(address) if recovered => remote_versions(gh, number, address, confirmed)?,
            _ => Vec::new(),
        };
        // Versions keep their order: a version maps to a remote version
        // after the one its predecessor maps to.
        let mut floor = 0;
        for version in &mut artifact.versions {
            if let Some(n) = version.remote {
                floor = floor.max(n);
                continue;
            }
            if let Some((n, _)) = remote
                .iter()
                .find(|(n, digest)| *n > floor && *digest == version.digest.as_str())
            {
                version.remote = Some(*n);
                floor = *n;
                continue;
            }
            let Some(content) = version.content.clone() else {
                item.reflection.failure = Some(format!(
                    "artifact `{}` holds a version whose content this computer does not have",
                    artifact.name
                ));
                break;
            };
            let target = RefusedTarget::Artifact {
                name: artifact.name.clone(),
                sequence: version.sequence,
            };
            let sent = match artifact.address {
                None => {
                    let path = format!("/issues/{number}/artifacts");
                    let body = json!({
                        "type": _envelope::artifact_type(artifact.name.as_str()),
                        "name": artifact.name.as_str(),
                        "body": content,
                    });
                    let identity = request_identity("POST", &path, &body, None);
                    match send(&mut item.reflection, target, identity, || {
                        gh.post(&path, &body)
                    })? {
                        Sent::Accepted(created) => {
                            artifact.address = created["number"].as_u64();
                            true
                        }
                        Sent::Refused => false,
                    }
                }
                Some(address) => {
                    let path = format!("/issues/{number}/artifacts/{address}");
                    let body = json!({"body": content});
                    let identity = request_identity("PATCH", &path, &body, None);
                    matches!(
                        send(&mut item.reflection, target, identity, || gh
                            .patch(&path, &body))?,
                        Sent::Accepted(_)
                    )
                }
            };
            if !sent {
                // A refused version is followed by the next.
                continue;
            }
            let Some(address) = artifact.address else {
                return Err(GitHubError::Decode {
                    path: format!("/issues/{number}/artifacts"),
                    message: "a created artifact without a number".to_owned(),
                });
            };
            version.remote = match newest_version(gh, number, address)? {
                Some((n, digest)) if n > floor && digest == version.digest.as_str() => Some(n),
                _ => {
                    remote = remote_versions(gh, number, address, floor)?;
                    remote
                        .iter()
                        .rev()
                        .find(|(n, digest)| *n > floor && *digest == version.digest.as_str())
                        .map(|(n, _)| *n)
                }
            };
            floor = version.remote.unwrap_or(floor);
        }
    }
    Ok(())
}

/// How many artifact versions one page of GitHub's answer holds.
const VERSIONS_PAGE: u64 = 100;

/// One page of an artifact's versions on GitHub, newest first, as
/// (version, digest), and whether GitHub has a next page.
fn versions_page(
    gh: &GitHub,
    number: u64,
    address: u64,
    per_page: u64,
    page: u64,
) -> Result<(Vec<(u64, String)>, bool), GitHubError> {
    let read = gh.read(&format!(
        "/issues/{number}/artifacts/{address}?per_page={per_page}&page={page}"
    ))?;
    let versions = read["versions"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|v| {
                    Some((
                        v["version"].as_u64()?,
                        digest_of(v["body"].as_str()?).as_str().to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((versions, read["has_next_page"].as_bool().unwrap_or(false)))
}

/// The newest version of an artifact on GitHub, as (version, digest).
fn newest_version(
    gh: &GitHub,
    number: u64,
    address: u64,
) -> Result<Option<(u64, String)>, GitHubError> {
    Ok(versions_page(gh, number, address, 1, 1)?
        .0
        .into_iter()
        .next())
}

/// An artifact's versions on GitHub newer than `floor`, oldest first, as
/// (version, digest): GitHub's pages, newest first, are read until one
/// reaches `floor` or the history ends, so every version newer than it is
/// read however long the history (SC-10).
fn remote_versions(
    gh: &GitHub,
    number: u64,
    address: u64,
    floor: u64,
) -> Result<Vec<(u64, String)>, GitHubError> {
    let mut versions = Vec::new();
    let mut page = 1;
    loop {
        let (read, has_next) = versions_page(gh, number, address, VERSIONS_PAGE, page)?;
        let numbers: Vec<u64> = read.iter().map(|(n, _)| *n).collect();
        versions.extend(read.into_iter().filter(|(n, _)| *n > floor));
        if walk_ends(&numbers, has_next, floor) {
            break;
        }
        page += 1;
    }
    versions.sort_by_key(|(n, _)| *n);
    Ok(versions)
}

/// Whether a newest-first walk of an artifact's versions ends after a page
/// holding `numbers`: the history ends there, or the page reached `floor`,
/// below which every version is already confirmed.
fn walk_ends(numbers: &[u64], has_next: bool, floor: u64) -> bool {
    !has_next || numbers.is_empty() || numbers.iter().any(|n| *n <= floor)
}

fn reflect_pull_request(
    gh: &GitHub,
    item: &mut LocalItem,
    issue: IssueRef,
    root: &std::path::Path,
    base: &str,
) -> Result<(), GitHubError> {
    let Some(branch) = item.record.branch.clone() else {
        return Ok(());
    };
    let role = item
        .record
        .bindings
        .iter()
        .find(|binding| binding.attempt.branch.as_ref() == Some(&branch))
        .map(|binding| binding.role.as_str());
    let author = gh.for_role(role, instrukt_core::github::Operation::Author);
    if item.reflection.pull.is_none() {
        let owner = gh.repository().split('/').next().unwrap_or_default();
        let head = encode(&format!("{owner}:{branch}"));
        let existing = gh.list(&format!("/pulls?state=all&head={head}"))?;
        if let Some(number) = existing.first().and_then(|p| p["number"].as_u64()) {
            item.reflection.pull = Some(number);
        } else {
            match gh.read(&format!("/branches/{}", encode(&branch))) {
                Ok(_) => {}
                Err(GitHubError::NotFound { .. }) => {
                    item.reflection.pull_outstanding = Some(format!(
                        "the pull request waits for branch `{branch}` on GitHub"
                    ));
                    return Ok(());
                }
                Err(err) => return Err(err),
            }
            let body = json!({
                "title": item.record.slug.as_str(),
                "head": branch,
                "base": base,
                "body": pull_body(&item.record, issue),
            });
            let identity = request_identity(
                "POST",
                "/pulls",
                &body,
                item.reflection
                    .published_head
                    .as_ref()
                    .map(instrukt_core::ids::CommitSha::as_str),
            );
            match send(
                &mut item.reflection,
                RefusedTarget::PullCreation,
                identity,
                || author.post("/pulls", &body),
            )? {
                Sent::Accepted(pull) => item.reflection.pull = pull["number"].as_u64(),
                Sent::Refused => return Ok(()),
            }
        }
    }
    let Some(pull) = item.reflection.pull else {
        return Ok(());
    };
    item.reflection.pull_outstanding = Some(format!("pull request #{pull} is not up to date"));
    let verdicts = build_verdicts(&item.record);
    let owed: Vec<&BuildVerdictView> = verdicts
        .iter()
        .filter(|v| !item.reflection.reviews.contains(&v.sequence))
        .collect();
    let mut waiting = None;
    if !owed.is_empty() {
        let pull_view = gh.read(&format!("/pulls/{pull}"))?;
        let pull_head = pull_view["head"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut links: BTreeMap<u64, String> = BTreeMap::new();
        for verdict in owed {
            let BuildVerdictView {
                sequence,
                verdict,
                commit,
                report,
                role,
            } = verdict;
            // A verdict is reviewed once its report is a saved
            // review-findings version on the issue and its commit is on the
            // pull request (`github-record`, Issues and visible work).
            let findings = item
                .record
                .artifact(crate::evidence::artifact_names::REVIEW_FINDINGS)
                .and_then(|artifact| {
                    let version = artifact
                        .versions
                        .iter()
                        .find(|v| v.digest.as_str() == report.as_str() && v.remote.is_some())?;
                    Some((artifact.address?, version.content.clone()?))
                });
            let Some((address, findings)) = findings else {
                waiting = Some(format!(
                    "the review of commit {commit} waits for its review findings on the issue"
                ));
                continue;
            };
            if !on_pull_head(root, commit, &pull_head) {
                waiting = Some(format!(
                    "the review of commit {commit} waits for that commit on pull request #{pull}"
                ));
                continue;
            }
            let link = match links.get(&address) {
                Some(link) => link.clone(),
                None => {
                    let read = gh.read(&format!(
                        "/issues/{}/artifacts/{address}?per_page=1&page=1",
                        issue.number
                    ))?;
                    let link = read["artifact"]["html_url"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    links.insert(address, link.clone());
                    link
                }
            };
            let reviewer = gh.for_role(role.as_deref(), instrukt_core::github::Operation::Review);
            let bot = reviewer.bot()?;
            if bot.matches(&pull_view["user"]) {
                return Err(reviewer.app_failure(instrukt_core::github::AppError::Setup("native review requires a different roles.<role>.github_app from the pull request author".into())));
            }
            let body = review_body(verdict, commit, &findings, &link);
            let state = if *verdict == "approve" {
                "APPROVED"
            } else {
                "CHANGES_REQUESTED"
            };
            let reviews = gh.list(&format!("/pulls/{pull}/reviews"))?;
            let landed = reviews.iter().any(|review| {
                bot.matches(&review["user"])
                    && review["commit_id"].as_str() == Some(commit)
                    && review["body"].as_str() == Some(&body)
                    && review["state"].as_str() == Some(state)
            });
            if !landed {
                let path = format!("/pulls/{pull}/reviews");
                let review = json!({"event":if *verdict=="approve" {"APPROVE"} else {"REQUEST_CHANGES"}, "commit_id":commit, "body":body});
                let identity = request_identity("POST", &path, &review, None);
                let target = RefusedTarget::Review { verdict: *sequence };
                if let Sent::Refused = send(&mut item.reflection, target, identity, || {
                    reviewer.post(&path, &review)
                })? {
                    // A refused verdict holds back no other.
                    continue;
                }
            }
            item.reflection.reviews.push(*sequence);
        }
    }
    if item.record.delivery.is_some() && !item.reflection.pull_closed {
        author.patch(
            &format!("/pulls/{pull}"),
            &json!({"state": "closed", "body": pull_body(&item.record, issue)}),
        )?;
        item.reflection.pull_closed = true;
    }
    item.reflection.pull_outstanding = waiting;
    Ok(())
}

/// Whether `commit` is the pull request's head `head` or an ancestor of it
/// in the repository at `root`; not while this computer lacks either.
fn on_pull_head(root: &std::path::Path, commit: &str, head: &str) -> bool {
    !head.is_empty()
        && (commit == head
            || instrukt_core::git::succeeds(root, ["merge-base", "--is-ancestor", commit, head])
                .unwrap_or(false))
}

/// A build review's body: the verdict and the reviewed commit, then the
/// findings, readable in place and shortened within the body limit, with
/// the link to the complete review-findings artifact (SC-9).
fn review_body(verdict: &str, commit: &str, findings: &str, link: &str) -> String {
    let said = if verdict == "approve" {
        "approved"
    } else {
        "changes requested"
    };
    let head = format!("**Build review: {said}** · reviewed commit `{commit}`\n\n");
    let content = format!(
        "{}\n\n[Complete review findings]({link})",
        findings.trim_end()
    );
    super::review::shortened(&head, &content, &shortened_note(link))
}

/// The closing line of a shortened review body: where the complete
/// findings are, at `link`.
fn shortened_note(link: &str) -> String {
    format!("\n\n(shortened; [the complete review findings]({link}) give them whole)\n")
}

fn pull_body(record: &TodoRecord, issue: IssueRef) -> String {
    match &record.delivery {
        Some(delivery) => format!(
            "Implements #{}.\n\nDelivered in {}.\n",
            issue.number, delivery.commit
        ),
        None => format!("Implements #{}.\n", issue.number),
    }
}

/// One build-review verdict of a record, as its review reflects it.
struct BuildVerdictView {
    /// Its stamp's record order.
    sequence: u64,
    verdict: &'static str,
    commit: String,
    report: String,
    /// The role of the session that recorded it.
    role: Option<String>,
}

/// The record's build-review verdicts, oldest first.
fn build_verdicts(record: &TodoRecord) -> Vec<BuildVerdictView> {
    record
        .stamps
        .iter()
        .filter_map(|stamp| match &stamp.fact {
            StampFact::BuildVerdict {
                verdict,
                target,
                report,
            } => Some(BuildVerdictView {
                sequence: stamp.sequence,
                verdict: match verdict {
                    crate::evidence::BuildVerdict::Approve => "approve",
                    crate::evidence::BuildVerdict::ChangesRequested => "changes_requested",
                },
                commit: target.to_string(),
                report: report.to_string(),
                role: match &stamp.actor {
                    crate::evidence::Actor::Session(session) => record
                        .bindings
                        .iter()
                        .find(|b| b.session.as_ref() == Some(session))
                        .map(|b| b.role.to_string()),
                    _ => None,
                },
            }),
            _ => None,
        })
        .collect()
}

/// `text` percent-encoded for a URL path or query, keeping unreserved
/// characters and the `/` and `:` a branch reference uses.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub(crate) fn output_label(label: &str) -> bool {
    matches!(
        label,
        "instrukt:todo"
            | "kind:feature"
            | "kind:change"
            | "kind:bug"
            | "kind:task"
            | "status:active"
            | "status:iceboxed"
            | "status:structural_holder"
            | "status:delivered"
            | "status:removed"
            | "on-hold"
    ) || label == TODO_LABEL
}

#[cfg(test)]
mod tests {
    use super::{
        KeptRefusal, RefusedTarget, refused_before, request_identity, review_body, walk_ends,
    };
    use serde_json::json;

    #[test]
    fn a_request_is_identified_by_what_it_sends_and_the_head_it_concerns() {
        let body = json!({"body": "the report"});
        let same = request_identity("PATCH", "/issues/1/artifacts/2", &body, None);
        assert_eq!(
            same,
            request_identity("PATCH", "/issues/1/artifacts/2", &body, None)
        );
        assert_ne!(
            same,
            request_identity(
                "PATCH",
                "/issues/1/artifacts/2",
                &json!({"body": "changed"}),
                None
            )
        );
        assert_ne!(
            request_identity("POST", "/pulls", &body, Some("aaa")),
            request_identity("POST", "/pulls", &body, Some("bbb"))
        );
    }

    #[test]
    fn only_the_same_request_to_the_same_target_is_skipped_as_refused() {
        let kept = [KeptRefusal {
            target: RefusedTarget::Artifact {
                name: crate::evidence::ArtifactName::new("prd"),
                sequence: 3,
            },
            identity: "same".into(),
            detail: "Validation Failed".into(),
        }];
        let target = RefusedTarget::Artifact {
            name: crate::evidence::ArtifactName::new("prd"),
            sequence: 3,
        };
        assert!(refused_before(&kept, &target, "same"));
        assert!(!refused_before(&kept, &target, "changed"));
        assert!(!refused_before(&kept, &RefusedTarget::Issue, "same"));
    }

    #[test]
    fn long_findings_are_shortened_within_the_limit_and_keep_what_fits() {
        let link = "https://github.test/acme/demo/issues/1/artifacts/4";
        let line = "a finding\n";
        let findings = line.repeat(20_000);
        let body = review_body("changes_requested", "abc123", &findings, link);
        let length = body.chars().count();
        assert!(length <= super::super::review::BODY_LIMIT);
        // The findings fill the body to within two lines of the limit.
        assert!(length > super::super::review::BODY_LIMIT - 2 * line.chars().count());
    }

    #[test]
    fn a_version_walk_ends_where_the_history_ends() {
        assert!(walk_ends(&[3, 2, 1], false, 0));
        assert!(walk_ends(&[], true, 0));
    }

    #[test]
    fn a_version_walk_ends_at_the_last_confirmed_version() {
        assert!(walk_ends(&[205, 204, 203], true, 204));
        assert!(!walk_ends(&[205, 204, 203], true, 120));
        assert!(!walk_ends(&[205], true, 0));
    }
}
