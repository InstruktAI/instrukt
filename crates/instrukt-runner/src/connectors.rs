//! Connectors hosted by `instrukt run` (`slack-connector` spec): one Socket
//! Mode connection per declared Slack connector, opened outward with the
//! app token, acknowledging every envelope and reconnecting after a drop.
//! Only mentions and direct messages reach the agent.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use instrukt_core::config::{Configuration, SecretSource, SlackConnector};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use jiff::Timestamp;
use serde_json::Value;

use crate::conversations::{self, Addressed, Admission, Earlier};

/// The connector's key under `connectors` and in a member entry.
pub const SLACK: &str = "slack";

/// Where the connector reaches Slack's Web API. Slack's own address unless
/// `INSTRUKT_SLACK_API_URL` names another, as a stand-in at the network line.
fn api_base() -> String {
    std::env::var("INSTRUKT_SLACK_API_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| "https://slack.com/api".into())
        .trim_end_matches('/')
        .to_owned()
}

/// A Web API failure, naming the method and Slack's error code; never a token.
#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    #[error("Slack {method}: {cause}")]
    Api { method: String, cause: String },
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
}

/// One company's Slack Web API, with its tokens resolved from the company
/// home's own secrets file.
pub struct SlackApi {
    base: String,
    app_token: String,
    bot_token: String,
    agent: ureq::Agent,
}

impl SlackApi {
    pub fn for_company(
        home: &Home,
        company: &ProjectSlug,
        declared: &SlackConnector,
    ) -> Result<Self, SlackError> {
        let secrets = SecretSource::Company(company.clone());
        Ok(Self {
            base: api_base(),
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
            .post(format!("{}/{method}", self.base))
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
        body.get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| SlackError::Api {
                method: "apps.connections.open".into(),
                cause: "no url".into(),
            })
    }

    /// The bot's own user and bot ids, to mark the agent's own posts.
    fn identity(&self) -> Result<(String, Option<String>), SlackError> {
        let body = self.call("auth.test", false, &[])?;
        Ok((
            body.get("user_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
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

    /// Messages of a channel or thread strictly between `oldest` and
    /// `latest`, oldest first; at most `limit` of the most recent.
    fn read(
        &self,
        channel: &str,
        thread: Option<&str>,
        oldest: Option<&str>,
        latest: &str,
        limit: Option<u64>,
    ) -> Result<Vec<Value>, SlackError> {
        let limit_text = limit.unwrap_or(1000).to_string();
        let mut form = vec![
            ("channel", channel),
            ("latest", latest),
            ("inclusive", "false"),
            ("limit", limit_text.as_str()),
        ];
        if let Some(oldest) = oldest {
            form.push(("oldest", oldest));
        }
        let (method, mut messages) = match thread {
            Some(thread) => {
                form.push(("ts", thread));
                ("conversations.replies", Vec::new())
            }
            None => ("conversations.history", Vec::new()),
        };
        let body = self.call(method, false, &form)?;
        messages.extend(
            body.get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| {
                    let ts = m.get("ts").and_then(Value::as_str).unwrap_or_default();
                    ts_value(ts) < ts_value(latest)
                        && oldest.is_none_or(|oldest| ts_value(ts) > ts_value(oldest))
                }),
        );
        messages.sort_by(|a, b| {
            let at = |m: &Value| ts_value(m.get("ts").and_then(Value::as_str).unwrap_or_default());
            at(a).total_cmp(&at(b))
        });
        if let Some(limit) = limit.and_then(|l| usize::try_from(l).ok())
            && messages.len() > limit
        {
            messages.drain(..messages.len() - limit);
        }
        Ok(messages)
    }
}

fn ts_value(ts: &str) -> f64 {
    ts.parse().unwrap_or(0.0)
}

fn ts_time(ts: &str) -> Option<Timestamp> {
    let (seconds, micros) = ts.split_once('.').unwrap_or((ts, "0"));
    let seconds: i64 = seconds.parse().ok()?;
    let micros: i64 = format!("{micros:0<6}").get(..6)?.parse().ok()?;
    Timestamp::from_microsecond(seconds.checked_mul(1_000_000)?.checked_add(micros)?).ok()
}

/// `<channel>/<thread ts>` for a channel thread, `<channel>` for a direct
/// message.
fn split_conversation(conversation: &str) -> (&str, Option<&str>) {
    match conversation.split_once('/') {
        Some((channel, thread)) => (channel, Some(thread)),
        None => (conversation, None),
    }
}

/// Posting through the company's declared connector.
pub struct Posting<'a> {
    pub home: &'a Home,
}
impl conversations::Poster for Posting<'_> {
    fn post(
        &self,
        company: &ProjectSlug,
        connector: &str,
        conversation: &str,
        text: &str,
    ) -> Result<(), String> {
        if connector != SLACK {
            return Err(format!("unsupported connector `{connector}`"));
        }
        let config = Configuration::load_company(self.home, company).map_err(|e| e.to_string())?;
        let declared = config
            .slack_connector()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "the company no longer declares its slack connector".to_owned())?;
        SlackApi::for_company(self.home, company, &declared)
            .and_then(|api| api.post(conversation, text))
            .map_err(|e| e.to_string())
    }
}

/// The connections `instrukt run` holds open.
pub struct Connections {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Connections {
    /// Open one connection per declared Slack connector. `delivered` is told
    /// when an addressed message was accepted.
    pub fn start(
        home: &Home,
        delivered: std::sync::mpsc::SyncSender<()>,
    ) -> Result<Self, conversations::ConversationError> {
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        for (company, declared) in conversations::demand(home)? {
            let home = home.clone();
            let stop = Arc::clone(&stop);
            let delivered = delivered.clone();
            let name = format!("slack-{company}");
            let spawned = std::thread::Builder::new().name(name).spawn(move || {
                connection(&home, &company, &declared, &stop, &delivered);
            });
            match spawned {
                Ok(thread) => threads.push(thread),
                Err(error) => tracing::error!(event = "connector_start_failed", error = %error),
            }
        }
        Ok(Self { stop, threads })
    }
}

impl Drop for Connections {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                tracing::error!(event = "connector_thread_panicked");
            }
        }
    }
}

