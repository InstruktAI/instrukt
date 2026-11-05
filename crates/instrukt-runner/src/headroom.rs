//! The disk-headroom standing job (`disk-headroom`): on runner wake-ups, at
//! most once every five minutes, it measures the free space of the volume
//! holding the developer's home directory, warns once per crossing below
//! the warn level and, below the floor, removes regenerable output from
//! linked checkouts no running process uses, largest first, until free
//! space is back at the warn level. It acts only on what it read.
//!
//! The last run is the state database's one `disk_headroom` row: its level
//! makes a report fire once per crossing, and its report is what
//! `instrukt ticks status` shows. A report reaches the developer through
//! the runner's anomaly notification, which states the level and free
//! space and names `instrukt ticks status`.

mod _candidates;
mod _checkouts;
mod _processes;
mod _space;

use std::path::Path;

use instrukt_core::config::{Configuration, DiskHeadroom, Env, HeadroomSettings};
use instrukt_core::home::Home;
use instrukt_core::state_db::rusqlite::OptionalExtension;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

pub use _candidates::{Candidate, Kind};
pub use _space::Level;

use crate::state::{self, RunnerState, StateError};

/// The guard runs at most once in this long.
pub const CADENCE: SignedDuration = SignedDuration::from_mins(5);

/// A warn report names at most this many of the largest candidates.
const LARGEST_SHOWN: usize = 10;

/// A candidate the guard tried and failed to remove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    #[serde(flatten)]
    pub candidate: Candidate,
    pub error: String,
}

/// What one run of the guard told the developer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub level: Level,
    /// Free space when the run began; `None` when unmeasured.
    pub free_bytes: Option<u64>,
    pub warn_bytes: u64,
    pub floor_bytes: u64,
    /// At `warn`: the largest candidates, largest first.
    pub candidates: Vec<Candidate>,
    /// At `floor`: what was removed, in removal order.
    pub removed: Vec<Candidate>,
    /// At `floor`: what was left in place as in use.
    pub skipped: Vec<Candidate>,
    /// At `floor`: what could not be removed.
    pub failed: Vec<Failure>,
    /// At `floor`: free space once the run ended.
    pub free_after: Option<u64>,
    /// At `floor`: whether free space ended at or above the floor.
    pub restored: Option<bool>,
    /// Why the guard could not measure, or could not verify what is in
    /// use.
    pub reason: Option<String>,
}

impl Report {
    fn new(level: Level, free_bytes: Option<u64>, settings: &HeadroomSettings) -> Self {
        Self {
            level,
            free_bytes,
            warn_bytes: settings.warn_bytes,
            floor_bytes: settings.floor_bytes,
            candidates: Vec::new(),
            removed: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
            free_after: None,
            restored: None,
            reason: None,
        }
    }

    /// The notification text: the level and free space, and where the
    /// report lives.
    pub fn notification(&self) -> String {
        let free = self.free_bytes.map_or_else(
            || "free space unknown".to_owned(),
            |free| format!("{} free", _space::size_text(free)),
        );
        format!(
            "disk headroom {}: {free}; see `instrukt ticks status`",
            self.level.name()
        )
    }
}

/// The guard's last run, as the state database keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastRun {
    pub run_at: Timestamp,
    pub level: Level,
    pub free_bytes: Option<u64>,
    pub report: Option<Report>,
    pub reported_at: Option<Timestamp>,
}

/// The `disk_headroom` row's columns as stored: `run_at`, `level`,
/// `free_bytes`, `report`, `reported_at`.
type StoredRow = (String, String, Option<i64>, Option<String>, Option<String>);

