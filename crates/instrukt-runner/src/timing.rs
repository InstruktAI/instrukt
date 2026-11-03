//! Task timing (`task-timing`, "Task and schedule"): what a cron
//! expression means, which occurrence is due at a wake-up, and the
//! missed-run decision.
//!
//! Cron has five fields in the machine's local time zone with its
//! daylight-saving rules: minute (0–59), hour (0–23), day of month (1–31),
//! month (1–12 or `JAN`–`DEC`), day of week (0–7, 0 and 7 Sunday, or
//! `SUN`–`SAT`). Each field is `*` or a comma list of values, ranges
//! `a-b`, and steps `*/n`, `a-b/n` or `a/n`. When both day fields are
//! restricted, a day matching either one matches, as in cron.
//!
//! An occurrence is on time at a wake-up when it fell within the minute
//! before it; an earlier one not fired is missed. The window considered
//! starts after the occurrence the task last fired, or after the previous
//! wake-up when it never fired; it reaches back at most one year.

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::interfaces::{CatchUp, CronExpr, TaskSchedule, TaskTiming};

/// How far back an occurrence can be missed and still be considered.
const LOOKBACK: SignedDuration = SignedDuration::from_hours(366 * 24);

/// One wake-up's interval: an occurrence this recent is on time.
const ON_TIME: SignedDuration = SignedDuration::from_secs(60);

/// A cron field that does not parse or lies outside its range.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cron field `{field}` is invalid: {reason}")]
pub struct CronError {
    pub field: String,
    pub reason: &'static str,
}

/// The parsed meaning of a five-field cron expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSpec {
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    days_restricted: bool,
    weekdays_restricted: bool,
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const WEEKDAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

impl CronSpec {
    /// Parses five whitespace-separated fields.
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err(CronError {
                field: expr.to_owned(),
                reason: "expected five fields",
            });
        };
        let mut weekdays = field(weekday, 0, 7, &WEEKDAYS, 0)?;
        if weekdays & (1 << 7) != 0 {
            weekdays = (weekdays & !(1 << 7)) | 1;
        }
        Ok(Self {
            minutes: field(minute, 0, 59, &[], 0)?,
            hours: field(hour, 0, 23, &[], 0)?,
            days: field(day, 1, 31, &[], 0)?,
            months: field(month, 1, 12, &MONTHS, 1)?,
            weekdays,
            days_restricted: !day.starts_with('*'),
            weekdays_restricted: !weekday.starts_with('*'),
        })
    }

    /// Whether the minute starting at `at`, read in `tz`, is an occurrence.
    pub fn matches(&self, at: Timestamp, tz: &TimeZone) -> bool {
        let local = at.to_zoned(tz.clone());
        let bit = |set: u64, value: i64| {
            u64::try_from(value).is_ok_and(|v| v < 64 && set & (1 << v) != 0)
        };
        let day = bit(self.days, i64::from(local.day()));
        let weekday = bit(
            self.weekdays,
            i64::from(local.weekday().to_sunday_zero_offset()),
        );
        let day_matches = match (self.days_restricted, self.weekdays_restricted) {
            (true, true) => day || weekday,
            _ => day && weekday,
        };
        bit(self.minutes, i64::from(local.minute()))
            && bit(self.hours, i64::from(local.hour()))
            && bit(self.months, i64::from(local.month()))
            && day_matches
    }

    /// The latest occurrence in `(after, until]`, minute-aligned.
    pub fn latest_between(
        &self,
        after: Timestamp,
        until: Timestamp,
        tz: &TimeZone,
    ) -> Option<Timestamp> {
        let floor = until.as_second() - until.as_second().rem_euclid(60);
        let mut minute = Timestamp::from_second(floor).ok()?;
        let lower = after.max(until.checked_sub(LOOKBACK).ok()?);
        while minute > lower {
            if self.matches(minute, tz) {
                return Some(minute);
            }
            minute = minute.checked_sub(SignedDuration::from_secs(60)).ok()?;
        }
        None
    }
}

/// Parses one field into a bit set of the values it selects.
fn field(text: &str, min: u32, max: u32, names: &[&str], name_base: u32) -> Result<u64, CronError> {
    let refuse = |reason| CronError {
        field: text.to_owned(),
        reason,
    };
    let value = |part: &str| -> Result<u32, CronError> {
        let upper = part.to_ascii_uppercase();
        if let Some(index) = names.iter().position(|name| *name == upper) {
            return u32::try_from(index)
                .map(|i| i + name_base)
                .map_err(|_| refuse("unknown name"));
        }
        let number: u32 = part.parse().map_err(|_| refuse("not a number or name"))?;
        if number < min || number > max {
            return Err(refuse("out of range"));
        }
        Ok(number)
    };
    let mut set = 0u64;
    for item in text.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step.parse().map_err(|_| refuse("step is not a number"))?;
                if step == 0 {
                    return Err(refuse("step is zero"));
                }
                (range, step)
            }
            None => (item, 1),
        };
        let (from, to) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            let (a, b) = (value(a)?, value(b)?);
            if a > b {
                return Err(refuse("range runs backwards"));
            }
            (a, b)
        } else {
            let a = value(range)?;
            (a, if item.contains('/') { max } else { a })
        };
        let mut v = Some(from);
        while let Some(value) = v.filter(|value| *value <= to) {
            set |= 1 << value;
            v = value.checked_add(step);
        }
    }
    Ok(set)
}

