//! GitHub's state as the fake holds it: repositories, issues, pull
//! requests, comments, labels, statuses, check runs and artifacts, each
//! with the JSON GitHub's REST API shows for it.

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender};

use serde_json::{Value, json};

use super::_fake::{CannedResponse, CheckRun, RecordedRequest, StatusState};
use super::_projects::Project;

/// The login the fake attributes the client's own writes to.
const DEFAULT_LOGIN: &str = "instrukt-test";

/// The `author_association` GitHub reports for the client's own comments:
/// the token's user has write access to the repository.
pub(super) const CLIENT_ASSOCIATION: &str = "COLLABORATOR";

/// The longest commit-status description GitHub accepts.
const STATUS_DESCRIPTION_LIMIT: usize = 140;

/// The longest review body GitHub accepts.
pub(super) const REVIEW_BODY_LIMIT: usize = 65_536;

/// What an installation of each App may do on a repository unless a test set
/// otherwise.
fn default_installation_permissions() -> Value {
    json!({"issues":"write","contents":"write","pull_requests":"write","checks":"read","statuses":"read","organization_projects":"write"})
}

pub(super) struct Canned {
    pub(super) method: String,
    pub(super) path_suffix: String,
    pub(super) response: CannedResponse,
}

pub(super) struct PausedRequest {
    pub(super) method: String,
    pub(super) path_suffix: String,
    pub(super) reached: Sender<()>,
    pub(super) released: Receiver<()>,
}

pub(super) struct State {
    pub(super) base: String,
    pub(super) login: String,
    ids: u64,
    clock: u64,
    pub(super) repos: BTreeMap<String, Repo>,
    pub(super) requests: Vec<RecordedRequest>,
    pub(super) canned: VecDeque<Canned>,
    pub(super) pauses: VecDeque<PausedRequest>,
    pub(super) poll_interval: Option<u64>,
    /// GitHub projects (Projects v2), served through `/graphql`.
    pub(super) projects: Vec<Project>,
    pub(super) project_discovery_available: bool,
    /// Whether GitHub refuses this viewer's project visibility changes.
    pub(super) project_visibility_refused: bool,
    pub(super) auth_clock: std::path::PathBuf,
    pub(super) tokens: BTreeMap<String, (u64, jiff::Timestamp, bool)>,
    pub(super) active_token: Option<String>,
    /// Each App's installation permissions a test set, by App id.
    pub(super) installation_permissions: BTreeMap<u64, Value>,
    /// The account type GitHub shows for a login a test named
    /// (`Organization`, `User`); any other login shows as a user, or a bot
    /// for a `[bot]` login.
    pub(super) accounts: BTreeMap<String, String>,
    /// The organisations the project App is installed on.
    pub(super) org_installations: std::collections::BTreeSet<String>,
    /// Where a created repository's bare remote is initialised, as
    /// `<remotes>/<owner>/<name>.git`.
    pub(super) remotes: Option<std::path::PathBuf>,
}

impl State {
    pub(super) fn new(base: String, auth_clock: std::path::PathBuf) -> Self {
        Self {
            base,
            login: DEFAULT_LOGIN.to_owned(),
            ids: 1000,
            clock: 0,
            repos: BTreeMap::new(),
            requests: Vec::new(),
            canned: VecDeque::new(),
            pauses: VecDeque::new(),
            poll_interval: None,
            projects: Vec::new(),
            project_discovery_available: true,
            project_visibility_refused: false,
            auth_clock,
            tokens: BTreeMap::new(),
            active_token: None,
            installation_permissions: BTreeMap::new(),
            accounts: BTreeMap::new(),
            org_installations: std::collections::BTreeSet::new(),
            remotes: None,
        }
    }

    /// The permissions App `id`'s installation holds.
    pub(super) fn installation_permissions(&self, id: u64) -> Value {
        self.installation_permissions
            .get(&id)
            .cloned()
            .unwrap_or_else(default_installation_permissions)
    }

    pub(super) fn next_id(&mut self) -> u64 {
        self.ids += 1;
        self.ids
    }

    /// A strictly increasing timestamp, one second per call.
    pub(super) fn now(&mut self) -> String {
        self.clock += 1;
        let secs = self.clock;
        format!(
            "2026-01-{:02}T{:02}:{:02}:{:02}Z",
            1 + (secs / 86_400) % 28,
            secs / 3600 % 24,
            secs / 60 % 60,
            secs % 60
        )
    }

