//! Connectors hosted by `instrukt run` (`slack-connector` spec): one Socket
//! Mode connection per declared Slack connector, opened outward with the
//! app token. Each envelope is acknowledged as soon as what it carries is
//! retained; deciding who is answered from Slack's record of the sender,
//! reading the accompanying messages, posting refusals and keeping Slack's
//! status line follow from the retained record, retried until done. Only
//! mentions and direct messages reach the agent.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use instrukt_core::config::{Configuration, Env, SecretSource, SlackConnector};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use jiff::Timestamp;
use serde_json::Value;

use crate::conversations::{
    self, Accompanying, ConnectionState, Earlier, Incoming, Intake, Sender,
};

/// The connector's key under `connectors`.
pub const SLACK: &str = "slack";

/// The one fixed reply a sender who is not one of the company's own
/// receives, in a channel and in a direct message alike.
pub const TURN_AWAY: &str = "I only answer members of this workspace.";

/// The text of Slack's status line while the agent works; Slack puts the
/// app's name in front of it.
pub const WORKING: &str = "is working on your request";

/// Slack's own user that posts its notices; never one of the company's own.
const SLACKBOT: &str = "USLACKBOT";

/// The subtypes of messages people write; any other subtype is one of
/// Slack's own notices, such as a person joining a channel.
const WRITTEN: [&str; 4] = [
    "thread_broadcast",
    "file_share",
    "bot_message",
    "me_message",
];

/// How often a status that stays is set again: Slack drops it after two
/// minutes.
const STATUS_RENEWAL: Duration = Duration::from_secs(50);

/// The longest wait between two passes of a company's intake, so a status
/// that stays is renewed in time.
const PASS: Duration = Duration::from_secs(10);

/// The longest wait before retrying intake work that failed.
const MOST_BACKOFF: Duration = Duration::from_secs(60);

/// The setting that points the connector at a local stand-in for Slack.
/// Only a loopback address is accepted.
pub const API_URL_SETTING: &str = "INSTRUKT_SLACK_API_URL";

/// Slack's Web API.
const SLACK_API: &str = "https://slack.com/api";

/// How long retained intake work that failed first waits before it is
/// retried; each further failure doubles it, up to [`MOST_BACKOFF`].
const RETRY: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    /// A Web API failure, naming the method and Slack's error code; never a
    /// token.
    #[error("Slack {method}: {cause}")]
    Api { method: String, cause: String },
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
    #[error(
        "`{API_URL_SETTING}` may only name a loopback address (http://127.0.0.1, http://localhost or http://[::1]); unset it to reach Slack"
    )]
    Endpoint,
    #[error("Slack offered a Socket Mode address outside its own TLS hosts; nothing was connected")]
    Socket,
}

fn uri(text: &str) -> Option<tungstenite::http::Uri> {
    text.parse().ok()
}

fn loopback(uri: &tungstenite::http::Uri) -> bool {
    matches!(
        uri.host(),
        Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
    )
}

/// Where the connector reaches Slack: Slack's own TLS hosts, or a local
/// stand-in on a loopback address. Read once when the runner starts.
#[derive(Debug, Clone)]
pub struct SlackEndpoint {
    api: String,
    local: bool,
}

impl SlackEndpoint {
    /// Slack's own hosts unless [`API_URL_SETTING`] names a loopback
    /// address; any other value refuses.
    pub fn from_env(env: &Env) -> Result<Self, SlackError> {
        match env
            .get(API_URL_SETTING)
            .filter(|url| !url.trim().is_empty())
        {
            None => Ok(Self {
                api: SLACK_API.into(),
                local: false,
            }),
            Some(url) => {
                let parsed = uri(url).ok_or(SlackError::Endpoint)?;
                if !matches!(parsed.scheme_str(), Some("http" | "https")) || !loopback(&parsed) {
                    return Err(SlackError::Endpoint);
                }
                Ok(Self {
                    api: url.trim_end_matches('/').to_owned(),
                    local: true,
                })
            }
        }
    }

    /// Whether a Socket Mode address Slack offered may be connected to:
    /// one of Slack's TLS hosts, or the local stand-in's loopback address.
    fn admits_socket(&self, url: &str) -> bool {
        let Some(parsed) = uri(url) else {
            return false;
        };
        if self.local {
            return matches!(parsed.scheme_str(), Some("ws" | "wss")) && loopback(&parsed);
        }
        parsed.scheme_str() == Some("wss")
            && parsed
                .host()
                .is_some_and(|host| host == "slack.com" || host.ends_with(".slack.com"))
    }
}

/// One company's Slack Web API, with its tokens resolved from the company
/// home's own secrets file.
pub struct SlackApi {
    endpoint: SlackEndpoint,
    app_token: String,
    bot_token: String,
    agent: ureq::Agent,
}

/// A Slack message time (`<seconds>.<micros>`) as microseconds.
fn ts_micros(ts: &str) -> Option<i64> {
    let (seconds, micros) = ts.split_once('.').unwrap_or((ts, "0"));
    let seconds: i64 = seconds.parse().ok()?;
    let micros: i64 = format!("{micros:0<6}").get(..6)?.parse().ok()?;
    seconds.checked_mul(1_000_000)?.checked_add(micros)
}

