//! The HTTP adapter: what an answer or a transport failure means, after the
//! part's service rules, and the moment `Retry-After` names.
use std::time::Duration;

use http::HeaderMap;
use http::header::RETRY_AFTER;
use jiff::Timestamp;

use crate::_gate::Settings;
use crate::_judgment::{Extent, Named, Reason, Unavailable};

/// Configuration for the workspace-shared `reqwest::Client` builder.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub user_agent: String,
    pub follow_redirects: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            user_agent: "instrukt".to_owned(),
            follow_redirects: true,
        }
    }
}

/// Installs the workspace's `rustls` `ring` crypto provider once so mixed
/// transitive crate features (`ring` + `aws-lc-rs`) never panic when a TLS
/// client initializes.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Returns a credential-safe `scheme://host[:port]/path` display form of `url`
/// so userinfo, query parameters, and fragments never leak into diagnostics.
pub fn redact_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => {
            let mut safe = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
            if let Some(port) = parsed.port() {
                safe.push_str(&format!(":{port}"));
            }
            safe.push_str(parsed.path());
            safe
        }
        Err(_) => "<invalid-url>".to_owned(),
    }
}

/// Failure while streaming a bounded HTTP response body.
#[derive(Debug, thiserror::Error)]
pub enum BodyReadError {
    /// Transport error while streaming chunks (URL already scrubbed).
    #[error("unreadable HTTP body: {0}")]
    Transport(reqwest::Error),
    /// Response body exceeded `max_bytes` before completing.
    #[error("HTTP response body exceeds {max_bytes} bytes")]
    TooLarge { max_bytes: usize },
}

/// Reads a `reqwest::Response` body up to `max_bytes`, failing before
/// buffering unbounded payloads.
pub async fn read_bounded_body(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, BodyReadError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| BodyReadError::Transport(error.without_url()))?
    {
        if chunk.len() > max_bytes.saturating_sub(body.len()) {
            return Err(BodyReadError::TooLarge { max_bytes });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Builds a `reqwest::Client` with the workspace's shared timeouts, User-Agent,
/// TLS (`rustls`), and redirect policy.
pub fn build_client(config: &ClientConfig) -> Result<reqwest::Client, reqwest::Error> {
    ensure_crypto_provider();
    let mut builder = reqwest::Client::builder()
        .connect_timeout(config.connect_timeout)
        .timeout(config.request_timeout)
        .user_agent(config.user_agent.as_str());
    if !config.follow_redirects {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    builder.build()
}

/// Classifies a `reqwest::Error` directly through the outside gate's HTTP rules.
pub fn judge_reqwest_error(error: &reqwest::Error, settings: &Settings<'_>) -> Judgment {
    judge(
        &HttpAnswer::Transport(TransportFailure::from_reqwest(error)),
        settings,
    )
}

/// Classifies an HTTP status and headers directly through the outside gate's HTTP rules.
pub fn judge_response(
    status: u16,
    headers: &HeaderMap,
    received_at: Timestamp,
    code: Option<&str>,
    settings: &Settings<'_>,
) -> Judgment {
    judge(
        &HttpAnswer::Response {
            status,
            headers,
            received_at,
            code,
        },
        settings,
    )
}

/// A client-agnostic HTTP transport failure brought to the outside gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportFailure {
    /// Name not resolved, connection refused/reset, or no connection opened.
    Unreachable,
    /// Timed out connecting, waiting for headers, or reading the body.
    TimedOut,
    /// Any other client error (bad URI, TLS/config error, body decode): defect.
    Other,
}

impl TransportFailure {
    /// Classifies a `reqwest` client error for the outside gate.
    pub fn from_reqwest(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::TimedOut
        } else if error.is_connect() || error.is_request() || error.is_body() {
            Self::Unreachable
        } else {
            Self::Other
        }
    }

    /// Classifies a `std::io::Error` from a transport socket or stream.
    pub fn from_io(error: &std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Self::TimedOut,
            _ => Self::Unreachable,
        }
    }
}

/// One HTTP outcome as the part's client gave it.
#[derive(Debug, Clone, Copy)]
pub enum HttpAnswer<'a> {
    /// No answer: the client failed before or while reading it.
    Transport(TransportFailure),
    /// An answer. `code` is the service's own error code, which the part read
    /// from the body; the gate reads no body.
    Response {
        status: u16,
        headers: &'a HeaderMap,
        /// The host's clock when the answer was received.
        received_at: Timestamp,
        code: Option<&'a str>,
    },
}

/// A part's service rule: whether an answer is temporarily unavailable beyond
/// HTTP's own rules, its reason and its extent; `None` leaves the answer to
/// the next rule and then to the table.
pub type Rule<'a> = dyn Fn(&HttpAnswer<'_>) -> Option<Unavailable> + Send + Sync + 'a;

/// What an HTTP outcome is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judgment {
    Success,
    Unavailable(Unavailable),
    Defect,
}

