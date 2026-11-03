//! Reflection of local items to GitHub and pull-through from it
//! (`github-record`): conditional pulls, revision chains that make a lost
//! race visible, retries that recognize writes that already landed, and
//! direct reads of work that is not a local item.
//!
//! **The revision chain.** GitHub's REST API honours no `If-Match`, so a
//! record is never edited in place. Each revision is appended as a new
//! comment carrying the revision it was built on (its `base`). The
//! committed record is the chain of trusted envelopes, in comment order,
//! each built on the head before it: of two writers building on the same
//! revision, the first comment wins and the second is no successor.
//! After appending, a writer reads the chain again; if its comment is not
//! the head it lost the race and the item is marked conflicted, so two
//! writers never both report success.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use instrukt_core::config::BoardRef;

use super::_envelope::{self, Envelope, TODO_LABEL};
use super::_github::{Fetched, GitHub, GitHubError};
use super::_model::{TodoId, TodoRecord, digest_of};
use super::_store::{IssueRef, LocalItem, ReflectionState};
use crate::evidence::{StampFact, TodoStatus};

/// What a pull found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pulled {
    /// Nothing reflected yet, or GitHub holds what this computer knows.
    Current,
    /// A newer remote revision entered the local record.
    Adopted,
    /// A newer remote revision met pending local changes.
    Conflicted,
    /// A newer remote revision, left unread because a change is about to be
    /// recorded against the older one (`UC-GR3`: refresh first).
    Newer,
}

/// Whether a pull may bring a newer remote revision into the local record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PullFor {
    /// A read: a newer revision enters the local record.
    Read,
    /// A change: a newer revision means the change was made against a
    /// todo the caller has not read; it is refused until refreshed.
    Change,
}

/// The committed record on an issue: the head of its revision chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Committed {
    pub comment: u64,
    pub record: TodoRecord,
}

/// The head of the revision chain of todo `id` among `comments`: trusted
/// envelopes in comment order, each accepted only when built on the head
/// before it.
pub(crate) fn committed(comments: &[Value], id: &TodoId) -> Option<Committed> {
    let mut ordered: Vec<(u64, Envelope)> = comments
        .iter()
        .filter_map(|comment| {
            let number = comment["id"].as_u64()?;
            match trusted_envelope(comment)? {
                Ok(envelope) if &envelope.record.id == id => Some((number, envelope)),
                _ => None,
            }
        })
        .collect();
    ordered.sort_by_key(|(number, _)| *number);
    let mut head: Option<Committed> = None;
    for (comment, envelope) in ordered {
        let successor = match &head {
            None => true,
            Some(current) => {
                envelope.base == current.record.revision
                    && envelope.record.revision > current.record.revision
            }
        };
        if successor {
            head = Some(Committed {
                comment,
                record: envelope.record,
            });
        }
    }
    head
}

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

/// Answers on the todo's issue.
pub(crate) fn reply(gh: &GitHub, issue: IssueRef, text: &str) -> Result<(), GitHubError> {
    gh.post(
        &format!("/issues/{}/comments", issue.number),
        &json!({ "body": text }),
    )
    .map(|_| ())
}

/// Whether two records hold the same committed state: a revision that
/// already landed is recognized by its content, not by having seen the
/// answer to the request that wrote it.
fn same_record(a: &TodoRecord, b: &TodoRecord) -> bool {
    a.without_contents() == b.without_contents()
}

