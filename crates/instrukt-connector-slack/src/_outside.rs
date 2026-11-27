//! Translate Slack service outcomes for the shared outside gate.
use std::io;

use ::http::HeaderMap;
use instrukt_outside::http::{self, HttpAnswer, Judgment, Rule, TransportFailure};
use instrukt_outside::{Extent, Key, Named, Reason, Settings, Tried, Unavailable};
use jiff::Timestamp;
use slack_morphism::ClientResult;
use slack_morphism::SlackEnvelopeMessage;
use slack_morphism::errors::SlackClientError;
use tokio::time::error::Elapsed;

use crate::{SLACK, SlackError};

const REJECTED_POST: [&str; 9] = [
    "missing_scope",
    "channel_not_found",
    "not_in_channel",
    "is_archived",
    "restricted_action",
    "restricted_action_read_only_channel",
    "restricted_action_thread_locked",
    "msg_too_long",
    "no_text",
];

/// Slack's unavailability and a refused token apply per token.
pub(crate) fn token_key(company: &str, connector: &str, app: bool) -> Key {
    Key::new(SLACK, [company, connector, if app { "app" } else { "bot" }])
}

fn unavailable(reason: Reason, extent: Extent) -> Unavailable {
    Unavailable {
        reason,
        extent,
        named: None,
    }
}

fn slack_rule(answer: &HttpAnswer<'_>) -> Option<Unavailable> {
    let HttpAnswer::Response {
        code: Some(code), ..
    } = answer
    else {
        return None;
    };
    let (reason, extent) = match *code {
        "ratelimited" => (Reason::RateLimited, Extent::Key),
        "service_unavailable" => (Reason::Unavailable { status: None }, Extent::Scope),
        "request_timeout" | "internal_error" | "fatal_error" => {
            (Reason::Unavailable { status: None }, Extent::Call)
        }
        "invalid_auth" | "not_authed" | "account_inactive" | "token_revoked" | "token_expired" => {
            (Reason::Refused, Extent::Scope)
        }
        _ => return None,
    };
    Some(unavailable(reason, extent))
}

fn failure<T>(method: &str, cause: &str, judgment: Judgment) -> Tried<T, SlackError> {
    let api = SlackError::Api {
        method: method.into(),
        cause: cause.into(),
    };
    match judgment {
        Judgment::Unavailable(wait) => Tried::Unavailable(wait, api),
        _ if method == "chat.postMessage" && REJECTED_POST.contains(&cause) => {
            Tried::Defect(SlackError::RejectedPost {
                method: method.into(),
                cause: cause.into(),
            })
        }
        _ => Tried::Defect(api),
    }
}

pub(crate) fn sdk_answer<T>(
    method: &str,
    result: Result<ClientResult<T>, Elapsed>,
    received_at: Timestamp,
) -> Tried<T, SlackError> {
    match result {
        Ok(Ok(value)) => Tried::Done(value),
        Ok(Err(error)) => sdk_failure(method, error, received_at),
        Err(_) => transport(method, "request timed out", TransportFailure::TimedOut),
    }
}

fn sdk_failure<T>(
    method: &str,
    error: SlackClientError,
    received_at: Timestamp,
) -> Tried<T, SlackError> {
    let settings = Settings {
        rules: &[&slack_rule],
        ..Settings::default()
    };
    match error {
        SlackClientError::ApiError(error) => {
            let judgment = http::judge_response(
                200,
                &HeaderMap::new(),
                received_at,
                Some(&error.code),
                &settings,
            );
            failure(method, &error.code, judgment)
        }
        SlackClientError::RateLimitError(error) => {
            let wait = Unavailable {
                named: error
                    .retry_after
                    .map(|delay| Named::After { delay, received_at }),
                ..unavailable(Reason::RateLimited, Extent::Key)
            };
            failure(
                method,
                error.code.as_deref().unwrap_or("HTTP 429"),
                Judgment::Unavailable(wait),
            )
        }
        SlackClientError::HttpError(error) => {
            let status = error.status_code.as_u16();
            let envelope = error.http_response_body.as_deref().and_then(slack_envelope);
            let code = envelope.as_ref().and_then(slack_code);
            let judgment =
                http::judge_response(status, &HeaderMap::new(), received_at, code, &settings);
            failure(method, code.unwrap_or(&format!("HTTP {status}")), judgment)
        }
        SlackClientError::SystemError(error) => {
            let mut cause = error
                .cause
                .as_deref()
                .map(|err| err as &dyn std::error::Error);
            while let Some(error) = cause {
                if let Some(io) = error.downcast_ref::<io::Error>() {
                    if io.kind() == io::ErrorKind::InvalidData {
                        return failure(
                            method,
                            "invalid Slack response or TLS peer",
                            Judgment::Defect,
                        );
                    }
                    return transport(
                        method,
                        "Slack connection failed",
                        TransportFailure::from_io(io),
                    );
                }
                if error.is::<::http::Error>() {
                    return failure(method, "invalid Slack HTTP request", Judgment::Defect);
                }
                cause = error.source();
            }
            transport(
                method,
                "Slack connection failed",
                TransportFailure::Unreachable,
            )
        }
        SlackClientError::EndOfStream(_) => transport(
            method,
            "Slack closed the connection",
            TransportFailure::Unreachable,
        ),
        SlackClientError::ProtocolError(error) => failure(
            method,
            &format!("invalid Slack response: {}", error.json_error),
            Judgment::Defect,
        ),
        SlackClientError::HttpProtocolError(_) | SlackClientError::SocketModeProtocolError(_) => {
            failure(method, "invalid Slack protocol response", Judgment::Defect)
        }
    }
}

