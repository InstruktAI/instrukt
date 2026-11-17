//! Connectors hosted by `instrukt run` (`slack-connector` spec): one Socket
//! Mode connection per declared Slack connector, opened outward with the
//! app token. Each envelope is acknowledged as soon as what it carries is
//! retained; reading the accompanying messages and posting refusals follow
//! from the retained record, retried until done. Only mentions and direct
//! messages reach the agent.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use instrukt_core::config::{Configuration, Env, SecretSource, SlackConnector};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use jiff::Timestamp;
use serde_json::Value;

use crate::conversations::{self, Accompanying, ConnectionState, Earlier, Incoming, Intake};

/// The connector's key under `connectors` and in a member entry.
pub const SLACK: &str = "slack";

/// The setting that points the connector at a local stand-in for Slack.
/// Only a loopback address is accepted.
pub const API_URL_SETTING: &str = "INSTRUKT_SLACK_API_URL";

/// Slack's Web API.
const SLACK_API: &str = "https://slack.com/api";

/// How long retained intake work that failed waits before it is retried.
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

    /// The bot's own user and bot ids, to mark the agent's own posts.
    fn identity(&self) -> Result<(String, Option<String>), SlackError> {
        let body = self.call("auth.test", false, &[])?;
        let user = body
            .get("user_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| SlackError::Api {
                method: "auth.test".into(),
                cause: "no user_id".into(),
            })?;
        Ok((
            user.to_owned(),
            body.get("bot_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ))
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
    let mut backoff = Duration::from_secs(1);
    while !stop.load(Ordering::Acquire) {
        state(home, company, ConnectionState::Connecting, None);
        let failure = session(home, endpoint, company, declared, stop, wake).err();
        if stop.load(Ordering::Acquire) {
            break;
        }
        if let Some(error) = &failure {
            tracing::warn!(event = "connector_failed", company = %company, connector = SLACK, error = %error);
            state(home, company, ConnectionState::Failed, Some(error));
        }
        let until = Instant::now() + backoff;
        while Instant::now() < until && !stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(100));
        }
        if let Ok(api) = SlackApi::with(home, company, declared, endpoint) {
            intake(home, company, declared, &api, None, wake);
        }
        backoff = if failure.is_some() {
            (backoff * 2).min(Duration::from_secs(60))
        } else {
            Duration::from_secs(1)
        };
    }
    state(home, company, ConnectionState::Stopped, None);
}

/// One Socket Mode connection, from opening to its drop or a disconnect.
fn session(
    home: &Home,
    endpoint: &SlackEndpoint,
    company: &ProjectSlug,
    declared: &SlackConnector,
    stop: &AtomicBool,
    wake: &SyncSender<()>,
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
    let mut due = Instant::now();
    let mut unretained = false;
    while !stop.load(Ordering::Acquire) {
        if Instant::now() >= due {
            intake(home, company, declared, &api, Some(&me), wake);
            due = Instant::now() + RETRY;
        }
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
                due = Instant::now();
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
fn receive(
    home: &Home,
    company: &ProjectSlug,
    me: &(String, Option<String>),
    envelope: &Value,
) -> Result<(), String> {
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
                && sender != me.0 =>
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
        Intake::Retained | Intake::Refused | Intake::Repeated => Ok(()),
    }
}

/// Retained intake work of one company: post owed refusals, and read what
/// accompanies each received message. A failure stays with its record and
/// is retried; the relay is woken for what became deliverable.
fn intake(
    home: &Home,
    company: &ProjectSlug,
    declared: &SlackConnector,
    api: &SlackApi,
    me: Option<&(String, Option<String>)>,
    wake: &SyncSender<()>,
) {
    let result = (|| -> Result<(), conversations::ConversationError> {
        for refusal in conversations::unsent_refusals(home, company, SLACK)? {
            let outcome = api
                .post(&refusal.conversation, conversations::TURN_AWAY)
                .map_err(|e| e.to_string());
            conversations::refusal_posted(
                home,
                company,
                SLACK,
                &refusal,
                outcome,
                Timestamp::now(),
            )?;
        }
        let owned;
        let me = match me {
            Some(me) => me,
            None => {
                let Ok(identity) = api.identity() else {
                    return Ok(());
                };
                owned = identity;
                &owned
            }
        };
        let mut readable = false;
        for unread in conversations::unread(home, company, SLACK)? {
            let context = accompanying(api, declared, me, &unread).map_err(|e| e.to_string());
            readable |= context.is_ok();
            conversations::captured(home, company, SLACK, &unread, context)?;
        }
        if readable {
            let _ = wake.try_send(());
        }
        Ok(())
    })();
    if let Err(error) = result {
        tracing::warn!(event = "connector_intake_failed", company = %company, connector = SLACK, error = %error);
    }
}

/// What accompanies a received message: of the messages before the first
/// in a conversation — of the thread, or of the channel when it starts the
/// thread — or of those since the previous one, at most `context_messages`
/// of the most recent, and whether earlier ones were left out.
fn accompanying(
    api: &SlackApi,
    declared: &SlackConnector,
    me: &(String, Option<String>),
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
                .is_some_and(|bot| Some(bot) == me.1.as_deref())
                || (!user.is_empty() && user == me.0);
            let speaker = m
                .get("user_profile")
                .and_then(|p| p.get("display_name").or_else(|| p.get("real_name")))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .unwrap_or(user);
            Earlier {
                speaker: speaker.to_owned(),
                own,
                at: m.get("ts").and_then(Value::as_str).and_then(ts_time),
                text: m
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }
        })
        .collect();
    Ok(Accompanying { messages, left_out })
}

#[cfg(test)]
mod tests {
    use super::{SlackEndpoint, split_conversation, ts_micros};
    use instrukt_core::config::Env;

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
