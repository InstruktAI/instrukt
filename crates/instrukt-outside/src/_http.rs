//! The HTTP adapter: what an answer or a transport failure means, after the
//! part's service rules, and the moment `Retry-After` names.
use std::time::Duration;

use jiff::Timestamp;
use ureq::http::HeaderMap;
use ureq::http::header::RETRY_AFTER;

use crate::_gate::Settings;
use crate::_judgment::{Extent, Named, Reason, Unavailable};

/// One HTTP outcome as the part's client gave it.
#[derive(Debug, Clone, Copy)]
pub enum HttpAnswer<'a> {
    /// No answer: the client failed before or while reading it.
    Transport(&'a ureq::Error),
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
        HttpAnswer::Transport(error) => match transport(error) {
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
fn transport(error: &ureq::Error) -> Option<Reason> {
    match error {
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => Some(Reason::Unreachable),
        ureq::Error::Timeout(_) | ureq::Error::BodyStalled => Some(Reason::TimedOut),
        ureq::Error::Io(error) => Some(match error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Reason::TimedOut,
            _ => Reason::Unreachable,
        }),
        _ => None,
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
    use super::{HttpAnswer, Judgment, judge, retry_after};
    use crate::_gate::Settings;
    use crate::_judgment::{Extent, Named, Reason, Unavailable};
    use jiff::Timestamp;
    use std::time::Duration;
    use ureq::http::HeaderMap;

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
    fn each_status_row_has_its_judgment_and_extent() {
        let none = headers(None);
        let cases = [
            (200, Judgment::Success),
            (302, Judgment::Success),
            (429, unavailable(Reason::RateLimited, Extent::Key)),
            (
                503,
                unavailable(Reason::Unavailable { status: Some(503) }, Extent::Scope),
            ),
            (
                408,
                unavailable(Reason::Unavailable { status: Some(408) }, Extent::Call),
            ),
            (
                425,
                unavailable(Reason::Unavailable { status: Some(425) }, Extent::Call),
            ),
            (
                502,
                unavailable(Reason::Unavailable { status: Some(502) }, Extent::Scope),
            ),
            (
                504,
                unavailable(Reason::Unavailable { status: Some(504) }, Extent::Scope),
            ),
            (
                500,
                unavailable(Reason::Unavailable { status: Some(500) }, Extent::Call),
            ),
            (400, Judgment::Defect),
            (401, Judgment::Defect),
            (404, Judgment::Defect),
        ];
        for (code, expected) in cases {
            assert_eq!(status(code, &none), expected, "{code}");
        }
        let unreachable = ureq::Error::ConnectionFailed;
        let timed_out = ureq::Error::Io(std::io::ErrorKind::TimedOut.into());
        let defect = ureq::Error::BadUri("nonsense".into());
        let transport = |error| judge(&HttpAnswer::Transport(error), &Settings::default());
        assert_eq!(
            transport(&unreachable),
            unavailable(Reason::Unreachable, Extent::Scope)
        );
        assert_eq!(
            transport(&timed_out),
            unavailable(Reason::TimedOut, Extent::Scope)
        );
        assert_eq!(transport(&defect), Judgment::Defect);
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
}
