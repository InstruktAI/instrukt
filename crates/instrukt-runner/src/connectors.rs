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

/// The subtypes Slack marks its own notices with: what Slack itself writes
/// about a conversation. Every other message, in whatever form Slack records
/// it, is a message.
const NOTICES: [&str; 20] = [
    "channel_join",
    "channel_leave",
    "channel_topic",
    "channel_purpose",
    "channel_name",
    "channel_archive",
    "channel_unarchive",
    "group_join",
    "group_leave",
    "group_topic",
    "group_purpose",
    "group_name",
    "group_archive",
    "group_unarchive",
    "channel_convert_to_private",
    "channel_convert_to_public",
    "channel_posting_permissions",
    "pinned_item",
    "unpinned_item",
    "reminder_add",
];

/// About how often a status that stays is set again: Slack drops it after
/// two minutes. The status is a courtesy; nothing promises this timing.
const STATUS_INTERVAL: Duration = Duration::from_secs(30);

/// How often the status pass looks at what is unfinished.
const STATUS_TICK: Duration = Duration::from_secs(1);

/// How long one status call may take.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest wait before retrying intake work that failed.
const MOST_BACKOFF: Duration = Duration::from_secs(60);

/// The setting that points the connector at a local stand-in for Slack.
/// Only a loopback address is accepted.
pub const API_URL_SETTING: &str = "INSTRUKT_SLACK_API_URL";

/// Slack's Web API.
const SLACK_API: &str = "https://slack.com/api";

/// How long retained intake work waits between passes, and how long it first
/// waits after a pass that failed; each further failure in a row doubles
/// that wait, up to [`MOST_BACKOFF`].
const RETRY: Duration = Duration::from_secs(2);

/// The time the limits read: the runner's clock.
pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// Until when Slack limited each method for each company's workspace, as
/// its `Retry-After` said, on the runner's clock. The runner owns one and
/// every client of its connectors shares it, so no call of a limited method
/// for that workspace is made before its wait is over while the runner runs.
/// A wait that has passed is retired.
#[derive(Clone)]
pub struct Limits {
    clock: Clock,
    waits: Arc<std::sync::Mutex<HashMap<(String, String), Timestamp>>>,
}

impl std::fmt::Debug for Limits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Limits").finish_non_exhaustive()
    }
}

impl Limits {
    fn new(clock: Clock) -> Self {
        Self {
            clock,
            waits: Arc::default(),
        }
    }

    /// How many whole seconds, rounded up, `method` stays limited for
    /// `company`'s workspace, if it is.
    fn remaining(&self, company: &str, method: &str) -> Option<u64> {
        let mut waits = self.waits.lock().ok()?;
        let now = (self.clock)();
        waits.retain(|_, until| *until > now);
        let until = waits.get(&(company.to_owned(), method.to_owned()))?;
        let left = until.duration_since(now);
        let seconds = u64::try_from(left.as_secs()).unwrap_or(0);
        Some(seconds + u64::from(left.subsec_nanos() > 0))
    }

    /// Keeps the wait of `seconds` Slack named for `method`.
    fn limit(&self, company: &str, method: &str, seconds: u64) {
        let wait = jiff::SignedDuration::from_secs(i64::try_from(seconds).unwrap_or(i64::MAX));
        let until = (self.clock)().checked_add(wait).unwrap_or(Timestamp::MAX);
        if let Ok(mut waits) = self.waits.lock() {
            waits.insert((company.to_owned(), method.to_owned()), until);
        }
    }
}

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
    /// Slack limited the method for this workspace and named a wait.
    #[error("Slack {method}: limited by Slack; called again in {seconds} s")]
    Limited { method: String, seconds: u64 },
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
    /// The waits Slack named, shared by every client this endpoint makes.
    limits: Limits,
    api: String,
    local: bool,
}