    pub(super) fn repo(&mut self, full_name: &str) -> &mut Repo {
        self.repos
            .get_mut(full_name)
            .unwrap_or_else(|| panic!("fake GitHub has no repository {full_name}; add_repo it"))
    }

    pub(super) fn create_issue(
        &mut self,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
        login: &str,
    ) -> u64 {
        let id = self.next_id();
        let now = self.now();
        let r = self.repo(repo);
        r.ensure_labels(labels, id);
        r.next_number += 1;
        let number = r.next_number;
        r.issues.insert(
            number,
            Issue {
                id,
                number,
                title: title.to_owned(),
                body: body.to_owned(),
                state: "open".into(),
                state_reason: None,
                labels: labels.to_vec(),
                login: login.to_owned(),
                created_at: now.clone(),
                updated_at: now,
                closed_at: None,
                sub_issues: Vec::new(),
                parent: None,
                blocked_by: Vec::new(),
                artifacts: BTreeMap::new(),
                next_artifact: 0,
                pull: None,
            },
        );
        number
    }

    pub(super) fn create_comment(
        &mut self,
        repo: &str,
        number: u64,
        login: &str,
        association: &str,
        body: &str,
    ) -> Option<u64> {
        let id = self.next_id();
        let now = self.now();
        let r = self.repo(repo);
        let issue = r.issues.get_mut(&number)?;
        issue.updated_at = now.clone();
        r.comments.insert(
            id,
            Comment {
                id,
                issue: number,
                body: body.to_owned(),
                login: login.to_owned(),
                association: association.to_owned(),
                created_at: now.clone(),
                updated_at: now,
            },
        );
        Some(id)
    }
}

pub(super) struct Repo {
    id: u64,
    owner: String,
    name: String,
    /// Whether the repository is public; private by default.
    pub(super) public: bool,
    next_number: u64,
    pub(super) issues: BTreeMap<u64, Issue>,
    pub(super) comments: BTreeMap<u64, Comment>,
    pub(super) labels: BTreeMap<String, Label>,
    pub(super) branches: BTreeMap<String, String>,
    /// The repository's default branch.
    pub(super) default_branch: String,
    /// Each collaborator added, with the permission they were given.
    pub(super) collaborators: BTreeMap<String, String>,
    statuses: BTreeMap<String, Vec<Status>>,
    pub(super) check_runs: BTreeMap<String, Vec<(String, u64, CheckRun)>>,
}

pub(super) struct Issue {
    pub(super) id: u64,
    pub(super) number: u64,
    pub(super) title: String,
    pub(super) body: String,
    pub(super) state: String,
    pub(super) state_reason: Option<String>,
    pub(super) labels: Vec<String>,
    pub(super) login: String,
    created_at: String,
    pub(super) updated_at: String,
    pub(super) closed_at: Option<String>,
    pub(super) sub_issues: Vec<u64>,
    pub(super) parent: Option<u64>,
    pub(super) blocked_by: Vec<u64>,
    pub(super) artifacts: BTreeMap<u64, Artifact>,
    pub(super) next_artifact: u64,
    pub(super) pull: Option<Pull>,
}

pub(super) struct Pull {
    pub(super) head: String,
    pub(super) base: String,
    pub(super) merged: bool,
    pub(super) reviews: Vec<Value>,
    pub(super) draft: bool,
    /// The repository the head branch lives in when it is not this one: a
    /// fork. Its head commit is still read from this repository's branch
    /// of the same name.
    pub(super) fork: Option<String>,
    /// GitHub's `author_association` for the pull request's author.
    pub(super) association: String,
}

impl Pull {
    pub(super) fn new(head: String, base: String) -> Self {
        Self {
            head,
            base,
            merged: false,
            reviews: Vec::new(),
            draft: false,
            fork: None,
            association: CLIENT_ASSOCIATION.to_owned(),
        }
    }
}

pub(super) struct Comment {
    id: u64,
    pub(super) issue: u64,
    pub(super) body: String,
    pub(super) login: String,
    /// GitHub's `author_association` for the comment's author.
    pub(super) association: String,
    created_at: String,
    pub(super) updated_at: String,
}

pub(super) struct Label {
    pub(super) id: u64,
    pub(super) name: String,
    pub(super) color: String,
    pub(super) description: Option<String>,
}

/// One posted commit status. Every post is kept, as GitHub keeps them.
pub(super) struct Status {
    pub(super) id: u64,
    pub(super) context: String,
    pub(super) state: StatusState,
    pub(super) description: Option<String>,
    pub(super) target_url: Option<String>,
    /// The login that posted it.
    pub(super) creator: String,
    pub(super) updated_at: String,
}