fn transport<T>(method: &str, cause: &str, transport: TransportFailure) -> Tried<T, SlackError> {
    failure(
        method,
        cause,
        http::judge(&HttpAnswer::Transport(transport), &Settings::default()),
    )
}

fn scope_finding(required: &[String], reported: Option<&str>) -> Option<String> {
    let Some(reported) = reported else {
        return Some("Slack did not report the bot token's scopes".into());
    };
    let granted: std::collections::HashSet<_> = reported.split(',').map(str::trim).collect();
    let missing: Vec<_> = required
        .iter()
        .map(String::as_str)
        .filter(|scope| !granted.contains(scope))
        .collect();
    (!missing.is_empty()).then(|| format!("Slack bot token missing scopes: {}", missing.join(", ")))
}

fn slack_envelope(text: &str) -> Option<SlackEnvelopeMessage> {
    serde_json::from_str(text).ok()
}

fn slack_code(envelope: &SlackEnvelopeMessage) -> Option<&str> {
    (!envelope.ok).then(|| envelope.error.as_deref().unwrap_or("not ok"))
}

pub(crate) fn admission_answer(
    status: u16,
    headers: &HeaderMap,
    received_at: Timestamp,
    text: &str,
    required: &[String],
) -> Tried<(), SlackError> {
    let envelope = slack_envelope(text);
    let code = envelope.as_ref().and_then(slack_code);
    let accepted = envelope.as_ref().is_some_and(|body| body.ok);
    let finding = accepted
        .then(|| {
            let scopes = headers
                .get("x-oauth-scopes")
                .and_then(|header| header.to_str().ok());
            scope_finding(required, scopes)
        })
        .flatten();
    let scope_rule = |_: &HttpAnswer<'_>| {
        finding
            .as_ref()
            .map(|_| unavailable(Reason::Refused, Extent::Key))
    };
    let rules: [&Rule<'_>; 2] = [&slack_rule, &scope_rule];
    let settings = Settings {
        rules: &rules,
        ..Settings::default()
    };
    let judgment = http::judge_response(status, headers, received_at, code, &settings);
    if judgment == Judgment::Success && (200..300).contains(&status) && accepted {
        Tried::Done(())
    } else {
        let cause = finding
            .or_else(|| code.map(str::to_owned))
            .unwrap_or_else(|| {
                if (200..300).contains(&status) {
                    "no valid Slack admission response".into()
                } else {
                    format!("HTTP {status}")
                }
            });
        failure("auth.test", &cause, judgment)
    }
}

pub(crate) fn admission_transport(
    response: Option<(u16, &HeaderMap)>,
    received_at: Timestamp,
    error: &reqwest::Error,
) -> Tried<(), SlackError> {
    let settings = Settings::default();
    let judgment = response
        .map(|(status, headers)| {
            http::judge_response(status, headers, received_at, None, &settings)
        })
        .filter(|judgment| matches!(judgment, Judgment::Unavailable(_)))
        .unwrap_or_else(|| http::judge_reqwest_error(error, &settings));
    failure("auth.test", &error.to_string(), judgment)
}

