//! A fake GitHub: a local HTTP server standing in for GitHub's REST API at
//! the network line, the one fake the testing policy allows
//! (`docs/project/policy/testing.md`). instrukt's client stays real; the
//! test points it here through the global layer's `github.api_url`.
//!
//! It fakes GitHub's observable HTTP contract, not instrukt: resources
//! behave as GitHub documents them, every `GET` answers with an `ETag` and
//! honours `If-None-Match` with `304`, list endpoints paginate with
//! `per_page`/`page` and a `Link` header. Served (all under
//! `/repos/{owner}/{repo}`):
//!
//! - repository `GET /`, `GET /branches/{branch}`;
//! - issues `GET|POST /issues`, `GET|PATCH /issues/{n}` (pull requests
//!   share the number space and appear in the issue list with a
//!   `pull_request` key, as on GitHub);
//! - comments `GET|POST /issues/{n}/comments`,
//!   `GET|PATCH|DELETE /issues/comments/{id}`;
//! - labels `GET|POST /labels`, `GET|PATCH|DELETE /labels/{name}`,
//!   `GET|POST|PUT|DELETE /issues/{n}/labels`,
//!   `DELETE /issues/{n}/labels/{name}` (labels used on an issue are
//!   created on the fly, as on GitHub);
//! - sub-issues `GET|POST /issues/{n}/sub_issues`,
//!   `DELETE /issues/{n}/sub_issue`, `GET /issues/{n}/parent`;
//! - dependencies `GET|POST /issues/{n}/dependencies/blocked_by`,
//!   `DELETE /issues/{n}/dependencies/blocked_by/{issue_id}`;
//! - issue artifacts (preview API) `GET|POST /issues/{n}/artifacts`,
//!   `GET|PATCH|DELETE /issues/{n}/artifacts/{a}`, the read answering
//!   `{artifact, versions}` newest first;
//! - pull requests `GET|POST /pulls`, `GET|PATCH /pulls/{n}`,
//!   `GET|POST /pulls/{n}/reviews` (creation refuses with `422` while the
//!   head branch is not on the fake);
//! - CI `GET /commits/{ref}/status` (combined status),
//!   `GET /commits/{ref}/check-runs`, `POST /statuses/{sha}`.
//!
//! A revision conflict is what GitHub shows: another writer changed the
//! resource. GitHub's REST issue and comment writes honour no `If-Match`,
//! so there is no `412`; a test simulates the other computer with
//! [`FakeGitHub::edit_issue`] or [`FakeGitHub::edit_comment`], and the
//! client meets the newer revision when it reads. Unreachability is
//! [`FakeGitHub::stop`] (connections refused) until [`FakeGitHub::resume`]
//! on the same address. Rate limiting and other failures are one-shot
//! answers queued with [`FakeGitHub::respond_once`] or
//! [`FakeGitHub::rate_limit_next`].

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

use super::TestEnv;

/// The login the fake attributes the client's own writes to.
const DEFAULT_LOGIN: &str = "instrukt-test";

/// The `author_association` GitHub reports for the client's own comments:
/// the token's user has write access to the repository.
const CLIENT_ASSOCIATION: &str = "COLLABORATOR";

/// One request the fake received, with the status it answered.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    /// The path without the query.
    pub path: String,
    /// The raw query, without `?`; empty when there is none.
    pub query: String,
    /// Header names in lowercase.
    pub headers: BTreeMap<String, String>,
    pub body: String,
    pub status: u16,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// The body parsed as JSON (`Value::Null` when empty or not JSON).
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// A canned answer for the next request matching a method and path.
#[derive(Debug, Clone)]
pub struct CannedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

/// A request whose response is held until this guard is dropped.
pub struct RequestPause {
    reached: Receiver<()>,
    _release: Sender<()>,
}

impl RequestPause {
    pub fn wait_until_received(&self, timeout: Duration) -> bool {
        self.reached.recv_timeout(timeout).is_ok()
    }
}

/// How an issue is seeded.
#[derive(Debug, Clone, Default)]
pub struct IssueSeed {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub closed: bool,
    /// The author's login; the fake's own login when `None`.
    pub login: Option<String>,
}

/// A check run's state (`status` and, once completed, `conclusion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckRun {
    Queued,
    InProgress,
    Completed(Conclusion),
}

/// A completed check run's conclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conclusion {
    Success,
    Failure,
    Neutral,
    Cancelled,
    Skipped,
    TimedOut,
    ActionRequired,
}

impl Conclusion {
    fn as_str(self) -> &'static str {
        match self {
            Conclusion::Success => "success",
            Conclusion::Failure => "failure",
            Conclusion::Neutral => "neutral",
            Conclusion::Cancelled => "cancelled",
            Conclusion::Skipped => "skipped",
            Conclusion::TimedOut => "timed_out",
            Conclusion::ActionRequired => "action_required",
        }
    }
}

/// A commit status state (`POST /statuses/{sha}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusState {
    Pending,
    Success,
    Failure,
    Error,
}

impl StatusState {
    fn as_str(self) -> &'static str {
        match self {
            StatusState::Pending => "pending",
            StatusState::Success => "success",
            StatusState::Failure => "failure",
            StatusState::Error => "error",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "pending" => StatusState::Pending,
            "success" => StatusState::Success,
            "failure" => StatusState::Failure,
            "error" => StatusState::Error,
            _ => return None,
        })
    }
}

/// The running fake. Dropping it stops the server.
pub struct FakeGitHub {
    state: Arc<Mutex<State>>,
    addr: SocketAddr,
    running: Option<Running>,
    auth_clock: tempfile::TempDir,
}

struct Running {
    server: Arc<tiny_http::Server>,
    thread: JoinHandle<()>,
}