fn ts_time(ts: &str) -> Option<Timestamp> {
    Timestamp::from_microsecond(ts_micros(ts)?).ok()
}

fn message_ts(m: &Value) -> Option<i64> {
    m.get("ts").and_then(Value::as_str).and_then(ts_micros)
}

impl SlackApi {
    pub fn for_company(
        home: &Home,
        company: &ProjectSlug,
        endpoint: &SlackEndpoint,
    ) -> Result<Self, SlackError> {
        let declared = Configuration::load_company(home, company)?
            .slack_connector()?
            .ok_or_else(|| SlackError::Api {
                method: "connector".into(),
                cause: "the company declares no slack connector".into(),
            })?;
        Self::with(home, company, &declared, endpoint)
    }

    fn with(
        home: &Home,
        company: &ProjectSlug,
        declared: &SlackConnector,
        endpoint: &SlackEndpoint,
    ) -> Result<Self, SlackError> {
        let secrets = SecretSource::Company(company.clone());
        Ok(Self {
            endpoint: endpoint.clone(),
            app_token: instrukt_core::config::resolve_secret_from(
                home,
                &secrets,
                &declared.app_token,
            )?,
            bot_token: instrukt_core::config::resolve_secret_from(
                home,
                &secrets,
                &declared.bot_token,
            )?,
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(30)))
                .http_status_as_error(false)
                .build()
                .into(),
        })
    }

    fn call(&self, method: &str, app: bool, form: &[(&str, &str)]) -> Result<Value, SlackError> {
        let token = if app {
            &self.app_token
        } else {
            &self.bot_token
        };
        let fail = |cause: String| SlackError::Api {
            method: method.into(),
            cause,
        };
        let mut response = self
            .agent
            .post(format!("{}/{method}", self.endpoint.api))
            .header("Authorization", format!("Bearer {token}"))
            .send_form(form.iter().copied())
            .map_err(|e| fail(e.to_string()))?;
        let status = response.status();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|_| fail(format!("HTTP {status} without an answer")))?;
        let body: Value = serde_json::from_str(&text)
            .map_err(|_| fail(format!("HTTP {status} without a JSON answer")))?;
        if body.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(body)
        } else {
            Err(fail(
                body.get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("not ok")
                    .to_owned(),
            ))
        }
    }

    /// The Socket Mode address for a new connection.
    fn connection_url(&self) -> Result<String, SlackError> {
        let body = self.call("apps.connections.open", true, &[])?;
        let url = body
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| SlackError::Api {
                method: "apps.connections.open".into(),
                cause: "no url".into(),
            })?;
        if !self.endpoint.admits_socket(url) {
            return Err(SlackError::Socket);
        }
        Ok(url.to_owned())
    }

    /// The bot's own user and bot ids, to mark the agent's own posts, and
    /// the workspace the app is installed in.
    fn identity(&self) -> Result<Me, SlackError> {
        let body = self.call("auth.test", false, &[])?;
        let field = |name: &str| {
            body.get(name)
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| SlackError::Api {
                    method: "auth.test".into(),
                    cause: format!("no {name}"),
                })
        };
        Ok(Me {
            user: field("user_id")?,
            team: field("team_id")?,
            bot: body
                .get("bot_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    /// Slack's record of the person `user`.
    fn user(&self, user: &str) -> Result<Value, SlackError> {
        let body = self.call("users.info", false, &[("user", user)])?;
        body.get("user").cloned().ok_or_else(|| SlackError::Api {
            method: "users.info".into(),
            cause: "no user".into(),
        })
    }

    /// Set Slack's status line on a conversation's thread, or clear it with
    /// an empty `status`.
    fn status(&self, channel: &str, thread: &str, status: &str) -> Result<(), SlackError> {
        self.call(
            "assistant.threads.setStatus",
            false,
            &[
                ("channel_id", channel),
                ("thread_ts", thread),
                ("status", status),
            ],
        )
        .map(drop)
    }

    /// Post `text` into a conversation: its thread in a channel, or the
    /// direct-message channel.
    pub fn post(&self, conversation: &str, text: &str) -> Result<(), SlackError> {
        let (channel, thread) = split_conversation(conversation);
        let mut form = vec![("channel", channel), ("text", text)];
        if let Some(thread) = thread {
            form.push(("thread_ts", thread));
        }
        self.call("chat.postMessage", false, &form).map(drop)
    }

    /// The most recent `cap` messages of a channel or thread strictly
    /// between `oldest` and `latest`, oldest first, and whether the range
    /// held more. Channel history arrives newest first, so the read stops
    /// once one message beyond the cap is in hand; a thread's replies arrive
    /// oldest first, so they are read to the end of the range while only the
    /// most recent `cap` are held.
    fn read(
        &self,
        channel: &str,
        thread: Option<&str>,
        oldest: Option<&str>,
        latest: &str,
        cap: usize,
    ) -> Result<(Vec<Value>, bool), SlackError> {
        let method = if thread.is_some() {
            "conversations.replies"
        } else {
            "conversations.history"
        };
        let latest_us = ts_micros(latest).ok_or_else(|| SlackError::Api {
            method: method.into(),
            cause: "invalid message time".into(),
        })?;
        let oldest_us = oldest.and_then(ts_micros);
        // History needs one beyond the cap; replies take the largest page.
        let limit = if thread.is_none() {
            cap.saturating_add(1).min(200)
        } else {
            200
        }
        .to_string();
        let mut cursor: Option<String> = None;
        let mut held: std::collections::VecDeque<(i64, Value)> = std::collections::VecDeque::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut left_out = false;
        loop {
            let mut form = vec![
                ("channel", channel),
                ("latest", latest),
                ("inclusive", "false"),
                ("limit", limit.as_str()),
            ];
            if let Some(oldest) = oldest {
                form.push(("oldest", oldest));
            }
            if let Some(thread) = thread {
                form.push(("ts", thread));
            }
            if let Some(cursor) = &cursor {
                form.push(("cursor", cursor));
            }
            let body = self.call(method, false, &form)?;
            let page = body
                .get("messages")
                .and_then(Value::as_array)
                .ok_or_else(|| SlackError::Api {
                    method: method.into(),
                    cause: "no messages".into(),
                })?;
            for message in page {
                // Slack's own notices accompany no message and do not
                // count toward the cap.
                if notice(message) {
                    continue;
                }
                let Some(at) = message_ts(message)
                    .filter(|at| *at < latest_us && oldest_us.is_none_or(|o| *at > o))
                else {
                    continue;
                };
                if !seen.insert(at) {
                    continue;
                }
                held.push_back((at, message.clone()));
                if thread.is_some() && held.len() > cap {
                    // Oldest first: the earliest held message is left out.
                    held.pop_front();
                    left_out = true;
                }
            }
            // Newest first: one beyond the cap shows earlier ones exist.
            if thread.is_none() && held.len() > cap {
                left_out = true;
                break;
            }
            cursor = body
                .get("response_metadata")
                .and_then(|m| m.get("next_cursor"))
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_owned);
            if cursor.is_none() || body.get("has_more").and_then(Value::as_bool) == Some(false) {
                break;
            }
        }
        let mut found: Vec<(i64, Value)> = held.into_iter().collect();
        found.sort_by_key(|(at, _)| *at);
        if found.len() > cap {
            found.drain(..found.len() - cap);
        }
        Ok((found.into_iter().map(|(_, m)| m).collect(), left_out))
    }
}

/// The connector's own identity in its workspace.
#[derive(Debug, Clone)]
struct Me {
    user: String,
    bot: Option<String>,
    /// The workspace the app is installed in.
    team: String,
}

/// Whether `message` is one of Slack's own notices rather than something a
/// person or an app wrote.
fn notice(message: &Value) -> bool {
    message
        .get("subtype")
        .and_then(Value::as_str)
        .is_some_and(|subtype| !WRITTEN.contains(&subtype))
}

/// Whether Slack's record shows a person who is one of the company's own:
/// an active account, no bot and not Slackbot, no guest, and of the
/// workspace the app is installed in. A record that leaves one of these
/// undecided admits no one. The name is the display name, the real name
/// when that is empty, the Slack id when Slack gives neither.
fn admission(record: &Value, me: &Me) -> Option<Sender> {
    let flag = |name: &str| record.get(name).and_then(Value::as_bool);
    let id = record.get("id").and_then(Value::as_str)?;
    let enterprise_teams = record
        .get("enterprise_user")
        .and_then(|e| e.get("teams"))
        .and_then(Value::as_array);
    let in_workspace = record.get("team_id").and_then(Value::as_str) == Some(me.team.as_str())
        || enterprise_teams.is_some_and(|teams| {
            teams
                .iter()
                .any(|team| team.as_str() == Some(me.team.as_str()))
        });
    let own = flag("deleted") == Some(false)
        && flag("is_bot") == Some(false)
        && id != SLACKBOT
        && flag("is_restricted") == Some(false)
        && flag("is_ultra_restricted") == Some(false)
        && in_workspace;
    own.then(|| Sender {
        name: name(record).unwrap_or_else(|| id.to_owned()),
        email: record
            .get("profile")
            .and_then(|p| p.get("email"))
            .and_then(Value::as_str)
            .filter(|email| !email.is_empty())
            .map(str::to_owned),
    })
}

/// A person's name in Slack's record: their display name, or their real
/// name when that is empty.
fn name(record: &Value) -> Option<String> {
    let profile = record.get("profile");
    [
        profile.and_then(|p| p.get("display_name")),
        profile.and_then(|p| p.get("real_name")),
        record.get("real_name"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::trim)
    .find(|name| !name.is_empty())
    .map(str::to_owned)
}

/// The people's names one intake pass needs, each asked of Slack once. A
/// name Slack cannot give is the Slack id.
struct Names<'a> {
    api: &'a SlackApi,
    known: HashMap<String, String>,
}
impl Names<'_> {
    fn of(&mut self, user: &str) -> String {
        if let Some(name) = self.known.get(user) {
            return name.clone();
        }
        let found = self
            .api
            .user(user)
            .ok()
            .and_then(|record| name(&record))
            .unwrap_or_else(|| user.to_owned());
        self.known.insert(user.to_owned(), found.clone());
        found
    }

    /// `text` with every mention written as `@` and that person's name;
    /// nothing else changes.
    fn written(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("<@") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            match after.find('>') {
                Some(end) => {
                    let inner = &after[..end];
                    let user = inner.split('|').next().unwrap_or(inner);
                    out.push('@');
                    out.push_str(&self.of(user));
                    rest = &after[end + 1..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }
}

/// The Slack ids `text` mentions in Slack's markup.
pub fn mentioned(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('>') else {
            break;
        };
        let inner = &after[..end];
        found.push(inner.split('|').next().unwrap_or(inner).to_owned());
        rest = &after[end + 1..];
    }
    found
}

/// `<channel>/<thread ts>` for a channel thread, `<channel>` for a direct
/// message.
fn split_conversation(conversation: &str) -> (&str, Option<&str>) {
    match conversation.split_once('/') {
        Some((channel, thread)) => (channel, Some(thread)),
        None => (conversation, None),
    }
}

/// Wakes the runner's supervision when a connection thread ends, whether it
/// returned or unwound.
struct ExitNotice(SyncSender<()>);
impl Drop for ExitNotice {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

struct Hosted {
    company: ProjectSlug,
    declared: SlackConnector,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The connections `instrukt run` holds open, supervised by the run loop.
pub struct Connections {
    home: Home,
    endpoint: SlackEndpoint,
    stop: Arc<AtomicBool>,
    wake: SyncSender<()>,
    supervise: SyncSender<()>,
    hosted: Vec<Hosted>,
}

impl Connections {
    /// Open one connection per declared Slack connector. `wake` tells the
    /// relay that retained work arrived; `supervise` tells the run loop that
    /// a connection thread ended.
    pub fn start(
        home: &Home,
        endpoint: &SlackEndpoint,
        wake: SyncSender<()>,
        supervise: SyncSender<()>,
    ) -> Result<Self, conversations::ConversationError> {
        let mut connections = Self {
            home: home.clone(),
            endpoint: endpoint.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            wake,
            supervise,
            hosted: Vec::new(),
        };
        for (company, declared) in conversations::demand(home)? {
            let thread = connections.spawn(&company, &declared);
            connections.hosted.push(Hosted {
                company,
                declared,
                thread,
            });
        }
        Ok(connections)
    }

    fn spawn(
        &self,
        company: &ProjectSlug,
        declared: &SlackConnector,
    ) -> Option<std::thread::JoinHandle<()>> {
        let home = self.home.clone();
        let endpoint = self.endpoint.clone();
        let stop = Arc::clone(&self.stop);
        let wake = self.wake.clone();
        let notice = ExitNotice(self.supervise.clone());
        let company = company.clone();
        let declared = declared.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("slack-{company}"))
            .spawn(move || {
                let _notice = notice;
                connection(&home, &endpoint, &company, &declared, &stop, &wake);
            });
        match spawned {
            Ok(thread) => Some(thread),
            Err(error) => {
                tracing::error!(event = "connector_start_failed", error = %error);
                None
            }
        }
    }

    /// Restart every connection whose thread ended unexpectedly, retaining
    /// the cause as that connector's failure.
    pub fn check(&mut self) {
        for index in 0..self.hosted.len() {
            let ended = self.hosted[index]
                .thread
                .as_ref()
                .is_none_or(std::thread::JoinHandle::is_finished);
            if !ended || self.stop.load(Ordering::Acquire) {
                continue;
            }
            let cause = match self.hosted[index]
                .thread
                .take()
                .map(std::thread::JoinHandle::join)
            {
                Some(Err(_)) => "the connection's intake panicked; restarted",
                _ => "the connection's intake stopped unexpectedly; restarted",
            };
            let hosted = &self.hosted[index];
            tracing::error!(event = "connector_intake_stopped", company = %hosted.company, connector = SLACK, cause);
            state(
                &self.home,
                &hosted.company,
                ConnectionState::Failed,
                Some(cause),
            );
            let thread = self.spawn(&hosted.company, &hosted.declared);
            self.hosted[index].thread = thread;
        }
    }
}

impl Drop for Connections {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for hosted in &mut self.hosted {
            if let Some(thread) = hosted.thread.take()
                && thread.join().is_err()
            {
                tracing::error!(event = "connector_thread_panicked");
            }
        }
    }
}

fn state(home: &Home, company: &ProjectSlug, status: ConnectionState, failure: Option<&str>) {
    if let Err(error) = conversations::connector_state(home, company, SLACK, status, failure) {
        tracing::warn!(event = "connector_state_unrecorded", company = %company, error = %error);
    }
}

/// Keep one company's connection open until `stop`: a refused or dropped
/// connection is recorded and retried, and retained intake work continues
/// while it is down.
fn connection(
    home: &Home,
    endpoint: &SlackEndpoint,
    company: &ProjectSlug,
    declared: &SlackConnector,
    stop: &AtomicBool,
    wake: &SyncSender<()>,
) {
    // Intake runs beside the socket, so reading what accompanies a message
    // or posting a refusal never holds back the next acknowledgement. Its
    // end ends this connection, which the run loop then restarts.
    let ended = AtomicBool::new(false);
    let (nudge, nudges) = std::sync::mpsc::sync_channel::<()>(1);
    let ended = &ended;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            struct Ended<'a>(&'a AtomicBool);
            impl Drop for Ended<'_> {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            let _ended = Ended(ended);
            intake_worker(home, endpoint, company, declared, stop, wake, &nudges);
        });
        let quit = || stop.load(Ordering::Acquire) || ended.load(Ordering::Acquire);
        let mut backoff = Duration::from_secs(1);
        while !quit() {
            state(home, company, ConnectionState::Connecting, None);
            let failure = session(home, endpoint, company, declared, &quit, &nudge).err();
            if quit() {
                break;
            }
            if let Some(error) = &failure {
                tracing::warn!(event = "connector_failed", company = %company, connector = SLACK, error = %error);
                state(home, company, ConnectionState::Failed, Some(error));
            }
            let until = Instant::now() + backoff;
            while Instant::now() < until && !quit() {
                std::thread::sleep(Duration::from_millis(100));
            }
            backoff = if failure.is_some() {
                (backoff * 2).min(Duration::from_secs(60))
            } else {
                Duration::from_secs(1)
            };
        }
    });
    if stop.load(Ordering::Acquire) {
        state(home, company, ConnectionState::Stopped, None);
    }
}