fn state(home: &Home, company: &ProjectSlug, status: &str, failure: Option<&str>) {
    if let Err(error) = conversations::connector_state(home, company, SLACK, status, failure) {
        tracing::warn!(event = "connector_state_unrecorded", company = %company, error = %error);
    }
}

/// Keep one company's connection open until `stop`: a refused or dropped
/// connection is recorded and retried, and nothing else stops.
fn connection(
    home: &Home,
    company: &ProjectSlug,
    declared: &SlackConnector,
    stop: &AtomicBool,
    delivered: &std::sync::mpsc::SyncSender<()>,
) {
    let mut backoff = Duration::from_secs(1);
    while !stop.load(Ordering::Acquire) {
        state(home, company, conversations::CONNECTING, None);
        let failure = match session(home, company, declared, stop, delivered) {
            Ok(()) => {
                backoff = Duration::from_secs(1);
                None
            }
            Err(error) => Some(error),
        };
        if stop.load(Ordering::Acquire) {
            break;
        }
        if let Some(error) = &failure {
            tracing::warn!(event = "connector_failed", company = %company, connector = SLACK, error = %error);
            state(home, company, conversations::FAILED, Some(error));
        }
        let until = std::time::Instant::now() + backoff;
        while std::time::Instant::now() < until && !stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(100));
        }
        if failure.is_some() {
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    }
    state(home, company, conversations::STOPPED, None);
}