impl SlackEndpoint {
    /// Slack's own hosts unless [`API_URL_SETTING`] names a loopback
    /// address; any other value refuses. The endpoint starts knowing no wait
    /// and keeps those Slack names for as long as it lives, timed by `clock`.
    pub fn from_env(env: &Env, clock: Clock) -> Result<Self, SlackError> {
        match env
            .get(API_URL_SETTING)
            .filter(|url| !url.trim().is_empty())
        {
            None => Ok(Self {
                limits: Limits::new(clock),
                api: SLACK_API.into(),
                local: false,
            }),
            Some(url) => {
                let parsed = uri(url).ok_or(SlackError::Endpoint)?;
                if !matches!(parsed.scheme_str(), Some("http" | "https")) || !loopback(&parsed) {
                    return Err(SlackError::Endpoint);
                }
                Ok(Self {
                    limits: Limits::new(clock),
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
    company: ProjectSlug,
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
            company: company.clone(),
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
        self.call_within(method, app, form, None)
    }

    /// One Web API call, taking at most `timeout` when given. A method Slack
    /// limited for this workspace is not called before its wait is over; a
    /// limit Slack answers with is kept with the wait it names.
    fn call_within(
        &self,
        method: &str,
        app: bool,
        form: &[(&str, &str)],
        timeout: Option<Duration>,
    ) -> Result<Value, SlackError> {
        let company = self.company.to_string();
        if let Some(seconds) = self.endpoint.limits.remaining(&company, method) {
            return Err(SlackError::Limited {
                method: method.into(),
                seconds,
            });
        }
        let token = if app {
            &self.app_token
        } else {
            &self.bot_token
        };
        let fail = |cause: String| SlackError::Api {
            method: method.into(),
            cause,
        };
        let mut request = self
            .agent
            .post(format!("{}/{method}", self.endpoint.api))
            .header("Authorization", format!("Bearer {token}"));
        if let Some(timeout) = timeout {
            request = request.config().timeout_global(Some(timeout)).build();
        }
        let mut response = request
            .send_form(form.iter().copied())
            .map_err(|e| fail(e.to_string()))?;
        let status = response.status();
        if status.as_u16() == 429 {
            let seconds = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(1);
            self.endpoint.limits.limit(&company, method, seconds);
            tracing::warn!(event = "connector_rate_limited", company = %self.company, method, seconds);
            return Err(SlackError::Limited {
                method: method.into(),
                seconds,
            });
        }
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
        self.call_within(
            "assistant.threads.setStatus",
            false,
            &[
                ("channel_id", channel),
                ("thread_ts", thread),
                ("status", status),
            ],
            Some(STATUS_TIMEOUT),
        )
        .map(drop)
    }

    /// Post an answer into a conversation as the agent wrote it, as Markdown
    /// Slack renders (`markdown_text`): its thread in a channel, or the
    /// direct-message channel.
    pub fn answer(&self, conversation: &str, markdown: &str) -> Result<(), SlackError> {
        let (channel, thread) = split_conversation(conversation);
        let mut form = vec![("channel", channel), ("markdown_text", markdown)];
        if let Some(thread) = thread {
            form.push(("thread_ts", thread));
        }
        self.call("chat.postMessage", false, &form).map(drop)
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

/// The subtypes of a message a person newly wrote, besides none: one that
/// carried a file, one written with `/me`, and a thread reply also sent to
/// the conversation.
const NEWLY_WRITTEN: [&str; 3] = ["file_share", "me_message", "thread_broadcast"];

/// Whether `event` is a message a person newly wrote, which can start a turn:
/// no subtype or one of [`NEWLY_WRITTEN`], and not marked as an edit of an
/// earlier message. An edit, a deletion and every other record start nothing.
/// What accompanies an addressed message is not filtered by this.
fn newly_written(event: &Value) -> bool {
    let subtype = event.get("subtype").and_then(Value::as_str);
    subtype.is_none_or(|subtype| NEWLY_WRITTEN.contains(&subtype)) && event.get("edited").is_none()
}

/// Whether `message` is one of Slack's own notices about a conversation.
/// Every other message is kept, in whatever form Slack records it.
fn notice(message: &Value) -> bool {
    message
        .get("subtype")
        .and_then(Value::as_str)
        .is_some_and(|subtype| NOTICES.contains(&subtype))
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
        let demand = conversations::demand(home)?;
        let hosted: Vec<(String, String)> = demand
            .iter()
            .map(|(company, _)| (company.to_string(), SLACK.to_owned()))
            .collect();
        if let Err(error) = crate::runner::publish_hosted(home, &hosted) {
            tracing::warn!(event = "connector_hosting_unpublished", error = %error);
        }
        for (company, declared) in demand {
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
    // end ends this connection, which the run loop then restarts. Slack's
    // status line has a worker of its own beside both.
    let ended = AtomicBool::new(false);
    let (nudge, nudges) = std::sync::mpsc::sync_channel::<()>(1);
    let (admitted, admissions) = std::sync::mpsc::sync_channel::<()>(1);
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
            intake_worker(
                home,
                endpoint,
                company,
                declared,
                stop,
                Wakes {
                    relay: wake,
                    status: &admitted,
                },
                &nudges,
            );
        });
        scope.spawn(move || {
            let quit = || stop.load(Ordering::Acquire) || ended.load(Ordering::Acquire);
            status_worker(home, endpoint, company, declared, &quit, &admissions);
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

/// Whom intake wakes: the relay, for what became deliverable, and the
/// status worker, for a message just admitted.
#[derive(Clone, Copy)]
struct Wakes<'a> {
    relay: &'a SyncSender<()>,
    status: &'a SyncSender<()>,
}

/// The company's retained intake work, until `stop`: run when a retained
/// envelope nudges it, every [`RETRY`] while it runs clean, and after a
/// failure once its wait has passed: [`RETRY`] after the first failure in a
/// row, doubling up to [`MOST_BACKOFF`]. A method Slack limited is not
/// called before its wait is over, also on a nudge.
fn intake_worker(
    home: &Home,
    endpoint: &SlackEndpoint,
    company: &ProjectSlug,
    declared: &SlackConnector,
    stop: &AtomicBool,
    wakes: Wakes<'_>,
    nudges: &std::sync::mpsc::Receiver<()>,
) {
    let mut me = None;
    let mut failures: u32 = 0;
    let mut retry_at = Instant::now();
    let mut nudged = true;
    while !stop.load(Ordering::Acquire) {
        if nudged || Instant::now() >= retry_at {
            let clean = match SlackApi::with(home, company, declared, endpoint) {
                Ok(api) => {
                    if me.is_none() {
                        me = api.identity().ok();
                    }
                    intake(home, company, declared, &api, me.as_ref(), wakes)
                }
                // A failure before any request stays with the outstanding work.
                Err(error) => {
                    if let Err(record) =
                        conversations::intake_failed(home, company, SLACK, &error.to_string())
                    {
                        tracing::warn!(event = "connector_intake_failed", company = %company, connector = SLACK, error = %record);
                    }
                    false
                }
            };
            failures = if clean { 0 } else { failures.saturating_add(1) };
            let wait = if failures == 0 {
                RETRY
            } else {
                RETRY
                    .saturating_mul(1 << (failures - 1).min(5))
                    .min(MOST_BACKOFF)
            };
            retry_at = Instant::now() + wait;
        }
        nudged = false;
        while Instant::now() < retry_at && !stop.load(Ordering::Acquire) {
            if nudges.recv_timeout(Duration::from_millis(100)).is_ok() {
                nudged = true;
                break;
            }
        }
    }
}

/// One status place: a channel conversation's thread, or a direct message's
/// addressed message.
type Place = (String, String);

/// Slack's status line for one company, a courtesy kept on a best-effort
/// basis apart from intake and the relay, so no status call holds a
/// message. A plain periodic pass reads the places with unfinished admitted
/// messages, sets each whose last attempt is [`STATUS_INTERVAL`] old or
/// more, and clears once each place this process set that has no unfinished
/// work left. A message's admission wakes the pass, so its first status
/// comes promptly. A call that fails or is skipped is left to the next
/// regular setting, and a clear that does not happen to Slack's own two
/// minutes; nothing is made up for and nothing set by an earlier run is
/// cleared.
fn status_worker(
    home: &Home,
    endpoint: &SlackEndpoint,
    company: &ProjectSlug,
    declared: &SlackConnector,
    stop: &dyn Fn() -> bool,
    admitted: &std::sync::mpsc::Receiver<()>,
) {
    let mut attempted: HashMap<Place, Instant> = HashMap::new();
    let mut failing: Option<String> = None;
    while !stop() {
        let pass = SlackApi::with(home, company, declared, endpoint)
            .map_err(|error| error.to_string())
            .and_then(|api| {
                let working =
                    conversations::working(home, company, SLACK).map_err(|e| e.to_string())?;
                Ok((api, working))
            });
        match pass {
            Ok((api, working)) => {
                if let Some(cause) = failing.take() {
                    tracing::info!(event = "connector_status_recovered", company = %company, cause = %cause);
                }
                keep(&api, &working, &mut attempted);
            }
            // A continuing cause is logged once, when it starts or changes.
            Err(cause) => {
                if failing.as_deref() != Some(cause.as_str()) {
                    tracing::warn!(event = "connector_status_unprepared", company = %company, error = %cause);
                    failing = Some(cause);
                }
            }
        }
        let until = Instant::now() + STATUS_TICK;
        while Instant::now() < until && !stop() {
            if admitted.recv_timeout(Duration::from_millis(100)).is_ok() {
                break;
            }
        }
    }
}

/// The status place of a conversation's message.
fn place(conversation: &str, message: &str) -> Place {
    let (channel, thread) = split_conversation(conversation);
    (channel.to_owned(), thread.unwrap_or(message).to_owned())
}

/// One status pass: set what stays and is due, clear once what no longer
/// stays. Each call's outcome is only logged.
fn keep(api: &SlackApi, working: &[(String, String)], attempted: &mut HashMap<Place, Instant>) {
    let wanted: std::collections::HashSet<Place> = working
        .iter()
        .map(|(conversation, message)| place(conversation, message))
        .collect();
    for target in &wanted {
        let due = attempted
            .get(target)
            .is_none_or(|at| at.elapsed() >= STATUS_INTERVAL);
        if due {
            attempted.insert(target.clone(), Instant::now());
            call(api, target, WORKING);
        }
    }
    let ended: Vec<Place> = attempted
        .keys()
        .filter(|target| !wanted.contains(*target))
        .cloned()
        .collect();
    for target in ended {
        attempted.remove(&target);
        call(api, &target, "");
    }
}

fn call(api: &SlackApi, target: &Place, status: &str) {
    if let Err(error) = api.status(&target.0, &target.1, status) {
        tracing::debug!(event = "connector_status_skipped", company = %api.company, connector = SLACK, error = %error);
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
    if !newly_written(event) {
        return Ok(());
    }
    let thread = match text("type") {
        Some("app_mention") => Some(text("thread_ts").unwrap_or(ts)),
        Some("message")
            if text("channel_type") == Some("im")
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
    wakes: Wakes<'_>,
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
                            // Its status is arranged now, not after the batch.
                            let _ = wakes.status.try_send(());
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
                    // Slack answered that it shows the app no such user: the
                    // sender is not one of the company's own. Every other
                    // failure is no answer about the sender, and the message
                    // waits.
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
            let _ = wakes.relay.try_send(());
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
    use super::{Me, SlackEndpoint, admission, notice, split_conversation, ts_micros};
    use instrukt_core::config::Env;
    use serde_json::{Value, json};

    fn system() -> super::Clock {
        std::sync::Arc::new(jiff::Timestamp::now)
    }

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
        assert!(admitted.email.is_some());
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
    fn only_the_listed_subtypes_are_slacks_own_notices() {
        assert!(notice(&json!({"subtype": "channel_join"})));
        assert!(notice(&json!({"subtype": "reminder_add"})));
        assert!(!notice(&json!({"subtype": "thread_broadcast"})));
        assert!(!notice(&json!({"subtype": "a_form_slack_adds_later"})));
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
        let at = |url: &str| {
            SlackEndpoint::from_env(&Env::from_pairs([(super::API_URL_SETTING, url)]), system())
        };
        assert!(SlackEndpoint::from_env(&Env::default(), system()).is_ok_and(|e| !e.local));
        assert!(at("http://127.0.0.1:9/api").is_ok_and(|e| e.local));
        for refused in [
            "http://example.com/api",
            "https://slack.example/api",
            "ftp://127.0.0.1/",
            "nonsense",
        ] {
            assert!(at(refused).is_err(), "{refused}");
        }
        let slack = SlackEndpoint::from_env(&Env::default(), system()).unwrap();
        assert!(slack.admits_socket("wss://wss-primary.slack.com/link/?ticket=1"));
        assert!(!slack.admits_socket("ws://wss-primary.slack.com/link"));
        assert!(!slack.admits_socket("wss://evil.example/link"));
        let local = at("http://127.0.0.1:9/api").unwrap();
        assert!(local.admits_socket("ws://127.0.0.1:10/"));
        assert!(!local.admits_socket("ws://example.com/"));
    }
}
