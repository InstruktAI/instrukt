//! The Slack connector (`slack-connector` spec): one Socket Mode connection
//! per company, opened outward with the app token; Slack's record of a
//! sender decides who is answered; the accompanying messages are read from
//! the conversation; answers are posted as the Markdown the agent wrote; and
//! Slack's status line shows the agent working. Everything around these
//! operations is the connector host's (`instrukt-company`'s
//! `connector_host`); this crate knows only the connector interface
//! (`instrukt-connector`).
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use instrukt_connector::{
    Accompanying, Admission, AnswerError, AnswerReceipt, Captured, Clock, Connector,
    ConnectorError, Declaration, Declared, DirectMessageError, DirectMessageReceipt, Earlier,
    Field, Inbox, Incoming, Kind, Place, RefusalReason, Rule, SecretResolver, Sender, Status,
};
use jiff::Timestamp;
use serde_json::Value;

/// The target every log statement of the connector carries: the module the
/// host and Slack shared before the cut, so the same settings select it.
const LOG_TARGET: &str = "instrukt_runner::connectors";

/// The connector's key under `connectors`.
pub const SLACK: &str = "slack";

/// The one fixed reply a sender who is not one of the company's own
/// receives, in a channel and in a direct message alike.
pub const TURN_AWAY: &str = "I only answer members of this workspace.";

/// The one fixed reply a sender who is not a workspace administrator
/// receives on an `admins_only` connector, in a channel and in a direct
/// message alike.
pub const TURN_AWAY_NOT_ADMIN: &str = "I only answer workspace administrators.";

/// The one notice's sentence, after the waiting person's mention.
pub const NOTICE: &str = "The assistant is temporarily unavailable. Your message is saved and you'll get the answer here when it's back — no need to send it again.";

/// The text of Slack's status line from a message's admission until the
/// agent has written in its running turn; Slack puts the app's name in front
/// of it.
pub const WORKING: &str = "is working on your request";

/// The status text while the assistant cannot work on a message.
pub const UNAVAILABLE: &str = "is temporarily unavailable — your message is saved";

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

/// The most characters Slack takes in one posted message: a longer answer is
/// posted in parts, in order.
pub const SLACK_MESSAGE_LIMIT: usize = 12_000;

/// How many earlier messages accompany an addressed message when the
/// declaration names none.
const DEFAULT_CONTEXT_MESSAGES: u64 = 30;

/// What `context_messages` is, as its finding and editor schema say it.
const CONTEXT_MESSAGES: &str = "the largest number of earlier messages that accompany an addressed message: a positive whole number; 30 when omitted";

/// The fields a `connectors.slack` entry takes.
pub static SLACK_DECLARATION: Declaration = Declaration {
    key: SLACK,
    message_chars: SLACK_MESSAGE_LIMIT,
    fields: &[
        Field {
            name: "app_token",
            rule: Rule::SecretReference { required: true },
        },
        Field {
            name: "bot_token",
            rule: Rule::SecretReference { required: true },
        },
        Field {
            name: "context_messages",
            rule: Rule::PositiveCount {
                default: DEFAULT_CONTEXT_MESSAGES,
                meaning: CONTEXT_MESSAGES,
            },
        },
    ],
};

/// About how often a status that stays is set again: Slack drops it after
/// two minutes. The status is a courtesy; nothing promises this timing.
pub(crate) const STATUS_INTERVAL: Duration = Duration::from_secs(30);

/// The default bound on a Slack Web API call.
const WEB_API_TIMEOUT: Duration = Duration::from_secs(30);
/// How long one status call may take.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// The setting that points the connector at a local stand-in for Slack.
/// Only a loopback address is accepted.
pub const API_URL_SETTING: &str = "INSTRUKT_SLACK_API_URL";

/// Slack's Web API.
const SLACK_API: &str = "https://slack.com/api";

/// Until when Slack limited each method for each `(company, connector)`, as
/// its `Retry-After` said, on the runner's clock. The runner owns one and
/// every client of its connectors shares it, so no call of a limited method
/// for that connector is made before its wait is over while the runner runs,
/// while separate Slack apps in one company home never collide.
/// A wait that has passed is retired.
type LimitKey = (String, String, String);
type LimitWaits = Arc<std::sync::Mutex<HashMap<LimitKey, Timestamp>>>;