impl Status {
    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "node_id": format!("SC_{}", self.id),
            "context": self.context,
            "state": self.state.as_str(),
            "description": self.description,
            "target_url": self.target_url,
            "creator": actor(&self.creator),
            "created_at": self.updated_at,
            "updated_at": self.updated_at,
        })
    }
}

pub(super) struct Artifact {
    pub(super) number: u64,
    pub(super) kind: String,
    pub(super) name: String,
    pub(super) creator: String,
    pub(super) updated_by: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    /// Oldest first.
    pub(super) versions: Vec<ArtifactVersion>,
}

pub(super) struct ArtifactVersion {
    pub(super) version: u64,
    pub(super) name: String,
    pub(super) body: String,
    pub(super) created_at: String,
    pub(super) login: String,
}

pub(super) fn actor(login: &str) -> Value {
    let id = login.bytes().map(u64::from).sum::<u64>();
    json!({"login":login,"id":id,"node_id":format!("BOT_{id}"),"type":if login.ends_with("[bot]"){"Bot"}else{"User"}})
}

impl Repo {
    pub(super) fn new(id: u64, owner: &str, name: &str) -> Self {
        Self {
            id,
            owner: owner.to_owned(),
            name: name.to_owned(),
            public: false,
            next_number: 0,
            issues: BTreeMap::new(),
            comments: BTreeMap::new(),
            labels: BTreeMap::new(),
            branches: BTreeMap::new(),
            default_branch: "main".to_owned(),
            collaborators: BTreeMap::new(),
            statuses: BTreeMap::new(),
            check_runs: BTreeMap::new(),
        }
    }