impl FakeGitHub {
    /// Starts the fake on 127.0.0.1 with an ephemeral port.
    pub fn start() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        let auth_clock = tempfile::tempdir().unwrap();
        let clock_file = auth_clock.path().join("clock");
        std::fs::write(&clock_file, "system").unwrap();
        let state = Arc::new(Mutex::new(State::new(format!("http://{addr}"), clock_file)));
        let running = Running::spawn(server, Arc::clone(&state));
        Self {
            state,
            addr,
            running: Some(running),
            auth_clock,
        }
    }

    /// The API address to configure as `github.api_url`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Writes `github.api_url` into `env`'s global layer
    /// (`~/.instrukt/instrukt.yaml`), keeping whatever else it holds.
    pub fn configure(&self, env: &TestEnv) {
        let path = env.instrukt_home().join("instrukt.yaml");
        let mut layer: serde_yaml_ng::Value = match std::fs::read_to_string(&path) {
            Ok(text) => serde_yaml_ng::from_str(&text).unwrap(),
            Err(_) => serde_yaml_ng::from_str("schema_version: 1").unwrap(),
        };
        let root = layer.as_mapping_mut().unwrap();
        let github = root
            .entry("github".into())
            .or_insert_with(|| serde_yaml_ng::Value::Mapping(serde_yaml_ng::Mapping::new()));
        github
            .as_mapping_mut()
            .unwrap()
            .insert("api_url".into(), self.url().into());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_yaml_ng::to_string(&layer).unwrap()).unwrap();
        configure_apps(env, &self.url(), &self.auth_clock.path().join("clock"));
    }

    /// Stops answering: connections to the address are refused.
    pub fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.server.unblock();
            running.thread.join().unwrap();
        }
    }

    /// Answers again on the same address, with the state it had.
    pub fn resume(&mut self) {
        if self.running.is_none() {
            let server = tiny_http::Server::http(self.addr).unwrap();
            self.running = Some(Running::spawn(server, Arc::clone(&self.state)));
        }
    }

    // --- seeding -----------------------------------------------------

    /// Adds the repository `owner/name` with default branch `main`.
    /// Requests for an unknown repository answer `404`.
    pub fn add_repo(&self, full_name: &str) -> &Self {
        let mut state = self.lock();
        let id = state.next_id();
        let (owner, name) = full_name.split_once('/').unwrap();
        state
            .repos
            .insert(full_name.to_owned(), Repo::new(id, owner, name));
        self
    }

    /// The login the client's own writes are attributed to.
    pub fn set_login(&self, login: &str) -> &Self {
        self.lock().login = login.to_owned();
        self
    }

    /// Seeds an issue as another writer would; returns its number.
    pub fn seed_issue(&self, repo: &str, seed: IssueSeed) -> u64 {
        let mut state = self.lock();
        let login = seed.login.clone().unwrap_or_else(|| state.login.clone());
        let number = state.create_issue(repo, &seed.title, &seed.body, &seed.labels, &login);
        if seed.closed {
            let now = state.now();
            let issue = state.repo(repo).issues.get_mut(&number).unwrap();
            issue.state = "closed".into();
            issue.state_reason = Some("completed".into());
            issue.closed_at = Some(now);
        }
        number
    }

    /// Seeds a pull request in the repository's shared issue-number space.
    pub fn seed_pull_request(&self, repo: &str, seed: IssueSeed) -> u64 {
        let number = self.seed_issue(repo, seed);
        self.lock().repo(repo).issues.get_mut(&number).unwrap().pull = Some(Pull {
            head: format!("pull-{number}"),
            base: "main".into(),
            merged: false,
            reviews: Vec::new(),
        });
        number
    }

    /// Changes an issue out of band, as another computer or a person on
    /// GitHub would: the issue's `updated_at` and `ETag` move.
    pub fn edit_issue(&self, repo: &str, number: u64, edit: impl FnOnce(&mut IssueView)) {
        let mut state = self.lock();
        let now = state.now();
        let issue = state.repo(repo).issues.get_mut(&number).unwrap();
        let mut view = IssueView {
            title: issue.title.clone(),
            body: issue.body.clone(),
            labels: issue.labels.clone(),
            closed: issue.state == "closed",
        };
        edit(&mut view);
        issue.title = view.title;
        issue.body = view.body;
        issue.labels = view.labels;
        let closed = issue.state == "closed";
        if view.closed != closed {
            issue.state = if view.closed { "closed" } else { "open" }.into();
            issue.closed_at = view.closed.then(|| now.clone());
        }
        issue.updated_at = now;
    }

    /// Adds a comment by `login` (an approver, another computer, a
    /// stranger), whose relationship to the repository GitHub reports as
    /// `association` (`author_association`: `OWNER`, `MEMBER`,
    /// `COLLABORATOR`, `CONTRIBUTOR`, `NONE`, …); returns its id.
    pub fn seed_comment(
        &self,
        repo: &str,
        number: u64,
        login: &str,
        association: &str,
        body: &str,
    ) -> u64 {
        self.lock()
            .create_comment(repo, number, login, association, body)
            .unwrap()
    }

    /// Changes a comment's body out of band.
    pub fn edit_comment(&self, repo: &str, id: u64, body: &str) {
        let mut state = self.lock();
        let now = state.now();
        let comment = state.repo(repo).comments.get_mut(&id).unwrap();
        comment.body = body.to_owned();
        comment.updated_at = now;
    }

    /// Puts branch `name` at `sha` on the fake.
    pub fn set_branch(&self, repo: &str, name: &str, sha: &str) -> &Self {
        self.lock()
            .repo(repo)
            .branches
            .insert(name.to_owned(), sha.to_owned());
        self
    }

    /// Records a commit status for `sha` under `context`.
    pub fn set_status(&self, repo: &str, sha: &str, context: &str, state: StatusState) -> &Self {
        let mut guard = self.lock();
        let id = guard.next_id();
        let now = guard.now();
        guard.repo(repo).set_status(sha, context, state, id, now);
        self
    }

    /// Records (or replaces) the check run `name` for `sha`.
    pub fn set_check_run(&self, repo: &str, sha: &str, name: &str, run: CheckRun) -> &Self {
        let mut guard = self.lock();
        let id = guard.next_id();
        let runs = guard
            .repo(repo)
            .check_runs
            .entry(sha.to_owned())
            .or_default();
        runs.retain(|(existing, _, _)| existing != name);
        runs.push((name.to_owned(), id, run));
        self
    }

    /// Answers the next request with `method` whose path ends with
    /// `path_suffix` with `response` instead of serving it.
    pub fn respond_once(&self, method: &str, path_suffix: &str, response: CannedResponse) {
        self.lock().canned.push_back(Canned {
            method: method.to_owned(),
            path_suffix: path_suffix.to_owned(),
            response,
        });
    }

    /// Holds the next matching response at the network boundary. Dropping
    /// the returned guard lets the server answer, including during unwind.
    pub fn pause_once(&self, method: &str, path_suffix: &str) -> RequestPause {
        let (reached, received) = mpsc::channel();
        let (release, released) = mpsc::channel();
        self.lock().pauses.push_back(PausedRequest {
            method: method.to_owned(),
            path_suffix: path_suffix.to_owned(),
            reached,
            released,
        });
        RequestPause {
            reached: received,
            _release: release,
        }
    }

    /// Answers the next `count` requests of any kind as GitHub's primary
    /// rate limit does: `403` with `x-ratelimit-remaining: 0` and
    /// `x-ratelimit-reset: <reset_epoch_seconds>` (Unix seconds, as GitHub
    /// sends it).
    pub fn rate_limit_next(&self, count: usize, reset_epoch_seconds: u64) {
        for _ in 0..count {
            self.respond_once(
                "",
                "",
                CannedResponse {
                    status: 403,
                    headers: vec![
                        ("x-ratelimit-limit".into(), "5000".into()),
                        ("x-ratelimit-remaining".into(), "0".into()),
                        ("x-ratelimit-reset".into(), reset_epoch_seconds.to_string()),
                    ],
                    body: json!({
                        "message": "API rate limit exceeded",
                        "documentation_url": "https://docs.github.com/rest/overview/rate-limits-for-the-rest-api"
                    }),
                },
            );
        }
    }

    /// Sends `X-Poll-Interval: <seconds>` on every answer from now on;
    /// `None` stops sending it.
    pub fn set_poll_interval(&self, seconds: Option<u64>) {
        self.lock().poll_interval = seconds;
    }

    // --- inspection --------------------------------------------------

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.lock().requests.clone()
    }

    /// The requests received so far whose method is `method` and whose
    /// path ends with `path_suffix`.
    pub fn requests_to(&self, method: &str, path_suffix: &str) -> Vec<RecordedRequest> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == method && r.path.ends_with(path_suffix))
            .collect()
    }

    /// Forgets the requests recorded so far.
    pub fn clear_requests(&self) {
        self.lock().requests.clear();
    }

    /// An issue (or pull request) as `GET /issues/{n}` shows it.
    pub fn issue(&self, repo: &str, number: u64) -> Option<Value> {
        let mut state = self.lock();
        let base = state.base.clone();
        let full = repo.to_owned();
        state
            .repos
            .get_mut(repo)
            .and_then(|r| r.issue_json(&base, &full, number))
    }

    /// Every issue and pull request of `repo`, oldest first.
    pub fn issues(&self, repo: &str) -> Vec<Value> {
        let mut state = self.lock();
        let base = state.base.clone();
        let r = state.repo(repo);
        let numbers: Vec<u64> = r.issues.keys().copied().collect();
        numbers
            .into_iter()
            .filter_map(|n| r.issue_json(&base, repo, n))
            .collect()
    }

    /// An issue's comments, oldest first.
    pub fn comments(&self, repo: &str, number: u64) -> Vec<Value> {
        let mut state = self.lock();
        let base = state.base.clone();
        let r = state.repo(repo);
        r.comments
            .values()
            .filter(|c| c.issue == number)
            .map(|c| c.json(&base, repo))
            .collect()
    }

    /// An issue's artifacts with every version, as `GET
    /// /issues/{n}/artifacts/{a}` shows each.
    pub fn artifacts(&self, repo: &str, number: u64) -> Vec<Value> {
        let mut state = self.lock();
        let r = state.repo(repo);
        r.issues
            .get(&number)
            .map(|issue| {
                issue
                    .artifacts
                    .values()
                    .map(|a| json!({"artifact": a.json(), "versions": a.versions_json()}))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }
}

impl Drop for FakeGitHub {
    fn drop(&mut self) {
        if let Some(running) = self.running.take() {
            running.server.unblock();
            // A panic in the server thread already failed a request; the
            // test reports its own failure.
            let _joined = running.thread.join();
        }
    }
}

impl TestEnv {
    /// Starts a fake GitHub and points this machine's `github.api_url` at
    /// it.
    pub fn with_fake_github(&self) -> FakeGitHub {
        let fake = FakeGitHub::start();
        fake.configure(self);
        fake
    }
}

/// The editable part of an issue, for out-of-band edits.
#[derive(Debug, Clone)]
pub struct IssueView {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub closed: bool,
}

impl Running {
    fn spawn(server: tiny_http::Server, state: Arc<Mutex<State>>) -> Self {
        let server = Arc::new(server);
        let serving = Arc::clone(&server);
        let thread = std::thread::spawn(move || {
            for request in serving.incoming_requests() {
                serve(request, &state);
            }
        });
        Self { server, thread }
    }
}

// ---------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------

struct Canned {
    method: String,
    path_suffix: String,
    response: CannedResponse,
}

struct PausedRequest {
    method: String,
    path_suffix: String,
    reached: Sender<()>,
    released: Receiver<()>,
}

struct State {
    base: String,
    login: String,
    ids: u64,
    clock: u64,
    repos: BTreeMap<String, Repo>,
    requests: Vec<RecordedRequest>,
    canned: VecDeque<Canned>,
    pauses: VecDeque<PausedRequest>,
    poll_interval: Option<u64>,
    /// GitHub projects (Projects v2), served through `/graphql`.
    projects: Vec<Project>,
    project_discovery_available: bool,
    auth_clock: std::path::PathBuf,
    tokens: BTreeMap<String, (u64, jiff::Timestamp)>,
    active_token: Option<String>,
}

impl State {
    fn new(base: String, auth_clock: std::path::PathBuf) -> Self {
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
            auth_clock,
            tokens: BTreeMap::new(),
            active_token: None,
        }
    }

    fn next_id(&mut self) -> u64 {
        self.ids += 1;
        self.ids
    }

    /// A strictly increasing timestamp, one second per call.
    fn now(&mut self) -> String {
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

    fn repo(&mut self, full_name: &str) -> &mut Repo {
        self.repos
            .get_mut(full_name)
            .unwrap_or_else(|| panic!("fake GitHub has no repository {full_name}; add_repo it"))
    }

    fn create_issue(
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

    fn create_comment(
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

struct Repo {
    id: u64,
    owner: String,
    name: String,
    next_number: u64,
    issues: BTreeMap<u64, Issue>,
    comments: BTreeMap<u64, Comment>,
    labels: BTreeMap<String, Label>,
    branches: BTreeMap<String, String>,
    statuses: BTreeMap<String, Vec<Status>>,
    check_runs: BTreeMap<String, Vec<(String, u64, CheckRun)>>,
}

struct Issue {
    id: u64,
    number: u64,
    title: String,
    body: String,
    state: String,
    state_reason: Option<String>,
    labels: Vec<String>,
    login: String,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
    sub_issues: Vec<u64>,
    parent: Option<u64>,
    blocked_by: Vec<u64>,
    artifacts: BTreeMap<u64, Artifact>,
    next_artifact: u64,
    pull: Option<Pull>,
}

struct Pull {
    head: String,
    base: String,
    merged: bool,
    reviews: Vec<Value>,
}

struct Comment {
    id: u64,
    issue: u64,
    body: String,
    login: String,
    /// GitHub's `author_association` for the comment's author.
    association: String,
    created_at: String,
    updated_at: String,
}

struct Label {
    id: u64,
    name: String,
    color: String,
    description: Option<String>,
}

struct Status {
    id: u64,
    context: String,
    state: StatusState,
    updated_at: String,
}

struct Artifact {
    number: u64,
    kind: String,
    name: String,
    creator: String,
    updated_by: String,
    created_at: String,
    updated_at: String,
    /// Oldest first.
    versions: Vec<ArtifactVersion>,
}

struct ArtifactVersion {
    version: u64,
    name: String,
    body: String,
    created_at: String,
    login: String,
}

fn actor(login: &str) -> Value {
    let id = login.bytes().map(u64::from).sum::<u64>();
    json!({"login":login,"id":id,"node_id":format!("BOT_{id}"),"type":if login.ends_with("[bot]"){"Bot"}else{"User"}})
}

impl Repo {
    fn new(id: u64, owner: &str, name: &str) -> Self {
        Self {
            id,
            owner: owner.to_owned(),
            name: name.to_owned(),
            next_number: 0,
            issues: BTreeMap::new(),
            comments: BTreeMap::new(),
            labels: BTreeMap::new(),
            branches: BTreeMap::new(),
            statuses: BTreeMap::new(),
            check_runs: BTreeMap::new(),
        }
    }

    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "full_name": format!("{}/{}", self.owner, self.name),
            "node_id":format!("R_{}",self.id),
            "owner": actor(&self.owner),
            "default_branch": "main",
            "private": true,
        })
    }

    fn ensure_labels(&mut self, names: &[String], id_seed: u64) {
        for (offset, name) in names.iter().enumerate() {
            self.labels.entry(name.clone()).or_insert_with(|| Label {
                id: id_seed * 100 + offset as u64,
                name: name.clone(),
                color: "ededed".into(),
                description: None,
            });
        }
    }

    fn label_json(&self, name: &str) -> Value {
        match self.labels.get(name) {
            Some(label) => label.json(),
            None => json!({"name": name}),
        }
    }

    fn issue_json(&self, base: &str, full: &str, number: u64) -> Option<Value> {
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

    fn pull_json(&self, base: &str, full: &str, number: u64) -> Option<Value> {
        let issue = self.issues.get(&number)?;
        let pull = issue.pull.as_ref()?;
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
            "head": {"ref": pull.head, "sha": self.branches.get(&pull.head)},
            "base": {"ref": pull.base, "sha": self.branches.get(&pull.base)},
            "merged": pull.merged,
            "created_at": issue.created_at,
            "updated_at": issue.updated_at,
            "closed_at": issue.closed_at,
        }))
    }

    fn set_status(&mut self, sha: &str, context: &str, state: StatusState, id: u64, now: String) {
        let statuses = self.statuses.entry(sha.to_owned()).or_default();
        statuses.retain(|s| s.context != context);
        statuses.push(Status {
            id,
            context: context.to_owned(),
            state,
            updated_at: now,
        });
    }

    fn combined_status(&self, sha: &str) -> Value {
        let statuses = self
            .statuses
            .get(sha)
            .map(Vec::as_slice)
            .unwrap_or_default();
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
                "description": null,
                "target_url": null,
                "created_at": s.updated_at,
                "updated_at": s.updated_at,
            })).collect::<Vec<_>>(),
        })
    }

    fn check_runs_json(&self, sha: &str) -> Value {
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
    fn json(&self, base: &str, full: &str) -> Value {
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
    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "color": self.color,
            "description": self.description,
        })
    }
}

