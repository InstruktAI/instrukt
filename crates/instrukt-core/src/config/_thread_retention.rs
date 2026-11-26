//! The computer's thread-retention window, `runner.archive_finished_after`
//! (`project/instrukt/spec/feature/configuration/layered-configuration`;
//! `project/instrukt/spec/feature/runner/thread-retention`): `off`, or how
//! long a finished thread stays quiet before the runner archives it.

use jiff::{SignedDuration, Timestamp};
use serde_yaml_ng::Value;

use super::_error::{Finding, FindingKind};
use super::_schema::KeyFinding;

/// The global-layer key that holds the retention window.
pub const ARCHIVE_FINISHED_AFTER_KEY: &str = "runner.archive_finished_after";

/// The value that switches thread retention off, and the key's default.
pub(super) const OFF: &str = "off";

/// The key's contract, as schema discovery prints it.
pub(super) const WINDOW_CONTRACT: &str = "How long a finished runtime thread of a registered \
     project stays quiet before the runner archives it: a whole number from 1 with a unit \
     s, m, h, d or w naming a span instrukt can represent, or `off`, under which no thread \
     is archived.";

/// The published grammar of the key, as a JSON Schema pattern; [`parse`]
/// reads the same grammar and adds the representability bound.
pub(super) const WINDOW_PATTERN: &str = "^(off|[1-9][0-9]*[smhdw])$";

/// What a retention window value says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionWindow {
    /// No thread is archived.
    Off,
    /// A finished thread quiet for at least this long is archived.
    After(SignedDuration),
}

/// Why a text is no retention window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRefusal {
    /// Neither `off` nor a whole number from 1 followed by a unit.
    Grammar,
    /// A span longer than any two instants instrukt can tell apart.
    Unrepresentable,
}

impl WindowRefusal {
    fn reason(self) -> &'static str {
        match self {
            Self::Grammar => {
                "expected `off` or a whole number from 1 followed by s, m, h, d or w (as `7d`)"
            }
            Self::Unrepresentable => "expected a span instrukt can represent",
        }
    }
}

/// The window `text` names, or why it names none.
pub fn parse(text: &str) -> Result<RetentionWindow, WindowRefusal> {
    if text == OFF {
        return Ok(RetentionWindow::Off);
    }
    let Some(unit) = text.chars().last() else {
        return Err(WindowRefusal::Grammar);
    };
    let digits = &text[..text.len() - unit.len_utf8()];
    let seconds_per_unit: i64 = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        'w' => 7 * 24 * 60 * 60,
        _ => return Err(WindowRefusal::Grammar),
    };
    if !digits.starts_with(|c: char| ('1'..='9').contains(&c))
        || !digits.chars().all(|c| c.is_ascii_digit())
    {
        return Err(WindowRefusal::Grammar);
    }
    let span = digits
        .parse::<i64>()
        .ok()
        .and_then(|count| count.checked_mul(seconds_per_unit))
        .map(SignedDuration::from_secs)
        .filter(|span| *span <= Timestamp::MAX.duration_since(Timestamp::MIN))
        .ok_or(WindowRefusal::Unrepresentable)?;
    Ok(RetentionWindow::After(span))
}

/// The window the global layer `value` admits: `None` when it sets none,
/// sets `off` or sets a value outside the key's shape, which is a finding
/// of its own.
pub(super) fn admitted(value: Option<&Value>) -> Option<SignedDuration> {
    let raw = value?
        .get("runner")?
        .get("archive_finished_after")?
        .as_str()?;
    match parse(raw) {
        Ok(RetentionWindow::After(span)) => Some(span),
        Ok(RetentionWindow::Off) | Err(_) => None,
    }
}

/// The finding of the window value `value` at `path`, if it is no window.
pub(super) fn check(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let refusal = match value.as_str() {
        Some(text) => parse(text).err(),
        None => Some(WindowRefusal::Grammar),
    };
    if let Some(refusal) = refusal {
        out.push(KeyFinding::new(
            path,
            FindingKind::InvalidValue,
            refusal.reason(),
        ));
    }
}

/// Whether `finding` withholds thread retention on this computer: the
/// selected runtime cannot archive threads while a window is set. Every
/// other finding at the key is a finding of the machine layer.
pub fn withholds_thread_retention(finding: &Finding) -> bool {
    finding.key == ARCHIVE_FINISHED_AFTER_KEY
        && finding.kind == FindingKind::RuntimeCapabilityMissing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_switches_retention_off() {
        assert_eq!(parse("off"), Ok(RetentionWindow::Off));
    }

    #[test]
    fn a_count_of_days_is_that_many_whole_days() {
        assert_eq!(
            parse("7d"),
            Ok(RetentionWindow::After(SignedDuration::from_hours(7 * 24)))
        );
    }

    #[test]
    fn a_count_of_weeks_is_seven_days_each() {
        assert_eq!(
            parse("2w"),
            Ok(RetentionWindow::After(SignedDuration::from_hours(14 * 24)))
        );
    }

    #[test]
    fn a_zero_count_is_no_window() {
        assert_eq!(parse("0h"), Err(WindowRefusal::Grammar));
    }

    #[test]
    fn a_count_without_a_unit_is_no_window() {
        assert_eq!(parse("24"), Err(WindowRefusal::Grammar));
    }

    #[test]
    fn an_unknown_unit_is_no_window() {
        assert_eq!(parse("3y"), Err(WindowRefusal::Grammar));
    }

    #[test]
    fn a_unit_without_a_count_is_no_window() {
        assert_eq!(parse("d"), Err(WindowRefusal::Grammar));
    }

    #[test]
    fn a_span_longer_than_any_instants_apart_is_refused() {
        assert_eq!(parse("99999999w"), Err(WindowRefusal::Unrepresentable));
    }

    #[test]
    fn a_count_past_the_integer_range_is_refused() {
        assert_eq!(
            parse("99999999999999999999s"),
            Err(WindowRefusal::Unrepresentable)
        );
    }

    #[test]
    fn a_capability_finding_at_the_window_withholds_retention_and_a_shape_finding_does_not() {
        let at = |kind| Finding {
            document: "instrukt.yaml".into(),
            key: ARCHIVE_FINISHED_AFTER_KEY.into(),
            kind,
            message: String::new(),
        };
        assert!(withholds_thread_retention(&at(
            FindingKind::RuntimeCapabilityMissing
        )));
        assert!(!withholds_thread_retention(&at(FindingKind::InvalidValue)));
    }
}
