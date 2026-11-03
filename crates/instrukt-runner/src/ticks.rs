//! Wake-up accounting (`task-timing`, "Wake-up installation and
//! visibility"): the last wake-up is kept in `state/runner/`, and status
//! warns when none arrived for five minutes.

use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

use crate::state::{self, RunnerState, StateError};

/// No wake-up for this long is worth a warning.
pub const STALE_AFTER: SignedDuration = SignedDuration::from_mins(5);

/// What `instrukt ticks status` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TicksStatus {
    /// The last recorded wake-up; `None` when none was ever recorded.
    pub last_wakeup: Option<Timestamp>,
    /// No wake-up arrived within [`STALE_AFTER`] of `now`.
    pub stale: bool,
}

/// The last recorded wake-up.
pub fn last_wakeup(state: &RunnerState) -> Result<Option<Timestamp>, StateError> {
    let path = state.last_wakeup();
    let Some(text) = state::read_text(&path)? else {
        return Ok(None);
    };
    text.trim()
        .parse::<Timestamp>()
        .map(Some)
        .map_err(|err| StateError::Corrupt {
            path,
            message: err.to_string(),
        })
}

/// Records a wake-up at `now`.
pub fn record_wakeup(state: &RunnerState, now: Timestamp) -> Result<(), StateError> {
    state::write_bytes(&state.last_wakeup(), format!("{now}\n").as_bytes())
}

/// The status at `now` given the last wake-up.
pub fn status(last_wakeup: Option<Timestamp>, now: Timestamp) -> TicksStatus {
    TicksStatus {
        last_wakeup,
        stale: last_wakeup.is_none_or(|last| now.duration_since(last) > STALE_AFTER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn a_wake_up_older_than_five_minutes_is_stale() {
        let now = ts("2026-01-05T10:00:00Z");
        assert!(!status(Some(ts("2026-01-05T09:55:00Z")), now).stale);
        assert!(status(Some(ts("2026-01-05T09:54:59Z")), now).stale);
        assert!(status(None, now).stale);
    }
}