impl Artifact {
    fn json(&self) -> Value {
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
    fn versions_json(&self) -> Value {
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

// ---------------------------------------------------------------------
// Serving
// ---------------------------------------------------------------------

struct Reply {
    status: u16,
    body: Option<Value>,
    headers: Vec<(String, String)>,
}

impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: Some(body),
            headers: Vec::new(),
        }
    }

    fn ok(body: Value) -> Self {
        Self::json(200, body)
    }

    fn created(body: Value) -> Self {
        Self::json(201, body)
    }

    fn no_content() -> Self {
        Self {
            status: 204,
            body: None,
            headers: Vec::new(),
        }
    }

    fn not_found() -> Self {
        Self::json(
            404,
            json!({"message": "Not Found", "documentation_url": "https://docs.github.com/rest"}),
        )
    }

    fn invalid(message: &str) -> Self {
        Self::json(
            422,
            json!({"message": "Validation Failed", "errors": [{"message": message}]}),
        )
    }
}

fn serve(mut request: tiny_http::Request, state: &Mutex<State>) {
    let method = request.method().as_str().to_owned();
    let url = request.url().to_owned();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (url.clone(), String::new()),
    };
    let headers: BTreeMap<String, String> = request
        .headers()
        .iter()
        .map(|h| {
            (
                h.field.as_str().as_str().to_ascii_lowercase(),
                h.value.to_string(),
            )
        })
        .collect();
    let mut body = String::new();
    let _read = request.as_reader().read_to_string(&mut body);

    let mut guard = state.lock().unwrap();
    let canned = guard
        .canned
        .iter()
        .position(|c| {
            (c.method.is_empty() || c.method == method)
                && (path.ends_with(&c.path_suffix) || url.ends_with(&c.path_suffix))
        })
        .and_then(|at| guard.canned.remove(at));
    let authorization = headers
        .get("authorization")
        .map(String::as_str)
        .unwrap_or("");
    let now = auth_time(&guard.auth_clock);
    let jwt = jwt_app(authorization, now);
    let token = authorization.strip_prefix("Bearer ").and_then(|t| {
        guard
            .tokens
            .get(t)
            .filter(|(_, expiry)| *expiry > now)
            .map(|(id, _)| (*id, t.to_owned()))
    });
    let app_id = jwt.or(token.as_ref().map(|(id, _)| *id));
    let previous_login = guard.login.clone();
    let previous_token = guard.active_token.clone();
    guard.active_token = token.as_ref().map(|(_, token)| token.clone());
    if let Some(id) = app_id {
        guard.login = app_login(id).into();
    }
    let jwt_endpoint = path == "/app"
        || path.starts_with("/app/installations/")
        || path.ends_with("/installation");
    let public_app = path.starts_with("/apps/");
    let mut reply =
        if jwt_endpoint && jwt.is_none() || (!jwt_endpoint && !public_app && token.is_none()) {
            Reply::json(401, json!({"message":"Bad credentials"}))
        } else if public_app && jwt.is_none() {
            Reply::not_found()
        } else {
            match canned {
                Some(canned) => Reply {
                    status: canned.response.status,
                    body: Some(canned.response.body),
                    headers: canned.response.headers,
                },
                None => route(&mut guard, &method, &path, &query, &body),
            }
        };
    guard.active_token = previous_token;
    guard.login = previous_login;
    if let Some(seconds) = guard.poll_interval {
        reply
            .headers
            .push(("X-Poll-Interval".into(), seconds.to_string()));
    }
    let text = reply
        .body
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_default();
    let mut status = reply.status;
    if method == "GET" && status == 200 {
        let etag = etag_of(&text);
        if headers
            .get("if-none-match")
            .is_some_and(|given| given.split(',').any(|tag| same_etag(tag, &etag)))
        {
            status = 304;
        }
        reply.headers.push(("ETag".into(), etag));
    }
    let pause = guard
        .pauses
        .iter()
        .position(|p| p.method == method && path.ends_with(&p.path_suffix))
        .and_then(|at| guard.pauses.remove(at));
    guard.requests.push(RecordedRequest {
        method,
        path,
        query,
        headers: headers
            .into_iter()
            .map(|(key, value)| {
                let value = if key == "authorization" {
                    "<redacted>".into()
                } else {
                    value
                };
                (key, value)
            })
            .collect(),
        body,
        status,
    });
    drop(guard);
    if let Some(pause) = pause
        && pause.reached.send(()).is_ok()
    {
        // The test owns the response boundary; disconnect releases it
        // even if an assertion in the test unwinds.
        let _released = pause.released.recv();
    }

    let mut response = if status == 304 || reply.body.is_none() {
        tiny_http::Response::from_string(String::new()).with_status_code(status)
    } else {
        tiny_http::Response::from_string(text)
            .with_status_code(status)
            .with_header(header("Content-Type", "application/json; charset=utf-8"))
    };
    if !reply
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("x-ratelimit-remaining"))
    {
        response.add_header(header("X-RateLimit-Limit", "5000"));
        response.add_header(header("X-RateLimit-Remaining", "4999"));
    }
    for (name, value) in &reply.headers {
        response.add_header(header(name, value));
    }
    let _sent = request.respond(response);
}

fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

fn etag_of(text: &str) -> String {
    use std::hash::{DefaultHasher, Hash as _, Hasher as _};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("W/\"{:016x}\"", hasher.finish())
}

fn same_etag(given: &str, etag: &str) -> bool {
    let bare = |tag: &str| tag.trim().trim_start_matches("W/").to_owned();
    given.trim() == "*" || bare(given) == bare(etag)
}

fn query_param<'q>(query: &'q str, name: &str) -> Option<&'q str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// Percent-decodes a path segment or query value.
fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Slices `items` by `per_page`/`page` and adds GitHub's `Link` header.
fn paginate(base: &str, path: &str, query: &str, items: Vec<Value>) -> Reply {
    let per_page = query_param(query, "per_page")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(30)
        .clamp(1, 100);
    let page = query_param(query, "page")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let last = items.len().div_ceil(per_page).max(1);
    let slice: Vec<Value> = items
        .into_iter()
        .skip((page - 1) * per_page)
        .take(per_page)
        .collect();
    let mut reply = Reply::ok(Value::Array(slice));
    if last > 1 {
        let link_to = |n: usize| {
            let mut params: Vec<String> = query
                .split('&')
                .filter(|p| !p.is_empty() && !p.starts_with("page=") && !p.starts_with("per_page="))
                .map(str::to_owned)
                .collect();
            params.push(format!("per_page={per_page}"));
            params.push(format!("page={n}"));
            format!("<{base}{path}?{}>", params.join("&"))
        };
        let mut links = Vec::new();
        if page < last {
            links.push(format!("{}; rel=\"next\"", link_to(page + 1)));
            links.push(format!("{}; rel=\"last\"", link_to(last)));
        }
        if page > 1 {
            links.push(format!("{}; rel=\"first\"", link_to(1)));
            links.push(format!("{}; rel=\"prev\"", link_to(page - 1)));
        }
        if !links.is_empty() {
            reply.headers.push(("Link".into(), links.join(", ")));
        }
    }
    reply
}

fn route(state: &mut State, method: &str, path: &str, query: &str, body: &str) -> Reply {
    if path == "/app" && method == "GET" {
        let id = match state.login.as_str() {
            "instruktai-author[bot]" => 2,
            "instruktai-reviewer[bot]" => 3,
            _ => 1,
        };
        return Reply::ok(json!({"id":id,"slug":app_login(id).trim_end_matches("[bot]")}));
    }
    if let Some(slug) = path.strip_prefix("/apps/") {
        let id = match slug {
            "instruktai" => 1,
            "instruktai-author" => 2,
            "instruktai-reviewer" => 3,
            _ => return Reply::not_found(),
        };
        return Reply::ok(json!({"id":id,"slug":slug}));
    }
    if let Some(login) = path.strip_prefix("/users/") {
        return if method == "GET" {
            Reply::ok(actor(login))
        } else {
            Reply::not_found()
        };
    }
    if method == "POST"
        && path.starts_with("/app/installations/")
        && path.ends_with("/access_tokens")
    {
        let id = path
            .split('/')
            .nth(3)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1);
        let input: Value = serde_json::from_str(body).unwrap();
        let expected = match state.login.as_str() {
            "instruktai-author[bot]" => 2,
            "instruktai-reviewer[bot]" => 3,
            _ => 1,
        };
        if id != expected {
            return Reply::not_found();
        }
        let expires = auth_time(&state.auth_clock)
            .checked_add(jiff::SignedDuration::from_secs(3600))
            .unwrap();
        let token = format!("app-token-{id}-{}", state.tokens.len() + 1);
        state.tokens.insert(token.clone(), (id, expires));
        return Reply::created(
            json!({"token":token,"expires_at":expires.to_string(),"permissions":input["permissions"]}),
        );
    }
    if method == "GET" && path == "/installation/repositories" {
        return Reply::ok(
            json!({"repositories":state.repos.values().map(Repo::json).collect::<Vec<_>>()}),
        );
    }
    if method == "POST" && path == "/graphql" {
        let input: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        return graphql(state, &input);
    }
    let segments: Vec<String> = path
        .trim_start_matches('/')
        .split('/')
        .map(decode)
        .collect();
    let segs: Vec<&str> = segments.iter().map(String::as_str).collect();
    let input: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    if let ("POST", ["orgs", owner, "projectsV2", number, "views"]) = (method, segs.as_slice()) {
        let id = state.next_id();
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| p.owner == *owner && Some(p.number) == number.parse().ok())
        else {
            return Reply::not_found();
        };
        let Some(columns) = input
            .get("vertical_group_by")
            .map_or(Some(Vec::new()), |v| v.as_array().cloned())
        else {
            return Reply::invalid("invalid columns");
        };
        if columns.iter().any(|id| {
            !project
                .fields
                .iter()
                .any(|f| Some(field_database_id(&f.id)) == id.as_u64())
        }) {
            return Reply::invalid("unknown column field");
        }
        let number = project
            .views
            .iter()
            .filter_map(|v| v["number"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        let view = json!({"id":id,"node_id":format!("PVTV_{id}"),"number":number,"name":input["name"],"layout":input["layout"],"filter":input["filter"].as_str().unwrap_or(""),"vertical_group_by":columns});
        project.views.push(view.clone());
        return Reply::created(json!({"value":view}));
    }
    let ["repos", owner, name, rest @ ..] = segs.as_slice() else {
        return Reply::not_found();
    };
    let full = format!("{owner}/{name}");
    if !state.repos.contains_key(&full) {
        return Reply::not_found();
    }
    let base = state.base.clone();
    let login = state.login.clone();
    let number = |text: &str| text.parse::<u64>().ok();
    match (method, rest) {
        ("GET", ["installation"]) => {
            let id = match state.login.as_str() {
                "instruktai-author[bot]" => 2,
                "instruktai-reviewer[bot]" => 3,
                _ => 1,
            };
            Reply::ok(
                json!({"id":id,"app_id":id,"permissions":{"issues":"write","contents":"write","pull_requests":"write","checks":"read","statuses":"read","organization_projects":"write"}}),
            )
        }
        ("GET", []) => Reply::ok(state.repo(&full).json()),
        ("GET", ["collaborators", login, "permission"]) => {
            let allowed = state.repo(&full).comments.values().any(|c| {
                c.login == *login
                    && matches!(c.association.as_str(), "OWNER" | "MEMBER" | "COLLABORATOR")
            });
            Reply::ok(json!({"permission":if allowed{"write"}else{"read"}}))
        }
        ("GET", ["branches", branch @ ..]) => {
            let branch = branch.join("/");
            match state.repo(&full).branches.get(&branch) {
                Some(sha) => Reply::ok(json!({"name": branch, "commit": {"sha": sha}})),
                None => Reply::not_found(),
            }
        }
        ("GET", ["commits", sha, "status"]) => Reply::ok(state.repo(&full).combined_status(sha)),
        ("GET", ["commits", sha, "check-runs"]) => {
            Reply::ok(state.repo(&full).check_runs_json(sha))
        }
        ("POST", ["statuses", sha]) => {
            let Some(status) = input["state"].as_str().and_then(StatusState::parse) else {
                return Reply::invalid("state is required");
            };
            let context = input["context"].as_str().unwrap_or("default").to_owned();
            let id = state.next_id();
            let now = state.now();
            state.repo(&full).set_status(sha, &context, status, id, now);
            Reply::created(json!({"id": id, "context": context, "state": status.as_str()}))
        }
        ("GET", ["issues"]) => list_issues(state, &base, &full, path, query),
        ("POST", ["issues"]) => {
            let Some(title) = input["title"].as_str() else {
                return Reply::invalid("title is required");
            };
            let labels = string_list(&input["labels"]);
            let body = input["body"].as_str().unwrap_or("");
            let n = state.create_issue(&full, title, body, &labels, &login);
            Reply::created(
                state
                    .repo(&full)
                    .issue_json(&base, &full, n)
                    .unwrap_or_default(),
            )
        }
        ("GET", ["issues", "comments", id]) => match number(id).and_then(|id| {
            state
                .repo(&full)
                .comments
                .get(&id)
                .map(|c| c.json(&base, &full))
        }) {
            Some(value) => Reply::ok(value),
            None => Reply::not_found(),
        },
        ("PATCH", ["issues", "comments", id]) => {
            let now = state.now();
            let r = state.repo(&full);
            let Some(comment) = number(id).and_then(|id| r.comments.get_mut(&id)) else {
                return Reply::not_found();
            };
            if let Some(body) = input["body"].as_str() {
                comment.body = body.to_owned();
                comment.updated_at = now;
            }
            Reply::ok(comment.json(&base, &full))
        }
        ("DELETE", ["issues", "comments", id]) => {
            match number(id).and_then(|id| state.repo(&full).comments.remove(&id)) {
                Some(_) => Reply::no_content(),
                None => Reply::not_found(),
            }
        }
        (_, ["issues", n, tail @ ..]) => {
            let Some(n) = number(n).filter(|n| state.repo(&full).issues.contains_key(n)) else {
                return Reply::not_found();
            };
            issue_route(
                state, method, &base, &full, n, tail, path, query, &input, &login,
            )
        }
        ("GET", ["labels"]) => {
            let items = state.repo(&full).labels.values().map(Label::json).collect();
            paginate(&base, path, query, items)
        }
        ("POST", ["labels"]) => {
            let Some(name) = input["name"].as_str() else {
                return Reply::invalid("name is required");
            };
            let id = state.next_id();
            let r = state.repo(&full);
            if r.labels.contains_key(name) {
                return Reply::invalid("already_exists");
            }
            let label = Label {
                id,
                name: name.to_owned(),
                color: input["color"].as_str().unwrap_or("ededed").to_owned(),
                description: input["description"].as_str().map(str::to_owned),
            };
            let value = label.json();
            r.labels.insert(name.to_owned(), label);
            Reply::created(value)
        }
        ("GET", ["labels", label]) => match state.repo(&full).labels.get(*label) {
            Some(label) => Reply::ok(label.json()),
            None => Reply::not_found(),
        },
        ("PATCH", ["labels", label]) => {
            let r = state.repo(&full);
            let Some(mut existing) = r.labels.remove(*label) else {
                return Reply::not_found();
            };
            if let Some(new_name) = input["new_name"].as_str() {
                existing.name = new_name.to_owned();
                for issue in r.issues.values_mut() {
                    for l in &mut issue.labels {
                        if l == label {
                            *l = new_name.to_owned();
                        }
                    }
                }
            }
            if let Some(color) = input["color"].as_str() {
                existing.color = color.to_owned();
            }
            if let Some(description) = input["description"].as_str() {
                existing.description = Some(description.to_owned());
            }
            let value = existing.json();
            r.labels.insert(existing.name.clone(), existing);
            Reply::ok(value)
        }
        ("DELETE", ["labels", label]) => {
            let r = state.repo(&full);
            if r.labels.remove(*label).is_none() {
                return Reply::not_found();
            }
            for issue in r.issues.values_mut() {
                issue.labels.retain(|l| l != label);
            }
            Reply::no_content()
        }
        ("GET", ["pulls"]) => {
            let wanted = query_param(query, "state").unwrap_or("open");
            let head = query_param(query, "head").map(decode);
            let r = state.repo(&full);
            let mut numbers: Vec<u64> = r
                .issues
                .values()
                .filter(|i| i.pull.is_some())
                .filter(|i| wanted == "all" || i.state == wanted)
                .filter(|i| {
                    head.as_deref().is_none_or(|h| {
                        let branch = h.split_once(':').map_or(h, |(_, b)| b);
                        i.pull.as_ref().is_some_and(|p| p.head == branch)
                    })
                })
                .map(|i| i.number)
                .collect();
            numbers.reverse();
            let items = numbers
                .into_iter()
                .filter_map(|n| r.pull_json(&base, &full, n))
                .collect();
            paginate(&base, path, query, items)
        }
        ("POST", ["pulls"]) => {
            let (Some(title), Some(head), Some(base_ref)) = (
                input["title"].as_str(),
                input["head"].as_str(),
                input["base"].as_str(),
            ) else {
                return Reply::invalid("title, head and base are required");
            };
            let head = head.split_once(':').map_or(head, |(_, b)| b).to_owned();
            if !state.repo(&full).branches.contains_key(&head) {
                return Reply::invalid(&format!("head branch {head} does not exist"));
            }
            let body = input["body"].as_str().unwrap_or("").to_owned();
            let n = state.create_issue(&full, title, &body, &[], &login);
            let r = state.repo(&full);
            if let Some(issue) = r.issues.get_mut(&n) {
                issue.pull = Some(Pull {
                    head,
                    base: base_ref.to_owned(),
                    merged: false,
                    reviews: Vec::new(),
                });
            }
            Reply::created(r.pull_json(&base, &full, n).unwrap_or_default())
        }
        (_, ["pulls", n, tail @ ..]) => {
            let Some(n) = number(n).filter(|n| {
                state
                    .repo(&full)
                    .issues
                    .get(n)
                    .is_some_and(|i| i.pull.is_some())
            }) else {
                return Reply::not_found();
            };
            pull_route(state, method, &base, &full, n, tail, &input, &login)
        }
        _ => Reply::not_found(),
    }
}

fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .or_else(|| item["name"].as_str())
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn list_issues(state: &mut State, base: &str, full: &str, path: &str, query: &str) -> Reply {
    let wanted = query_param(query, "state").unwrap_or("open");
    let labels: Vec<String> = query_param(query, "labels")
        .map(|l| decode(l).split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let r = state.repo(full);
    let mut numbers: Vec<u64> = r
        .issues
        .values()
        .filter(|i| wanted == "all" || i.state == wanted)
        .filter(|i| labels.iter().all(|l| i.labels.contains(l)))
        .map(|i| i.number)
        .collect();
    // GitHub's default order: newest created first.
    numbers.reverse();
    let items = numbers
        .into_iter()
        .filter_map(|n| r.issue_json(base, full, n))
        .collect();
    paginate(base, path, query, items)
}

#[allow(
    clippy::too_many_arguments,
    reason = "one dispatch over the request's parts"
)]
fn issue_route(
    state: &mut State,
    method: &str,
    base: &str,
    full: &str,
    n: u64,
    tail: &[&str],
    path: &str,
    query: &str,
    input: &Value,
    login: &str,
) -> Reply {
    let issue_reply = |state: &mut State| {
        Reply::ok(
            state
                .repo(full)
                .issue_json(base, full, n)
                .unwrap_or_default(),
        )
    };
    match (method, tail) {
        ("GET", []) => issue_reply(state),
        ("PATCH", []) => {
            let now = state.now();
            let id_seed = state.next_id();
            let r = state.repo(full);
            if let Some(labels) = input.get("labels") {
                let labels = string_list(labels);
                r.ensure_labels(&labels, id_seed);
                if let Some(issue) = r.issues.get_mut(&n) {
                    issue.labels = labels;
                }
            }
            let Some(issue) = r.issues.get_mut(&n) else {
                return Reply::not_found();
            };
            if let Some(title) = input["title"].as_str() {
                issue.title = title.to_owned();
            }
            if let Some(body) = input["body"].as_str() {
                issue.body = body.to_owned();
            }
            if let Some(new_state) = input["state"].as_str() {
                if new_state != issue.state {
                    issue.closed_at = (new_state == "closed").then(|| now.clone());
                }
                issue.state = new_state.to_owned();
                issue.state_reason = match (new_state, input["state_reason"].as_str()) {
                    (_, Some(reason)) => Some(reason.to_owned()),
                    ("closed", None) => Some("completed".into()),
                    ("open", None) => Some("reopened".into()),
                    _ => None,
                };
            }
            issue.updated_at = now;
            issue_reply(state)
        }
        ("GET", ["comments"]) => {
            let items = state
                .repo(full)
                .comments
                .values()
                .filter(|c| c.issue == n)
                .map(|c| c.json(base, full))
                .collect();
            paginate(base, path, query, items)
        }
        ("POST", ["comments"]) => {
            let Some(body) = input["body"].as_str() else {
                return Reply::invalid("body is required");
            };
            match state.create_comment(full, n, login, CLIENT_ASSOCIATION, body) {
                Some(id) => Reply::created(
                    state
                        .repo(full)
                        .comments
                        .get(&id)
                        .map(|c| c.json(base, full))
                        .unwrap_or_default(),
                ),
                None => Reply::not_found(),
            }
        }
        ("GET", ["events"]) => Reply::ok(json!([])),
        ("GET", ["labels"]) => {
            let r = state.repo(full);
            let items = r.issues[&n]
                .labels
                .iter()
                .map(|l| r.label_json(l))
                .collect();
            paginate(base, path, query, items)
        }
        ("POST" | "PUT", ["labels"]) => {
            let labels = string_list(input.get("labels").unwrap_or(input));
            let id_seed = state.next_id();
            let now = state.now();
            let r = state.repo(full);
            r.ensure_labels(&labels, id_seed);
            if let Some(issue) = r.issues.get_mut(&n) {
                if method == "PUT" {
                    issue.labels.clear();
                }
                for label in labels {
                    if !issue.labels.contains(&label) {
                        issue.labels.push(label);
                    }
                }
                issue.updated_at = now;
            }
            let items = r.issues[&n]
                .labels
                .iter()
                .map(|l| r.label_json(l))
                .collect();
            Reply::ok(Value::Array(items))
        }
        ("DELETE", ["labels"]) => {
            let now = state.now();
            if let Some(issue) = state.repo(full).issues.get_mut(&n) {
                issue.labels.clear();
                issue.updated_at = now;
            }
            Reply::no_content()
        }
        ("DELETE", ["labels", label]) => {
            let now = state.now();
            let r = state.repo(full);
            let Some(issue) = r.issues.get_mut(&n) else {
                return Reply::not_found();
            };
            if !issue.labels.iter().any(|l| l == label) {
                return Reply::not_found();
            }
            issue.labels.retain(|l| l != label);
            issue.updated_at = now;
            let items = r.issues[&n]
                .labels
                .iter()
                .map(|l| r.label_json(l))
                .collect();
            Reply::ok(Value::Array(items))
        }
        ("GET", ["sub_issues"]) => {
            let r = state.repo(full);
            let items = r.issues[&n]
                .sub_issues
                .iter()
                .filter_map(|child| r.issue_json(base, full, *child))
                .collect();
            paginate(base, path, query, items)
        }
        ("POST", ["sub_issues"]) => {
            let Some(child) = input["sub_issue_id"]
                .as_u64()
                .and_then(|id| number_of(state.repo(full), id))
            else {
                return Reply::invalid("sub_issue_id names no issue of this repository");
            };
            let r = state.repo(full);
            if r.issues[&child].parent.is_some() && input["replace_parent"] != json!(true) {
                return Reply::invalid("the issue already has a parent");
            }
            if let Some(old) = r.issues[&child].parent
                && let Some(old) = r.issues.get_mut(&old)
            {
                old.sub_issues.retain(|c| *c != child);
            }
            if let Some(issue) = r.issues.get_mut(&child) {
                issue.parent = Some(n);
            }
            if let Some(issue) = r.issues.get_mut(&n)
                && !issue.sub_issues.contains(&child)
            {
                issue.sub_issues.push(child);
            }
            Reply::created(r.issue_json(base, full, n).unwrap_or_default())
        }
        ("DELETE", ["sub_issue"]) => {
            let Some(child) = input["sub_issue_id"]
                .as_u64()
                .and_then(|id| number_of(state.repo(full), id))
            else {
                return Reply::not_found();
            };
            let r = state.repo(full);
            if let Some(issue) = r.issues.get_mut(&n) {
                issue.sub_issues.retain(|c| *c != child);
            }
            if let Some(issue) = r.issues.get_mut(&child) {
                issue.parent = None;
            }
            issue_reply(state)
        }
        ("GET", ["parent"]) => {
            let r = state.repo(full);
            match r.issues[&n]
                .parent
                .and_then(|p| r.issue_json(base, full, p))
            {
                Some(value) => Reply::ok(value),
                None => Reply::not_found(),
            }
        }
        ("GET", ["dependencies", "blocked_by"]) => {
            let r = state.repo(full);
            let items = r.issues[&n]
                .blocked_by
                .iter()
                .filter_map(|b| r.issue_json(base, full, *b))
                .collect();
            paginate(base, path, query, items)
        }
        ("POST", ["dependencies", "blocked_by"]) => {
            let Some(blocker) = input["issue_id"]
                .as_u64()
                .and_then(|id| number_of(state.repo(full), id))
            else {
                return Reply::invalid("issue_id names no issue of this repository");
            };
            if let Some(issue) = state.repo(full).issues.get_mut(&n)
                && !issue.blocked_by.contains(&blocker)
            {
                issue.blocked_by.push(blocker);
            }
            Reply::created(
                state
                    .repo(full)
                    .issue_json(base, full, n)
                    .unwrap_or_default(),
            )
        }
        ("DELETE", ["dependencies", "blocked_by", id]) => {
            let Some(blocker) = id
                .parse::<u64>()
                .ok()
                .and_then(|id| number_of(state.repo(full), id))
            else {
                return Reply::not_found();
            };
            if let Some(issue) = state.repo(full).issues.get_mut(&n) {
                issue.blocked_by.retain(|b| *b != blocker);
            }
            issue_reply(state)
        }
        ("GET", ["artifacts"]) => {
            let kind = query_param(query, "type");
            let items = state.repo(full).issues[&n]
                .artifacts
                .values()
                .filter(|a| kind.is_none_or(|k| a.kind == k))
                .map(Artifact::json)
                .collect();
            paginate(base, path, query, items)
        }
        ("POST", ["artifacts"]) => {
            let (Some(kind), Some(name)) = (input["type"].as_str(), input["name"].as_str()) else {
                return Reply::invalid("type and name are required");
            };
            if !matches!(kind, "generic" | "plan" | "embedded_html" | "link") {
                return Reply::invalid("type is not an artifact type");
            }
            let now = state.now();
            let r = state.repo(full);
            let Some(issue) = r.issues.get_mut(&n) else {
                return Reply::not_found();
            };
            issue.next_artifact += 1;
            let number = issue.next_artifact;
            let artifact = Artifact {
                number,
                kind: kind.to_owned(),
                name: name.to_owned(),
                creator: login.to_owned(),
                updated_by: login.to_owned(),
                created_at: now.clone(),
                updated_at: now.clone(),
                versions: vec![ArtifactVersion {
                    version: 1,
                    name: name.to_owned(),
                    body: input["body"].as_str().unwrap_or("").to_owned(),
                    created_at: now,
                    login: login.to_owned(),
                }],
            };
            let value = artifact.json();
            issue.artifacts.insert(number, artifact);
            Reply::created(value)
        }
        (_, ["artifacts", a]) => {
            let now = state.now();
            let r = state.repo(full);
            let Some(issue) = r.issues.get_mut(&n) else {
                return Reply::not_found();
            };
            let Some(a) = a
                .parse::<u64>()
                .ok()
                .filter(|a| issue.artifacts.contains_key(a))
            else {
                return Reply::not_found();
            };
            match method {
                "GET" => {
                    let artifact = &issue.artifacts[&a];
                    Reply::ok(
                        json!({"artifact": artifact.json(), "versions": artifact.versions_json()}),
                    )
                }
                "PATCH" => {
                    let Some(artifact) = issue.artifacts.get_mut(&a) else {
                        return Reply::not_found();
                    };
                    let version = artifact.versions.last().map_or(0, |v| v.version) + 1;
                    let name = input["name"].as_str().unwrap_or(&artifact.name).to_owned();
                    artifact.name.clone_from(&name);
                    artifact.versions.push(ArtifactVersion {
                        version,
                        name,
                        body: input["body"].as_str().unwrap_or("").to_owned(),
                        created_at: now.clone(),
                        login: login.to_owned(),
                    });
                    artifact.updated_at = now;
                    artifact.updated_by = login.to_owned();
                    Reply::ok(artifact.json())
                }
                "DELETE" => {
                    issue.artifacts.remove(&a);
                    Reply::no_content()
                }
                _ => Reply::not_found(),
            }
        }
        _ => Reply::not_found(),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "one dispatch over the request's parts"
)]
fn pull_route(
    state: &mut State,
    method: &str,
    base: &str,
    full: &str,
    n: u64,
    tail: &[&str],
    input: &Value,
    login: &str,
) -> Reply {
    match (method, tail) {
        ("GET", []) => Reply::ok(
            state
                .repo(full)
                .pull_json(base, full, n)
                .unwrap_or_default(),
        ),
        ("PATCH", []) => {
            let now = state.now();
            let r = state.repo(full);
            if let Some(issue) = r.issues.get_mut(&n) {
                if let Some(title) = input["title"].as_str() {
                    issue.title = title.to_owned();
                }
                if let Some(body) = input["body"].as_str() {
                    issue.body = body.to_owned();
                }
                if let Some(new_state) = input["state"].as_str() {
                    issue.closed_at = (new_state == "closed").then(|| now.clone());
                    issue.state = new_state.to_owned();
                }
                issue.updated_at = now;
            }
            Reply::ok(r.pull_json(base, full, n).unwrap_or_default())
        }
        ("GET", ["reviews"]) => {
            let reviews = state.repo(full).issues[&n]
                .pull
                .as_ref()
                .map(|p| p.reviews.clone())
                .unwrap_or_default();
            Reply::ok(Value::Array(reviews))
        }
        ("POST", ["reviews"]) => {
            if matches!(input["event"].as_str(), Some("APPROVE" | "REQUEST_CHANGES"))
                && state.repo(full).issues[&n].login == login
            {
                return Reply::invalid("Can not review your own pull request");
            }
            let id = state.next_id();
            let now = state.now();
            let review_state = match input["event"].as_str() {
                Some("APPROVE") => "APPROVED",
                Some("REQUEST_CHANGES") => "CHANGES_REQUESTED",
                Some("COMMENT") => "COMMENTED",
                _ => "PENDING",
            };
            let review = json!({
                "id": id,
                "user": actor(login),
                "body": input["body"].as_str().unwrap_or(""),
                "state": review_state,
                "commit_id": input["commit_id"],
                "submitted_at": now,
            });
            if let Some(pull) = state
                .repo(full)
                .issues
                .get_mut(&n)
                .and_then(|i| i.pull.as_mut())
            {
                pull.reviews.push(review.clone());
            }
            Reply::ok(review)
        }
        _ => Reply::not_found(),
    }
}