/// Judge a WebSocket handshake or pre-hello error for the outside gate without
/// ever logging or exposing the single-use Socket Mode ticket URL.
pub(crate) fn socket_judgment(
    error: &tokio_tungstenite::tungstenite::Error,
    received_at: Timestamp,
) -> Option<Unavailable> {
    use tokio_tungstenite::tungstenite::Error as WsError;
    let settings = Settings::default();
    let judgment = match error {
        WsError::Io(io_err) => http::judge(
            &HttpAnswer::Transport(TransportFailure::from_io(io_err)),
            &settings,
        ),
        WsError::Http(response) => http::judge_response(
            response.status().as_u16(),
            response.headers(),
            received_at,
            None,
            &settings,
        ),
        WsError::ConnectionClosed | WsError::AlreadyClosed => http::judge(
            &HttpAnswer::Transport(TransportFailure::Unreachable),
            &settings,
        ),
        WsError::Tls(_) => Judgment::Defect,
        _ => http::judge(
            &HttpAnswer::Transport(TransportFailure::Unreachable),
            &settings,
        ),
    };
    match judgment {
        Judgment::Unavailable(wait) => Some(wait),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{admission_answer, scope_finding, sdk_answer, token_key};
    use crate::SlackError;
    use http::HeaderMap;
    use instrukt_outside::{Attempted, Extent, Gate, Reason, Report, Settings, Tried};
    use jiff::Timestamp;
    use slack_morphism::errors::{SlackClientApiError, SlackClientError};
    use std::sync::Arc;
    use std::time::Instant;

    fn now() -> Timestamp {
        "2026-11-20T10:00:00Z".parse().unwrap()
    }

    /// A gate whose clocks stand still, so no wait runs out during a test.
    fn gate() -> Gate {
        let base = Instant::now();
        Gate::new(Arc::new(now), Arc::new(move || base), Report::to(|_| {}))
    }

    fn refused(method: &str, code: &str) -> Tried<(), SlackError> {
        sdk_answer(
            method,
            Ok(Err(SlackClientError::ApiError(SlackClientApiError::new(
                code.into(),
            )))),
            now(),
        )
    }

    fn done() -> Tried<(), SlackError> {
        Tried::Done(())
    }

    fn reaches(gate: &Gate, app: bool, method: &str) -> bool {
        let key = token_key("acme", "slack", app).item(method);
        matches!(
            gate.attempt(&key, &Settings::default(), done),
            Attempted::Done(_)
        )
    }

    #[test]
    fn a_ratelimited_body_holds_only_that_method() {
        let gate = gate();
        let key = token_key("acme", "slack", false).item("conversations.history");
        let answer = gate.attempt(&key, &Settings::default(), || {
            refused("conversations.history", "ratelimited")
        });
        assert!(matches!(answer, Attempted::Wait { ref wait, .. } if wait.extent() == Extent::Key));
        assert!(!reaches(&gate, false, "conversations.history"));
        assert!(reaches(&gate, false, "users.info"));
    }

    #[test]
    fn slack_unavailable_holds_every_method_of_that_token_only() {
        let gate = gate();
        let key = token_key("acme", "slack", false).item("users.info");
        gate.attempt(&key, &Settings::default(), || {
            refused("users.info", "service_unavailable")
        });
        let mut requested = false;
        let other = token_key("acme", "slack", false).item("conversations.info");
        let answer = gate.attempt(&other, &Settings::default(), || {
            requested = true;
            done()
        });
        assert!(!requested);
        assert!(matches!(answer, Attempted::Wait { error: None, .. }));
        assert!(reaches(&gate, true, "apps.connections.open"));
    }

    #[test]
    fn a_timed_out_post_does_not_hold_back_another_post_on_the_token() {
        let gate = gate();
        let key = token_key("acme", "slack", false).item("chat.postMessage");
        let lost = gate.attempt(&key, &Settings::default(), || {
            refused("chat.postMessage", "request_timeout")
        });
        assert!(matches!(lost, Attempted::Wait { error: Some(_), .. }));
        let mut requested = false;
        let next = gate.attempt(&key, &Settings::default(), || {
            requested = true;
            done()
        });
        assert!(requested);
        assert!(matches!(next, Attempted::Done(_)));
    }

    #[test]
    fn a_refused_token_is_a_wait_not_a_defect() {
        let Tried::Unavailable(unavailable, _) = refused("auth.test", "invalid_auth") else {
            panic!("a wait");
        };
        assert_eq!(unavailable.reason, Reason::Refused);
        assert_eq!(unavailable.extent, Extent::Scope);
    }

    #[test]
    fn a_missing_scope_at_admission_is_a_wait_naming_the_scope() {
        let required = vec!["channels:read".to_owned(), "chat:write".to_owned()];
        let mut headers = HeaderMap::new();
        headers.insert("x-oauth-scopes", "chat:write".parse().unwrap());
        let answer = admission_answer(200, &headers, now(), r#"{"ok":true}"#, &required);
        let Tried::Unavailable(unavailable, SlackError::Api { cause, .. }) = answer else {
            panic!("a wait");
        };
        assert_eq!(unavailable.extent, Extent::Key);
        assert_eq!(cause, scope_finding(&required, Some("chat:write")).unwrap());
    }
}