/// The timing decision for one task at one wake-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Fire this occurrence; `catch_up` when it was missed.
    Fire {
        occurrence: Timestamp,
        catch_up: bool,
    },
    /// Nothing is due.
    NotDue,
    /// The last firing has no recorded outcome; it is not fired again.
    InFlight,
}

/// Decides `timing` at the wake-up `now`, the previous wake-up having been
/// at `previous_wake`.
pub fn decide(
    timing: &TaskTiming,
    previous_wake: Option<Timestamp>,
    now: Timestamp,
    tz: &TimeZone,
) -> Decision {
    if timing
        .last_fired
        .as_ref()
        .is_some_and(|firing| firing.outcome.is_none())
    {
        return Decision::InFlight;
    }
    let occurrence = match &timing.schedule {
        TaskSchedule::At(at) => {
            if timing.last_fired.is_some() || *at > now {
                return Decision::NotDue;
            }
            *at
        }
        TaskSchedule::Cron(expr) => {
            let Ok(spec) = CronSpec::parse(expr.as_str()) else {
                return Decision::NotDue;
            };
            let after = timing
                .last_fired
                .as_ref()
                .map(|firing| firing.occurrence)
                .or(previous_wake)
                .unwrap_or_else(|| now.checked_sub(ON_TIME).unwrap_or(now));
            match spec.latest_between(after, now, tz) {
                Some(occurrence) => occurrence,
                None => return Decision::NotDue,
            }
        }
    };
    let on_time = now.duration_since(occurrence) < ON_TIME;
    match (on_time, timing.catch_up) {
        (true, _) => Decision::Fire {
            occurrence,
            catch_up: false,
        },
        (false, CatchUp::Once) => Decision::Fire {
            occurrence,
            catch_up: true,
        },
        (false, CatchUp::Skip) => Decision::NotDue,
    }
}

