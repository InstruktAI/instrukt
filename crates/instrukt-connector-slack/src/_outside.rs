//! How Slack's answers reach the outside gate: the keys its calls reach,
//! Slack's own service rules, and the judgment of a Web API answer and of a
//! Socket Mode opening. Nothing here makes a request.
use instrukt_outside::http::{self, HttpAnswer, Judgment, Rule};
use instrukt_outside::{Extent, Key, Reason, Settings, Tried, Unavailable};
use jiff::Timestamp;
use serde_json::Value;
use tungstenite::http::HeaderMap;

use crate::{SLACK, SlackError};

/// What one Web API call answers when it succeeded: the body and the
/// `x-oauth-scopes` Slack reported.
pub(crate) type Answered = (Value, Option<String>);

/// The posting errors that confirm `chat.postMessage` rejected the post.
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

/// A finding admission makes of the scopes Slack reported, or of their
/// absence; `None` when the token holds every required scope.
pub(crate) type Finding<'a> = &'a (dyn Fn(Option<&str>) -> Option<String> + Sync);

/// The scope of one token of one company's named connector: Slack's
/// unavailability and a refused token apply per token.
pub(crate) fn token_key(company: &str, connector: &str, app: bool) -> Key {
    Key::new(SLACK, [company, connector, if app { "app" } else { "bot" }])
}

/// Slack's service rules over a Web API answer's error code.
pub(crate) fn slack_rule(answer: &HttpAnswer<'_>) -> Option<Unavailable> {
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
    Some(Unavailable {
        reason,
        extent,
        named: None,
    })
}

/// The scopes of `required` that `reported`, Slack's `x-oauth-scopes`, does
/// not hold; every one when Slack reported none.
pub(crate) fn missing_scopes<'a>(required: &'a [String], reported: Option<&str>) -> Vec<&'a str> {
    let granted: std::collections::HashSet<&str> = reported
        .into_iter()
        .flat_map(|scopes| scopes.split(','))
        .map(str::trim)
        .collect();
    required
        .iter()
        .map(String::as_str)
        .filter(|scope| !granted.contains(scope))
        .collect()
}

/// Admission's finding: a scope list Slack cannot provide, or the missing
/// scopes it names.
pub(crate) fn scope_finding(required: &[String], reported: Option<&str>) -> Option<String> {
    if reported.is_none() {
        return Some("Slack did not report the bot token's scopes".into());
    }
    let missing = missing_scopes(required, reported);
    (!missing.is_empty()).then(|| format!("Slack bot token missing scopes: {}", missing.join(", ")))
}