    pub(super) fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub(super) fn json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "full_name": self.full_name(),
            "fork": false,
            "node_id":format!("R_{}",self.id),
            "owner": actor(&self.owner),
            "default_branch": self.default_branch,
            "private": !self.public,
            "visibility": if self.public { "public" } else { "private" },
        })
    }

    pub(super) fn ensure_labels(&mut self, names: &[String], id_seed: u64) {
        for (offset, name) in names.iter().enumerate() {
            self.labels.entry(name.clone()).or_insert_with(|| Label {
                id: id_seed * 100 + offset as u64,
                name: name.clone(),
                color: "ededed".into(),
                description: None,
            });
        }
    }

    pub(super) fn label_json(&self, name: &str) -> Value {
        match self.labels.get(name) {
            Some(label) => label.json(),
            None => json!({"name": name}),
        }
    }

    pub(super) fn issue_json(&self, base: &str, full: &str, number: u64) -> Option<Value> {
        let issue = self.issues.get(&number)?;
        let url = format!("{base}/repos/{full}/issues/{number}");
        let comments = self.comments.values().filter(|c| c.issue == number).count();
        let mut value = json!({
            "id": issue.id,
            "node_id": format!("I_{}", issue.id),
            "number": number,
            "url": url,
            "html_url": format!("https://github.test/{full}/issues/{number}"),
            "title": issue.title,
            "body": issue.body,
            "state": issue.state,
            "state_reason": issue.state_reason,
            "labels": issue.labels.iter().map(|l| self.label_json(l)).collect::<Vec<_>>(),
            "user": actor(&issue.login),
            "comments": comments,
            "created_at": issue.created_at,
            "updated_at": issue.updated_at,
            "closed_at": issue.closed_at,
            "sub_issues_summary": {
                "total": issue.sub_issues.len(),
                "completed": issue.sub_issues.iter()
                    .filter(|n| self.issues.get(n).is_some_and(|i| i.state == "closed"))
                    .count(),
            },
        });
        if issue.pull.is_some() {
            value["pull_request"] = json!({"url": format!("{base}/repos/{full}/pulls/{number}")});
        }
        Some(value)
    }

    pub(super) fn pull_json(&self, base: &str, full: &str, number: u64) -> Option<Value> {
        let issue = self.issues.get(&number)?;
        let pull = issue.pull.as_ref()?;
        let head_repo = match &pull.fork {
            Some(fork) => {
                let (owner, _) = fork.split_once('/').unwrap_or((fork, ""));
                json!({"full_name": fork, "fork": true, "owner": actor(owner)})
            }
            None => self.json(),
        };
        Some(json!({
            "id": issue.id,
            "node_id": format!("PR_{}", issue.id),
            "number": number,
            "url": format!("{base}/repos/{full}/pulls/{number}"),
            "html_url": format!("https://github.test/{full}/pull/{number}"),
            "title": issue.title,
            "body": issue.body,
            "state": issue.state,
            "user": actor(&issue.login),
            "author_association": pull.association,
            "draft": pull.draft,
            "head": {"ref": pull.head, "sha": self.branches.get(&pull.head), "repo": head_repo},
            "base": {"ref": pull.base, "sha": self.branches.get(&pull.base), "repo": self.json()},
            "merged": pull.merged,
            "created_at": issue.created_at,
            "updated_at": issue.updated_at,
            "closed_at": issue.closed_at,
        }))
    }

    /// Keeps a posted status; refuses a description GitHub would.
    pub(super) fn post_status(&mut self, sha: &str, status: Status) -> Result<(), String> {
        if status
            .description
            .as_ref()
            .is_some_and(|d| d.chars().count() > STATUS_DESCRIPTION_LIMIT)
        {
            return Err(format!(
                "description is too long (maximum is {STATUS_DESCRIPTION_LIMIT} characters)"
            ));
        }
        self.statuses
            .entry(sha.to_owned())
            .or_default()
            .push(status);
        Ok(())
    }

    /// Every status posted for `sha`, newest first, as GitHub lists them.
    pub(super) fn statuses_json(&self, sha: &str) -> Vec<Value> {
        self.statuses
            .get(sha)
            .map(|all| all.iter().rev().map(Status::json).collect())
            .unwrap_or_default()
    }

    /// GitHub's combined status: each context's latest status.
    pub(super) fn combined_status(&self, sha: &str) -> Value {
        let all = self
            .statuses
            .get(sha)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let statuses: Vec<&Status> = all
            .iter()
            .enumerate()
            .filter(|(at, s)| !all[at + 1..].iter().any(|later| later.context == s.context))
            .map(|(_, s)| s)
            .collect();
        let state = if statuses
            .iter()
            .any(|s| matches!(s.state, StatusState::Failure | StatusState::Error))
        {
            "failure"
        } else if statuses.is_empty() || statuses.iter().any(|s| s.state == StatusState::Pending) {
            "pending"
        } else {
            "success"
        };
        json!({
            "state": state,
            "sha": sha,
            "total_count": statuses.len(),
            "statuses": statuses.iter().map(|s| json!({
                "id": s.id,
                "context": s.context,
                "state": s.state.as_str(),
                "description": s.description,
                "target_url": s.target_url,
                "created_at": s.updated_at,
                "updated_at": s.updated_at,
            })).collect::<Vec<_>>(),
        })
    }

    pub(super) fn check_runs_json(&self, sha: &str) -> Value {
        let runs = self
            .check_runs
            .get(sha)
            .map(Vec::as_slice)
            .unwrap_or_default();
        json!({
            "total_count": runs.len(),
            "check_runs": runs.iter().map(|(name, id, run)| {
                let (status, conclusion) = match run {
                    CheckRun::Queued => ("queued", None),
                    CheckRun::InProgress => ("in_progress", None),
                    CheckRun::Completed(c) => ("completed", Some(c.as_str())),
                };
                json!({
                    "id": id,
                    "name": name,
                    "head_sha": sha,
                    "status": status,
                    "conclusion": conclusion,
                })
            }).collect::<Vec<_>>(),
        })
    }
}

impl Comment {
    pub(super) fn json(&self, base: &str, full: &str) -> Value {
        json!({
            "id": self.id,
            "node_id": format!("IC_{}", self.id),
            "url": format!("{base}/repos/{full}/issues/comments/{}", self.id),
            "issue_url": format!("{base}/repos/{full}/issues/{}", self.issue),
            "body": self.body,
            "user": actor(&self.login),
            "author_association": self.association,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }
}

impl Label {
    pub(super) fn json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "color": self.color,
            "description": self.description,
        })
    }
}

impl Artifact {
    pub(super) fn json(&self) -> Value {
        let body = self.versions.last().map(|v| v.body.as_str()).unwrap_or("");
        json!({
            "number": self.number,
            "type": self.kind,
            "name": self.name,
            "body": body,
            "creator": actor(&self.creator),
            "updated_by_actor": actor(&self.updated_by),
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }

    /// Newest first, as GitHub serves them.
    pub(super) fn versions_json(&self) -> Value {
        Value::Array(
            self.versions
                .iter()
                .rev()
                .map(|v| {
                    json!({
                        "version": v.version,
                        "name": v.name,
                        "body": v.body,
                        "created_at": v.created_at,
                        "actor": actor(&v.login),
                    })
                })
                .collect(),
        )
    }
}