/// The company's retained intake work, until `stop`: run when a retained
/// envelope nudges it, and again after a failure once its backoff has
/// passed. Slack's status line is kept on every pass.
fn intake_worker(
    home: &Home,
    endpoint: &SlackEndpoint,
    company: &ProjectSlug,
    declared: &SlackConnector,
    stop: &AtomicBool,
    wake: &SyncSender<()>,
    nudges: &std::sync::mpsc::Receiver<()>,
) {
    let mut me = None;
    let mut statuses = Statuses::default();
    let mut backoff = RETRY;
    let mut retry_at = Instant::now();
    let mut nudged = true;
    while !stop.load(Ordering::Acquire) {
        match SlackApi::with(home, company, declared, endpoint) {
            Ok(api) => {
                if me.is_none() {
                    me = api.identity().ok();
                }
                if nudged || Instant::now() >= retry_at {
                    let clean = intake(home, company, declared, &api, me.as_ref(), wake);
                    // A failed lookup or read is retried later, within
                    // Slack's limits; a clean pass resets the wait.
                    backoff = if clean {
                        RETRY
                    } else {
                        (backoff * 2).min(MOST_BACKOFF)
                    };
                    retry_at = Instant::now() + backoff;
                }
                statuses.keep(home, company, &api);
            }
            // A failure before any request stays with the outstanding work.
            Err(error) => {
                if let Err(record) =
                    conversations::intake_failed(home, company, SLACK, &error.to_string())
                {
                    tracing::warn!(event = "connector_intake_failed", company = %company, connector = SLACK, error = %record);
                }
                retry_at = Instant::now() + RETRY;
            }
        }
        nudged = false;
        let until = Instant::now()
            + PASS.min(
                retry_at
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(100)),
            );
        while Instant::now() < until && !stop.load(Ordering::Acquire) {
            if nudges.recv_timeout(Duration::from_millis(100)).is_ok() {
                nudged = true;
                break;
            }
        }
    }
}

