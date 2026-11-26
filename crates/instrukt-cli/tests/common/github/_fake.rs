//! The fake's control surface: starting it, seeding GitHub's state,
//! stopping and resuming its address, and inspecting what it received.

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

use super::super::TestEnv;
use super::_apps::configure_apps;
use super::_front::{Front, Reach, accept, lock_front, serve_requests};
use super::_model::{Canned, PausedRequest, Pull, Repo, State, Status};

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

/// How a pull request is seeded.
#[derive(Debug, Clone, Default)]
pub struct PullSeed {
    pub title: String,
    /// The head branch; its commit is this repository's branch of that name.
    pub head: String,
    pub base: String,
    pub draft: bool,
    /// The author's login; the fake's own login when `None`.
    pub login: Option<String>,
    /// GitHub's `author_association`; `COLLABORATOR` when `None`.
    pub association: Option<String>,
    /// The fork the head branch lives in, when it is not this repository.
    pub fork: Option<String>,
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
    pub(super) fn as_str(self) -> &'static str {
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
    pub(super) fn as_str(self) -> &'static str {
        match self {
            StatusState::Pending => "pending",
            StatusState::Success => "success",
            StatusState::Failure => "failure",
            StatusState::Error => "error",
        }
    }

    pub(super) fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "pending" => StatusState::Pending,
            "success" => StatusState::Success,
            "failure" => StatusState::Failure,
            "error" => StatusState::Error,
            _ => return None,
        })
    }
}

/// The running fake. Dropping it stops the server and releases its
/// address. A failure inside the fake is recorded with its operation and
/// cause: a socket operation of one of the fake's threads that failed, a
/// relay or request-copy thread that could not be started, a panic in one
/// of the fake's threads, or a drop that could not end the accept thread
/// and so leaves the address held. Dropping the fake then fails the test,
/// or, when the test is already failing, adds the report to its output.
/// A socket `resume` opens itself is not recorded: when it fails, `resume`
/// fails the test at once with the operation and cause.
pub struct FakeGitHub {
    state: Arc<Mutex<State>>,
    addr: SocketAddr,
    /// The listener on `addr`, held from `start` to drop.
    _listener: Arc<TcpListener>,
    front: Arc<Mutex<Front>>,
    accepting: Option<JoinHandle<()>>,
    server: Arc<tiny_http::Server>,
    serving: Option<JoinHandle<()>>,
    auth_clock: tempfile::TempDir,
}