/// The guard's last run; `None` before the first.
pub fn last_run(state: &RunnerState) -> Result<Option<LastRun>, StateError> {
    let row = state.read(|conn, path| {
        let row: Option<StoredRow> = conn
            .query_row(
                "SELECT run_at, level, free_bytes, report, reported_at FROM disk_headroom",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(state::sql(path))?;
        row.map(|(run_at, level, free_bytes, report, reported_at)| {
            let time = |text: &str| {
                text.parse::<Timestamp>()
                    .map_err(|e| state::invalid(path, e))
            };
            Ok(LastRun {
                run_at: time(&run_at)?,
                level: Level::from_name(&level)
                    .ok_or_else(|| state::invalid(path, format!("unknown level `{level}`")))?,
                free_bytes: free_bytes.and_then(|free| u64::try_from(free).ok()),
                report: report
                    .map(|text| serde_json::from_str(&text).map_err(|e| state::invalid(path, e)))
                    .transpose()?,
                reported_at: reported_at.as_deref().map(time).transpose()?,
            })
        })
        .transpose()
    })?;
    Ok(row.flatten())
}

fn record(
    state: &RunnerState,
    now: Timestamp,
    level: Level,
    free_bytes: Option<u64>,
    report: Option<&Report>,
) -> Result<(), StateError> {
    let report_text = report
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| state::invalid(Path::new("disk_headroom.report"), e))?;
    state.write(|tx| {
        let path = state::db_path(tx);
        let kept: Option<(Option<String>, Option<String>)> = tx
            .query_row("SELECT report, reported_at FROM disk_headroom", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()
            .map_err(state::sql(&path))?;
        let (report, reported_at) = match report_text {
            Some(text) => (Some(text), Some(now.to_string())),
            None => kept.unwrap_or_default(),
        };
        tx.execute("DELETE FROM disk_headroom", [])
            .map_err(state::sql(&path))?;
        tx.execute(
            "INSERT INTO disk_headroom (run_at, level, free_bytes, report, reported_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                now.to_string(),
                level.name(),
                free_bytes.and_then(|free| i64::try_from(free).ok()),
                report,
                reported_at,
            ),
        )
        .map_err(state::sql(&path))?;
        Ok(())
    })
}

/// One wake-up's turn of the guard at `now`. Never fails the wake-up: what
/// it cannot do is logged.
pub fn guard(home: &Home, env: &Env, now: Timestamp) {
    let config = match Configuration::load_global(home) {
        Ok(config) => config,
        Err(err) => {
            tracing::warn!(event = "disk_headroom_config_unreadable", error = %err);
            return;
        }
    };
    let settings = match config.runner_disk_headroom(env) {
        Ok(DiskHeadroom::Off) => return,
        Ok(DiskHeadroom::On(settings)) => settings,
        Err(err) => {
            tracing::warn!(event = "disk_headroom_config_invalid", error = %err);
            return;
        }
    };
    let state = RunnerState::new(home);
    let last = match last_run(&state) {
        Ok(last) => last,
        Err(err) => {
            tracing::warn!(event = "disk_headroom_state_unreadable", error = %err);
            return;
        }
    };
    if last
        .as_ref()
        .is_some_and(|last| now.duration_since(last.run_at) < CADENCE)
    {
        return;
    }
    let previous = last.map(|last| last.level);
    let (level, free_bytes, report) = run_once(home, &settings, previous);
    if let Err(err) = record(&state, now, level, free_bytes, report.as_ref()) {
        tracing::warn!(event = "disk_headroom_state_unwritable", error = %err);
        return;
    }
    if let Some(report) = report {
        tracing::info!(event = "disk_headroom_reported", level = level.name());
        crate::anomaly::notify(&config, env, &report.notification(), None);
    }
}

/// Measures, classifies and acts; the level, the free space and the
/// report to make, if any.
fn run_once(
    home: &Home,
    settings: &HeadroomSettings,
    previous: Option<Level>,
) -> (Level, Option<u64>, Option<Report>) {
    let volume = home.user_home();
    let free = match _space::free_bytes(volume) {
        Ok(free) => free,
        Err(err) => {
            tracing::warn!(event = "disk_headroom_unmeasured", volume = %volume.display(), error = %err);
            let report = (previous != Some(Level::Unmeasured)).then(|| Report {
                reason: Some(format!(
                    "cannot measure the free space of {}: {err}",
                    volume.display()
                )),
                ..Report::new(Level::Unmeasured, None, settings)
            });
            return (Level::Unmeasured, None, report);
        }
    };
    let level = Level::of(free, settings);
    let report = match level {
        Level::Ok | Level::Unmeasured => None,
        Level::Warn => (!matches!(previous, Some(Level::Warn | Level::Floor))).then(|| {
            let mut candidates = candidates(home, settings);
            candidates.truncate(LARGEST_SHOWN);
            Report {
                candidates,
                ..Report::new(Level::Warn, Some(free), settings)
            }
        }),
        Level::Floor => {
            let report = reclaim(volume, home, settings, free);
            (previous != Some(Level::Floor) || !report.removed.is_empty()).then_some(report)
        }
    };
    (level, Some(free), report)
}

/// Every candidate of every checkout the guard may clean, largest first.
fn candidates(home: &Home, settings: &HeadroomSettings) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = _checkouts::discover(home, &settings.roots)
        .iter()
        .flat_map(|checkout| _candidates::in_checkout(checkout))
        .collect();
    found.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.path.cmp(&b.path)));
    found
}

