//! One single-shot native-async Socket Mode connection: `apps.connections.open`
//! and the WebSocket handshake through Slack's `hello` are judged by the
//! outside gate under the app token, using `slack-morphism`'s
//! `SlackEventCallbackBody` while keeping reconnect pacing, `admits_socket`,
//! `?ticket=` redaction, and failure reporting exclusively in `connector_host`
//! and `instrukt-outside` (see `project/instrukt/adr/0001-slack-socket-mode-transport`).
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use instrukt_connector::{ConnectorError, Inbox, Incoming};
use instrukt_outside::{Attempted, Extent, Reason, Settings, Tried, Unavailable};
use serde_json::{Value, json};
use slack_morphism::prelude::SlackEventCallbackBody;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::instrument::WithSubscriber as _;

use crate::_outside::{socket_judgment, token_key};
use crate::{
    LOG_TARGET, Me, SlackApi, WEB_API_TIMEOUT, failed, newly_written, slack_failed, ts_time,
};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Bound how long a WebSocket close handshake may stall shutdown.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

async fn close_bounded(stream: &mut WsStream) -> Result<(), String> {
    match tokio::time::timeout(CLOSE_TIMEOUT, stream.close(None)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("close timed out".into()),
    }
}

async fn send_cancellable(
    stream: &mut WsStream,
    message: Message,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<bool, tokio_tungstenite::tungstenite::Error> {
    tokio::select! {
        _ = wait_shutdown(shutdown) => Ok(false),
        res = tokio::time::timeout(WEB_API_TIMEOUT, stream.send(message)) => match res {
            Ok(Ok(())) => Ok(true),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(tokio_tungstenite::tungstenite::Error::Io(
                std::io::Error::new(std::io::ErrorKind::TimedOut, "Socket Mode write timed out"),
            )),
        }
    }
}

async fn wait_shutdown(shutdown: &mut watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow() {
            return;
        }
    }
}

