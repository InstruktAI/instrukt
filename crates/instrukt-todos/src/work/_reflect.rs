//! Reflection of local items to GitHub (`github-record`): the local
//! record is the one authority, and GitHub shows it one way — the issue,
//! its output labels, the native artifacts, the pull request and its
//! reviews, and the board. Nothing read from GitHub changes the record
//! except people's controls on the issue (`/instrukt` commands and the
//! `hold` label), which the session applies once each. Retries recognize
//! writes that already landed: an issue by the identity its body names, an
//! artifact version by its digest, a review by its body and commit.

use serde_json::{Value, json};

use instrukt_core::config::BoardRef;

use super::_envelope::{self, TODO_LABEL};
use super::_github::{Fetched, GitHub, GitHubError};
use super::_model::{TodoRecord, digest_of};
use super::_store::{IssueRef, LocalItem, ReflectionState};
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
/// identity — is recognized by its text and not posted twice.
pub(crate) fn reply(
    gh: &GitHub,
    issue: IssueRef,
    source: u64,
    text: &str,
) -> Result<(), GitHubError> {
    let text = format!("{text}\n\n<!-- instrukt:command-reply:{source} -->");
    let comments = gh.list(&format!("/issues/{}/comments", issue.number))?;
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

/// Reads the item's issue for people's controls: a conditional request
/// (an unchanged issue is not read further), then its conversation, whose
/// unanswered `/instrukt` commands become `reflection.inbound`. A gone
/// issue leaves the item to be reflected again by the next flush. Nothing
/// read changes the record.
pub(crate) fn read_issue(gh: &GitHub, reflection: &mut ReflectionState) -> Result<(), GitHubError> {
    let Some(issue) = reflection.issue else {
        return Ok(());
    };
    let etag = match gh.get(
        &format!("/issues/{}", issue.number),
        reflection.etag.as_deref(),
    ) {
        Ok(Fetched::NotModified) => return Ok(()),
        Ok(Fetched::Fresh { etag, .. }) => etag,
        Err(GitHubError::NotFound { .. }) => {
            // The issue is gone; the next flush reflects the item again.
            *reflection = ReflectionState {
                pending: true,
                ..ReflectionState::default()
            };
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    let comments = gh.list(&format!("/issues/{}/comments", issue.number))?;
    reflection.inbound = commands(&comments, &reflection.commands_seen);
    reflection.etag = etag;
    Ok(())
}

/// Writes the item's pending changes to GitHub: its issue, any artifact
/// versions not yet uploaded and the issue's visible fields; then the
/// linked pull request and the board. No comment is read or written: the
/// record lives on this computer only.
pub(crate) fn flush(
    gh: &GitHub,
    item: &mut LocalItem,
    base: &str,
    board: Option<&BoardRef>,
    needs_you: bool,
) -> Result<(), GitHubError> {
    if item.reflection.pending {
        let issue = ensure_issue(gh, item)?;
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
        gh.patch(&format!("/issues/{}", issue.number), &fields.update_json())?;
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
        reflect_pull_request(gh, item, issue, base)?;
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
            build_verdicts(record).len() > item.reflection.reviews_reflected
                || (record.delivery.is_some() && !item.reflection.pull_closed)
        }
    }
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
            let created = author.post(
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
    for (verdict, commit, report, role) in verdicts.iter().skip(item.reflection.reviews_reflected) {
        let reviewer = gh.for_role(role.as_deref(), instrukt_core::github::Operation::Review);
        let bot = reviewer.bot()?;
        let pull_actor = gh.read(&format!("/pulls/{pull}"))?;
        if bot.matches(&pull_actor["user"]) {
            return Err(reviewer.app_failure(instrukt_core::github::AppError::Setup("native review requires a different roles.<role>.github_app from the pull request author".into())));
        }
        let body = format!(
            "[instrukt build review] verdict: {verdict} · reviewed commit {commit} · report {report}"
        );
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
            reviewer.post(
                &format!("/pulls/{pull}/reviews"),
                &json!({"event":if *verdict=="approve" {"APPROVE"} else {"REQUEST_CHANGES"}, "commit_id":commit, "body":body}),
            )?;
        }
        item.reflection.reviews_reflected += 1;
    }
    if item.record.delivery.is_some() && !item.reflection.pull_closed {
        author.patch(
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
fn build_verdicts(record: &TodoRecord) -> Vec<(&'static str, String, String, Option<String>)> {
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
                match &stamp.actor {
                    crate::evidence::Actor::Session(session) => record
                        .bindings
                        .iter()
                        .find(|b| b.session.as_ref() == Some(session))
                        .map(|b| b.role.to_string()),
                    _ => None,
                },
            )),
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
