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
    Accompanying, Admission, AnswerError, Captured, Clock, Connector, ConnectorError, Declaration,
    Declared, Earlier, Field, Inbox, Incoming, Kind, Place, Rule, SecretResolver, Sender, Status,
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

/// The upper end of a read that reaches to now: a Slack time no message
/// carries yet.
const UNTIL_NOW: &str = "9999999999.999999";

/// How many earlier messages accompany an addressed message when the
/// declaration names none.
const DEFAULT_CONTEXT_MESSAGES: u64 = 30;

/// What `context_messages` is, as its finding and editor schema say it.
const CONTEXT_MESSAGES: &str = "the largest number of earlier messages that accompany an addressed message: a positive whole number; 30 when omitted";

/// The fields a `connectors.slack` entry takes.
pub static SLACK_DECLARATION: Declaration = Declaration {
    key: SLACK,
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

/// One company's Slack Web API, with the tokens its admitted generation
/// resolved, as its connector's opening drew them through the resolver.
pub struct SlackApi {
    company: String,
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
    /// The Web API of `company`'s Slack connector, with its two tokens.
    fn new(company: &str, endpoint: &SlackEndpoint, app_token: String, bot_token: String) -> Self {
        Self {
            company: company.to_owned(),
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
        let company = self.company.clone();
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
            tracing::warn!(target: LOG_TARGET, event = "connector_rate_limited", company = %self.company, method, seconds);
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
    pub fn answer(
        &self,
        conversation: &str,
        markdown: &str,
    ) -> Result<Option<Timestamp>, SlackError> {
        let (channel, thread) = split_conversation(conversation);
        let mut form = vec![("channel", channel), ("markdown_text", markdown)];
        if let Some(thread) = thread {
            form.push(("thread_ts", thread));
        }
        self.call("chat.postMessage", false, &form)
            .map(|post| post.get("ts").and_then(Value::as_str).and_then(ts_time))
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
    known: std::sync::MutexGuard<'a, HashMap<String, String>>,
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

/// The Slack kind of this process: its endpoint, built once when the runner
/// starts, and the bot identity each company's tokens answered with, asked
/// of Slack once while those tokens stay.
pub struct SlackKind {
    endpoint: SlackEndpoint,
    identities: Arc<Mutex<HashMap<String, (String, Me)>>>,
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
        declared: &Declared,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn Connector>, ConnectorError> {
        let token = |field: &str| -> Result<String, ConnectorError> {
            let reference = declared.text(field).ok_or_else(|| ConnectorError {
                cause: format!("the slack connector declares no {field}"),
            })?;
            secrets.resolve(reference)
        };
        let api = SlackApi::new(
            company,
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
            names: Mutex::default(),
        }))
    }
}

/// One company's Slack connector.
struct Slack {
    api: SlackApi,
    context_messages: u64,
    identities: Arc<Mutex<HashMap<String, (String, Me)>>>,
    /// The people's names asked of Slack while this connector is open.
    names: Mutex<HashMap<String, String>>,
}

impl Slack {
    /// The bot's identity in its workspace, asked of Slack once for these
    /// tokens.
    fn me(&self) -> Result<Me, SlackError> {
        let company = self.api.company.clone();
        if let Ok(identities) = self.identities.lock()
            && let Some((token, me)) = identities.get(&company)
            && *token == self.api.bot_token
        {
            return Ok(me.clone());
        }
        let me = self.api.identity()?;
        if let Ok(mut identities) = self.identities.lock() {
            identities.insert(company, (self.api.bot_token.clone(), me.clone()));
        }
        Ok(me)
    }

    fn names(&self) -> Names<'_> {
        Names {
            api: &self.api,
            known: match self.names.lock() {
                Ok(known) => known,
                Err(poisoned) => poisoned.into_inner(),
            },
        }
    }
}

fn failed(error: impl std::fmt::Display) -> ConnectorError {
    ConnectorError {
        cause: error.to_string(),
    }
}

impl Connector for Slack {
    fn limits(&self) -> instrukt_connector::Limits {
        instrukt_connector::Limits {
            message_chars: SLACK_MESSAGE_LIMIT,
            answer_timeout: WEB_API_TIMEOUT,
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
                    self.names()
                        .known
                        .insert(sender.to_owned(), admitted.name.clone());
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
        previous: Option<&str>,
        text: &str,
    ) -> Result<Captured, ConnectorError> {
        let me = self.me().map_err(failed)?;
        let mut names = self.names();
        let text = names.written(text);
        let accompanying = accompanying(
            &self.api,
            self.context_messages,
            &me,
            &mut names,
            conversation,
            message,
            previous,
        )
        .map_err(failed)?;
        Ok(Captured { text, accompanying })
    }

    fn answer(&self, conversation: &str, part: &str) -> Result<Option<Timestamp>, AnswerError> {
        self.api
            .answer(conversation, part)
            .map_err(|error| AnswerError {
                rejected: matches!(
                    error,
                    SlackError::RejectedPost { .. } | SlackError::Limited { .. }
                ),
                cause: error.to_string(),
            })
    }

    /// Read as the accompanying messages are: the thread's replies for a
    /// channel, the direct-message channel's history otherwise.
    fn own_posts(
        &self,
        conversation: &str,
        since: &str,
    ) -> Result<Vec<instrukt_connector::OwnPost>, ConnectorError> {
        let me = self.me().map_err(failed)?;
        let (channel, thread) = split_conversation(conversation);
        let (messages, _) = self
            .api
            .read(channel, thread, Some(since), UNTIL_NOW, usize::MAX)
            .map_err(failed)?;
        Ok(messages
            .iter()
            .filter(|m| {
                m.get("bot_id")
                    .and_then(Value::as_str)
                    .is_some_and(|bot| Some(bot) == me.bot.as_deref())
                    || m.get("user").and_then(Value::as_str) == Some(me.user.as_str())
            })
            .filter_map(|m| {
                Some(instrukt_connector::OwnPost {
                    at: m.get("ts").and_then(Value::as_str).and_then(ts_time)?,
                    text: m
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
            })
            .collect())
    }

    fn turn_away(&self, conversation: &str) -> Result<(), ConnectorError> {
        self.api.post(conversation, TURN_AWAY).map_err(failed)
    }

    fn unavailable_notice(&self, member: &str) -> String {
        format!("<@{member}> {NOTICE}")
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
    tracing::info!(target: LOG_TARGET, event = "connector_connected", company = %company, connector = SLACK);
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
        let retained = receive(&me, &envelope).and_then(|incoming| match incoming {
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
                tracing::warn!(target: LOG_TARGET, event = "connector_message_unretained", company = %company, connector = SLACK, error = %error);
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
fn receive(me: &Me, envelope: &Value) -> Result<Option<Incoming>, String> {
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
    let incoming = Incoming {
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
    Ok(Some(incoming))
}

/// What accompanies a received message: of the messages before the first
/// in a conversation — of the thread, or of the channel when it starts the
/// thread — or of those since the previous one, at most `context_messages`
/// of the most recent, and whether earlier ones were left out. Speakers are
/// named and mentions written as names, as for the addressed message.
fn accompanying(
    api: &SlackApi,
    context_messages: u64,
    me: &Me,
    names: &mut Names<'_>,
    conversation: &str,
    message: &str,
    previous: Option<&str>,
) -> Result<Accompanying, SlackError> {
    let (channel, thread) = split_conversation(conversation);
    let ts = message;
    let cap = usize::try_from(context_messages).unwrap_or(usize::MAX);
    let (earlier, left_out) = match previous {
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
}
