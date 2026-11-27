//! One single-shot Socket Mode connection: `apps.connections.open` and the
//! WebSocket handshake through Slack's `hello` are judged by the outside gate
//! under the app token, using `slack-morphism`'s typed Socket Mode envelopes
//! and acknowledgements while keeping reconnect pacing and failure reporting
//! exclusively in `connector_host` / `instrukt-outside`.
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use instrukt_connector::{ConnectorError, Inbox, Incoming};
use instrukt_outside::runtime::block_on;
use instrukt_outside::{Attempted, Extent, Reason, Settings, Tried, Unavailable};
use serde_json::{Value, json};
use slack_morphism::prelude::SlackEventCallbackBody;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::_outside::{socket_judgment, token_key};
use crate::{
    LOG_TARGET, Me, SlackApi, WEB_API_TIMEOUT, failed, newly_written, slack_failed, ts_time,
};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How often the single-shot read loop polls the synchronous `quit` predicate
/// until Wave 2 converts `Connector::listen` to async cancellation.
const QUIT_INTERVAL: Duration = Duration::from_millis(200);

/// Opens one Socket Mode WebSocket and waits for Slack's `hello`, judged as one
/// attempt of the outside gate under the app token. Never logs the URL/ticket.
fn open_socket(
    api: &SlackApi,
    url: &str,
    quit: &dyn Fn() -> bool,
) -> Result<WsStream, ConnectorError> {
    let key = token_key(&api.company, &api.connector, true).item("socket");
    let gate = &api.endpoint.gate;
    let attempted = gate.attempt(&key, &Settings::default(), || {
        let judged = |error: tokio_tungstenite::tungstenite::Error| {
            // Never format the URL (which carries `?ticket=...`).
            let message = match &error {
                tokio_tungstenite::tungstenite::Error::Http(resp) => {
                    format!("HTTP {}", resp.status().as_u16())
                }
                tokio_tungstenite::tungstenite::Error::Io(io_err) => io_err.to_string(),
                tokio_tungstenite::tungstenite::Error::Tls(_) => "TLS handshake failed".into(),
                _ => "WebSocket connection failed".into(),
            };
            match socket_judgment(&error, gate.now()) {
                Some(unavailable) => Tried::Unavailable(unavailable, message),
                None => Tried::Defect(message),
            }
        };
        let connect_res = block_on(tokio::time::timeout(
            WEB_API_TIMEOUT,
            tokio_tungstenite::connect_async(url),
        ));
        let mut stream = match connect_res {
            Ok(Ok(Ok((stream, _)))) => stream,
            Ok(Ok(Err(error))) => return judged(error),
            Ok(Err(_)) => {
                return Tried::Unavailable(
                    Unavailable {
                        reason: Reason::TimedOut,
                        extent: Extent::Scope,
                        named: None,
                    },
                    "timed out connecting to Slack Socket Mode".into(),
                );
            }
            Err(error) => return Tried::Defect(error.to_string()),
        };
        let deadline = Instant::now() + WEB_API_TIMEOUT;
        while Instant::now() < deadline {
            if quit() {
                let _ = block_on(stream.close(None));
                return Tried::Defect("stopped before Slack's hello".into());
            }
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(QUIT_INTERVAL);
            match block_on(tokio::time::timeout(wait, stream.next())) {
                Ok(Ok(Some(Ok(Message::Text(text))))) => {
                    let hello = serde_json::from_str::<Value>(text.as_str())
                        .ok()
                        .is_some_and(|envelope| envelope["type"].as_str() == Some("hello"));
                    if hello {
                        return Tried::Done(stream);
                    }
                }
                Ok(Ok(Some(Ok(Message::Ping(payload))))) => {
                    match block_on(stream.send(Message::Pong(payload))) {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => return judged(error),
                        Err(error) => return Tried::Defect(error.to_string()),
                    }
                }
                Ok(Ok(Some(Ok(Message::Close(_))))) | Ok(Ok(None)) => {
                    return Tried::Unavailable(
                        Unavailable {
                            reason: Reason::Unreachable,
                            extent: Extent::Scope,
                            named: None,
                        },
                        "Slack closed Socket Mode before hello".into(),
                    );
                }
                Ok(Ok(Some(Ok(_)))) | Ok(Err(_)) => {}
                Ok(Ok(Some(Err(error)))) => return judged(error),
                Err(error) => return Tried::Defect(error.to_string()),
            }
        }
        let _ = block_on(stream.close(None));
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
/// the outside gate; the session that follows ends with `Ok(())` on drop,
/// Slack disconnect, or `quit()`, so `connector_host` owns reconnection pacing
/// and `ConnectionState::Failed`.
pub(crate) fn session(
    api: &SlackApi,
    inbox: &dyn Inbox,
    quit: &dyn Fn() -> bool,
) -> Result<(), ConnectorError> {
    if quit() {
        return Ok(());
    }
    let url = api.connection_url().map_err(slack_failed)?;
    let me = api.identity().map_err(slack_failed)?;
    let mut stream = open_socket(api, &url, quit)?;
    if quit() {
        let _ = block_on(stream.close(None));
        return Ok(());
    }
    inbox.connected();
    tracing::info!(target: LOG_TARGET, event = "connector_connected", company = %api.company, connector = %api.connector);
    let disconnected = |cause: &str| {
        tracing::info!(target: LOG_TARGET, event = "connector_disconnected", company = %api.company, connector = %api.connector, cause);
    };
    let mut unretained = false;
    while !quit() {
        let frame = match block_on(tokio::time::timeout(QUIT_INTERVAL, stream.next())) {
            Ok(Ok(Some(Ok(message)))) => message,
            Ok(Ok(Some(Err(error)))) => {
                disconnected(&format!("Socket Mode connection dropped: {error}"));
                return Ok(());
            }
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(_)) => continue,
            Err(error) => {
                disconnected(&format!("Socket Mode runtime error: {error}"));
                return Ok(());
            }
        };
        let text = match frame {
            Message::Text(text) => text,
            Message::Ping(payload) => {
                if let Err(error) = block_on(stream.send(Message::Pong(payload)))
                    .map_err(|e| e.to_string())
                    .and_then(|res| res.map_err(|e| e.to_string()))
                {
                    disconnected(&format!("Socket Mode pong failed: {error}"));
                    return Ok(());
                }
                continue;
            }
            Message::Close(_) => return Ok(()),
            _ => continue,
        };
        let Ok(envelope) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };
        match envelope.get("type").and_then(Value::as_str) {
            Some("hello") => continue,
            Some("disconnect") => return Ok(()),
            _ => {}
        }
        let Some(envelope_id) = envelope.get("envelope_id").and_then(Value::as_str) else {
            continue;
        };
        let retained = if envelope.get("type").and_then(Value::as_str) == Some("events_api") {
            match envelope.pointer("/payload/event") {
                Some(raw_event) => {
                    serde_json::from_value::<SlackEventCallbackBody>(raw_event.clone())
                        .map_err(|error| error.to_string())
                        .and_then(|event| receive(&api.connector, &me, event))
                        .and_then(|incoming| match incoming {
                            Some(incoming) => {
                                inbox.retain(incoming).map_err(|error| error.to_string())
                            }
                            None => Ok(()),
                        })
                }
                None => Ok(()),
            }
        } else {
            Ok(())
        };
        match retained {
            Ok(()) => {
                let ack_json = json!({ "envelope_id": envelope_id }).to_string();
                if let Err(error) = block_on(stream.send(Message::Text(ack_json.into())))
                    .map_err(|e| e.to_string())
                    .and_then(|res| res.map_err(|e| e.to_string()))
                {
                    disconnected(&format!("Socket Mode acknowledgement: {error}"));
                    return Ok(());
                }
                if unretained {
                    inbox.connected();
                    unretained = false;
                }
            }
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, event = "connector_message_unretained", company = %api.company, connector = %api.connector, %error);
                inbox.unretained(&format!(
                    "a message could not be retained and awaits Slack's retry: {error}"
                ));
                unretained = true;
            }
        }
    }
    if let Err(error) = block_on(stream.close(None))
        .map_err(|e| e.to_string())
        .and_then(|res| res.map_err(|e| e.to_string()))
    {
        tracing::debug!(target: LOG_TARGET, event = "connector_close_unsent", company = %api.company, connector = %api.connector, %error);
    }
    Ok(())
}