/// Slack's status line, kept for every admitted message whose turn has not
/// ended: on a channel conversation's thread while any of its messages is
/// unfinished, on a direct message's addressed message while that message
/// is. A status is set when it starts, again when its messages change (Slack
/// drops it when the app posts) and at least once a minute, and cleared when
/// it no longer stays. After a restart the same holds for every unfinished
/// message. A status Slack refuses is logged and fails nothing.
#[derive(Default)]
struct Statuses {
    /// Each status set: its channel and thread, the messages it stands for
    /// and when it was last set.
    set: HashMap<(String, String), (Vec<String>, Instant)>,
}
impl Statuses {
    fn keep(&mut self, home: &Home, company: &ProjectSlug, api: &SlackApi) {
        let working = match conversations::working(home, company, SLACK) {
            Ok(working) => working,
            Err(error) => {
                tracing::warn!(event = "connector_status_unread", company = %company, error = %error);
                return;
            }
        };
        let mut wanted: HashMap<(String, String), Vec<String>> = HashMap::new();
        for (conversation, message) in working {
            let (channel, thread) = split_conversation(&conversation);
            let target = (channel.to_owned(), thread.unwrap_or(&message).to_owned());
            wanted.entry(target).or_default().push(message);
        }
        let now = Instant::now();
        for (target, messages) in &wanted {
            let due = self.set.get(target).is_none_or(|(set, at)| {
                set != messages || now.duration_since(*at) >= STATUS_RENEWAL
            });
            if due {
                self.call(api, company, target, WORKING);
                self.set.insert(target.clone(), (messages.clone(), now));
            }
        }
        let ended: Vec<_> = self
            .set
            .keys()
            .filter(|target| !wanted.contains_key(*target))
            .cloned()
            .collect();
        for target in ended {
            self.call(api, company, &target, "");
            self.set.remove(&target);
        }
    }

