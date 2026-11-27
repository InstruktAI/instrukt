//! One Socket Mode connection: its opening through Slack's `hello`, judged
//! by the outside gate under the app token, and the session that follows,
//! which retains each addressed message before acknowledging it.
use std::time::{Duration, Instant};

use instrukt_connector::{ConnectorError, Inbox, Incoming};
use instrukt_outside::{Attempted, Extent, Reason, Settings, Tried, Unavailable};
use serde_json::Value;

use crate::_outside::{socket_judgment, token_key};
use crate::{
    LOG_TARGET, Me, SlackApi, WEB_API_TIMEOUT, failed, newly_written, slack_failed, ts_time,
};

type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

/// How long one read of the socket waits, so the connection notices `quit`.
const SOCKET_READ: Duration = Duration::from_millis(200);

/// The socket of a Socket Mode connection Slack accepted: connected and past
/// Slack's `hello`, as one attempt of the outside gate under the app token.
/// Waiting for `hello` is bounded by the Web API's timeout.
fn open_socket(
    api: &SlackApi,
    url: &str,
    quit: &dyn Fn() -> bool,
) -> Result<Socket, ConnectorError> {
    let key = token_key(&api.company, &api.connector, true).item("socket");
    let gate = &api.endpoint.gate;
    let attempted = gate.attempt(&key, &Settings::default(), || {
        let judged = |error: tungstenite::Error| match socket_judgment(&error, gate.now()) {
            Some(unavailable) => Tried::Unavailable(unavailable, error.to_string()),
            None => Tried::Defect(error.to_string()),
        };
        let mut socket = match tungstenite::connect(url) {
            Ok((socket, _)) => socket,
            Err(error) => return judged(error),
        };
        let timeout = match socket.get_mut() {
            tungstenite::stream::MaybeTlsStream::Plain(stream) => {
                stream.set_read_timeout(Some(SOCKET_READ))
            }
            tungstenite::stream::MaybeTlsStream::Rustls(stream) => {
                stream.get_mut().set_read_timeout(Some(SOCKET_READ))
            }
            _ => Ok(()),
        };
        if let Err(error) = timeout {
            return judged(tungstenite::Error::Io(error));
        }
        let deadline = Instant::now() + WEB_API_TIMEOUT;
        while Instant::now() < deadline {
            if quit() {
                return Tried::Defect("stopped before Slack's hello".into());
            }
            match socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    let hello = serde_json::from_str::<Value>(&text)
                        .ok()
                        .is_some_and(|envelope| envelope["type"].as_str() == Some("hello"));
                    if hello {
                        return Tried::Done(socket);
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => return judged(error),
            }
        }
        Tried::Unavailable(
            Unavailable {
                reason: Reason::TimedOut,
                extent: Extent::Scope,
                named: None,
            },
            "no hello from Slack in time".into(),
        )
    });
    let cause = |error: Option<String>| match error {
        Some(error) => format!("Socket Mode connection: {error}"),
        None => "Socket Mode connection".into(),
    };
    match attempted {
        Attempted::Done(socket) => Ok(socket),
        Attempted::Defect(error) => Err(failed(cause(Some(error)))),
        Attempted::Wait { wait, error } => Err(ConnectorError::waiting(cause(error), wait)),
    }
}

/// One Socket Mode connection, from opening to its end. Opening is judged by
/// the outside gate; the session that follows ends with `Ok` however it
/// ends, and a drop or a failed acknowledgement is recorded with its cause.
pub(crate) fn session(
    api: &SlackApi,
    inbox: &dyn Inbox,
    quit: &dyn Fn() -> bool,
) -> Result<(), ConnectorError> {
    let company = &api.company;
    let url = api.connection_url().map_err(slack_failed)?;
    let me = api.identity().map_err(slack_failed)?;
    let mut socket = open_socket(api, &url, quit)?;
    inbox.connected();
    tracing::info!(target: LOG_TARGET, event = "connector_connected", company = %company, connector = %api.connector);
    let disconnected = |cause: &str| {
        tracing::info!(target: LOG_TARGET, event = "connector_disconnected", company = %company, connector = %api.connector, cause);
    };
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
            Err(error) => {
                disconnected(&format!("Socket Mode connection dropped: {error}"));
                return Ok(());
            }
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
                Some(incoming) => inbox.retain(incoming).map_err(|error| error.to_string()),
                None => Ok(()),
            });
        match retained {
            Ok(()) => {
                let acknowledged = socket.send(tungstenite::Message::text(
                    serde_json::json!({ "envelope_id": id }).to_string(),
                ));
                if let Err(error) = acknowledged {
                    disconnected(&format!("Socket Mode acknowledgement: {error}"));
                    return Ok(());
                }
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
    if let Err(error) = socket.close(None) {
        tracing::debug!(target: LOG_TARGET, event = "connector_close_unsent", company = %company, connector = %api.connector, %error);
    }
    Ok(())
}

/// The addressed message one Events API envelope carries: a mention in a
/// channel or a direct message; none for anything else. Nothing here waits
/// on the network.
pub(crate) fn receive(
    connector: &str,
    me: &Me,
    envelope: &Value,
) -> Result<Option<Incoming>, String> {
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
        thread: thread.map(str::to_owned),
    };
    Ok(Some(incoming))
}
