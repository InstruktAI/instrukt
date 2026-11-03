//! Reflection of local items to GitHub and pull-through from it
//! (`github-record`): conditional pulls, revision-checked writes,
//! retries that reconcile against what GitHub already holds, and direct
//! reads of work that is not a local item.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::_envelope::{self, TODO_LABEL};
use super::_github::{Fetched, GitHub, GitHubError};
use super::_model::{TodoRecord, digest_of};
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

/// Pulls the item's record from GitHub with a conditional request. For a
/// read, a newer revision is adopted when nothing local is pending; for a
/// change it is reported as [`Pulled::Newer`] and nothing is adopted.
/// Pending local changes meeting a newer revision mark the item conflicted.
pub(crate) fn pull(
    gh: &GitHub,
    item: &mut LocalItem,
    purpose: PullFor,
) -> Result<Pulled, GitHubError> {
    let (Some(issue), Some(comment)) = (item.reflection.issue, item.reflection.record_comment)
    else {
        return Ok(Pulled::Current);
    };
    let fetched = match gh.get(
        &format!("/issues/comments/{comment}"),
        item.reflection.etag.as_deref(),
    ) {
        Ok(fetched) => fetched,
        Err(GitHubError::NotFound { .. }) => {
            // The record comment is gone; the next flush writes it again.
            item.reflection.record_comment = None;
            item.reflection.etag = None;
            item.reflection.pending = true;
            return Ok(Pulled::Current);
        }
        Err(err) => return Err(err),
    };
    let Fetched::Fresh { body, etag } = fetched else {
        return Ok(Pulled::Current);
    };
    let text = body["body"].as_str().unwrap_or_default();
    let seen = body_digest(text);
    if item.reflection.remote_body.as_deref() == Some(seen.as_str()) {
        item.reflection.etag = etag;
        return Ok(Pulled::Current);
    }
    let Some(parsed) = trusted_record(&body) else {
        if _envelope::parse_record_comment(text).is_some() {
            // An envelope by an author without write access: never applied.
            return Ok(Pulled::Current);
        }
        return Err(GitHubError::Decode {
            path: format!("/issues/comments/{comment}"),
            message: "the record comment carries no readable record".to_owned(),
        });
    };
    let Ok(remote) = parsed else {
        return Err(GitHubError::Decode {
            path: format!("/issues/comments/{comment}"),
            message: "the record comment carries no readable record".to_owned(),
        });
    };
    if remote.id != item.record.id {
        return Err(GitHubError::Decode {
            path: format!("/issues/comments/{comment}"),
            message: "the record comment names another todo".to_owned(),
        });
    }
    if item.reflection.pending {
        item.reflection.conflicted = true;
        return Ok(Pulled::Conflicted);
    }
    if purpose == PullFor::Change {
        return Ok(Pulled::Newer);
    }
    let record = with_contents(gh, issue.number, remote, &item.record)?;
    item.record = record;
    item.reflection.etag = etag;
    item.reflection.remote_body = Some(seen);
    item.reflection.remote_revision = Some(item.record.revision);
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
    let Some((comment, text)) = find_record_comment(gh, issue.number, &item.record)? else {
        item.reflection.issue = Some(issue);
        item.reflection.conflicted = false;
        return Ok(());
    };
    let Some(Ok(remote)) = _envelope::parse_record_comment(&text) else {
        return Err(GitHubError::Decode {
            path: format!("/issues/comments/{comment}"),
            message: "the record comment carries no readable record".to_owned(),
        });
    };
    let record = with_contents(gh, issue.number, remote, &item.record)?;
    let kept = std::mem::take(&mut item.reflection);
    item.record = record;
    item.reflection = ReflectionState {
        issue: Some(issue),
        record_comment: Some(comment),
        etag: None,
        remote_revision: Some(item.record.revision),
        remote_body: Some(body_digest(&text)),
        pending: false,
        conflicted: false,
        pull: kept.pull,
        pull_closed: kept.pull_closed,
        reviews_reflected: kept.reviews_reflected,
        failure: None,
    };
    Ok(())
}