    fn call(&self, api: &SlackApi, company: &ProjectSlug, target: &(String, String), status: &str) {
        if let Err(error) = api.status(&target.0, &target.1, status) {
            tracing::warn!(event = "connector_status_refused", company = %company, connector = SLACK, error = %error);
        }
    }
}

/// One Socket Mode connection, from opening to its drop or a disconnect.
fn session(
    home: &Home,
    endpoint: &SlackEndpoint,
    company: &ProjectSlug,
    declared: &SlackConnector,
    quit: &dyn Fn() -> bool,
    nudge: &SyncSender<()>,
) -> Result<(), String> {
    let api = SlackApi::with(home, company, declared, endpoint).map_err(|e| e.to_string())?;
    let url = api.connection_url().map_err(|e| e.to_string())?;
    let me = api.identity().map_err(|e| e.to_string())?;
    let (mut socket, _) =
        tungstenite::connect(url.as_str()).map_err(|e| format!("Socket Mode connection: {e}"))?;
    match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => {
            stream.set_read_timeout(Some(Duration::from_millis(200)))
        }
        tungstenite::stream::MaybeTlsStream::Rustls(stream) => stream
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(200))),
        _ => Ok(()),
    }
    .map_err(|e| format!("Socket Mode connection: {e}"))?;
    state(home, company, ConnectionState::Connected, None);
    tracing::info!(event = "connector_connected", company = %company, connector = SLACK);
    let mut unretained = false;
    while !quit() {
        let message = match socket.read() {
            Ok(message) => message,
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(error) => return Err(format!("Socket Mode connection dropped: {error}")),
        };
        let text = match message {
            tungstenite::Message::Text(text) => text,
            tungstenite::Message::Close(_) => return Ok(()),
            _ => continue,
        };
        let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        match envelope.get("type").and_then(Value::as_str) {
            Some("hello") => continue,
            Some("disconnect") => return Ok(()),
            _ => {}
        }
        let Some(id) = envelope.get("envelope_id").and_then(Value::as_str) else {
            continue;
        };
        // Acknowledge only once what the envelope carries is retained, so
        // Slack retries what instrukt could not keep; reading the
        // accompanying messages and posting refusals come after.
        match receive(home, company, &me, &envelope) {
            Ok(()) => {
                socket
                    .send(tungstenite::Message::text(
                        serde_json::json!({ "envelope_id": id }).to_string(),
                    ))
                    .map_err(|e| format!("Socket Mode acknowledgement: {e}"))?;
                if unretained {
                    state(home, company, ConnectionState::Connected, None);
                    unretained = false;
                }
                let _ = nudge.try_send(());
            }
            Err(error) => {
                tracing::warn!(event = "connector_message_unretained", company = %company, connector = SLACK, error = %error);
                state(
                    home,
                    company,
                    ConnectionState::Connected,
                    Some(&format!(
                        "a message could not be retained and awaits Slack's retry: {error}"
                    )),
                );
                unretained = true;
            }
        }
    }
    let _ = socket.close(None);
    Ok(())
}

