//! The clock of one invocation: the system clock, or the operator's
//! override `INSTRUKT_NOW=<RFC 3339>`, or `INSTRUKT_NOW=@<path>` for the
//! time in a file (layered-configuration § Environment).
//!
//! The override is read here, and every command takes its time from this
//! clock. It is never silent: while it is set, every command says so on
//! stderr. An unparseable value refuses the command rather than falling back
//! to the system clock. A file is read again on each request for the time,
//! and the limits on the programs instrukt starts are measured on it. The
//! first reading must succeed. After that, a failed reading advances the
//! last valid value by elapsed monotonic time.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Context as _;
use jiff::Timestamp;

/// The environment variable that overrides the clock.
pub const OVERRIDE_VAR: &str = "INSTRUKT_NOW";

#[derive(Debug, Clone)]
pub enum Clock {
    System,
    Fixed(Timestamp),
    /// The time in this file, read on each request.
    File(Arc<FileClock>),
}

#[derive(Debug)]
pub struct FileClock {
    path: PathBuf,
    last: Mutex<LastReading>,
}

#[derive(Debug)]
struct LastReading {
    time: Timestamp,
    read_at: Instant,
    warned: bool,
}

impl FileClock {
    fn open(path: &Path) -> anyhow::Result<Self> {
        let time = read_file(path)?;
        Ok(Self {
            path: path.into(),
            last: Mutex::new(LastReading {
                time,
                read_at: Instant::now(),
                warned: false,
            }),
        })
    }

    fn now(&self) -> Timestamp {
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match read_file(&self.path) {
            Ok(time) => {
                last.time = time;
                last.read_at = Instant::now();
                time
            }
            Err(error) => {
                if !last.warned {
                    last.warned = true;
                    eprintln!(
                        "instrukt: {}; continuing from the last clock reading",
                        crate::output::error_line(&error)
                    );
                }
                last.time
                    .saturating_add(last.read_at.elapsed())
                    .unwrap_or(Timestamp::MAX)
            }
        }
    }
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
                if let Some(path) = text.strip_prefix('@') {
                    let clock = Arc::new(FileClock::open(Path::new(path))?);
                    eprintln!(
                        "instrukt: clock overridden to the time in {}",
                        clock.path.display()
                    );
                    let limits = Arc::clone(&clock);
                    instrukt_core::process::measure_limits_on(Box::new(move || Ok(limits.now())));
                    return Ok(Clock::File(clock));
                }
                let at = parse_rfc3339(text).with_context(|| {
                    format!("{OVERRIDE_VAR}=`{text}` is not an RFC 3339 time; unset it or give one")
                })?;
                eprintln!("instrukt: clock overridden to {at}");
                Ok(Clock::Fixed(at))
            }
        }
    }

    /// The time now, including elapsed time since the last valid file reading
    /// if the file cannot be read.
    pub fn now(&self) -> Timestamp {
        match self {
            Clock::System => Timestamp::now(),
            Clock::Fixed(at) => *at,
            Clock::File(clock) => clock.now(),
        }
    }

    /// This clock as the shared function the work record takes.
    pub fn shared(&self) -> Arc<dyn Fn() -> Timestamp + Send + Sync> {
        let clock = self.clone();
        Arc::new(move || clock.now())
    }
}

/// The RFC 3339 time `path` holds, with or without a final newline.
fn read_file(path: &Path) -> anyhow::Result<Timestamp> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("{OVERRIDE_VAR}=@{}: cannot read the time", path.display()))?;
    parse_rfc3339(text.strip_suffix('\n').unwrap_or(&text)).with_context(|| {
        format!(
            "{OVERRIDE_VAR}=@{} holds no RFC 3339 time; unset it or give one",
            path.display()
        )
    })
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
    fn a_utc_date_time_is_accepted() {
        assert!(parse_rfc3339("2026-11-03T12:00:00Z").is_ok());
    }

    #[test]
    fn lowercase_designators_are_accepted() {
        assert!(parse_rfc3339("2026-11-03t12:00:00z").is_ok());
    }

    #[test]
    fn a_fractional_second_is_accepted() {
        assert!(parse_rfc3339("2026-11-03T12:00:00.250Z").is_ok());
    }

    #[test]
    fn a_numeric_offset_is_accepted() {
        assert!(parse_rfc3339("2026-11-03T07:00:00-05:00").is_ok());
    }

    #[test]
    fn an_annotation_after_the_zone_is_refused() {
        assert!(parse_rfc3339("2026-11-03T12:00:00Z[foo=bar]").is_err());
    }

    #[test]
    fn an_annotation_after_an_offset_is_refused() {
        assert!(parse_rfc3339("2026-11-03T12:00:00+01:00[Europe/Amsterdam]").is_err());
    }

    #[test]
    fn omitted_seconds_are_refused() {
        assert!(parse_rfc3339("2026-11-03T12:00Z").is_err());
    }

    #[test]
    fn the_basic_format_is_refused() {
        assert!(parse_rfc3339("20261103T120000Z").is_err());
    }

    #[test]
    fn a_space_separator_is_refused() {
        assert!(parse_rfc3339("2026-11-03 12:00:00Z").is_err());
    }

    #[test]
    fn an_empty_fraction_is_refused() {
        assert!(parse_rfc3339("2026-11-03T12:00:00.Z").is_err());
    }

    #[test]
    fn an_offset_without_a_colon_is_refused() {
        assert!(parse_rfc3339("2026-11-03T12:00:00+0100").is_err());
    }

    #[test]
    fn a_time_without_a_zone_is_refused() {
        assert!(parse_rfc3339("2026-11-03T12:00:00").is_err());
    }

    #[test]
    fn a_well_shaped_but_impossible_date_is_refused() {
        assert!(parse_rfc3339("2026-13-03T12:00:00Z").is_err());
    }

    #[test]
    fn equal_instants_parse_equal_across_offsets() {
        assert_eq!(
            parse_rfc3339("2026-11-03T13:00:00+01:00").unwrap(),
            parse_rfc3339("2026-11-03T12:00:00Z").unwrap()
        );
    }
}
