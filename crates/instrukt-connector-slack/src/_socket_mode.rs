//! Bridge the SDK's Socket Mode callbacks to the synchronous connector inbox.
use std::io;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use http::StatusCode;
use instrukt_connector::{ConnectorError, Inbox, Incoming};
use instrukt_outside::runtime::block_on;
use serde_json::Value;
use slack_morphism::errors::SlackClientError;
use slack_morphism::listener::SlackClientEventsListenerEnvironment;
use slack_morphism::prelude::{SlackEventCallbackBody, SlackPushEventCallback};
use slack_morphism::{
    SlackClientSocketModeConfig, SlackClientSocketModeListener, SlackSocketModeListenerCallbacks,
};
use tokio::sync::oneshot;

use crate::{LOG_TARGET, Me, SlackApi, failed, newly_written, slack_failed, ts_time};

const QUIT_INTERVAL: Duration = Duration::from_millis(200);

enum InboxEvent {
    Connected,
    Message(
        Result<Incoming, String>,
        oneshot::Sender<Result<(), String>>,
    ),
}

pub(crate) fn session(
    api: &SlackApi,
    inbox: &dyn Inbox,
    quit: &dyn Fn() -> bool,
) -> Result<(), ConnectorError> {
    if quit() {
        return Ok(());
    }
    let me = Arc::new(api.identity().map_err(slack_failed)?);
    let connector = Arc::new(api.connector.clone());
    let (send, events) = mpsc::channel();
    let connected = send.clone();

    let mut callbacks = SlackSocketModeListenerCallbacks::new();
    callbacks.hello_callback = Box::new(move |_, _, _| {
        let connected = connected.clone();
        async move {
            if connected.send(InboxEvent::Connected).is_err() {
                tracing::debug!(target: LOG_TARGET, "Slack listener is shutting down");
            }
        }
    });
    callbacks.push_events_callback = Box::new(move |event: SlackPushEventCallback, _, _| {
        let send = send.clone();
        let me = Arc::clone(&me);
        let connector = Arc::clone(&connector);
        async move {
            let message = match receive(&connector, &me, event.event) {
                Ok(None) => return Ok(()),
                Ok(Some(incoming)) => Ok(incoming),
                Err(error) => Err(error),
            };
            let (reply, retained) = oneshot::channel();
            send.send(InboxEvent::Message(message, reply))
                .map_err(|_| io::Error::other("Slack inbox closed"))?;
            retained
                .await?
                .map_err(|error| io::Error::other(error).into())
        }
    });
    let environment = SlackClientEventsListenerEnvironment::new(Arc::clone(&api.client))
        .with_error_handler(|error, _, _| {
            if let Some(error) = error.downcast_ref::<SlackClientError>() {
                // SDK errors can contain the raw Slack frame or a connection ticket.
                tracing::warn!(target: LOG_TARGET, kind = ?std::mem::discriminant(error), "Slack Socket Mode reported an error");
            }
            StatusCode::INTERNAL_SERVER_ERROR
        });
    let config = SlackClientSocketModeConfig::new()
        .with_max_connections_count(1)
        .with_initial_backoff_in_seconds(0);
    let listener = SlackClientSocketModeListener::new(&config, Arc::new(environment), callbacks);
    block_on(async {
        listener.listen_for(&api.app_token).await?;
        listener.start().await;
        Ok::<_, SlackClientError>(())
    })
    .map_err(failed)?
    .map_err(|_| failed("could not register the Slack Socket Mode listener"))?;

    let mut unretained = false;
    while !quit() {
        match events.recv_timeout(QUIT_INTERVAL) {
            Ok(InboxEvent::Connected) => {
                inbox.connected();
                unretained = false;
                tracing::info!(target: LOG_TARGET, event = "connector_connected", company = %api.company, connector = %api.connector);
            }
            Ok(InboxEvent::Message(message, reply)) => {
                let result = message
                    .and_then(|message| inbox.retain(message).map_err(|error| error.to_string()));
                match &result {
                    Ok(()) if unretained => {
                        inbox.connected();
                        unretained = false;
                    }
                    Err(error) => {
                        tracing::warn!(target: LOG_TARGET, event = "connector_message_unretained", company = %api.company, connector = %api.connector, %error);
                        inbox.unretained(&format!(
                            "a message could not be retained and awaits Slack's retry: {error}"
                        ));
                        unretained = true;
                    }
                    Ok(()) => {}
                }
                if reply.send(result).is_err() {
                    tracing::debug!(target: LOG_TARGET, "Slack delivery ended while retaining its message");
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(events);
    block_on(listener.shutdown()).map_err(failed)
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