/// Opens one Socket Mode WebSocket and waits for Slack's `hello`, judged as one
/// attempt of the outside gate under the app token. Never logs the URL/ticket.
async fn open_socket(
    api: &SlackApi,
    url: &str,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<WsStream, ConnectorError> {
    instrukt_outside::http::ensure_crypto_provider();
    let key = token_key(&api.company, &api.connector, true).item("socket");
    let gate = &api.endpoint.gate;
    let attempted = gate
        .attempt_async(&key, &Settings::default(), || async {
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
            // Suppress tracing/log-bridged events during the WebSocket handshake
            // so `tungstenite`'s trace log never writes the `?ticket=` URL.
            let connect = tokio_tungstenite::connect_async(url)
                .with_subscriber(tracing::subscriber::NoSubscriber::default());
            let mut stream = tokio::select! {
                _ = wait_shutdown(shutdown) => {
                    return Tried::Defect("stopped before Socket Mode connected".into());
                }
                res = tokio::time::timeout(WEB_API_TIMEOUT, connect) => {
                    match res {
                        Ok(Ok((stream, _))) => stream,
                        Ok(Err(error)) => return judged(error),
                        Err(_) => {
                            return Tried::Unavailable(
                                Unavailable {
                                    reason: Reason::TimedOut,
                                    extent: Extent::Scope,
                                    named: None,
                                },
                                "timed out connecting to Slack Socket Mode".into(),
                            );
                        }
                    }
                }
            };
            let deadline = tokio::time::Instant::now() + WEB_API_TIMEOUT;
            loop {
                tokio::select! {
                    _ = wait_shutdown(shutdown) => {
                        let _ = close_bounded(&mut stream).await;
                        return Tried::Defect("stopped before Slack's hello".into());
                    }
                    res = tokio::time::timeout_at(deadline, stream.next()) => {
                        match res {
                            Ok(Some(Ok(Message::Text(text)))) => {
                                let hello = serde_json::from_str::<Value>(text.as_str())
                                    .ok()
                                    .is_some_and(|envelope| envelope["type"].as_str() == Some("hello"));
                                if hello {
                                    return Tried::Done(stream);
                                }
                            }
                            Ok(Some(Ok(Message::Ping(payload)))) => {
                                match send_cancellable(&mut stream, Message::Pong(payload), shutdown).await {
                                    Ok(true) => {}
                                    Ok(false) => {
                                        let _ = close_bounded(&mut stream).await;
                                        return Tried::Defect("stopped before Slack's hello".into());
                                    }
                                    Err(error) => return judged(error),
                                }
                            }
                            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => {
                                return Tried::Unavailable(
                                    Unavailable {
                                        reason: Reason::Unreachable,
                                        extent: Extent::Scope,
                                        named: None,
                                    },
                                    "Slack closed Socket Mode before hello".into(),
                                );
                            }
                            Ok(Some(Ok(_))) => {}
                            Ok(Some(Err(error))) => return judged(error),
                            Err(_) => {
                                let _ = close_bounded(&mut stream).await;
                                return Tried::Unavailable(
                                    Unavailable {
                                        reason: Reason::TimedOut,
                                        extent: Extent::Scope,
                                        named: None,
                                    },
                                    "no hello from Slack in time".into(),
                                );
                            }
                        }
                    }
                }
            }
        })
        .await;
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
/// Slack disconnect, or `shutdown`, so `connector_host` owns reconnection
/// pacing and `ConnectionState::Failed`.
pub(crate) async fn session(
    api: &SlackApi,
    inbox: Arc<dyn Inbox>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), ConnectorError> {
    if *shutdown.borrow() {
        return Ok(());
    }
    let url = tokio::select! {
        _ = wait_shutdown(&mut shutdown) => return Ok(()),
        res = api.connection_url() => res.map_err(slack_failed)?,
    };
    let me = tokio::select! {
        _ = wait_shutdown(&mut shutdown) => return Ok(()),
        res = api.identity() => res.map_err(slack_failed)?,
    };
    let mut stream = open_socket(api, &url, &mut shutdown).await?;
    if *shutdown.borrow() {
        let _ = close_bounded(&mut stream).await;
        return Ok(());
    }
    let connected_inbox = Arc::clone(&inbox);
    let _ = tokio::task::spawn_blocking(move || connected_inbox.connected()).await;
    tracing::info!(target: LOG_TARGET, event = "connector_connected", company = %api.company, connector = %api.connector);
    let disconnected = |cause: &str| {
        tracing::info!(target: LOG_TARGET, event = "connector_disconnected", company = %api.company, connector = %api.connector, cause);
    };
    let mut unretained = false;
    loop {
        let frame = tokio::select! {
            _ = wait_shutdown(&mut shutdown) => {
                if let Err(error) = close_bounded(&mut stream).await {
                    tracing::debug!(target: LOG_TARGET, event = "connector_close_unsent", company = %api.company, connector = %api.connector, %error);
                }
                return Ok(());
            }
            next = stream.next() => match next {
                Some(Ok(message)) => message,
                Some(Err(error)) => {
                    disconnected(&format!("Socket Mode connection dropped: {error}"));
                    return Ok(());
                }
                None => return Ok(()),
            }
        };
        let text = match frame {
            Message::Text(text) => text,
            Message::Ping(payload) => {
                match send_cancellable(&mut stream, Message::Pong(payload), &mut shutdown).await {
                    Ok(true) => continue,
                    Ok(false) => {
                        let _ = close_bounded(&mut stream).await;
                        return Ok(());
                    }
                    Err(error) => {
                        disconnected(&format!("Socket Mode pong failed: {error}"));
                        return Ok(());
                    }
                }
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
        // Parse synchronously, then run synchronous SQLite retention on the
        // blocking pool while preserving strict retain-before-ack ordering.
        let retained = if envelope.get("type").and_then(Value::as_str) == Some("events_api") {
            match envelope.pointer("/payload/event") {
                Some(raw_event) => {
                    match serde_json::from_value::<SlackEventCallbackBody>(raw_event.clone())
                        .map_err(|error| error.to_string())
                        .and_then(|event| receive(&api.connector, &me, event))
                    {
                        Ok(Some(incoming)) => {
                            let retain_inbox = Arc::clone(&inbox);
                            match tokio::task::spawn_blocking(move || retain_inbox.retain(incoming))
                                .await
                            {
                                Ok(Ok(())) => Ok(()),
                                Ok(Err(error)) => Err(error.to_string()),
                                Err(error) => Err(format!("retention task failed: {error}")),
                            }
                        }
                        Ok(None) => Ok(()),
                        Err(error) => Err(error),
                    }
                }
                None => Ok(()),
            }
        } else {
            Ok(())
        };
        match retained {
            Ok(()) => {
                let ack_json = json!({ "envelope_id": envelope_id }).to_string();
                match send_cancellable(&mut stream, Message::Text(ack_json.into()), &mut shutdown)
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        let _ = close_bounded(&mut stream).await;
                        return Ok(());
                    }
                    Err(error) => {
                        disconnected(&format!("Socket Mode acknowledgement: {error}"));
                        return Ok(());
                    }
                }
                if unretained {
                    let recovered_inbox = Arc::clone(&inbox);
                    let _ = tokio::task::spawn_blocking(move || recovered_inbox.connected()).await;
                    unretained = false;
                }
            }
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, event = "connector_message_unretained", company = %api.company, connector = %api.connector, %error);
                let unretained_inbox = Arc::clone(&inbox);
                let message =
                    format!("a message could not be retained and awaits Slack's retry: {error}");
                let _ = tokio::task::spawn_blocking(move || unretained_inbox.unretained(&message))
                    .await;
                unretained = true;
            }
        }
    }
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