/// The number of the issue whose REST `id` is `id`.
fn number_of(repo: &Repo, id: u64) -> Option<u64> {
    repo.issues.values().find(|i| i.id == id).map(|i| i.number)
}

// ---------------------------------------------------------------------
// Projects (v2) over GraphQL: exactly the documents the work record sends
// ---------------------------------------------------------------------

/// A GitHub project of an owner.
struct Project {
    id: String,
    owner: String,
    number: u64,
    title: String,
    fields: Vec<ProjectField>,
    items: Vec<ProjectItem>,
    views: Vec<Value>,
    workflows: Vec<Value>,
}

/// A single-select field (other field types are seeded as fields without
/// options and are never served as single-select).
struct ProjectField {
    id: String,
    name: String,
    single_select: bool,
    options: Vec<FieldOption>,
}

struct FieldOption {
    id: String,
    name: String,
    color: String,
    description: String,
}

struct ProjectItem {
    id: String,
    /// The node id of the issue the item shows.
    content: String,
    /// Field id → option id.
    values: BTreeMap<String, String>,
}

impl FieldOption {
    fn json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "color": self.color, "description": self.description})
    }
}

impl ProjectField {
    fn json(&self) -> Value {
        if self.single_select {
            json!({"id": self.id, "databaseId": field_database_id(&self.id), "name": self.name, "options": self.options.iter().map(FieldOption::json).collect::<Vec<_>>()})
        } else {
            // Not a single-select field: the inline fragment selects nothing.
            json!({})
        }
    }
}

