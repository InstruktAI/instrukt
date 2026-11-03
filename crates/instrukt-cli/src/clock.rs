//! The clock of one invocation: the system clock, or the operator's
//! override `INSTRUKT_NOW=<RFC 3339>` (layered-configuration § Environment).
//!
//! The override is read once, here, and every command takes its time from
//! this clock. It is never silent: while it is set, every command says so on
//! stderr. An unparseable value refuses the command rather than falling back
//! to the system clock.

use std::sync::Arc;

use anyhow::Context as _;
use jiff::Timestamp;

/// The environment variable that overrides the clock.
pub const OVERRIDE_VAR: &str = "INSTRUKT_NOW";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    System,
    Fixed(Timestamp),
}

impl Clock {
    /// The clock the process environment selects.
    pub fn from_process() -> anyhow::Result<Self> {
        match std::env::var_os(OVERRIDE_VAR) {
            None => Ok(Clock::System),
            Some(raw) => {
                let text = raw
                    .to_str()
                    .with_context(|| format!("{OVERRIDE_VAR} is not valid UTF-8"))?;
                let at = parse_rfc3339(text).with_context(|| {
                    format!("{OVERRIDE_VAR}={text:?} is not an RFC 3339 time; unset it or give one")
                })?;
                eprintln!("instrukt: clock overridden to {at}");
                Ok(Clock::Fixed(at))
            }
        }
    }

    pub fn now(self) -> Timestamp {
        match self {
            Clock::System => Timestamp::now(),
            Clock::Fixed(at) => at,
        }
    }

    /// This clock as the shared function the work record takes.
    pub fn shared(self) -> Arc<dyn Fn() -> Timestamp + Send + Sync> {
        Arc::new(move || self.now())
    }
}

/// Parses an RFC 3339 `date-time` and nothing broader: `YYYY-MM-DDTHH:MM:SS`,
/// an optional fraction, then `Z` or a `±HH:MM` offset. The general timestamp
/// parser also accepts RFC 9557 annotations (`[…]`), basic formats and
/// omitted seconds, which the override does not announce.
fn parse_rfc3339(text: &str) -> anyhow::Result<Timestamp> {
    anyhow::ensure!(is_rfc3339(text.as_bytes()), "not RFC 3339");
    Ok(text.parse()?)
}

fn is_rfc3339(b: &[u8]) -> bool {
    let digits =
        |r: std::ops::Range<usize>| b.get(r).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    let at = |i: usize, set: &[u8]| b.get(i).is_some_and(|c| set.contains(c));
    let shape = digits(0..4)
        && at(4, b"-")
        && digits(5..7)
        && at(7, b"-")
        && digits(8..10)
        && at(10, b"Tt")
        && digits(11..13)
        && at(13, b":")
        && digits(14..16)
        && at(16, b":")
        && digits(17..19);
    if !shape {
        return false;
    }
    let mut i = 19;
    if at(i, b".") {
        let start = i + 1;
        i = start;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    match b.get(i) {
        Some(b'Z' | b'z') => i + 1 == b.len(),
        Some(b'+' | b'-') => {
            b.len() == i + 6 && digits(i + 1..i + 3) && at(i + 3, b":") && digits(i + 4..i + 6)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_rfc3339_date_times_are_accepted() {
        for good in [
            "2026-11-03T12:00:00Z",
            "2026-11-03t12:00:00z",
            "2026-11-03T12:00:00.250Z",
            "2026-11-03T13:00:00+01:00",
            "2026-11-03T07:00:00-05:00",
        ] {
            assert!(parse_rfc3339(good).is_ok(), "{good:?} should be accepted");
        }
        for bad in [
            "2026-11-03T12:00:00Z[foo=bar]",
            "2026-11-03T12:00:00+01:00[Europe/Amsterdam]",
            "2026-11-03T12:00Z",
            "20261103T120000Z",
            "2026-11-03 12:00:00Z",
            "2026-11-03T12:00:00.Z",
            "2026-11-03T12:00:00+0100",
            "2026-11-03T12:00:00",
            "2026-13-03T12:00:00Z",
            "",
        ] {
            assert!(parse_rfc3339(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn equal_instants_parse_equal_across_offsets() {
        assert_eq!(
            parse_rfc3339("2026-11-03T13:00:00+01:00").unwrap(),
            parse_rfc3339("2026-11-03T12:00:00Z").unwrap()
        );
    }
}