/// Retain one Events API envelope if it is a mention in a channel or a
/// direct message. Nothing here waits on the network.
fn receive(home: &Home, company: &ProjectSlug, me: &Me, envelope: &Value) -> Result<(), String> {
    if envelope.get("type").and_then(Value::as_str) != Some("events_api") {
        return Ok(());
    }
    let Some(event) = envelope.get("payload").and_then(|p| p.get("event")) else {
        return Ok(());
    };
    let text = |field: &str| event.get(field).and_then(Value::as_str);
    let (Some(channel), Some(ts), Some(sender)) = (text("channel"), text("ts"), text("user"))
    else {
        return Ok(());
    };
    let thread = match text("type") {
        Some("app_mention") => Some(text("thread_ts").unwrap_or(ts)),
        Some("message")
            if text("channel_type") == Some("im")
                && text("subtype").is_none()
                && event.get("bot_id").is_none()
                && sender != me.user =>
        {
            None
        }
        _ => return Ok(()),
    };
    let incoming = Incoming {
        company: company.clone(),
        connector: SLACK.into(),
        conversation: match thread {
            Some(thread) => format!("{channel}/{thread}"),
            None => channel.to_owned(),
        },
        message: ts.to_owned(),
        sender: sender.to_owned(),
        written_at: ts_time(ts).ok_or("message without a valid time")?,
        text: text("text").unwrap_or_default().to_owned(),
    };
    match conversations::retain(home, &incoming, Timestamp::now()).map_err(|e| e.to_string())? {
        Intake::Retained | Intake::Repeated => Ok(()),
    }
}