/// Removes candidates largest first until free space is back at the warn
/// level, re-measuring and re-reading what processes hold before each
/// removal; a candidate whose checkout a process holds is skipped.
fn reclaim(volume: &Path, home: &Home, settings: &HeadroomSettings, free: u64) -> Report {
    let mut report = Report::new(Level::Floor, Some(free), settings);
    let mut pending = candidates(home, settings).into_iter();
    while let Some(candidate) = pending.next() {
        match _space::free_bytes(volume) {
            Ok(now_free) if now_free >= settings.warn_bytes => break,
            Ok(_) => {}
            Err(err) => {
                report.reason = Some(format!("cannot re-measure free space: {err}"));
                break;
            }
        }
        let snapshot = _processes::snapshot();
        match snapshot.holds(&candidate.checkout) {
            Some(false) => {}
            Some(true) => {
                tracing::info!(event = "disk_headroom_in_use", path = %candidate.path.display());
                report.skipped.push(candidate);
                continue;
            }
            None => {
                if let _processes::Snapshot::Incomplete(why) = snapshot {
                    tracing::warn!(event = "disk_headroom_processes_unread", reason = %why);
                    report.reason = Some(format!(
                        "what running processes use could not be verified: {why}"
                    ));
                }
                report.skipped.push(candidate);
                report.skipped.extend(pending);
                break;
            }
        }
        match std::fs::remove_dir_all(&candidate.path) {
            Ok(()) => {
                tracing::info!(event = "disk_headroom_removed", path = %candidate.path.display(), size_bytes = candidate.size_bytes);
                report.removed.push(candidate);
            }
            Err(err) => {
                tracing::warn!(event = "disk_headroom_remove_failed", path = %candidate.path.display(), error = %err);
                report.failed.push(Failure {
                    candidate,
                    error: err.to_string(),
                });
            }
        }
    }
    report.free_after = _space::free_bytes(volume).ok();
    report.restored = report.free_after.map(|after| after >= settings.floor_bytes);
    report
}

/// `instrukt ticks status`'s lines for the guard: its last run and last
/// report. `off` when the guard is switched off.
pub fn status_lines(on: bool, last: Option<&LastRun>) -> Vec<String> {
    let size = _space::size_text;
    let mut lines = Vec::new();
    if !on {
        lines.push("disk headroom: off".to_owned());
        return lines;
    }
    let Some(last) = last else {
        lines.push("disk headroom: no run recorded".to_owned());
        return lines;
    };
    let free = last.free_bytes.map_or_else(
        || "free space unknown".to_owned(),
        |free| format!("{} free", size(free)),
    );
    lines.push(format!(
        "disk headroom: {}, {free} (last run {})",
        last.level.name(),
        last.run_at
    ));
    let (Some(report), Some(at)) = (&last.report, last.reported_at) else {
        return lines;
    };
    let free = report.free_bytes.map_or_else(
        || "free space unknown".to_owned(),
        |free| format!("{} free", size(free)),
    );
    lines.push(format!(
        "last report ({at}): {}, {free}; warn below {}, floor {}",
        report.level.name(),
        size(report.warn_bytes),
        size(report.floor_bytes)
    ));
    let entry = |candidate: &Candidate| {
        format!(
            "    {}  {}  (checkout {})",
            size(candidate.size_bytes),
            candidate.path.display(),
            candidate.checkout.display()
        )
    };
    let section = |lines: &mut Vec<String>, title: &str, items: &[Candidate]| {
        if !items.is_empty() {
            lines.push(format!("  {title}:"));
            lines.extend(items.iter().map(entry));
        }
    };
    section(
        &mut lines,
        "largest regenerable directories",
        &report.candidates,
    );
    section(&mut lines, "removed", &report.removed);
    section(&mut lines, "skipped as in use", &report.skipped);
    if !report.failed.is_empty() {
        lines.push("  could not remove:".to_owned());
        lines.extend(
            report
                .failed
                .iter()
                .map(|failure| format!("{}: {}", entry(&failure.candidate), failure.error)),
        );
    }
    if let Some(after) = report.free_after {
        lines.push(format!("  free after: {}", size(after)));
    }
    if report.restored == Some(false) {
        lines.push("  headroom could not be restored".to_owned());
    }
    if let Some(reason) = &report.reason {
        lines.push(format!("  {reason}"));
    }
    lines
}

/// Whether the guard is switched on in `config`; an unreadable setting
/// counts as on, since the guard then reports nothing it did not run.
pub fn switched_on(config: &Configuration, env: &Env) -> bool {
    !matches!(config.runner_disk_headroom(env), Ok(DiskHeadroom::Off))
}