/// Pulls the item's record from GitHub: a conditional read of its issue
/// (a new revision comment moves it), then the head of its revision chain.
/// For a read, a newer head is adopted when nothing local is pending; for a
/// change it is reported as [`Pulled::Newer`] and nothing is adopted.
/// Pending local changes meeting a newer head mark the item conflicted.
pub(crate) fn pull(
    gh: &GitHub,
    item: &mut LocalItem,
    purpose: PullFor,
) -> Result<Pulled, GitHubError> {
    let Some(issue) = item.reflection.issue else {
        return Ok(Pulled::Current);
    };
    let etag = match gh.get(
        &format!("/issues/{}", issue.number),
        item.reflection.etag.as_deref(),
    ) {
        Ok(Fetched::NotModified) => return Ok(Pulled::Current),
        Ok(Fetched::Fresh { etag, .. }) => etag,
        Err(GitHubError::NotFound { .. }) => {
            // The issue is gone; the next flush reflects the item again.
            item.reflection = ReflectionState {
                pending: true,
                ..ReflectionState::default()
            };
            return Ok(Pulled::Current);
        }
        Err(err) => return Err(err),
    };
    let comments = gh.list(&format!("/issues/{}/comments", issue.number))?;
    item.reflection.inbound = commands(&comments, &item.reflection.commands_seen);
    let Some(head) = committed(&comments, &item.record.id) else {
        item.reflection.etag = etag;
        return Ok(Pulled::Current);
    };
    if Some(head.comment) == item.reflection.record_comment {
        item.reflection.etag = etag;
        return Ok(Pulled::Current);
    }
    if same_record(&head.record, &item.record) {
        // This computer's own write, whose answer never arrived.
        item.reflection.record_comment = Some(head.comment);
        item.reflection.remote_revision = Some(head.record.revision);
        item.reflection.etag = etag;
        return Ok(Pulled::Current);
    }
    if item.reflection.pending {
        item.reflection.conflicted = true;
        return Ok(Pulled::Conflicted);
    }
    if purpose == PullFor::Change {
        return Ok(Pulled::Newer);
    }
    let record = with_contents(gh, issue.number, head.record, &item.record)?;
    item.record = record;
    item.reflection.record_comment = Some(head.comment);
    item.reflection.remote_revision = Some(item.record.revision);
    item.reflection.etag = etag;
    item.reflection.conflicted = false;
    Ok(Pulled::Adopted)
}

/// Replaces the item with GitHub's committed record, dropping local
/// changes that were not reflected (`instrukt todo refresh`).
pub(crate) fn refresh(gh: &GitHub, item: &mut LocalItem) -> Result<(), GitHubError> {
    let issue = match item.reflection.issue {
        Some(issue) => issue,
        None => match find_issue(gh, &item.record)? {
            Some(issue) => issue,
            None => {
                // Never reflected: nothing on GitHub to refresh from.
                item.reflection.conflicted = false;
                return Ok(());
            }
        },
    };
    let comments = gh.list(&format!("/issues/{}/comments", issue.number))?;
    let Some(head) = committed(&comments, &item.record.id) else {
        item.reflection.issue = Some(issue);
        item.reflection.conflicted = false;
        return Ok(());
    };
    let record = with_contents(gh, issue.number, head.record, &item.record)?;
    let kept = std::mem::take(&mut item.reflection);
    item.record = record;
    item.reflection = ReflectionState {
        issue: Some(issue),
        record_comment: Some(head.comment),
        remote_revision: Some(item.record.revision),
        pull: kept.pull,
        pull_closed: kept.pull_closed,
        reviews_reflected: kept.reviews_reflected,
        commands_seen: kept.commands_seen,
        ..ReflectionState::default()
    };
    Ok(())
}

/// Writes the item's pending changes to GitHub: its issue, any artifact
/// versions not yet uploaded, then the next revision comment built on the
/// head this computer knows, verified by reading the chain back; then the
/// issue's visible fields and the linked pull request. A head this
/// computer has not seen, or a lost race, marks the item conflicted.
pub(crate) fn flush(
    gh: &GitHub,
    item: &mut LocalItem,
    base: &str,
    board: Option<&BoardRef>,
) -> Result<(), GitHubError> {
    if item.reflection.conflicted {
        return Ok(());
    }
    if item.reflection.pending {
        let issue = ensure_issue(gh, item)?;
        let comments = gh.list(&format!("/issues/{}/comments", issue.number))?;
        let head = committed(&comments, &item.record.id);
        if let Some(head) = &head
            && Some(head.comment) != item.reflection.record_comment
        {
            if same_record(&head.record, &item.record) {
                // A revision this computer wrote landed though its answer
                // was lost.
                item.reflection.record_comment = Some(head.comment);
                item.reflection.remote_revision = Some(head.record.revision);
            } else {
                conflict(
                    item,
                    issue,
                    "a newer remote revision met a pending local change",
                );
                return Ok(());
            }
        }
        let landed =
            head.is_some() && item.reflection.remote_revision == Some(item.record.revision);
        if !landed {
            upload_artifacts(gh, item, issue)?;
            let posted = write_revision(gh, item, issue)?;
            let comments = gh.list(&format!("/issues/{}/comments", issue.number))?;
            match committed(&comments, &item.record.id) {
                Some(head) if head.comment == posted => {
                    item.reflection.record_comment = Some(posted);
                    item.reflection.remote_revision = Some(item.record.revision);
                }
                _ => {
                    conflict(item, issue, "another computer committed a revision first");
                    return Ok(());
                }
            }
        }
        let fields = _envelope::issue(&item.record);
        gh.patch(&format!("/issues/{}", issue.number), &fields.update_json())?;
        item.reflection.etag = None;
        item.reflection.pending = false;
        item.reflection.failure = None;
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
        reflect_pull_request(gh, item, issue, base)?;
    }
    if let (Some(board), Some(issue)) = (board, item.reflection.issue)
        && board_owed(item, board)
    {
        reflect_board(gh, board, item, issue)?;
    }
    Ok(())
}