/// Retained intake work of one company: post owed refusals, decide for each
/// received message from Slack's record of its sender read now whether it is
/// answered, and read what accompanies each admitted one. A failure stays
/// with its record and is retried; while Slack cannot be asked, a message
/// waits and nothing is delivered or replied. The relay is woken for what
/// became deliverable. Returns whether the pass ended without a failure.
fn intake(
    home: &Home,
    company: &ProjectSlug,
    declared: &SlackConnector,
    api: &SlackApi,
    me: Option<&Me>,
    wake: &SyncSender<()>,
) -> bool {
    let mut clean = true;
    let result = (|| -> Result<(), conversations::ConversationError> {
        let owned;
        let me = match me {
            Some(me) => me,
            None => match api.identity() {
                Ok(identity) => {
                    owned = identity;
                    &owned
                }
                Err(error) => {
                    clean = false;
                    for unread in conversations::unread(home, company, SLACK)? {
                        conversations::waiting(home, company, SLACK, &unread, &error.to_string())?;
                    }
                    return Ok(());
                }
            },
        };
        let mut names = Names {
            api,
            known: HashMap::new(),
        };
        let mut readable = false;
        for unread in conversations::unread(home, company, SLACK)? {
            if !unread.admitted {
                let Some(sender) = unread.sender.as_deref() else {
                    clean = false;
                    conversations::waiting(
                        home,
                        company,
                        SLACK,
                        &unread,
                        "the message has no sender",
                    )?;
                    continue;
                };
                match api.user(sender) {
                    Ok(record) => match admission(&record, me) {
                        Some(admitted) => {
                            names.known.insert(sender.to_owned(), admitted.name.clone());
                            conversations::admitted(home, company, SLACK, &unread, &admitted)?;
                        }
                        None => {
                            conversations::refused(
                                home,
                                company,
                                SLACK,
                                &unread,
                                sender,
                                Timestamp::now(),
                            )?;
                            continue;
                        }
                    },
                    // Slack answered that it knows no such person.
                    Err(SlackError::Api { cause, .. })
                        if cause == "user_not_found" || cause == "user_not_visible" =>
                    {
                        conversations::refused(
                            home,
                            company,
                            SLACK,
                            &unread,
                            sender,
                            Timestamp::now(),
                        )?;
                        continue;
                    }
                    Err(error) => {
                        clean = false;
                        conversations::waiting(
                            home,
                            company,
                            SLACK,
                            &unread,
                            &format!("who sent it cannot be looked up yet: {error}"),
                        )?;
                        continue;
                    }
                }
            }
            let text = names.written(&unread.text);
            let context =
                accompanying(api, declared, me, &mut names, &unread).map_err(|e| e.to_string());
            readable |= context.is_ok();
            clean &= context.is_ok();
            conversations::captured(home, company, SLACK, &unread, &text, context)?;
        }
        if readable {
            let _ = wake.try_send(());
        }
        // Replies owed, those decided now among them.
        for refusal in conversations::unsent_refusals(home, company, SLACK)? {
            let outcome = api
                .post(&refusal.conversation, TURN_AWAY)
                .map_err(|e| e.to_string());
            clean &= outcome.is_ok();
            conversations::refusal_posted(
                home,
                company,
                SLACK,
                &refusal,
                outcome,
                Timestamp::now(),
            )?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        clean = false;
        tracing::warn!(event = "connector_intake_failed", company = %company, connector = SLACK, error = %error);
    }
    clean
}

/// What accompanies a received message: of the messages before the first
/// in a conversation — of the thread, or of the channel when it starts the
/// thread — or of those since the previous one, at most `context_messages`
/// of the most recent, and whether earlier ones were left out. Speakers are
/// named and mentions written as names, as for the addressed message.
fn accompanying(
    api: &SlackApi,
    declared: &SlackConnector,
    me: &Me,
    names: &mut Names<'_>,
    unread: &conversations::Unread,
) -> Result<Accompanying, SlackError> {
    let (channel, thread) = split_conversation(&unread.conversation);
    let ts = unread.message.as_str();
    let cap = usize::try_from(declared.context_messages).unwrap_or(usize::MAX);
    let (earlier, left_out) = match &unread.previous {
        Some(previous) => api.read(channel, thread, Some(previous), ts, cap)?,
        None => {
            let within = thread.filter(|thread| *thread != ts);
            api.read(channel, within, None, ts, cap)?
        }
    };
    let messages = earlier
        .iter()
        .map(|m| {
            let user = m.get("user").and_then(Value::as_str).unwrap_or_default();
            let own = m
                .get("bot_id")
                .and_then(Value::as_str)
                .is_some_and(|bot| Some(bot) == me.bot.as_deref())
                || (!user.is_empty() && user == me.user);
            let speaker = if user.is_empty() {
                m.get("username")
                    .or_else(|| m.get("bot_id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            } else {
                names.of(user)
            };
            Earlier {
                speaker,
                own,
                at: m.get("ts").and_then(Value::as_str).and_then(ts_time),
                text: names.written(m.get("text").and_then(Value::as_str).unwrap_or_default()),
            }
        })
        .collect();
    Ok(Accompanying { messages, left_out })
}

#[cfg(test)]
mod tests {
    use super::{Me, SlackEndpoint, admission, mentioned, notice, split_conversation, ts_micros};
    use instrukt_core::config::Env;
    use serde_json::{Value, json};

    fn me() -> Me {
        Me {
            user: "UAGENT".into(),
            bot: Some("BAGENT".into()),
            team: "T1".into(),
        }
    }

    fn member() -> Value {
        json!({"id": "U1", "team_id": "T1", "deleted": false, "is_bot": false, "is_restricted": false, "is_ultra_restricted": false, "profile": {"display_name": "", "real_name": "Alex Example", "email": "alex@example.com"}})
    }

    #[test]
    fn only_a_full_member_of_the_installed_workspace_is_answered() {
        let admitted = admission(&member(), &me()).unwrap();
        assert_eq!(admitted.email.as_deref(), Some("alex@example.com"));
        let changed = |field: &str, value: Value| {
            let mut record = member();
            record[field] = value;
            admission(&record, &me())
        };
        assert_eq!(changed("deleted", json!(true)), None);
        assert_eq!(changed("is_bot", json!(true)), None);
        assert_eq!(changed("is_restricted", json!(true)), None);
        assert_eq!(changed("is_ultra_restricted", json!(true)), None);
        assert_eq!(changed("team_id", json!("T2")), None);
        assert_eq!(changed("id", json!("USLACKBOT")), None);
        // A record that leaves a deciding field undecided admits no one.
        let mut undecided = member();
        undecided
            .as_object_mut()
            .unwrap()
            .remove("is_ultra_restricted");
        assert_eq!(admission(&undecided, &me()), None);
        // In an Enterprise organization, the workspaces it lists decide.
        let mut enterprise = changed_team(member());
        enterprise["enterprise_user"] = json!({"teams": ["T9", "T1"]});
        assert!(admission(&enterprise, &me()).is_some());
    }

    fn changed_team(mut record: Value) -> Value {
        record["team_id"] = json!("E1");
        record
    }

    #[test]
    fn a_name_falls_back_from_the_display_name_to_the_real_name_to_the_id() {
        let name = |profile: Value| {
            let mut record = member();
            record["profile"] = profile;
            record.as_object_mut().unwrap().remove("real_name");
            admission(&record, &me()).unwrap().name
        };
        assert_eq!(
            name(json!({"display_name": "alex", "real_name": "Alex Example"})),
            "alex"
        );
        assert_eq!(
            name(json!({"display_name": "", "real_name": "Alex Example"})),
            "Alex Example"
        );
        assert_eq!(name(json!({})), "U1");
    }

    #[test]
    fn mentions_and_notices_are_recognised_in_slacks_markup() {
        assert_eq!(mentioned("hi <@U1> and <@U2|sam>, <#C1>"), ["U1", "U2"]);
        assert!(mentioned("no mention").is_empty());
        assert!(notice(&json!({"subtype": "channel_join"})));
        assert!(!notice(&json!({"subtype": "thread_broadcast"})));
        assert!(!notice(&json!({"text": "a message"})));
    }

    #[test]
    fn slack_times_order_by_their_microseconds() {
        assert_eq!(ts_micros("1700000000.000100"), Some(1_700_000_000_000_100));
        assert!(ts_micros("1700000000.000000") < ts_micros("1700000000.0001"));
        assert_eq!(split_conversation("C1/170.1"), ("C1", Some("170.1")));
        assert_eq!(split_conversation("D1"), ("D1", None));
    }

    #[test]
    fn only_slack_or_a_loopback_stand_in_receives_tokens() {
        let at =
            |url: &str| SlackEndpoint::from_env(&Env::from_pairs([(super::API_URL_SETTING, url)]));
        assert!(SlackEndpoint::from_env(&Env::default()).is_ok_and(|e| !e.local));
        assert!(at("http://127.0.0.1:9/api").is_ok_and(|e| e.local));
        for refused in [
            "http://example.com/api",
            "https://slack.example/api",
            "ftp://127.0.0.1/",
            "nonsense",
        ] {
            assert!(at(refused).is_err(), "{refused}");
        }
        let slack = SlackEndpoint::from_env(&Env::default()).unwrap();
        assert!(slack.admits_socket("wss://wss-primary.slack.com/link/?ticket=1"));
        assert!(!slack.admits_socket("ws://wss-primary.slack.com/link"));
        assert!(!slack.admits_socket("wss://evil.example/link"));
        let local = at("http://127.0.0.1:9/api").unwrap();
        assert!(local.admits_socket("ws://127.0.0.1:10/"));
        assert!(!local.admits_socket("ws://example.com/"));
    }
}