/// The answer to `method` read in full: `text` is its body. A rule of
/// Slack's, or admission's `finding`, may judge it temporary before HTTP's
/// table does.
pub(crate) fn answer_view(
    method: &str,
    status: u16,
    headers: &HeaderMap,
    received_at: Timestamp,
    text: &str,
    finding: Option<Finding<'_>>,
) -> Tried<Answered, SlackError> {
    let body: Option<Value> = serde_json::from_str(text).ok();
    let ok = body
        .as_ref()
        .and_then(|body| body.get("ok"))
        .and_then(Value::as_bool);
    let code: Option<String> = (ok == Some(false)).then(|| {
        body.as_ref()
            .and_then(|body| body.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("not ok")
            .to_owned()
    });
    let scopes = headers
        .get("x-oauth-scopes")
        .and_then(|header| header.to_str().ok());
    let found = (ok == Some(true))
        .then(|| finding.and_then(|finding| finding(scopes)))
        .flatten();
    let scope_rule = |_: &HttpAnswer<'_>| {
        found.as_ref().map(|_| Unavailable {
            reason: Reason::Refused,
            extent: Extent::Key,
            named: None,
        })
    };
    let rules: [&Rule<'_>; 2] = [&slack_rule, &scope_rule];
    let settings = Settings {
        rules: &rules,
        ..Settings::default()
    };
    let answer = HttpAnswer::Response {
        status,
        headers,
        received_at,
        code: code.as_deref(),
    };
    let api = |cause: String| SlackError::Api {
        method: method.into(),
        cause,
    };
    let cause = || {
        found
            .clone()
            .or_else(|| code.clone())
            .unwrap_or_else(|| format!("HTTP {status}"))
    };
    match http::judge(&answer, &settings) {
        Judgment::Unavailable(unavailable) => Tried::Unavailable(unavailable, api(cause())),
        Judgment::Defect => Tried::Defect(api(cause())),
        Judgment::Success => match (body, code.as_deref()) {
            (None, _) => Tried::Defect(api(format!("HTTP {status} without a JSON answer"))),
            (Some(body), None) if ok == Some(true) => {
                Tried::Done((body, scopes.map(str::to_owned)))
            }
            (Some(_), code) => {
                let code = code.unwrap_or("not ok");
                if method == "chat.postMessage" && REJECTED_POST.contains(&code) {
                    Tried::Defect(SlackError::RejectedPost {
                        method: method.into(),
                        cause: code.into(),
                    })
                } else {
                    Tried::Defect(api(code.into()))
                }
            }
        },
    }
}

/// A Web API call the client could not complete: the answer's status when
/// one arrived and its body could not be read, then the transport failure.
pub(crate) fn transport_view(
    method: &str,
    status: Option<(u16, &HeaderMap)>,
    received_at: Timestamp,
    error: &ureq::Error,
) -> Tried<Answered, SlackError> {
    let api = SlackError::Api {
        method: method.into(),
        cause: error.to_string(),
    };
    if let Some((status, headers)) = status {
        let answer = HttpAnswer::Response {
            status,
            headers,
            received_at,
            code: None,
        };
        if let Judgment::Unavailable(unavailable) = http::judge(&answer, &Settings::default()) {
            return Tried::Unavailable(unavailable, api);
        }
    }
    let failure = match error {
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => {
            http::TransportFailure::Unreachable
        }
        ureq::Error::Timeout(_) | ureq::Error::BodyStalled => http::TransportFailure::TimedOut,
        ureq::Error::Io(io) => http::TransportFailure::from_io(io),
        _ => http::TransportFailure::Other,
    };
    match http::judge(&HttpAnswer::Transport(failure), &Settings::default()) {
        Judgment::Unavailable(unavailable) => Tried::Unavailable(unavailable, api),
        _ => Tried::Defect(api),
    }
}

/// A Socket Mode opening that failed before Slack's `hello`: no connection,
/// a closed one, or a handshake Slack refused, judged by HTTP's table. Any
/// other failure (an address, TLS or protocol error) is a defect.
pub(crate) fn socket_judgment(
    error: &tungstenite::Error,
    received_at: Timestamp,
) -> Option<Unavailable> {
    let unavailable = |reason| Unavailable {
        reason,
        extent: Extent::Scope,
        named: None,
    };
    match error {
        tungstenite::Error::Io(error) => Some(unavailable(match error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Reason::TimedOut,
            _ => Reason::Unreachable,
        })),
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
            Some(unavailable(Reason::TimedOut))
        }
        tungstenite::Error::Http(response) => {
            let answer = HttpAnswer::Response {
                status: response.status().as_u16(),
                headers: response.headers(),
                received_at,
                code: None,
            };
            match http::judge(&answer, &Settings::default()) {
                Judgment::Unavailable(unavailable) => Some(unavailable),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{answer_view, scope_finding, socket_judgment, token_key};
    use crate::SlackError;
    use instrukt_outside::{Attempted, Extent, Gate, Reason, Report, Settings, Tried};
    use jiff::Timestamp;
    use std::sync::Arc;
    use std::time::Instant;
    use tungstenite::http::HeaderMap;

    fn now() -> Timestamp {
        "2026-11-20T10:00:00Z".parse().unwrap()
    }

    /// A gate whose clocks stand still, so no wait runs out during a test.
    fn gate() -> Gate {
        let base = Instant::now();
        Gate::new(Arc::new(now), Arc::new(move || base), Report::to(|_| {}))
    }

    fn refused(method: &str, code: &str) -> Tried<super::Answered, SlackError> {
        answer_view(
            method,
            200,
            &HeaderMap::new(),
            now(),
            &format!(r#"{{"ok":false,"error":"{code}"}}"#),
            None,
        )
    }

    fn done() -> Tried<super::Answered, SlackError> {
        Tried::Done((serde_json::json!({"ok": true}), None))
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
        let finding = |reported: Option<&str>| scope_finding(&required, reported);
        let mut headers = HeaderMap::new();
        headers.insert("x-oauth-scopes", "chat:write".parse().unwrap());
        let answer = answer_view(
            "auth.test",
            200,
            &headers,
            now(),
            r#"{"ok":true}"#,
            Some(&finding),
        );
        let Tried::Unavailable(unavailable, SlackError::Api { cause, .. }) = answer else {
            panic!("a wait");
        };
        assert_eq!(unavailable.extent, Extent::Key);
        assert_eq!(cause, scope_finding(&required, Some("chat:write")).unwrap());
        assert_eq!(
            super::missing_scopes(&required, Some("chat:write")),
            ["channels:read"]
        );
    }

    #[test]
    fn a_socket_that_cannot_connect_or_is_refused_503_is_unavailable() {
        let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        let judged = socket_judgment(&tungstenite::Error::Io(refused), now()).unwrap();
        assert_eq!(judged.reason, Reason::Unreachable);
        let response = tungstenite::http::Response::builder()
            .status(503)
            .body(None)
            .unwrap();
        let judged = socket_judgment(&tungstenite::Error::Http(Box::new(response)), now()).unwrap();
        assert_eq!(judged.reason, Reason::Unavailable { status: Some(503) });
        assert_eq!(judged.extent, Extent::Scope);
        let url = tungstenite::Error::Url(tungstenite::error::UrlError::NoHostName);
        assert_eq!(socket_judgment(&url, now()), None);
    }
}