impl FakeGitHub {
    /// The project numbers visible on an account.
    pub fn project_numbers(&self, owner: &str) -> Vec<u64> {
        self.lock()
            .projects
            .iter()
            .filter(|project| project.owner == owner)
            .map(|project| project.number)
            .collect()
    }

    /// Sets account project-discovery availability at the API boundary.
    pub fn set_project_discovery_available(&self, available: bool) {
        self.lock().project_discovery_available = available;
    }

    /// Adds the project `<owner>/<number>`.
    pub fn add_project(&self, owner: &str, number: u64) -> &Self {
        let mut state = self.lock();
        let id = format!("PVT_{}", state.next_id());
        state.projects.push(Project {
            id,
            owner: owner.to_owned(),
            number,
            title: String::new(),
            fields: Vec::new(),
            items: Vec::new(),
            views: Vec::new(),
            workflows: Vec::new(),
        });
        self
    }

    /// Adds a field to project `<owner>/<number>`: a single-select field
    /// with `options`, or, with `None`, a field of another type.
    pub fn add_project_field(
        &self,
        owner: &str,
        number: u64,
        name: &str,
        options: Option<&[&str]>,
    ) {
        let mut state = self.lock();
        let field_id = format!("PVTF_{}", state.next_id());
        let mut made = Vec::new();
        for option in options.unwrap_or_default() {
            made.push(FieldOption {
                id: format!("OPT_{}", state.next_id()),
                name: (*option).to_owned(),
                color: "BLUE".into(),
                description: String::new(),
            });
        }
        let project = state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap();
        project.fields.push(ProjectField {
            id: field_id,
            name: name.to_owned(),
            single_select: options.is_some(),
            options: made,
        });
    }

    pub fn remove_project_field(&self, owner: &str, number: u64, name: &str) {
        let mut state = self.lock();
        state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .fields
            .retain(|f| f.name != name);
    }
    pub fn clear_project_items(&self, owner: &str, number: u64) {
        let mut state = self.lock();
        state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .items
            .clear();
    }
    pub fn reviews(&self, repo: &str, number: u64) -> Vec<Value> {
        self.lock().repos[repo].issues[&number]
            .pull
            .as_ref()
            .unwrap()
            .reviews
            .clone()
    }
    pub fn seed_project_view(
        &self,
        owner: &str,
        number: u64,
        name: &str,
        layout: &str,
        filter: &str,
        columns: &[&str],
    ) {
        let mut state = self.lock();
        let id = state.next_id();
        let project = state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap();
        let fields = columns
            .iter()
            .map(|name| {
                field_database_id(&project.fields.iter().find(|f| f.name == *name).unwrap().id)
            })
            .collect::<Vec<_>>();
        let number = project
            .views
            .iter()
            .filter_map(|v| v["number"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        project.views.push(json!({"id":id,"node_id":format!("PVTV_{id}"),"number":number,"name":name,"layout":layout,"filter":filter,"vertical_group_by":fields}));
    }
    pub fn edit_project_view(&self, owner: &str, number: u64, name: &str, patch: &Value) {
        let mut state = self.lock();
        let view = state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .views
            .iter_mut()
            .find(|v| v["name"] == name)
            .unwrap();
        for (key, value) in patch.as_object().unwrap() {
            view[key] = value.clone();
        }
    }
    pub fn remove_project_view(&self, owner: &str, number: u64, name: &str) {
        self.lock()
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .views
            .retain(|v| v["name"] != name);
    }
    pub fn project_views(&self, owner: &str, number: u64) -> Vec<Value> {
        self.lock()
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .views
            .clone()
    }
    pub fn project_field_details(&self, owner: &str, number: u64) -> Vec<Value> {
        self.lock()
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .fields
            .iter()
            .map(ProjectField::json)
            .collect()
    }
    pub fn drift_project_option(&self, owner: &str, number: u64, name: &str) {
        let mut state = self.lock();
        let field = state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .fields
            .iter_mut()
            .find(|f| f.name == name)
            .unwrap();
        for option in &mut field.options {
            option.color = "BLUE".into();
            option.description = "drifted".into();
        }
    }
    pub fn seed_project_workflow(&self, owner: &str, number: u64, workflow: Value) {
        self.lock()
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .workflows
            .push(workflow);
    }
    pub fn project_workflows(&self, owner: &str, number: u64) -> Vec<Value> {
        self.lock()
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .workflows
            .clone()
    }
    /// A project's fields, each with its option names, in order.
    pub fn project_fields(&self, owner: &str, number: u64) -> Vec<(String, Vec<String>)> {
        let state = self.lock();
        let project = state
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap();
        project
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.options.iter().map(|o| o.name.clone()).collect(),
                )
            })
            .collect()
    }

    /// A project's items: the issue node id each shows, and each set field
    /// as (field name, option name).
    pub fn project_items(&self, owner: &str, number: u64) -> Vec<(String, Vec<(String, String)>)> {
        let state = self.lock();
        let project = state
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap();
        project
            .items
            .iter()
            .map(|item| {
                let values = item
                    .values
                    .iter()
                    .filter_map(|(field, option)| {
                        let field = project.fields.iter().find(|f| &f.id == field)?;
                        let option = field.options.iter().find(|o| &o.id == option)?;
                        Some((field.name.clone(), option.name.clone()))
                    })
                    .collect();
                (item.content.clone(), values)
            })
            .collect()
    }
}

fn graphql_error(message: &str) -> Reply {
    Reply::ok(json!({"data": null, "errors": [{"message": message}]}))
}

fn field_payload(field: &ProjectField) -> Value {
    field.json()
}