/// The judgment of `answer`: the part's service rules first, then HTTP's
/// status semantics. `Retry-After` sets the moment of every temporary answer
/// unless a rule named its own.
pub fn judge(answer: &HttpAnswer<'_>, settings: &Settings<'_>) -> Judgment {
    let named = || match answer {
        HttpAnswer::Response {
            headers,
            received_at,
            ..
        } => retry_after(headers, *received_at),
        HttpAnswer::Transport(_) => None,
    };
    if let Some(mut unavailable) = settings.rules.iter().find_map(|rule| rule(answer)) {
        unavailable.named = unavailable.named.or_else(named);
        return Judgment::Unavailable(unavailable);
    }
    let unavailable = |reason, extent| {
        Judgment::Unavailable(Unavailable {
            reason,
            extent,
            named: named(),
        })
    };
    match answer {
        HttpAnswer::Transport(failure) => match transport(*failure) {
            Some(reason) => unavailable(reason, Extent::Scope),
            None => Judgment::Defect,
        },
        HttpAnswer::Response { status, .. } => match status {
            ..400 => Judgment::Success,
            429 => unavailable(Reason::RateLimited, Extent::Key),
            408 | 425 | 500 => unavailable(
                Reason::Unavailable {
                    status: Some(*status),
                },
                Extent::Call,
            ),
            500..600 => unavailable(
                Reason::Unavailable {
                    status: Some(*status),
                },
                Extent::Scope,
            ),
            _ => Judgment::Defect,
        },
    }
}

/// A transport failure that is temporary: no connection, or none that
/// answered in time.
fn transport(failure: TransportFailure) -> Option<Reason> {
    match failure {
        TransportFailure::Unreachable => Some(Reason::Unreachable),
        TransportFailure::TimedOut => Some(Reason::TimedOut),
        TransportFailure::Other => None,
    }
}

/// The moment `Retry-After` names: whole seconds counted from `received_at`,
/// or an HTTP-date in any of the three formats a recipient accepts. An
/// unreadable value is absent.
pub fn retry_after(headers: &HeaderMap, received_at: Timestamp) -> Option<Named> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        let seconds = value.parse::<u64>().unwrap_or(u64::MAX);
        return Some(Named::After {
            delay: Duration::from_secs(seconds),
            received_at,
        });
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Timestamp::try_from(date).ok().map(Named::At)
}

#[cfg(test)]
mod tests {
    use super::{HttpAnswer, Judgment, TransportFailure, judge, retry_after};
    use crate::_gate::Settings;
    use crate::_judgment::{Extent, Named, Reason, Unavailable};
    use http::HeaderMap;
    use jiff::Timestamp;
    use std::time::Duration;

    fn now() -> Timestamp {
        "2026-11-20T10:00:00Z".parse().unwrap()
    }

    fn headers(retry: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(retry) = retry {
            headers.insert("retry-after", retry.parse().unwrap());
        }
        headers
    }

    fn status(status: u16, headers: &HeaderMap) -> Judgment {
        judge(
            &HttpAnswer::Response {
                status,
                headers,
                received_at: now(),
                code: None,
            },
            &Settings::default(),
        )
    }

    fn unavailable(reason: Reason, extent: Extent) -> Judgment {
        Judgment::Unavailable(Unavailable {
            reason,
            extent,
            named: None,
        })
    }

    #[test]
    fn an_answer_below_400_is_a_success() {
        assert_eq!(status(200, &headers(None)), Judgment::Success);
    }

    #[test]
    fn a_429_rate_limits_the_key() {
        assert_eq!(
            status(429, &headers(None)),
            unavailable(Reason::RateLimited, Extent::Key)
        );
    }

    #[test]
    fn a_503_makes_the_scope_unavailable() {
        assert_eq!(
            status(503, &headers(None)),
            unavailable(Reason::Unavailable { status: Some(503) }, Extent::Scope)
        );
    }

    #[test]
    fn a_failed_request_holds_only_the_call() {
        assert_eq!(
            status(408, &headers(None)),
            unavailable(Reason::Unavailable { status: Some(408) }, Extent::Call)
        );
    }

    #[test]
    fn another_5xx_makes_the_scope_unavailable() {
        assert_eq!(
            status(502, &headers(None)),
            unavailable(Reason::Unavailable { status: Some(502) }, Extent::Scope)
        );
    }

    #[test]
    fn another_4xx_is_a_defect() {
        assert_eq!(status(404, &headers(None)), Judgment::Defect);
    }

    fn transport(failure: TransportFailure) -> Judgment {
        judge(&HttpAnswer::Transport(failure), &Settings::default())
    }

    #[test]
    fn no_connection_makes_the_scope_unreachable() {
        assert_eq!(
            transport(TransportFailure::Unreachable),
            unavailable(Reason::Unreachable, Extent::Scope)
        );
    }