impl FakeGitHub {
    /// Starts the fake on 127.0.0.1 with an ephemeral port.
    pub fn start() -> Self {
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").unwrap());
        let addr = listener.local_addr().unwrap();
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let backend = server.server_addr().to_ip().unwrap();
        let auth_clock = tempfile::tempdir().unwrap();
        let clock_file = auth_clock.path().join("clock");
        std::fs::write(&clock_file, "system").unwrap();
        let state = Arc::new(Mutex::new(State::new(format!("http://{addr}"), clock_file)));
        let front = Arc::new(Mutex::new(Front {
            reach: Reach::Answering,
            next_connection: 0,
            relayed: BTreeMap::new(),
            failures: Vec::new(),
        }));
        let accepting = accept(Arc::clone(&listener), backend, Arc::clone(&front));
        let serving = serve_requests(Arc::clone(&server), Arc::clone(&front), Arc::clone(&state));
        Self {
            state,
            addr,
            _listener: listener,
            front,
            accepting: Some(accepting),
            server,
            serving: Some(serving),
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

    /// Stops answering. When this returns, every connection the fake was
    /// relaying is closed, whatever exchange it was in: bytes of a response
    /// already sent stay with the client and the rest never come. A request
    /// the fake had already taken up has finished its model work; no later
    /// request reaches the fake's state or its request record. A connection
    /// made from now on ends before any byte of a response. The address
    /// stays the fake's.
    pub fn stop(&mut self) {
        {
            let mut front = lock_front(&self.front);
            // An address no thread accepts on stays so; `resume` says why.
            if !matches!(front.reach, Reach::Unattended) {
                front.reach = Reach::Stopped;
            }
            front.close_relayed();
        }
        // A request admitted before the switch does its model work under
        // the state lock; taking that lock once lets it finish before this
        // returns, and every later request finds its connection closed.
        drop(self.state.lock().unwrap_or_else(PoisonError::into_inner));
    }

    /// Answers again on the same address, with the state it had. A
    /// connection made while the fake was stopped stays unanswered even when
    /// it is accepted only now; from the moment this returns, every new
    /// connection is answered. When no thread accepts on the address any
    /// more, whether before this call or while it waits, it fails the test
    /// with the failure that ended accepting.
    pub fn resume(&mut self) {
        let (resumed, answering) = mpsc::channel();
        let marker = {
            let mut front = lock_front(&self.front);
            if matches!(front.reach, Reach::Unattended) {
                drop(front);
                self.cannot_resume();
            }
            if !matches!(front.reach, Reach::Stopped) {
                return;
            }
            // Made under the lock, so the accept thread can only classify
            // this connection once it knows it is the marker.
            let marker = TcpStream::connect(self.addr).unwrap_or_else(|cause| {
                panic!("the fake GitHub could not resume: connect to its address: {cause}")
            });
            let from = marker.local_addr().unwrap_or_else(|cause| {
                panic!("the fake GitHub could not resume: read its marker's address: {cause}")
            });
            front.reach = Reach::Resuming {
                marker: from,
                resumed,
            };
            marker
        };
        // The accept thread tells this wait once it takes the marker. If
        // accepting ends first, by any route, the thread replaces this reach
        // and so drops `resumed`, which ends the wait.
        if answering.recv().is_err() {
            self.cannot_resume();
        }
        drop(marker);
    }

    /// Fails a `resume` no thread can answer, with the recorded failure
    /// that ended accepting.
    fn cannot_resume(&self) -> ! {
        let failures = lock_front(&self.front).failures.join("\n");
        panic!("the fake GitHub stopped accepting before it answered again:\n{failures}");
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

    /// Makes the repository `owner/name` public.
    pub fn set_repo_public(&self, full_name: &str) -> &Self {
        self.lock()
            .repos
            .get_mut(full_name)
            .expect("repository was added")
            .public = true;
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
        self.lock().repo(repo).issues.get_mut(&number).unwrap().pull =
            Some(Pull::new(format!("pull-{number}"), "main".into()));
        number
    }

    /// Opens a pull request as `seed` describes; returns its number. Its
    /// head commit is this repository's branch `seed.head`.
    pub fn seed_pull(&self, repo: &str, seed: PullSeed) -> u64 {
        let number = self.seed_issue(
            repo,
            IssueSeed {
                title: seed.title,
                login: seed.login,
                ..IssueSeed::default()
            },
        );
        let mut pull = Pull::new(seed.head, seed.base);
        pull.draft = seed.draft;
        pull.fork = seed.fork;
        if let Some(association) = seed.association {
            pull.association = association;
        }
        self.lock().repo(repo).issues.get_mut(&number).unwrap().pull = Some(pull);
        number
    }

    /// Marks pull request `number` a draft or ready for review.
    pub fn set_pull_draft(&self, repo: &str, number: u64, draft: bool) {
        let mut state = self.lock();
        let now = state.now();
        let issue = state.repo(repo).issues.get_mut(&number).unwrap();
        issue.pull.as_mut().unwrap().draft = draft;
        issue.updated_at = now;
    }

    /// Makes `login` show on GitHub as an account of `kind`
    /// (`Organization` or `User`).
    pub fn set_account(&self, login: &str, kind: &str) -> &Self {
        self.lock()
            .accounts
            .insert(login.to_owned(), kind.to_owned());
        self
    }

    /// Installs the project App on the organisation `org`, with the
    /// permissions its installation holds
    /// ([`Self::set_installation_permissions`]).
    pub fn install_on_org(&self, org: &str) -> &Self {
        self.lock().org_installations.insert(org.to_owned());
        self
    }

    /// Initialises the bare remote of each repository created from now on
    /// as `<remotes>/<owner>/<name>.git`.
    pub fn serve_remotes(&self, remotes: &std::path::Path) -> &Self {
        self.lock().remotes = Some(remotes.to_path_buf());
        self
    }

    /// The collaborators added to `repo`, each with the permission given.
    pub fn collaborators(&self, repo: &str) -> Vec<(String, String)> {
        self.lock()
            .repos
            .get(repo)
            .map(|repo| repo.collaborators.clone().into_iter().collect())
            .unwrap_or_default()
    }

    /// Whether `repo` is private; `None` for a repository the fake does not
    /// hold.
    pub fn is_private(&self, repo: &str) -> Option<bool> {
        self.lock().repos.get(repo).map(|repo| !repo.public)
    }

    /// The default branch of `repo`.
    pub fn default_branch(&self, repo: &str) -> Option<String> {
        self.lock()
            .repos
            .get(repo)
            .map(|repo| repo.default_branch.clone())
    }

    /// What App `app_id`'s installation may do, as GitHub answers it for
    /// the installation and for every credential requested from it.
    pub fn set_installation_permissions(&self, app_id: u64, permissions: Value) -> &Self {
        self.lock()
            .installation_permissions
            .insert(app_id, permissions);
        self
    }

    /// Every status posted for `sha`, newest first.
    pub fn statuses(&self, repo: &str, sha: &str) -> Vec<Value> {
        self.lock().repos[repo].statuses_json(sha)
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

    /// Records a commit status for `sha` under `context`, posted by the
    /// fake's own login.
    pub fn set_status(&self, repo: &str, sha: &str, context: &str, state: StatusState) -> &Self {
        let mut guard = self.lock();
        let id = guard.next_id();
        let now = guard.now();
        let creator = guard.login.clone();
        guard
            .repo(repo)
            .post_status(
                sha,
                Status {
                    id,
                    context: context.to_owned(),
                    state,
                    description: None,
                    target_url: None,
                    creator,
                    updated_at: now,
                },
            )
            .unwrap();
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

    /// Holds the next matching response at the network boundary, while
    /// every other request is still served. Dropping the returned guard lets
    /// the server answer, including during unwind.
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

    pub fn reviews(&self, repo: &str, number: u64) -> Vec<Value> {
        self.lock().repos[repo].issues[&number]
            .pull
            .as_ref()
            .unwrap()
            .reviews
            .clone()
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }
}

impl Drop for FakeGitHub {
    fn drop(&mut self) {
        {
            let mut front = lock_front(&self.front);
            front.reach = Reach::Ending;
            front.close_relayed();
        }
        if let Some(accepting) = self.accepting.take() {
            // A blocking accept ends only on a connection, which then meets
            // `Ending`. An accept thread that already ended needs none.
            let woken = if accepting.is_finished() {
                Ok(None)
            } else {
                TcpStream::connect(self.addr).map(Some)
            };
            match woken {
                // The thread records its own panic.
                Ok(_wake) => {
                    let _ended = accepting.join();
                }
                // Joining a thread nothing can wake would wait for a client
                // that may never come. The thread is left blocked, holding
                // the address until the test process exits, and the test
                // fails below.
                Err(cause) => {
                    lock_front(&self.front).fail("wake the accept thread to end it", &cause)
                }
            }
        }
        self.server.unblock();
        if let Some(serving) = self.serving.take() {
            // The thread records its own panic.
            let _ended = serving.join();
        }
        let failures = std::mem::take(&mut lock_front(&self.front).failures);
        if failures.is_empty() {
            return;
        }
        let report = format!("the fake GitHub failed:\n{}", failures.join("\n"));
        if std::thread::panicking() {
            // A second panic would abort the test; its captured output
            // carries the fake's failure beside the test's own.
            eprintln!("{report}");
        } else {
            panic!("{report}");
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