/// Serves the GraphQL documents the work record sends; any other document
/// is an error.
fn graphql(state: &mut State, input: &Value) -> Reply {
    let query = input["query"].as_str().unwrap_or_default();
    let vars = &input["variables"];
    if query.contains("viewer { id login databaseId }") {
        let bot = actor(&state.login);
        return Reply::ok(
            json!({"data":{"viewer":{"id":bot["node_id"],"databaseId":bot["id"],"login":bot["login"]}}}),
        );
    }
    if query.contains("projectsV2(first:") {
        if !state.project_discovery_available {
            return graphql_error("project discovery is unavailable");
        }
        let owner = vars["owner"].as_str().unwrap();
        let nodes = state
            .projects
            .iter()
            .filter(|p| p.owner == owner)
            .map(|p| json!({"id":p.id,"number":p.number,"title":p.title}))
            .collect::<Vec<_>>();
        return Reply::ok(
            json!({"data":{"repositoryOwner":{"id":format!("OWNER_{owner}"),"projectsV2":{"nodes":nodes,"pageInfo":{"hasNextPage":false}}}}}),
        );
    }
    if query.contains("createProjectV2(input:") {
        let owner = vars["owner"]
            .as_str()
            .unwrap()
            .trim_start_matches("OWNER_")
            .to_owned();
        let number = state
            .projects
            .iter()
            .filter(|p| p.owner == owner)
            .map(|p| p.number)
            .max()
            .unwrap_or(0)
            + 1;
        let id = format!("PVT_{}", state.next_id());
        state.projects.push(Project {
            id: id.clone(),
            owner,
            number,
            title: vars["title"].as_str().unwrap().into(),
            fields: Vec::new(),
            items: Vec::new(),
            views: Vec::new(),
            workflows: Vec::new(),
        });
        return Reply::ok(
            json!({"data":{"createProjectV2":{"projectV2":{"id":id,"number":number}}}}),
        );
    }
    if query.contains("views(first:") {
        let project = state
            .projects
            .iter()
            .find(|p| {
                p.owner == vars["owner"].as_str().unwrap()
                    && Some(p.number) == vars["number"].as_u64()
            })
            .unwrap();
        let offset = vars["after"]
            .as_str()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        let nodes=project.views.iter().skip(offset).take(100).map(|v| {
            let fields=v["vertical_group_by"].as_array().unwrap().iter().filter_map(|id|project.fields.iter().find(|f|Some(field_database_id(&f.id))==id.as_u64())).map(|f|json!({"id":f.id})).collect::<Vec<_>>();
            json!({"id":v["node_id"],"name":v["name"],"number":v["number"],"filter":v["filter"],"layout":match v["layout"].as_str().unwrap(){"board"=>"BOARD_LAYOUT","table"=>"TABLE_LAYOUT",_=>"ROADMAP_LAYOUT"},"verticalGroupByFields":{"nodes":fields}})
        }).collect::<Vec<_>>();
        let more = offset + 100 < project.views.len();
        return Reply::ok(
            json!({"data":{"repositoryOwner":{"projectV2":{"id":project.id,"views":{"nodes":nodes,"pageInfo":{"hasNextPage":more,"endCursor":(offset+100).to_string()}}}}}}),
        );
    }
    if query.contains("createProjectV2View(") {
        let id = state.next_id();
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| p.id == vars["project"].as_str().unwrap())
        else {
            return graphql_error("no such project");
        };
        let number = project
            .views
            .iter()
            .filter_map(|v| v["number"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        let view = json!({"id":id,"node_id":format!("PVTV_{id}"),"number":number,"name":vars["name"],"layout":if vars["layout"]=="BOARD_LAYOUT"{"board"}else{"table"},"filter":"","vertical_group_by":[]});
        project.views.push(view.clone());
        return Reply::ok(
            json!({"data":{"createProjectV2View":{"projectV2View":{"id":view["node_id"]}}}}),
        );
    }
    if query.contains("updateProjectV2View(") {
        let Some(view) = state
            .projects
            .iter_mut()
            .flat_map(|p| &mut p.views)
            .find(|v| v["node_id"] == vars["view"])
        else {
            return graphql_error("no such view");
        };
        if let Some(layout) = vars.get("layout") {
            view["layout"] = if layout == "BOARD_LAYOUT" {
                "board"
            } else {
                "table"
            }
            .into();
        }
        if let Some(filter) = vars.get("filter") {
            view["filter"] = filter.clone();
        }
        return Reply::ok(
            json!({"data":{"updateProjectV2View":{"projectV2View":{"id":view["node_id"]}}}}),
        );
    }
    if query.contains("deleteProjectV2View(") {
        for project in &mut state.projects {
            project.views.retain(|v| v["node_id"] != vars["view"]);
        }
        return Reply::ok(
            json!({"data":{"deleteProjectV2View":{"projectV2View":{"id":vars["view"]}}}}),
        );
    }
    if query.contains("deleteProjectV2Workflow(") {
        for project in &mut state.projects {
            project.workflows.retain(|v| v["id"] != vars["workflow"]);
        }
        return Reply::ok(
            json!({"data":{"deleteProjectV2Workflow":{"deletedWorkflowId":vars["workflow"]}}}),
        );
    }
    if query.contains("repositoryOwner") {
        let owner = vars["owner"].as_str().unwrap_or_default();
        let number = vars["number"].as_u64().unwrap_or_default();
        let project = state
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .map(|p| {
                json!({"id": p.id, "fields": {"nodes": p.fields.iter().map(ProjectField::json).collect::<Vec<_>>()}})
            });
        return Reply::ok(json!({"data": {"repositoryOwner": {"projectV2": project}}}));
    }
    if query.contains("createProjectV2Field") {
        let field_id = format!("PVTF_{}", state.next_id());
        let mut options = Vec::new();
        for option in vars["options"].as_array().cloned().unwrap_or_default() {
            options.push(FieldOption {
                id: format!("OPT_{}", state.next_id()),
                name: option["name"].as_str().unwrap_or_default().to_owned(),
                color: option["color"].as_str().unwrap_or("GRAY").to_owned(),
                description: option["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| vars["project"] == p.id.as_str())
        else {
            return graphql_error("no such project");
        };
        project.fields.push(ProjectField {
            id: field_id,
            name: vars["name"].as_str().unwrap_or_default().to_owned(),
            single_select: true,
            options,
        });
        let payload = project.fields.last().map(field_payload).unwrap_or_default();
        return Reply::ok(json!({"data": {"createProjectV2Field": {"projectV2Field": payload}}}));
    }
    if query.contains("updateProjectV2Field(") {
        let inputs = vars["options"].as_array().cloned().unwrap_or_default();
        let mut fresh = Vec::new();
        for _ in &inputs {
            fresh.push(format!("OPT_{}", state.next_id()));
        }
        let field = state
            .projects
            .iter_mut()
            .flat_map(|p| p.fields.iter_mut())
            .find(|f| vars["field"] == f.id.as_str());
        let Some(field) = field else {
            return graphql_error("no such field");
        };
        // GitHub replaces the option set; an option sent with its id keeps
        // it.
        field.options = inputs
            .iter()
            .zip(fresh)
            .map(|(option, fresh)| FieldOption {
                id: option["id"].as_str().map_or(fresh, str::to_owned),
                name: option["name"].as_str().unwrap_or_default().to_owned(),
                color: option["color"].as_str().unwrap_or("GRAY").to_owned(),
                description: option["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            })
            .collect();
        let payload = field_payload(field);
        return Reply::ok(json!({"data": {"updateProjectV2Field": {"projectV2Field": payload}}}));
    }
    if query.contains("addProjectV2ItemById") {
        let item_id = format!("PVTI_{}", state.next_id());
        let content = vars["content"].as_str().unwrap_or_default().to_owned();
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| vars["project"] == p.id.as_str())
        else {
            return graphql_error("no such project");
        };
        let id = match project.items.iter().find(|i| i.content == content) {
            Some(existing) => existing.id.clone(),
            None => {
                project.items.push(ProjectItem {
                    id: item_id.clone(),
                    content,
                    values: BTreeMap::new(),
                });
                item_id
            }
        };
        return Reply::ok(json!({"data": {"addProjectV2ItemById": {"item": {"id": id}}}}));
    }
    if query.contains("clearProjectV2ItemFieldValue(") {
        let Some(item) = state
            .projects
            .iter_mut()
            .find(|p| p.id == vars["project"].as_str().unwrap())
            .and_then(|p| {
                p.items
                    .iter_mut()
                    .find(|i| i.id == vars["item"].as_str().unwrap())
            })
        else {
            return graphql_error("no such item");
        };
        item.values.remove(vars["field"].as_str().unwrap());
        return Reply::ok(
            json!({"data":{"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":item.id}}}}),
        );
    }
    if query.contains("updateProjectV2ItemFieldValue") {
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| vars["project"] == p.id.as_str())
        else {
            return graphql_error("no such project");
        };
        let Some(item) = project
            .items
            .iter_mut()
            .find(|i| vars["item"] == i.id.as_str())
        else {
            return graphql_error("no such item");
        };
        item.values.insert(
            vars["field"].as_str().unwrap_or_default().to_owned(),
            vars["option"].as_str().unwrap_or_default().to_owned(),
        );
        let id = item.id.clone();
        return Reply::ok(
            json!({"data": {"updateProjectV2ItemFieldValue": {"projectV2Item": {"id": id}}}}),
        );
    }
    graphql_error("the fake GitHub does not serve this document")
}

fn app_login(id: u64) -> &'static str {
    match id {
        2 => "instruktai-author[bot]",
        3 => "instruktai-reviewer[bot]",
        _ => "instruktai[bot]",
    }
}

/// Publicly committed RSA fixtures used only by the isolated fake App.
fn test_keys() -> (&'static str, &'static str) {
    (
        include_str!("../fixtures/test-app-private.pem"),
        include_str!("../fixtures/test-app-public.pem"),
    )
}
fn jwt_app(authorization: &str, now: jiff::Timestamp) -> Option<u64> {
    let jwt = authorization.strip_prefix("Bearer ")?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.validate_exp = false;
    let claims = jsonwebtoken::decode::<Value>(
        jwt,
        &jsonwebtoken::DecodingKey::from_rsa_pem(test_keys().1.as_bytes()).ok()?,
        &validation,
    )
    .ok()?
    .claims;
    let issued = claims["iat"].as_i64()?;
    let expires = claims["exp"].as_i64()?;
    let now = now.as_second();
    if issued > now
        || issued < now - 660
        || expires <= now
        || expires > now + 600
        || expires <= issued
        || expires - issued > 660
    {
        return None;
    }
    claims["iss"]
        .as_str()?
        .parse::<u64>()
        .ok()
        .filter(|id| (1..=3).contains(id))
}

pub fn configure_apps(env: &TestEnv, api: &str, clock: &std::path::Path) {
    std::fs::write(
        env.home().join(".fake-github-clock"),
        clock.to_str().unwrap(),
    )
    .unwrap();
    let file = env.instrukt_home().join("instrukt.yaml");
    let mut layer: serde_yaml_ng::Value = std::fs::read_to_string(&file)
        .ok()
        .map(|s| serde_yaml_ng::from_str(&s).unwrap())
        .unwrap_or_else(|| serde_yaml_ng::from_str("schema_version: 4").unwrap());
    let mut profiles = serde_json::Map::new();
    for (id, name) in [(1, "project"), (2, "author"), (3, "reviewer")] {
        let login = app_login(id);
        profiles.insert(name.into(),json!({"app_id":id,"slug":login.trim_end_matches("[bot]"),"bot":actor(login).as_object().unwrap().iter().filter(|(k,_)|k.as_str()!="type").map(|(k,v)|(k.clone(),v.clone())).collect::<serde_json::Map<_,_>>(),"private_key":"${TEST_APP_KEY}"}));
    }
    layer["github"]["api_url"] = api.into();
    layer["github"]["project_app"] = "project".into();
    layer["github"]["apps"] = serde_yaml_ng::to_value(profiles).unwrap();
    std::fs::write(file, serde_yaml_ng::to_string(&layer).unwrap()).unwrap();
    let home = instrukt_core::home::Home::at_user_home(
        env.instrukt_home().parent().unwrap().to_path_buf(),
    );
    instrukt_core::config::set_secret(&home, "TEST_APP_KEY", test_keys().0).unwrap();
}

fn field_database_id(id: &str) -> u64 {
    id.rsplit('_').next().unwrap().parse().unwrap()
}

fn auth_time(file: &std::path::Path) -> jiff::Timestamp {
    let value = std::fs::read_to_string(file).unwrap();
    if value == "system" {
        jiff::Timestamp::now()
    } else {
        value.parse().unwrap()
    }
}
pub fn sync_auth_clock(home: &std::path::Path, now: Option<&str>) {
    if let Ok(file) = std::fs::read_to_string(home.join(".fake-github-clock"))
        && std::path::Path::new(&file).exists()
    {
        std::fs::write(file, now.unwrap_or("system")).unwrap();
    }
}