#[derive(Clone)]
pub struct Limits {
    clock: Clock,
    waits: LimitWaits,
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
    /// `(company, connector)`, if it is.
    fn remaining(&self, company: &str, connector: &str, method: &str) -> Option<u64> {
        let mut waits = self.waits.lock().ok()?;
        let now = (self.clock)();
        waits.retain(|_, until| *until > now);
        let until = waits.get(&(company.to_owned(), connector.to_owned(), method.to_owned()))?;
        let left = until.duration_since(now);
        let seconds = u64::try_from(left.as_secs()).unwrap_or(0);
        Some(seconds + u64::from(left.subsec_nanos() > 0))
    }

    /// Keeps the wait of `seconds` Slack named for `method` on `(company, connector)`.
    fn limit(&self, company: &str, connector: &str, method: &str, seconds: u64) {
        let wait = jiff::SignedDuration::from_secs(i64::try_from(seconds).unwrap_or(i64::MAX));
        let until = (self.clock)().checked_add(wait).unwrap_or(Timestamp::MAX);
        if let Ok(mut waits) = self.waits.lock() {
            waits.insert(
                (company.to_owned(), connector.to_owned(), method.to_owned()),
                until,
            );
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    /// A Web API failure, naming the method and Slack's error code; never a
    /// token.
    #[error("Slack {method}: {cause}")]
    Api { method: String, cause: String },
    /// A transport failure or HTTP 5xx response from Slack (retryable).
    #[error("Slack {method}: {cause}")]
    Transient { method: String, cause: String },
    /// A Slack response confirms the post was rejected.
    #[error("Slack {method}: {cause}")]
    RejectedPost { method: String, cause: String },
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
    /// Slack's own hosts unless [`API_URL_SETTING`], as `lookup` reads the
    /// environment, names a loopback address; any other value refuses. The
    /// endpoint starts knowing no wait and keeps those Slack names for as
    /// long as it lives, timed by `clock`.
    pub fn from_lookup(
        lookup: &dyn Fn(&str) -> Option<String>,
        clock: Clock,
    ) -> Result<Self, SlackError> {
        match lookup(API_URL_SETTING)
            .as_deref()
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

/// One company's named Slack Web API, with the tokens its admitted generation
/// resolved, as its connector's opening drew them through the resolver.
pub struct SlackApi {
    company: String,
    connector: String,
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
    /// The Web API of `(company, connector)`, with its two tokens.
    fn new(
        company: &str,
        connector: &str,
        endpoint: &SlackEndpoint,
        app_token: String,
        bot_token: String,
    ) -> Self {
        Self {
            company: company.to_owned(),
            connector: connector.to_owned(),
            endpoint: endpoint.clone(),
            app_token,
            bot_token,
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(WEB_API_TIMEOUT))
                .http_status_as_error(false)
                .build()
                .into(),
        }
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
        self.call_with_headers(method, app, form, timeout)
            .map(|(body, _)| body)
    }

    fn call_with_headers(
        &self,
        method: &str,
        app: bool,
        form: &[(&str, &str)],
        timeout: Option<Duration>,
    ) -> Result<(Value, Option<String>), SlackError> {
        let company = self.company.clone();
        let connector = self.connector.clone();
        if let Some(seconds) = self.endpoint.limits.remaining(&company, &connector, method) {
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
        let mut response =
            request
                .send_form(form.iter().copied())
                .map_err(|e| SlackError::Transient {
                    method: method.into(),
                    cause: e.to_string(),
                })?;
        let status = response.status();
        if status.as_u16() == 429 {
            let seconds = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(1);
            self.endpoint
                .limits
                .limit(&company, &connector, method, seconds);
            tracing::warn!(target: LOG_TARGET, event = "connector_rate_limited", company = %self.company, connector = %self.connector, method, seconds);
            return Err(SlackError::Limited {
                method: method.into(),
                seconds,
            });
        }
        if status.as_u16() >= 500 {
            return Err(SlackError::Transient {
                method: method.into(),
                cause: format!("HTTP {status}"),
            });
        }
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|_| SlackError::Transient {
                method: method.into(),
                cause: format!("HTTP {status} without an answer"),
            })?;
        let body: Value = serde_json::from_str(&text)
            .map_err(|_| fail(format!("HTTP {status} without a JSON answer")))?;
        if body.get("ok").and_then(Value::as_bool) == Some(true) {
            let scopes = response
                .headers()
                .get("x-oauth-scopes")
                .and_then(|header| header.to_str().ok())
                .map(str::to_owned);
            Ok((body, scopes))
        } else {
            let cause = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("not ok");
            if method == "chat.postMessage"
                && body.get("ok").and_then(Value::as_bool) == Some(false)
                && matches!(
                    cause,
                    "invalid_auth"
                        | "not_authed"
                        | "missing_scope"
                        | "channel_not_found"
                        | "not_in_channel"
                        | "is_archived"
                        | "restricted_action"
                        | "restricted_action_read_only_channel"
                        | "restricted_action_thread_locked"
                        | "msg_too_long"
                        | "no_text"
                )
            {
                return Err(SlackError::RejectedPost {
                    method: method.into(),
                    cause: cause.into(),
                });
            }
            Err(fail(cause.into()))
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
    pub fn answer(&self, conversation: &str, markdown: &str) -> Result<AnswerReceipt, SlackError> {
        let (channel, thread) = split_conversation(conversation);
        let mut form = vec![("channel", channel), ("markdown_text", markdown)];
        if let Some(thread) = thread {
            form.push(("thread_ts", thread));
        }
        self.call("chat.postMessage", false, &form).map(|post| {
            let id = post.get("ts").and_then(Value::as_str);
            AnswerReceipt {
                message_id: id.map(str::to_owned),
                posted_at: id.and_then(ts_time),
            }
        })
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

    /// Post a private direct message directly to a user's Slack ID using `chat.postMessage`
    /// (`chat:write` scope only, never `conversations.open`), tagged with `instrukt_private`
    /// metadata so it is excluded from every context read.
    pub fn post_private_dm(
        &self,
        recipient: &str,
        text: &str,
    ) -> Result<DirectMessageReceipt, SlackError> {
        const PRIVATE_METADATA: &str = r#"{"event_type":"instrukt_private","event_payload":{}}"#;
        let form = [
            ("channel", recipient),
            ("text", text),
            ("metadata", PRIVATE_METADATA),
        ];
        self.call("chat.postMessage", false, &form)
            .map(|post| DirectMessageReceipt {
                channel: post
                    .get("channel")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                message_id: post.get("ts").and_then(Value::as_str).map(str::to_owned),
            })
    }
}

/// Whether `message` carries Slack metadata `event_type: "instrukt_private"`,
/// marking a private platform DM that must never accompany any turn to the agent.
fn is_instrukt_private(message: &Value) -> bool {
    message
        .pointer("/metadata/event_type")
        .and_then(Value::as_str)
        == Some("instrukt_private")
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
    let is_admin = flag("is_admin") == Some(true)
        || flag("is_owner") == Some(true)
        || flag("is_primary_owner") == Some(true);
    own.then(|| Sender {
        name: name(record).unwrap_or_else(|| id.to_owned()),
        email: record
            .get("profile")
            .and_then(|p| p.get("email"))
            .and_then(Value::as_str)
            .filter(|email| !email.is_empty())
            .map(str::to_owned),
        is_admin,
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
    known: &'a Mutex<HashMap<String, String>>,
}
impl Names<'_> {
    fn of(&mut self, user: &str) -> String {
        if let Some(name) = self
            .known
            .lock()
            .ok()
            .and_then(|known| known.get(user).cloned())
        {
            return name;
        }
        let found = self
            .api
            .user(user)
            .ok()
            .and_then(|record| name(&record))
            .unwrap_or_else(|| user.to_owned());
        self.remember(user, &found);
        found
    }

    fn remember(&self, user: &str, name: &str) {
        if let Ok(mut known) = self.known.lock() {
            known.insert(user.to_owned(), name.to_owned());
        }
    }
}

fn written_mentions(text: &str, mut name: impl FnMut(&str) -> String) -> String {
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
                out.push_str(&name(user));
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

/// `<channel>/<thread ts>` for a channel thread, `<channel>` for a direct
/// message.
fn split_conversation(conversation: &str) -> (&str, Option<&str>) {
    match conversation.split_once('/') {
        Some((channel, thread)) => (channel, Some(thread)),
        None => (conversation, None),
    }
}

/// The Slack kind of this process: its endpoint, built once when the runner
/// starts, and the bot identity each `(company, connector)`'s tokens answered
/// with, asked of Slack once while those tokens stay.
type IdentityKey = (String, String);
type IdentityCache = Arc<Mutex<HashMap<IdentityKey, (String, Me)>>>;

pub struct SlackKind {
    endpoint: SlackEndpoint,
    identities: IdentityCache,
}

impl SlackKind {
    pub fn new(endpoint: SlackEndpoint) -> Self {
        Self {
            endpoint,
            identities: Arc::default(),
        }
    }

    /// The Slack kind of a process whose environment `lookup` answers, with
    /// this clock: its endpoint refuses a Slack API address that is neither
    /// Slack's own nor a loopback stand-in.
    pub fn build(
        lookup: &dyn Fn(&str) -> Option<String>,
        clock: Clock,
    ) -> Result<Box<dyn Kind>, ConnectorError> {
        SlackEndpoint::from_lookup(lookup, clock)
            .map(|endpoint| Box::new(Self::new(endpoint)) as Box<dyn Kind>)
            .map_err(|error| ConnectorError {
                cause: error.to_string(),
            })
    }
}

impl Kind for SlackKind {
    fn declaration(&self) -> &'static Declaration {
        &SLACK_DECLARATION
    }

    fn open(
        &self,
        company: &str,
        connector: &str,
        declared: &Declared,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn Connector>, ConnectorError> {
        let token = |field: &str| -> Result<String, ConnectorError> {
            let reference = declared.text(field).ok_or_else(|| ConnectorError {
                cause: format!("the slack connector `{connector}` declares no {field}"),
            })?;
            secrets.resolve(reference)
        };
        let api = SlackApi::new(
            company,
            connector,
            &self.endpoint,
            token("app_token")?,
            token("bot_token")?,
        );
        Ok(Box::new(Slack {
            api,
            context_messages: declared
                .count("context_messages")
                .unwrap_or(DEFAULT_CONTEXT_MESSAGES),
            identities: Arc::clone(&self.identities),
            names: Arc::default(),
        }))
    }
}

/// One company's named Slack connector.
struct Slack {
    api: SlackApi,
    context_messages: u64,
    identities: IdentityCache,
    /// The people's names asked of Slack while this connector is open.
    names: Arc<Mutex<HashMap<String, String>>>,
}

impl Slack {
    fn identity_key(&self) -> (String, String) {
        (self.api.company.clone(), self.api.connector.clone())
    }

    fn cached_identity(&self) -> Option<Me> {
        let identities = self.identities.lock().ok()?;
        let (token, me) = identities.get(&self.identity_key())?;
        (token == &self.api.bot_token).then(|| me.clone())
    }

    /// The bot's identity in its workspace, asked of Slack once for these
    /// tokens.
    fn me(&self) -> Result<Me, SlackError> {
        if let Some(me) = self.cached_identity() {
            return Ok(me);
        }
        let key = self.identity_key();
        let me = self.api.identity()?;
        if let Ok(mut identities) = self.identities.lock() {
            identities.insert(key, (self.api.bot_token.clone(), me.clone()));
        }
        Ok(me)
    }

    fn names(&self) -> Names<'_> {
        Names {
            api: &self.api,
            known: &self.names,
        }
    }
}

fn cached_names<T>(
    names: &Mutex<HashMap<String, String>>,
    render: impl FnOnce(&mut dyn FnMut(&str) -> String) -> T,
) -> T {
    let known = names.try_lock().ok();
    render(&mut |user| {
        known
            .as_ref()
            .and_then(|names| names.get(user))
            .cloned()
            .unwrap_or_else(|| user.to_owned())
    })
}

fn failed(error: impl std::fmt::Display) -> ConnectorError {
    ConnectorError {
        cause: error.to_string(),
    }
}

fn own_message(message: &Value, me: &Me) -> bool {
    message["bot_id"]
        .as_str()
        .is_some_and(|bot| Some(bot) == me.bot.as_deref())
        || message["user"].as_str() == Some(me.user.as_str())
}

mod read_back;

impl Connector for Slack {
    fn check_permissions(&self) -> Result<(), ConnectorError> {
        const REQUIRED: &[&str] = &[
            "app_mentions:read",
            "chat:write",
            "channels:history",
            "channels:read",
            "groups:history",
            "groups:read",
            "im:history",
            "im:read",
            "users:read",
            "users:read.email",
        ];
        let (_, scopes) = self
            .api
            .call_with_headers("auth.test", false, &[], None)
            .map_err(failed)?;
        let scopes = scopes.ok_or_else(|| failed("Slack did not report the bot token's scopes"))?;
        let granted: std::collections::HashSet<_> = scopes.split(',').map(str::trim).collect();
        let missing: Vec<_> = REQUIRED
            .iter()
            .filter(|scope| !granted.contains(**scope))
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(failed(format!(
                "Slack bot token missing scopes: {}",
                missing.join(", ")
            )));
        }
        Ok(())
    }

    fn read_back_page(
        &self,
        from: Timestamp,
        position: Option<String>,
        gaps: &[instrukt_connector::ReadBackGap],
    ) -> Result<instrukt_connector::ReadBackPage, instrukt_connector::ReadBackError> {
        self.history_page(from, position, gaps)
    }

    fn limits(&self) -> instrukt_connector::Limits {
        instrukt_connector::Limits {
            message_chars: SLACK_MESSAGE_LIMIT,
            working_refresh: Some(STATUS_INTERVAL),
        }
    }

    fn listen(&self, inbox: &dyn Inbox, quit: &dyn Fn() -> bool) -> Result<(), ConnectorError> {
        session(&self.api, inbox, quit).map_err(|cause| ConnectorError { cause })
    }

    /// Slack's record of the sender read now decides: one of the company's
    /// own, or not. Slack answering that it shows the app no such user is
    /// not one of the company's own; every other failure decides nothing.
    fn admit(&self, sender: &str) -> Admission {
        let me = match self.me() {
            Ok(me) => me,
            Err(error) => return Admission::Undecided(error.to_string()),
        };
        match self.api.user(sender) {
            Ok(record) => match admission(&record, &me) {
                Some(admitted) => {
                    self.names().remember(sender, &admitted.name);
                    Admission::Admitted(admitted)
                }
                None => Admission::NotOwn,
            },
            Err(SlackError::Api { cause, .. })
                if cause == "user_not_found" || cause == "user_not_visible" =>
            {
                Admission::NotOwn
            }
            Err(error) => {
                Admission::Undecided(format!("who sent it cannot be looked up yet: {error}"))
            }
        }
    }

    fn capture(
        &self,
        conversation: &str,
        message: &str,
        text: &str,
        excluded: &std::collections::HashSet<String>,
        readable: &dyn Fn(instrukt_connector::ContextSnapshot),
    ) -> Result<Captured, ConnectorError> {
        let limit = usize::try_from(self.context_messages).unwrap_or(usize::MAX);
        let (messages, stopped_early, complete_from) =
            read_context(self, conversation, message, excluded, limit).map_err(failed)?;
        let messages = Arc::new(messages);
        let cached_messages = Arc::clone(&messages);
        let names = Arc::clone(&self.names);
        let cached_text = text.to_owned();
        let me = self.cached_identity();
        let cached_me = me.clone();
        readable(Box::new(move || {
            let mut captured = cached_names(&names, |lookup| {
                render_context(
                    &cached_text,
                    &cached_messages,
                    limit,
                    cached_me.as_ref(),
                    lookup,
                )
            });
            captured.accompanying.stopped_early = stopped_early;
            captured.accompanying.complete_from = complete_from;
            captured
        }));
        let me = me.or_else(|| self.me().ok());
        let mut names = self.names();
        let mut captured = render_context(text, &messages, limit, me.as_ref(), &mut |user| {
            names.of(user)
        });
        let (channel, thread) = split_conversation(conversation);
        captured.display = self
            .api
            .call("conversations.info", false, &[("channel", channel)])
            .ok()
            .and_then(|info| {
                let info = &info["channel"];
                if thread.is_none() {
                    let user = info["user"].as_str()?;
                    let name = names.of(user);
                    (name != user).then_some(instrukt_connector::ConversationDisplay::Direct(name))
                } else {
                    info["name"]
                        .as_str()
                        .filter(|name| !name.is_empty())
                        .map(|name| {
                            instrukt_connector::ConversationDisplay::Channel(name.to_owned())
                        })
                }
            });
        captured.accompanying.stopped_early = stopped_early;
        captured.accompanying.complete_from = complete_from;
        Ok(captured)
    }

    fn answer(&self, conversation: &str, part: &str) -> Result<AnswerReceipt, AnswerError> {
        self.api
            .answer(conversation, part)
            .map_err(|error| AnswerError {
                cause: error.to_string(),
            })
    }

    fn turn_away(&self, conversation: &str, reason: RefusalReason) -> Result<(), ConnectorError> {
        let text = match reason {
            RefusalReason::NotOwn => TURN_AWAY,
            RefusalReason::NotAdmin => TURN_AWAY_NOT_ADMIN,
        };
        self.api.post(conversation, text).map_err(failed)
    }

    fn unavailable_notice(&self, conversation: &str, member: &str) -> Result<(), ConnectorError> {
        self.api
            .post(conversation, &format!("<@{member}> {NOTICE}"))
            .map_err(failed)
    }

    fn direct_message(
        &self,
        recipient: &str,
        text: &str,
    ) -> Result<DirectMessageReceipt, DirectMessageError> {
        self.api
            .post_private_dm(recipient, text)
            .map_err(|error| match error {
                SlackError::Limited { .. } | SlackError::Transient { .. } => {
                    DirectMessageError::Retryable(error.to_string())
                }
                SlackError::Api { ref cause, .. }
                    if matches!(
                        cause.as_str(),
                        "ratelimited"
                            | "request_timeout"
                            | "internal_error"
                            | "service_unavailable"
                            | "fatal_error"
                    ) =>
                {
                    DirectMessageError::Retryable(error.to_string())
                }
                other => DirectMessageError::Final(other.to_string()),
            })
    }

    /// A channel conversation's thread; a direct message's addressed message.
    fn place(&self, conversation: &str, message: &str) -> Place {
        let (channel, thread) = split_conversation(conversation);
        Place(channel.to_owned(), thread.unwrap_or(message).to_owned())
    }

    /// The two fixed texts, the agent's own message as it wrote it, or an
    /// empty status, which clears it.
    fn status(&self, place: &Place, state: Option<&Status>) -> Result<(), ConnectorError> {
        let text = match state {
            Some(Status::Working) => WORKING,
            Some(Status::Unavailable) => UNAVAILABLE,
            Some(Status::Update(text)) => text.as_str(),
            None => "",
        };
        self.api.status(&place.0, &place.1, text).map_err(failed)
    }
}

/// One Socket Mode connection, from opening to its drop or a disconnect.
fn session(api: &SlackApi, inbox: &dyn Inbox, quit: &dyn Fn() -> bool) -> Result<(), String> {
    let company = &api.company;
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
    inbox.connected();
    tracing::info!(target: LOG_TARGET, event = "connector_connected", company = %company, connector = %api.connector);
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
        let retained =
            receive(&api.connector, &me, &envelope).and_then(|incoming| match incoming {
                Some(incoming) => inbox.retain(incoming).map_err(|error| error.cause),
                None => Ok(()),
            });
        match retained {
            Ok(()) => {
                socket
                    .send(tungstenite::Message::text(
                        serde_json::json!({ "envelope_id": id }).to_string(),
                    ))
                    .map_err(|e| format!("Socket Mode acknowledgement: {e}"))?;
                if unretained {
                    inbox.connected();
                    unretained = false;
                }
            }
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, event = "connector_message_unretained", company = %company, connector = %api.connector, error = %error);
                inbox.unretained(&format!(
                    "a message could not be retained and awaits Slack's retry: {error}"
                ));
                unretained = true;
            }
        }
    }
    let _ = socket.close(None);
    Ok(())
}

/// The addressed message one Events API envelope carries: a mention in a
/// channel or a direct message; none for anything else. Nothing here waits
/// on the network.
fn receive(connector: &str, me: &Me, envelope: &Value) -> Result<Option<Incoming>, String> {
    if envelope.get("type").and_then(Value::as_str) != Some("events_api") {
        return Ok(None);
    }
    let Some(event) = envelope.get("payload").and_then(|p| p.get("event")) else {
        return Ok(None);
    };
    let text = |field: &str| event.get(field).and_then(Value::as_str);
    let (Some(channel), Some(ts), Some(sender)) = (text("channel"), text("ts"), text("user"))
    else {
        return Ok(None);
    };
    if !newly_written(event) {
        return Ok(None);
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
        _ => return Ok(None),
    };
    let is_direct = thread.is_none();
    let incoming = Incoming {
        connector: connector.to_owned(),
        conversation: match thread {
            Some(thread) => format!("{channel}/{thread}"),
            None => channel.to_owned(),
        },
        message: ts.to_owned(),
        sender: sender.to_owned(),
        written_at: ts_time(ts).ok_or("message without a valid time")?,
        text: text("text").unwrap_or_default().to_owned(),
        is_direct,
    };
    Ok(Some(incoming))
}

/// History is newest first. Replies are oldest first within a time range,
/// so visit recent ranges first and split a full range before trusting its
/// first page to contain the most recent replies.
fn read_context(
    slack: &Slack,
    conversation: &str,
    message: &str,
    excluded: &std::collections::HashSet<String>,
    limit: usize,
) -> Result<(Vec<Value>, bool, Option<Timestamp>), SlackError> {
    use std::collections::BTreeMap;

    let (channel, thread) = split_conversation(conversation);
    let thread = thread.filter(|thread| *thread != message);
    let method = if thread.is_some() {
        "conversations.replies"
    } else {
        "conversations.history"
    };
    let latest = ts_micros(message).ok_or_else(|| SlackError::Api {
        method: method.into(),
        cause: "invalid message time".into(),
    })?;
    // An exclusive lower bound one tick before the root includes the root.
    let root = thread.and_then(ts_micros).map(|at| at.saturating_sub(1));
    let recent = root.map(|root| root.max(latest.saturating_sub(60_000_000)));
    let mut earlier = recent
        .zip(root)
        .filter(|(low, root)| low > root)
        .map(|(low, root)| (Some(root), low.saturating_add(1)))
        .into_iter()
        .collect::<Vec<_>>();
    let mut range = (recent, latest);
    let mut cursor = None::<String>;
    let mut messages = BTreeMap::new();
    let current_upper = latest;
    loop {
        let (lower, upper) = range;
        let latest = slack_timestamp(upper);
        let oldest = lower.map(slack_timestamp);
        let mut args = vec![
            ("channel", channel),
            ("latest", latest.as_str()),
            ("inclusive", "false"),
            ("include_all_metadata", "true"),
            ("limit", "200"),
        ];
        if let Some(oldest) = &oldest {
            args.push(("oldest", oldest));
        }
        if let Some(thread) = thread {
            args.push(("ts", thread));
        }
        if let Some(cursor) = &cursor {
            args.push(("cursor", cursor));
        }
        let body = slack.api.call(method, false, &args)?;
        let page = body
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| SlackError::Api {
                method: method.into(),
                cause: "no messages".into(),
            })?;
        let next_cursor = body
            .pointer("/response_metadata/next_cursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty());
        let more =
            next_cursor.is_some() || body.get("has_more").and_then(Value::as_bool) == Some(true);
        for message in page {
            let Some(at) =
                message_ts(message).filter(|at| *at < upper && lower.is_none_or(|low| *at > low))
            else {
                continue;
            };
            if notice(message) || is_instrukt_private(message) || messages.contains_key(&at) {
                continue;
            }
            messages.insert(at, message.clone());
        }

        if (thread.is_none() || !more)
            && messages
                .range((
                    std::ops::Bound::Excluded(lower.unwrap_or(i64::MIN)),
                    std::ops::Bound::Excluded(current_upper),
                ))
                .filter(|(_, m)| m["ts"].as_str().is_some_and(|id| !excluded.contains(id)))
                .take(limit.saturating_add(1))
                .count()
                > limit
        {
            let stopped_early = more || !earlier.is_empty();
            let complete_from =
                lower.and_then(|low| Timestamp::from_microsecond(low.saturating_add(1)).ok());
            return Ok((
                messages.into_values().collect(),
                stopped_early,
                complete_from,
            ));
        }
        if !more {
            let Some(next) = earlier.pop() else {
                return Ok((messages.into_values().collect(), false, None));
            };
            range = next;
            cursor = None;
        } else if let Some(low) = lower.filter(|low| upper.saturating_sub(*low) > 2) {
            // Keep the part we already read for expiry, but don't count it
            // as a complete newest-first selection until its range is read.
            let mid = low.midpoint(upper);
            earlier.push((Some(low), mid.saturating_add(1)));
            range = (Some(mid), upper);
            cursor = None;
        } else if let Some(next) = next_cursor {
            cursor = Some(next.to_owned());
        } else if thread.is_none()
            && let Some(oldest) = page
                .iter()
                .filter_map(message_ts)
                .filter(|at| *at < upper)
                .min()
        {
            range.1 = oldest;
        } else {
            return Err(SlackError::Api {
                method: method.into(),
                cause: "missing continuation for context".into(),
            });
        }
    }
}

fn slack_timestamp(micros: i64) -> String {
    format!(
        "{}.{:06}",
        micros.div_euclid(1_000_000),
        micros.rem_euclid(1_000_000)
    )
}

fn render_context(
    text: &str,
    earlier: &[Value],
    limit: usize,
    me: Option<&Me>,
    lookup: &mut dyn FnMut(&str) -> String,
) -> Captured {
    let text = written_mentions(text, &mut *lookup);
    let messages = earlier
        .iter()
        .map(|m| {
            let user = m.get("user").and_then(Value::as_str).unwrap_or_default();
            let speaker = if user.is_empty() {
                m.get("username")
                    .or_else(|| m.get("bot_id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            } else {
                lookup(user)
            };
            Earlier {
                id: m.get("ts").and_then(Value::as_str).map(str::to_owned),
                speaker,
                own: me.is_some_and(|me| own_message(m, me)),
                at: m.get("ts").and_then(Value::as_str).and_then(ts_time),
                text: written_mentions(
                    m.get("text").and_then(Value::as_str).unwrap_or_default(),
                    &mut *lookup,
                ),
            }
        })
        .collect();
    Captured {
        display: None,
        text,
        accompanying: Accompanying {
            messages,
            limit: Some(limit),
            left_out: false,
            ..Accompanying::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Me, SlackEndpoint, admission, notice, split_conversation, ts_micros};
    use serde_json::{Value, json};
    use std::collections::HashMap;

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
        let lookup =
            |settings: HashMap<String, String>| move |name: &str| settings.get(name).cloned();
        let at = |url: &str| {
            let set = lookup(HashMap::from([(
                super::API_URL_SETTING.to_owned(),
                url.to_owned(),
            )]));
            SlackEndpoint::from_lookup(&set, system())
        };
        let unset = lookup(HashMap::new());
        assert!(SlackEndpoint::from_lookup(&unset, system()).is_ok_and(|e| !e.local));
        assert!(at("http://127.0.0.1:9/api").is_ok_and(|e| e.local));
        for refused in [
            "http://example.com/api",
            "https://slack.example/api",
            "ftp://127.0.0.1/",
            "nonsense",
        ] {
            assert!(at(refused).is_err(), "{refused}");
        }
        let slack = SlackEndpoint::from_lookup(&unset, system()).unwrap();
        assert!(slack.admits_socket("wss://wss-primary.slack.com/link/?ticket=1"));
        assert!(!slack.admits_socket("ws://wss-primary.slack.com/link"));
        assert!(!slack.admits_socket("wss://evil.example/link"));
        let local = at("http://127.0.0.1:9/api").unwrap();
        assert!(local.admits_socket("ws://127.0.0.1:10/"));
        assert!(!local.admits_socket("ws://example.com/"));
    }

    #[test]
    fn admission_marks_workspace_admin_owner_and_primary_owner_flags() {
        let me = Me {
            user: "UAGENT".into(),
            bot: Some("BAGENT".into()),
            team: "T1".into(),
        };
        let base = |extra: (&str, bool)| {
            let mut rec = json!({
                "id": "U1",
                "team_id": "T1",
                "deleted": false,
                "is_bot": false,
                "is_restricted": false,
                "is_ultra_restricted": false,
                "profile": {"display_name": "Steven", "email": "steven@example.com"},
            });
            rec[extra.0] = json!(extra.1);
            admission(&rec, &me).unwrap()
        };
        assert!(!base(("is_admin", false)).is_admin);
        assert!(base(("is_admin", true)).is_admin);
        assert!(base(("is_owner", true)).is_admin);
        assert!(base(("is_primary_owner", true)).is_admin);
    }

    #[test]
    fn rate_limits_are_isolated_per_company_and_connector() {
        use super::Limits;
        let now = std::sync::Arc::new(std::sync::Mutex::new(
            "2026-11-20T10:00:00Z".parse::<jiff::Timestamp>().unwrap(),
        ));
        let clock_ref = std::sync::Arc::clone(&now);
        let limits = Limits::new(std::sync::Arc::new(move || *clock_ref.lock().unwrap()));

        limits.limit("acme", "alpha", "chat.postMessage", 60);

        // Same company, different named connector: unaffected.
        assert_eq!(limits.remaining("acme", "beta", "chat.postMessage"), None);
        // Different company, same connector name: unaffected.
        assert_eq!(limits.remaining("other", "alpha", "chat.postMessage"), None);
        // Same company & connector, different method: unaffected.
        assert_eq!(limits.remaining("acme", "alpha", "users.info"), None);
        // Same (company, connector, method): rate-limited.
        assert_eq!(
            limits.remaining("acme", "alpha", "chat.postMessage"),
            Some(60)
        );

        // Once the clock advances past the Retry-After window, the limit clears.
        *now.lock().unwrap() = "2026-11-20T10:01:01Z".parse::<jiff::Timestamp>().unwrap();
        assert_eq!(limits.remaining("acme", "alpha", "chat.postMessage"), None);
    }

    #[test]
    fn receive_carries_the_declared_connector_name() {
        use super::receive;
        let envelope = json!({
            "type": "events_api",
            "payload": {
                "event": {
                    "type": "app_mention",
                    "channel": "C1",
                    "user": "U1",
                    "text": "<@UAGENT> hello",
                    "ts": "1700000001.000100"
                }
            }
        });
        let incoming = receive("assistant-two", &me(), &envelope).unwrap().unwrap();
        assert_eq!(incoming.connector, "assistant-two");
        assert_eq!(incoming.conversation, "C1/1700000001.000100");
        assert_eq!(incoming.sender, "U1");
    }
}