/// Whether the item is not on `board` or shows another status there.
pub(crate) fn board_owed(item: &LocalItem, board: &BoardRef) -> bool {
    board_item_on(item, board).is_none()
        || item.reflection.board_status.as_deref() != Some(_envelope::record_status(&item.record))
}

/// The item's board item id when its recorded membership is of `board`.
fn board_item_on<'i>(item: &'i LocalItem, board: &BoardRef) -> Option<&'i str> {
    let on = item.reflection.board.as_deref() == Some(board_key(board).as_str());
    item.reflection.board_item.as_deref().filter(|_| on)
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
) -> Result<(), GitHubError> {
    let status = _envelope::record_status(&item.record);
    // Membership of another board is no membership of this one.
    let member = board_item_on(item, board).map(str::to_owned);
    let attempt = (|| {
        let content = match member {
            Some(_) => String::new(),
            None => gh.read(&format!("/issues/{}", issue.number))?["node_id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| GitHubError::Decode {
                    path: format!("/issues/{}", issue.number),
                    message: "an issue without a node id".to_owned(),
                })?,
        };
        super::_board::reflect(gh, board, member.as_deref(), &content, status)
    })();
    match attempt {
        Ok(board_item) => {
            item.reflection.board_item = Some(board_item);
            item.reflection.board = Some(board_key(board));
            item.reflection.board_status = Some(status.to_owned());
            item.reflection.board_failure = None;
            Ok(())
        }
        Err(err) if err.is_reach() => Err(err),
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

fn conflict(item: &mut LocalItem, issue: IssueRef, why: &'static str) {
    item.reflection.conflicted = true;
    tracing::warn!(
        event = "work_reflection_conflict",
        todo = %item.record.slug,
        issue = issue.number,
        reason = why,
        "the local change is not reflected; the item is conflicted"
    );
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
            build_verdicts(record).len() > item.reflection.reviews_reflected
                || (record.delivery.is_some() && !item.reflection.pull_closed)
        }
    }
}

/// Every reflected todo of the repository, read directly (issues and the
/// heads of their revision chains), oldest first. `open_only` limits the
/// read to addressable work.
pub(crate) fn remote_records(
    gh: &GitHub,
    open_only: bool,
) -> Result<Vec<(TodoRecord, IssueRef)>, GitHubError> {
    let state = if open_only { "open" } else { "all" };
    let mut issues = gh.list(&format!("/issues?labels={TODO_LABEL}&state={state}"))?;
    issues.reverse();
    let mut records = Vec::new();
    for issue in issues {
        if issue.get("pull_request").is_some() {
            continue;
        }
        let Some(id) = issue["body"].as_str().and_then(_envelope::issue_identity) else {
            continue;
        };
        let (Some(number), Some(rest_id)) = (issue["number"].as_u64(), issue["id"].as_u64()) else {
            continue;
        };
        let comments = gh.list(&format!("/issues/{number}/comments"))?;
        if let Some(head) = committed(&comments, &id) {
            records.push((
                head.record,
                IssueRef {
                    number,
                    id: rest_id,
                },
            ));
        }
    }
    Ok(records)
}