impl CronExpr {
    /// The expression's meaning.
    pub fn spec(&self) -> Result<CronSpec, CronError> {
        CronSpec::parse(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interfaces::{Firing, TaskOutcome};

    fn utc() -> TimeZone {
        TimeZone::UTC
    }

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn cron(expr: &str, catch_up: CatchUp, last: Option<&str>) -> TaskTiming {
        TaskTiming {
            schedule: TaskSchedule::parse(&format!("cron:{expr}")).unwrap(),
            catch_up,
            last_fired: last.map(|occ| Firing {
                occurrence: ts(occ),
                started: ts(occ),
                outcome: Some(TaskOutcome::Success),
            }),
        }
    }

    #[test]
    fn a_step_past_the_field_selects_its_start_only() {
        let spec = CronSpec::parse("1/4294967295 * * * *").unwrap();
        assert!(spec.matches(ts("2026-01-05T09:01:00Z"), &utc()));
        assert!(!spec.matches(ts("2026-01-05T09:02:00Z"), &utc()));
        assert_eq!(
            TaskSchedule::parse("cron:*/4294967295 * * * *").map(|_| ()),
            Ok(())
        );
    }

    #[test]
    fn fields_select_values_ranges_steps_and_names() {
        let spec = CronSpec::parse("*/15 9-17 * JAN,jul MON-FRI").unwrap();
        assert!(spec.matches(ts("2026-01-05T09:45:00Z"), &utc()));
        assert!(!spec.matches(ts("2026-01-05T09:44:00Z"), &utc()));
        assert!(
            !spec.matches(ts("2026-01-04T09:45:00Z"), &utc()),
            "a Sunday"
        );
        assert!(
            !spec.matches(ts("2026-02-02T09:45:00Z"), &utc()),
            "February"
        );
        assert!(
            CronSpec::parse("0 0 * * 7")
                .unwrap()
                .matches(ts("2026-01-04T00:00:00Z"), &utc())
        );
        for bad in [
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "*/0 * * * *",
            "1/4294967296 * * * *",
            "5-1 * * * *",
            "* * * FOO *",
            "* * * *",
        ] {
            assert!(CronSpec::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn both_restricted_day_fields_match_either_day() {
        let spec = CronSpec::parse("0 12 1 * MON").unwrap();
        assert!(
            spec.matches(ts("2026-01-01T12:00:00Z"), &utc()),
            "the 1st, a Thursday"
        );
        assert!(spec.matches(ts("2026-01-05T12:00:00Z"), &utc()), "a Monday");
        assert!(!spec.matches(ts("2026-01-06T12:00:00Z"), &utc()));
    }

    #[test]
    fn local_time_follows_the_zone_and_its_daylight_saving() {
        let amsterdam = TimeZone::get("Europe/Amsterdam").unwrap();
        let spec = CronSpec::parse("0 9 * * *").unwrap();
        assert!(
            spec.matches(ts("2026-01-05T08:00:00Z"), &amsterdam),
            "winter: UTC+1"
        );
        assert!(
            spec.matches(ts("2026-07-06T07:00:00Z"), &amsterdam),
            "summer: UTC+2"
        );
    }

    #[test]
    fn an_occurrence_at_this_wake_up_fires_on_time() {
        let timing = cron("0 * * * *", CatchUp::Skip, Some("2026-01-05T09:00:00Z"));
        assert_eq!(
            decide(
                &timing,
                Some(ts("2026-01-05T09:59:10Z")),
                ts("2026-01-05T10:00:10Z"),
                &utc()
            ),
            Decision::Fire {
                occurrence: ts("2026-01-05T10:00:00Z"),
                catch_up: false
            }
        );
    }

    #[test]
    fn missed_occurrences_catch_up_once_or_are_skipped() {
        let now = ts("2026-01-05T14:30:00Z");
        let once = cron("0 * * * *", CatchUp::Once, Some("2026-01-05T09:00:00Z"));
        assert_eq!(
            decide(&once, None, now, &utc()),
            Decision::Fire {
                occurrence: ts("2026-01-05T14:00:00Z"),
                catch_up: true
            }
        );
        let skip = cron("0 * * * *", CatchUp::Skip, Some("2026-01-05T09:00:00Z"));
        assert_eq!(decide(&skip, None, now, &utc()), Decision::NotDue);
        let caught_up = cron("0 * * * *", CatchUp::Once, Some("2026-01-05T14:00:00Z"));
        assert_eq!(decide(&caught_up, Some(now), now, &utc()), Decision::NotDue);
    }

    #[test]
    fn a_task_that_never_fired_looks_back_to_the_previous_wake_up_only() {
        let timing = cron("0 * * * *", CatchUp::Once, None);
        let now = ts("2026-01-05T14:30:00Z");
        assert_eq!(decide(&timing, None, now, &utc()), Decision::NotDue);
        assert_eq!(
            decide(&timing, Some(ts("2026-01-05T14:20:00Z")), now, &utc()),
            Decision::NotDue
        );
        assert_eq!(
            decide(&timing, Some(ts("2026-01-05T12:20:00Z")), now, &utc()),
            Decision::Fire {
                occurrence: ts("2026-01-05T14:00:00Z"),
                catch_up: true
            }
        );
    }

    #[test]
    fn a_one_off_fires_once_when_its_time_has_come() {
        let at = |catch_up, fired: bool| TaskTiming {
            schedule: TaskSchedule::At(ts("2026-01-05T10:00:30Z")),
            catch_up,
            last_fired: fired.then(|| Firing {
                occurrence: ts("2026-01-05T10:00:30Z"),
                started: ts("2026-01-05T10:01:00Z"),
                outcome: Some(TaskOutcome::Error { reason: None }),
            }),
        };
        assert_eq!(
            decide(
                &at(CatchUp::Skip, false),
                None,
                ts("2026-01-05T10:00:00Z"),
                &utc()
            ),
            Decision::NotDue
        );
        assert_eq!(
            decide(
                &at(CatchUp::Skip, false),
                None,
                ts("2026-01-05T10:01:05Z"),
                &utc()
            ),
            Decision::Fire {
                occurrence: ts("2026-01-05T10:00:30Z"),
                catch_up: false
            }
        );
        assert_eq!(
            decide(
                &at(CatchUp::Skip, false),
                None,
                ts("2026-01-06T00:00:00Z"),
                &utc()
            ),
            Decision::NotDue
        );
        assert!(matches!(
            decide(
                &at(CatchUp::Once, false),
                None,
                ts("2026-01-06T00:00:00Z"),
                &utc()
            ),
            Decision::Fire { catch_up: true, .. }
        ));
        assert_eq!(
            decide(
                &at(CatchUp::Once, true),
                None,
                ts("2026-01-06T00:00:00Z"),
                &utc()
            ),
            Decision::NotDue
        );
    }

    #[test]
    fn a_firing_without_an_outcome_is_in_flight() {
        let mut timing = cron("* * * * *", CatchUp::Once, Some("2026-01-05T10:00:00Z"));
        if let Some(firing) = timing.last_fired.as_mut() {
            firing.outcome = None;
        }
        assert_eq!(
            decide(&timing, None, ts("2026-01-05T10:05:00Z"), &utc()),
            Decision::InFlight
        );
    }
}