/// The addressed Events API message: a mention in a
/// channel or a direct message; none for anything else. Nothing here waits
/// on the network.
pub(crate) fn receive(
    connector: &str,
    me: &Me,
    event: SlackEventCallbackBody,
) -> Result<Option<Incoming>, String> {
    let event = match event {
        SlackEventCallbackBody::Unknown(raw) => raw,
        event => serde_json::to_value(event).map_err(|error| error.to_string())?,
    };
    let text = |field: &str| event.get(field).and_then(Value::as_str);
    let (Some(channel), Some(ts), Some(sender)) = (text("channel"), text("ts"), text("user"))
    else {
        return Ok(None);
    };
    if !newly_written(&event) {
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use slack_morphism::prelude::SlackEventCallbackBody;

    use super::receive;
    use crate::Me;

    fn me() -> Me {
        Me {
            user: "UAGENT".into(),
            bot: Some("BAGENT".into()),
            team: "T1".into(),
        }
    }

    #[test]
    fn receive_admits_newly_written_dm_subtypes_and_ignores_edits() {
        for subtype in ["file_share", "me_message", "thread_broadcast"] {
            let body: SlackEventCallbackBody = serde_json::from_value(json!({
                "type": "message",
                "subtype": subtype,
                "channel": "D1",
                "channel_type": "im",
                "user": "U1",
                "text": "hello",
                "ts": "1700000001.000100"
            }))
            .unwrap();
            let incoming = receive("slack", &me(), body).unwrap();
            assert!(incoming.is_some(), "expected {subtype} DM to be admitted");
        }
        let changed: SlackEventCallbackBody = serde_json::from_value(json!({
            "type": "message",
            "subtype": "message_changed",
            "channel": "D1",
            "channel_type": "im",
            "user": "U1",
            "text": "edited",
            "ts": "1700000001.000200"
        }))
        .unwrap();
        assert!(receive("slack", &me(), changed).unwrap().is_none());
    }

    #[test]
    fn receive_ignores_edited_app_mention() {
        let edited: SlackEventCallbackBody = serde_json::from_value(json!({
            "type": "app_mention",
            "channel": "C1",
            "user": "U1",
            "text": "<@UAGENT> edited",
            "ts": "1700000001.000300",
            "edited": {"user": "U1", "ts": "1700000002.000000"}
        }))
        .unwrap();
        assert!(receive("slack", &me(), edited).unwrap().is_none());
    }
}