/// One Socket Mode connection, from opening to its drop or a disconnect.
fn session(
    home: &Home,
    company: &ProjectSlug,
    declared: &SlackConnector,
    stop: &AtomicBool,
    delivered: &std::sync::mpsc::SyncSender<()>,
) -> Result<(), String> {
    let api = SlackApi::for_company(home, company, declared).map_err(|e| e.to_string())?;
    let url = api.connection_url().map_err(|e| e.to_string())?;
    let me = api.identity().map_err(|e| e.to_string())?;
    let (mut socket, _) =
        tungstenite::connect(url.as_str()).map_err(|e| format!("Socket Mode connection: {e}"))?;
    match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => {
            stream.set_read_timeout(Some(Duration::from_millis(500)))
        }
        tungstenite::stream::MaybeTlsStream::Rustls(stream) => stream
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(500))),
        _ => Ok(()),
    }
    .map_err(|e| format!("Socket Mode connection: {e}"))?;
    state(home, company, conversations::CONNECTED, None);
    tracing::info!(event = "connector_connected", company = %company, connector = SLACK);
    while !stop.load(Ordering::Acquire) {
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
        // A message is acknowledged once it is retained, so Slack retries
        // what instrukt could not keep.
        match receive(home, company, declared, &api, &me, &envelope) {
            Ok(accepted) => {
                socket
                    .send(tungstenite::Message::text(
                        serde_json::json!({ "envelope_id": id }).to_string(),
                    ))
                    .map_err(|e| format!("Socket Mode acknowledgement: {e}"))?;
                if accepted {
                    let _ = delivered.try_send(());
                }
            }
            Err(error) => {
                tracing::warn!(event = "connector_message_unretained", company = %company, connector = SLACK, error = %error);
            }
        }
    }
    let _ = socket.close(None);
    Ok(())
}

/// One Events API envelope: a mention in a channel or a direct message.
/// Returns whether a message was retained for the agent.
fn receive(
    home: &Home,
    company: &ProjectSlug,
    declared: &SlackConnector,
    api: &SlackApi,
    me: &(String, Option<String>),
    envelope: &Value,
) -> Result<bool, String> {
    if envelope.get("type").and_then(Value::as_str) != Some("events_api") {
        return Ok(false);
    }
    let Some(event) = envelope.get("payload").and_then(|p| p.get("event")) else {
        return Ok(false);
    };
    let text = |field: &str| event.get(field).and_then(Value::as_str);
    let (Some(channel), Some(ts), Some(sender)) = (text("channel"), text("ts"), text("user"))
    else {
        return Ok(false);
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
        _ => return Ok(false),
    };
    let conversation = match thread {
        Some(thread) => format!("{channel}/{thread}"),
        None => channel.to_owned(),
    };
    let previous = conversations::previous_message(home, company, SLACK, &conversation)
        .map_err(|e| e.to_string())?;
    let earlier = match &previous {
        // A later message: everything after the previous addressed one.
        Some(previous) => api.read(channel, thread, Some(previous), ts, None),
        // The first: the most recent of the thread, or of the channel when
        // this tag starts the thread.
        None => {
            let within = thread.filter(|thread| *thread != ts);
            api.read(channel, within, None, ts, Some(declared.context_messages))
        }
    }
    .map_err(|e| e.to_string())?;
    let context = earlier
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
    let addressed = Addressed {
        company: company.clone(),
        connector: SLACK.into(),
        conversation,
        message: ts.to_owned(),
        sender: sender.to_owned(),
        written_at: ts_time(ts).ok_or("message without a valid time")?,
        text: text("text").unwrap_or_default().to_owned(),
        context,
    };
    let now = Timestamp::now();
    match conversations::admit(home, &addressed, now).map_err(|e| e.to_string())? {
        Admission::Accepted => Ok(true),
        Admission::AlreadyTurnedAway => Ok(false),
        Admission::TurnAway => {
            api.post(&addressed.conversation, conversations::TURN_AWAY)
                .map_err(|e| e.to_string())?;
            conversations::turned_away(home, &addressed, now).map_err(|e| e.to_string())?;
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{split_conversation, ts_time};

    #[test]
    fn slack_times_and_conversation_identities_read_back() {
        assert_eq!(
            ts_time("1700000000.000100").map(|t| t.as_microsecond()),
            Some(1_700_000_000_000_100)
        );
        assert_eq!(split_conversation("C1/170.1"), ("C1", Some("170.1")));
        assert_eq!(split_conversation("D1"), ("D1", None));
    }
}