    #[test]
    fn a_transport_timeout_times_the_scope_out() {
        assert_eq!(
            transport(TransportFailure::from_io(
                &std::io::ErrorKind::TimedOut.into()
            )),
            unavailable(Reason::TimedOut, Extent::Scope)
        );
    }

    #[test]
    fn another_transport_failure_is_a_defect() {
        assert_eq!(transport(TransportFailure::Other), Judgment::Defect);
    }

    #[test]
    fn delay_seconds_count_from_receipt_rounded_up() {
        let named = retry_after(&headers(Some("120")), now()).unwrap();
        assert_eq!(named.moment(), now() + jiff::SignedDuration::from_secs(120));
        let fractional = Named::After {
            delay: Duration::from_millis(1_500),
            received_at: now(),
        };
        assert_eq!(
            fractional.moment(),
            now() + jiff::SignedDuration::from_secs(2)
        );
    }

    #[test]
    fn each_http_date_format_is_read() {
        let at: Timestamp = "1994-11-06T08:49:37Z".parse().unwrap();
        for date in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(
                retry_after(&headers(Some(date)), now()),
                Some(Named::At(at)),
                "{date}"
            );
        }
    }

    #[test]
    fn an_unreadable_retry_after_is_absent() {
        assert_eq!(retry_after(&headers(Some("soon")), now()), None);
        assert_eq!(retry_after(&headers(Some("-5")), now()), None);
        assert_eq!(retry_after(&headers(None), now()), None);
    }

    #[test]
    fn a_huge_retry_after_saturates() {
        let named = retry_after(&headers(Some("99999999999999999999999")), now()).unwrap();
        assert_eq!(named.moment(), Timestamp::MAX);
    }

    #[test]
    fn a_service_rule_comes_before_the_table_and_keeps_retry_after() {
        let limited = |answer: &HttpAnswer<'_>| match answer {
            HttpAnswer::Response {
                code: Some("ratelimited"),
                ..
            } => Some(Unavailable {
                reason: Reason::RateLimited,
                extent: Extent::Key,
                named: None,
            }),
            _ => None,
        };
        let refused = |answer: &HttpAnswer<'_>| match answer {
            HttpAnswer::Response {
                code: Some("invalid_auth"),
                ..
            } => Some(Unavailable {
                reason: Reason::Refused,
                extent: Extent::Scope,
                named: None,
            }),
            _ => None,
        };
        let settings = Settings {
            rules: &[&limited, &refused],
            ..Settings::default()
        };
        let none = headers(None);
        // A 200 the table counts as success is temporary by the rule.
        let answer = |code, headers| HttpAnswer::Response {
            status: 200,
            headers,
            received_at: now(),
            code: Some(code),
        };
        assert_eq!(
            judge(&answer("invalid_auth", &none), &settings),
            unavailable(Reason::Refused, Extent::Scope)
        );
        let retry = headers(Some("30"));
        let limited_429 = HttpAnswer::Response {
            status: 429,
            headers: &retry,
            received_at: now(),
            code: Some("ratelimited"),
        };
        assert_eq!(
            judge(&limited_429, &settings),
            Judgment::Unavailable(Unavailable {
                reason: Reason::RateLimited,
                extent: Extent::Key,
                named: Some(Named::After {
                    delay: Duration::from_secs(30),
                    received_at: now(),
                }),
            })
        );
    }

    #[test]
    fn redact_url_strips_userinfo_query_and_fragment_and_rejects_invalid_urls() {
        assert_eq!(
            super::redact_url(
                "https://user:secret@auth.example.com:8443/oauth/token?api_key=leak#frag"
            ),
            "https://auth.example.com:8443/oauth/token"
        );
        assert_eq!(
            super::redact_url("http://127.0.0.1:9000/v1/chat/completions?token=secret"),
            "http://127.0.0.1:9000/v1/chat/completions"
        );
        assert_eq!(super::redact_url("not a url with secret"), "<invalid-url>");
    }

    #[test]
    fn read_bounded_body_accepts_exact_limit_and_rejects_overflow() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        let handle = std::thread::spawn(move || {
            for size in [16usize, 17usize] {
                let req = server.recv().unwrap();
                let body = vec![b'a'; size];
                req.respond(tiny_http::Response::from_data(body)).unwrap();
            }
        });
        let client = super::build_client(&super::ClientConfig::default()).unwrap();
        let url = format!("http://{addr}/test");
        let (exact, overflow) = crate::runtime::block_on(async {
            let resp_ok = client.get(&url).send().await.unwrap();
            let exact = super::read_bounded_body(resp_ok, 16).await;
            let resp_big = client.get(&url).send().await.unwrap();
            let overflow = super::read_bounded_body(resp_big, 16).await;
            (exact, overflow)
        })
        .unwrap();
        handle.join().unwrap();
        assert_eq!(exact.unwrap(), vec![b'a'; 16]);
        assert!(matches!(
            overflow,
            Err(super::BodyReadError::TooLarge { max_bytes: 16 })
        ));
    }
}