/// A remote record as a new local item, with the artifact contents GitHub
/// still serves and verifies.
pub(crate) fn localize(
    gh: &GitHub,
    record: TodoRecord,
    issue: IssueRef,
) -> Result<LocalItem, GitHubError> {
    let comments = gh.list(&format!("/issues/{}/comments", issue.number))?;
    let Some(head) = committed(&comments, &record.id) else {
        return Err(GitHubError::Decode {
            path: format!("/issues/{}/comments", issue.number),
            message: "the issue carries no committed record".to_owned(),
        });
    };
    let empty = TodoRecord {
        artifacts: Vec::new(),
        ..head.record.clone()
    };
    let record = with_contents(gh, issue.number, head.record, &empty)?;
    Ok(LocalItem {
        reflection: ReflectionState {
            issue: Some(issue),
            record_comment: Some(head.comment),
            remote_revision: Some(record.revision),
            ..ReflectionState::default()
        },
        record,
    })
}

fn ensure_issue(gh: &GitHub, item: &mut LocalItem) -> Result<IssueRef, GitHubError> {
    if let Some(issue) = item.reflection.issue {
        return Ok(issue);
    }
    let issue = match find_issue(gh, &item.record)? {
        Some(issue) => issue,
        None => {
            let created = gh.post("/issues", &_envelope::issue(&item.record).create_json())?;
            issue_ref(&created, "/issues")?
        }
    };
    item.reflection.issue = Some(issue);
    Ok(issue)
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
/// repository: only their envelopes and commands count.
pub(crate) const TRUSTED_ASSOCIATIONS: [&str; 3] = ["OWNER", "MEMBER", "COLLABORATOR"];

/// Whether the comment's author can write to the repository.
pub(crate) fn has_write_access(comment: &Value) -> bool {
    TRUSTED_ASSOCIATIONS.contains(&comment["author_association"].as_str().unwrap_or_default())
}

/// The envelope a comment carries, when its author can write to the
/// repository. An envelope by anyone else (anyone can comment on a public
/// repository) is reported and never applied.
fn trusted_envelope(comment: &Value) -> Option<Result<Envelope, serde_json::Error>> {
    let parsed = _envelope::parse_record_comment(comment["body"].as_str()?)?;
    if has_write_access(comment) {
        return Some(parsed);
    }
    tracing::warn!(
        event = "work_untrusted_record_skipped",
        comment = comment["id"].as_u64().unwrap_or_default(),
        author = comment["user"]["login"].as_str().unwrap_or_default(),
        association = comment["author_association"].as_str().unwrap_or_default(),
        "a record envelope by an author without write access is ignored"
    );
    None
}

/// Appends the item's revision, built on the confirmed remote revision;
/// returns the new comment's id.
fn write_revision(gh: &GitHub, item: &LocalItem, issue: IssueRef) -> Result<u64, GitHubError> {
    let base = item.reflection.remote_revision.unwrap_or(0);
    let body =
        _envelope::record_comment(&item.record, base).map_err(|err| GitHubError::Decode {
            path: "record".to_owned(),
            message: err.to_string(),
        })?;
    let path = format!("/issues/{}/comments", issue.number);
    let created = gh.post(&path, &json!({ "body": body }))?;
    created["id"].as_u64().ok_or(GitHubError::Decode {
        path,
        message: "a created comment without an id".to_owned(),
    })
}

/// Uploads every artifact version not yet on GitHub, then learns the
/// provider's version numbers by digest. A retry reuses versions GitHub
/// already holds instead of uploading them twice.
fn upload_artifacts(gh: &GitHub, item: &mut LocalItem, issue: IssueRef) -> Result<(), GitHubError> {
    let number = issue.number;
    let mut listed: Option<Vec<Value>> = None;
    for artifact in &mut item.record.artifacts {
        if artifact.versions.iter().all(|v| v.remote.is_some()) {
            continue;
        }
        if artifact.address.is_none() {
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
        let mut remote = match artifact.address {
            Some(address) => remote_versions(gh, number, address)?,
            None => Vec::new(),
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
            match artifact.address {
                None => {
                    let created = gh.post(
                        &format!("/issues/{number}/artifacts"),
                        &json!({
                            "type": _envelope::artifact_type(artifact.name.as_str()),
                            "name": artifact.name.as_str(),
                            "body": content,
                        }),
                    )?;
                    artifact.address = created["number"].as_u64();
                }
                Some(address) => {
                    gh.patch(
                        &format!("/issues/{number}/artifacts/{address}"),
                        &json!({"body": content}),
                    )?;
                }
            }
            let Some(address) = artifact.address else {
                return Err(GitHubError::Decode {
                    path: format!("/issues/{number}/artifacts"),
                    message: "a created artifact without a number".to_owned(),
                });
            };
            remote = remote_versions(gh, number, address)?;
            version.remote = remote
                .iter()
                .rev()
                .find(|(n, digest)| *n > floor && *digest == version.digest.as_str())
                .map(|(n, _)| *n);
            floor = version.remote.unwrap_or(floor);
        }
    }
    Ok(())
}

/// An artifact's versions on GitHub, oldest first, as (version, digest).
fn remote_versions(
    gh: &GitHub,
    number: u64,
    address: u64,
) -> Result<Vec<(u64, String)>, GitHubError> {
    let read = gh.read(&format!("/issues/{number}/artifacts/{address}"))?;
    let mut versions: Vec<(u64, String)> = read["versions"]
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
    versions.sort_by_key(|(n, _)| *n);
    Ok(versions)
}

fn reflect_pull_request(
    gh: &GitHub,
    item: &mut LocalItem,
    issue: IssueRef,
    base: &str,
) -> Result<(), GitHubError> {
    let Some(branch) = item.record.branch.clone() else {
        return Ok(());
    };
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
            let created = gh.post(
                "/pulls",
                &json!({
                    "title": item.record.slug.as_str(),
                    "head": branch,
                    "base": base,
                    "body": pull_body(&item.record, issue),
                }),
            );
            match created {
                Ok(pull) => item.reflection.pull = pull["number"].as_u64(),
                Err(GitHubError::Status {
                    status: 422,
                    message,
                    ..
                }) => {
                    item.reflection.pull_outstanding =
                        Some(format!("GitHub refused the pull request: {message}"));
                    return Ok(());
                }
                Err(err) => return Err(err),
            }
        }
    }
    let Some(pull) = item.reflection.pull else {
        return Ok(());
    };
    item.reflection.pull_outstanding = Some(format!("pull request #{pull} is not up to date"));
    let verdicts = build_verdicts(&item.record);
    for (verdict, commit, report) in verdicts.iter().skip(item.reflection.reviews_reflected) {
        gh.post(
            &format!("/pulls/{pull}/reviews"),
            &json!({
                "event": "COMMENT",
                "commit_id": commit,
                "body": format!(
                    "[instrukt build review] verdict: {verdict} · reviewed commit {commit} · report {report}"
                ),
            }),
        )?;
        item.reflection.reviews_reflected += 1;
    }
    if item.record.delivery.is_some() && !item.reflection.pull_closed {
        gh.patch(
            &format!("/pulls/{pull}"),
            &json!({"state": "closed", "body": pull_body(&item.record, issue)}),
        )?;
        item.reflection.pull_closed = true;
    }
    item.reflection.pull_outstanding = None;
    Ok(())
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

/// The record's build-review verdicts, oldest first: (verdict, reviewed
/// commit, report digest).
fn build_verdicts(record: &TodoRecord) -> Vec<(&'static str, String, String)> {
    record
        .stamps
        .iter()
        .filter_map(|stamp| match &stamp.fact {
            StampFact::BuildVerdict {
                verdict,
                target,
                report,
            } => Some((
                match verdict {
                    crate::evidence::BuildVerdict::Approve => "approve",
                    crate::evidence::BuildVerdict::ChangesRequested => "changes_requested",
                },
                target.to_string(),
                report.to_string(),
            )),
            _ => None,
        })
        .collect()
}

/// `remote` (without content) with every version's content verified
/// against its recorded digest: from `local` where this computer holds
/// content with that digest, otherwise from GitHub's artifact versions. A
/// version GitHub no longer serves, or serves with other content, stays
/// without content: unverified content is never evidence.
fn with_contents(
    gh: &GitHub,
    number: u64,
    remote: TodoRecord,
    local: &TodoRecord,
) -> Result<TodoRecord, GitHubError> {
    let mut remote = remote.without_contents();
    let mut known: BTreeMap<String, String> = BTreeMap::new();
    for artifact in &local.artifacts {
        for version in &artifact.versions {
            if let Some(content) = &version.content
                && digest_of(content) == version.digest
            {
                known.insert(version.digest.as_str().to_owned(), content.clone());
            }
        }
    }
    for artifact in &mut remote.artifacts {
        let mut served: Option<BTreeMap<u64, String>> = None;
        for version in &mut artifact.versions {
            if let Some(content) = known.get(version.digest.as_str()) {
                version.content = Some(content.clone());
                continue;
            }
            let (Some(address), Some(n)) = (artifact.address, version.remote) else {
                continue;
            };
            if served.is_none() {
                served = Some(
                    match gh.read(&format!("/issues/{number}/artifacts/{address}")) {
                        Ok(read) => read["versions"]
                            .as_array()
                            .map(|list| {
                                list.iter()
                                    .filter_map(|v| {
                                        Some((
                                            v["version"].as_u64()?,
                                            v["body"].as_str()?.to_owned(),
                                        ))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        Err(GitHubError::NotFound { .. }) => BTreeMap::new(),
                        Err(err) => return Err(err),
                    },
                );
            }
            let content = served.as_ref().and_then(|s| s.get(&n));
            if let Some(content) = content
                && digest_of(content) == version.digest
            {
                version.content = Some(content.clone());
            } else {
                tracing::warn!(
                    event = "work_artifact_unverifiable",
                    todo = %remote.slug,
                    artifact = %artifact.name,
                    version = n,
                    "GitHub does not serve this artifact version's recorded content"
                );
            }
        }
    }
    Ok(remote)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::Actor;
    use instrukt_core::ids::{ProjectSlug, TodoSlug};
    use instrukt_runner::interfaces::{CatchUp, TodoKind};

    fn record(revision: u64, slug: &str) -> TodoRecord {
        TodoRecord {
            schema: 1,
            id: TodoId::parse(&"cd".repeat(16)).unwrap(),
            project: ProjectSlug::parse("demo").unwrap(),
            slug: TodoSlug::parse(slug).unwrap(),
            kind: TodoKind::Change,
            status: TodoStatus::Active,
            created: jiff::Timestamp::UNIX_EPOCH,
            created_by: Actor::Person { name: "dev".into() },
            human_involved: None,
            revision,
            sequence: 0,
            artifacts: Vec::new(),
            stamps: Vec::new(),
            claim: None,
            bindings: Vec::new(),
            hold: None,
            hold_history: Vec::new(),
            after: Vec::new(),
            group: None,
            schedule: None,
            catch_up: CatchUp::Once,
            agent: None,
            branch: None,
            delivery: None,
            removal: None,
        }
    }

    fn comment(id: u64, base: u64, record: &TodoRecord) -> Value {
        json!({
            "id": id,
            "body": _envelope::record_comment(record, base).unwrap(),
            "author_association": "COLLABORATOR",
        })
    }

    #[test]
    fn of_two_revisions_built_on_the_same_head_the_first_is_committed() {
        let first = record(1, "depth");
        let mine = record(2, "mine");
        let theirs = record(2, "theirs");
        let comments = [
            comment(10, 0, &first),
            comment(11, 1, &theirs),
            comment(12, 1, &mine),
        ];
        let head = committed(&comments, &first.id).unwrap();
        assert_eq!((head.comment, head.record), (11, theirs));
    }

    #[test]
    fn a_revision_built_on_the_head_extends_the_chain() {
        let first = record(1, "depth");
        let second = record(4, "depth");
        let comments = [comment(10, 0, &first), comment(11, 1, &second)];
        assert_eq!(committed(&comments, &first.id).unwrap().comment, 11);
    }

    #[test]
    fn membership_of_one_board_is_none_of_another() {
        let a = BoardRef {
            owner: "acme".into(),
            number: 7,
        };
        let b = BoardRef {
            owner: "acme".into(),
            number: 8,
        };
        let mut item = LocalItem {
            record: record(1, "depth"),
            reflection: Default::default(),
        };
        item.reflection.board_item = Some("PVTI_a".into());
        item.reflection.board_status = Some(_envelope::record_status(&item.record).into());
        // Recorded before membership carried its board: owed again.
        assert!(board_owed(&item, &a));
        item.reflection.board = Some(board_key(&a));
        assert!(!board_owed(&item, &a));
        assert_eq!(board_item_on(&item, &b), None);
        assert!(board_owed(&item, &b));
    }
}