/// Writes the item's pending changes to GitHub: its issue, any artifact
/// versions not yet uploaded, then the record revision referencing them,
/// then the issue's visible fields and the linked pull request. A newer
/// remote revision marks the item conflicted and writes nothing more.
pub(crate) fn flush(gh: &GitHub, item: &mut LocalItem, base: &str) -> Result<(), GitHubError> {
    if item.reflection.conflicted {
        return Ok(());
    }
    if item.reflection.pending {
        let issue = ensure_issue(gh, item)?;
        if !remote_is_ours(gh, item, issue)? {
            item.reflection.conflicted = true;
            tracing::warn!(
                event = "work_reflection_conflict",
                todo = %item.record.slug,
                issue = issue.number,
                "a newer remote revision met a pending local change"
            );
            return Ok(());
        }
        upload_artifacts(gh, item, issue)?;
        write_record(gh, item, issue)?;
        let fields = _envelope::issue(&item.record);
        gh.patch(&format!("/issues/{}", issue.number), &fields.update_json())?;
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
    Ok(())
}

/// Whether the linked pull request still owes creation, a review or its
/// closing.
pub(crate) fn pull_request_owed(item: &LocalItem) -> bool {
    let record = &item.record;
    if !_envelope::is_repository_work(record.kind)
        || record.branch.is_none()
        || record.status == TodoStatus::Removed
    {
        return false;
    }
    match item.reflection.pull {
        // A delivered todo whose branch never reached GitHub opens none.
        None => record.delivery.is_none(),
        Some(_) => {
            build_verdicts(record).len() > item.reflection.reviews_reflected
                || (record.delivery.is_some() && !item.reflection.pull_closed)
        }
    }
}

/// Every reflected todo of the repository, read directly (issues and
/// their record comments), oldest first. `open_only` limits the read to
/// addressable work.
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
        let record = comments
            .iter()
            .rev()
            .find_map(|comment| match trusted_record(comment) {
                Some(Ok(record)) if record.id == id => Some(record),
                _ => None,
            });
        if let Some(record) = record {
            records.push((
                record,
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
/// still serves.
pub(crate) fn localize(
    gh: &GitHub,
    record: TodoRecord,
    issue: IssueRef,
) -> Result<LocalItem, GitHubError> {
    let Some((comment, text)) = find_record_comment(gh, issue.number, &record)? else {
        return Err(GitHubError::Decode {
            path: format!("/issues/{}/comments", issue.number),
            message: "the issue carries no record comment".to_owned(),
        });
    };
    let empty = TodoRecord {
        artifacts: Vec::new(),
        ..record.clone()
    };
    let record = with_contents(gh, issue.number, record, &empty)?;
    Ok(LocalItem {
        reflection: ReflectionState {
            issue: Some(issue),
            record_comment: Some(comment),
            remote_revision: Some(record.revision),
            remote_body: Some(body_digest(&text)),
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

/// The comment carrying `record`'s envelope on issue `number`, newest
/// first, with its body.
fn find_record_comment(
    gh: &GitHub,
    number: u64,
    record: &TodoRecord,
) -> Result<Option<(u64, String)>, GitHubError> {
    let comments = gh.list(&format!("/issues/{number}/comments"))?;
    Ok(comments
        .iter()
        .rev()
        .find_map(|comment| match trusted_record(comment) {
            Some(Ok(found)) if found.id == record.id => Some((
                comment["id"].as_u64()?,
                comment["body"].as_str()?.to_owned(),
            )),
            _ => None,
        }))
}

/// `author_association` values of authors who can write to the
/// repository: only their record envelopes are records.
const TRUSTED_ASSOCIATIONS: [&str; 3] = ["OWNER", "MEMBER", "COLLABORATOR"];

/// The record envelope a comment carries, when its author can write to the
/// repository. An envelope by anyone else (anyone can comment on a public
/// repository) is reported and never applied.
fn trusted_record(comment: &Value) -> Option<Result<TodoRecord, serde_json::Error>> {
    let parsed = _envelope::parse_record_comment(comment["body"].as_str()?)?;
    let association = comment["author_association"].as_str().unwrap_or_default();
    if TRUSTED_ASSOCIATIONS.contains(&association) {
        return Some(parsed);
    }
    tracing::warn!(
        event = "work_untrusted_record_skipped",
        comment = comment["id"].as_u64().unwrap_or_default(),
        author = comment["user"]["login"].as_str().unwrap_or_default(),
        association,
        "a record envelope by an author without write access is ignored"
    );
    None
}

/// Whether GitHub's record comment is the one this computer last wrote or
/// read, so replacing it overwrites nothing newer.
fn remote_is_ours(gh: &GitHub, item: &mut LocalItem, issue: IssueRef) -> Result<bool, GitHubError> {
    if item.reflection.record_comment.is_none() {
        let Some((comment, text)) = find_record_comment(gh, issue.number, &item.record)? else {
            return Ok(true);
        };
        // A record this computer wrote without learning the answer: ours
        // unless it is ahead of the local record.
        item.reflection.record_comment = Some(comment);
        let ahead = matches!(
            _envelope::parse_record_comment(&text),
            Some(Ok(remote)) if remote.revision > item.record.revision
        );
        if !ahead {
            item.reflection.remote_body = Some(body_digest(&text));
        }
        return Ok(!ahead);
    }
    let Some(comment) = item.reflection.record_comment else {
        return Ok(true);
    };
    match gh.get(
        &format!("/issues/comments/{comment}"),
        item.reflection.etag.as_deref(),
    ) {
        Ok(Fetched::NotModified) => Ok(true),
        Ok(Fetched::Fresh { body, etag }) => {
            let seen = body_digest(body["body"].as_str().unwrap_or_default());
            if item.reflection.remote_body.as_deref() == Some(seen.as_str()) {
                item.reflection.etag = etag;
                Ok(true)
            } else {
                // Only a trusted envelope is a newer revision.
                Ok(trusted_record(&body).is_none())
            }
        }
        Err(GitHubError::NotFound { .. }) => {
            item.reflection.record_comment = None;
            item.reflection.etag = None;
            Ok(true)
        }
        Err(err) => Err(err),
    }
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

fn write_record(gh: &GitHub, item: &mut LocalItem, issue: IssueRef) -> Result<(), GitHubError> {
    let body = _envelope::record_comment(&item.record).map_err(|err| GitHubError::Decode {
        path: "record".to_owned(),
        message: err.to_string(),
    })?;
    let payload = json!({ "body": body });
    match item.reflection.record_comment {
        Some(comment) => {
            gh.patch(&format!("/issues/comments/{comment}"), &payload)?;
        }
        None => {
            let created = gh.post(&format!("/issues/{}/comments", issue.number), &payload)?;
            item.reflection.record_comment = created["id"].as_u64();
        }
    }
    item.reflection.etag = None;
    item.reflection.remote_body = Some(body_digest(&body));
    item.reflection.remote_revision = Some(item.record.revision);
    Ok(())
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
                // Not on GitHub yet: creation stays pending.
                Err(GitHubError::NotFound { .. }) => return Ok(()),
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
                Err(GitHubError::Status { status: 422, .. }) => return Ok(()),
                Err(err) => return Err(err),
            }
        }
    }
    let Some(pull) = item.reflection.pull else {
        return Ok(());
    };
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

/// `remote` with the artifact contents `local` holds for the same digests,
/// and the rest read from GitHub's artifact versions and verified against
/// their recorded digests. A version GitHub no longer serves, or serves
/// with other content, stays without content.
fn with_contents(
    gh: &GitHub,
    number: u64,
    mut remote: TodoRecord,
    local: &TodoRecord,
) -> Result<TodoRecord, GitHubError> {
    let mut known: BTreeMap<String, String> = BTreeMap::new();
    for artifact in &local.artifacts {
        for version in &artifact.versions {
            if let Some(content) = &version.content {
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

fn body_digest(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::Actor;
    use crate::work::_model::TodoId;
    use instrukt_core::ids::{ProjectSlug, TodoSlug};
    use instrukt_runner::interfaces::{CatchUp, TodoKind};

    fn envelope() -> String {
        _envelope::record_comment(&TodoRecord {
            schema: 1,
            id: TodoId::parse(&"cd".repeat(16)).unwrap(),
            project: ProjectSlug::parse("demo").unwrap(),
            slug: TodoSlug::parse("depth").unwrap(),
            kind: TodoKind::Change,
            status: TodoStatus::Active,
            created: jiff::Timestamp::UNIX_EPOCH,
            created_by: Actor::Person { name: "dev".into() },
            human_involved: None,
            revision: 1,
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
        })
        .unwrap()
    }

    fn comment(association: &str) -> Value {
        json!({
            "id": 7,
            "body": envelope(),
            "user": {"login": "someone"},
            "author_association": association,
        })
    }

    #[test]
    fn envelopes_by_authors_with_write_access_are_records() {
        for association in TRUSTED_ASSOCIATIONS {
            assert!(
                matches!(trusted_record(&comment(association)), Some(Ok(_))),
                "{association}"
            );
        }
    }

    #[test]
    fn envelopes_by_anyone_else_are_never_records() {
        for association in ["CONTRIBUTOR", "FIRST_TIME_CONTRIBUTOR", "NONE", ""] {
            assert!(
                trusted_record(&comment(association)).is_none(),
                "{association:?}"
            );
        }
        let mut unlabelled = comment("OWNER");
        if let Some(fields) = unlabelled.as_object_mut() {
            fields.remove("author_association");
        }
        assert!(trusted_record(&unlabelled).is_none());
    }
}
