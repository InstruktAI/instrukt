//! Fleet attention (`fleet-attention`): which facts of a project's active
//! work are attention conditions, their `runner.attention.*` events and the
//! project's attention episode.
//!
//! Work calls [`record`] inside the transaction of every change it makes on
//! this computer to a todo's status, hold or attempts, passing that change's
//! facts; the runner decides which of them are conditions and appends their
//! events in the same transaction. The runner's own observations append
//! through [`condition`]. Every append that is new opens the project's
//! episode while runnable work remains; the change that leaves no runnable
//! work closes it with `runner.attention.fleet_quiet`.

use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName, TodoSlug};
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, Transaction, params};
use jiff::Timestamp;

use crate::event_log;
use crate::events::{
    AppendOutcome, AttentionHoldSource, EmissionContext, Event, EventHomeOwner, EventLogError,
};
use crate::state::{self, StateError};

/// A dispatch hold as one side of a record change carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldFact {
    pub source: AttentionHoldSource,
    pub reason: String,
    pub placed_at: Timestamp,
}

/// An attempt the change records as failed, with how many earlier attempts
/// of the same role on the todo had already failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedAttempt {
    pub role: RoleName,
    pub attempt: u64,
    pub reason: String,
    pub earlier_failures: usize,
}

/// The facts of one change work makes to a todo's status, hold or attempts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordChange {
    pub project: ProjectSlug,
    pub todo: TodoSlug,
    pub hold_before: Option<HoldFact>,
    pub hold_after: Option<HoldFact>,
    /// The todo was active before the change and is not after it.
    pub left_active: bool,
    pub failed_attempt: Option<FailedAttempt>,
    /// After the change, the project has an active todo that is unheld and
    /// not a structural holder.
    pub runnable_remains: bool,
}

/// Appends the conditions `change` establishes, inside work's transaction
/// `tx`, and opens or closes the project's attention episode.
pub fn record(
    tx: &Transaction<'_>,
    home: &Home,
    now: Timestamp,
    change: &RecordChange,
) -> Result<(), EventLogError> {
    let conditions = conditions(change);
    let closing = !change.runnable_remains && open_episode_of(tx, &change.project)?.is_some();
    if conditions.is_empty() && !closing {
        return Ok(());
    }
    let context = context(home, now, None)?;
    for event in &conditions {
        condition(tx, &context, event, change.runnable_remains)?;
    }
    if closing {
        close_episode(tx, &context, &change.project, &change.todo)?;
    }
    Ok(())
}

/// The condition events `change` establishes: a hold released and one
/// placed (a replacement both), and a role's repeated failure.
fn conditions(change: &RecordChange) -> Vec<Event> {
    let project = &change.project;
    let todo = &change.todo;
    let moved = |a: &HoldFact, b: &HoldFact| a.placed_at != b.placed_at || a.source != b.source;
    let mut events = Vec::new();
    if let Some(before) = &change.hold_before
        && change
            .hold_after
            .as_ref()
            .is_none_or(|after| moved(before, after))
    {
        events.push(Event::AttentionHoldReleased {
            project: project.clone(),
            todo: todo.clone(),
            source: before.source,
            placed_at: before.placed_at,
        });
    }
    if let Some(after) = &change.hold_after
        && change
            .hold_before
            .as_ref()
            .is_none_or(|before| moved(before, after))
    {
        events.push(Event::AttentionHoldPlaced {
            project: project.clone(),
            todo: todo.clone(),
            source: after.source,
            reason: after.reason.clone(),
            placed_at: after.placed_at,
        });
    }
    if let Some(failed) = &change.failed_attempt
        && failed.earlier_failures > 0
    {
        events.push(Event::AttentionAttemptsFailing {
            project: project.clone(),
            todo: todo.clone(),
            role: failed.role.clone(),
            attempt: failed.attempt,
            reason: failed.reason.clone(),
        });
    }
    events
}

/// Appends the condition `event` in `tx`; a new occurrence opens the
/// project's episode when `runnable` work remains.
pub(crate) fn condition(
    tx: &Transaction<'_>,
    context: &EmissionContext,
    event: &Event,
    runnable: bool,
) -> Result<AppendOutcome, EventLogError> {
    let outcome = event_log::append(tx, event, context)?;
    if outcome == AppendOutcome::Appended {
        open_episode(tx, event.project(), &event.occurrence_id(), runnable)?;
    }
    Ok(outcome)
}

/// Opens `project`'s attention episode, recording `opened_by` as the
/// occurrence that opened it, when `runnable` work remains; an episode
/// already open keeps the occurrence that opened it.
pub fn open_episode(
    tx: &Transaction<'_>,
    project: &ProjectSlug,
    opened_by: &str,
    runnable: bool,
) -> Result<(), StateError> {
    if !runnable {
        return Ok(());
    }
    tx.execute(
        "INSERT OR IGNORE INTO attention_episodes (project, opened_by) VALUES (?1, ?2)",
        params![project.as_str(), opened_by],
    )
    .map_err(state::sql(&state::db_path(tx)))?;
    Ok(())
}

/// Ends `project`'s open episode with `runner.attention.fleet_quiet` about
/// `todo`, the todo whose change closed it; with none open, nothing.
fn close_episode(
    tx: &Transaction<'_>,
    context: &EmissionContext,
    project: &ProjectSlug,
    todo: &TodoSlug,
) -> Result<(), EventLogError> {
    let Some(opened_by) = open_episode_of(tx, project)? else {
        return Ok(());
    };
    let quiet = Event::AttentionFleetQuiet {
        project: project.clone(),
        todo: todo.clone(),
        opened_by,
    };
    event_log::append(tx, &quiet, context)?;
    tx.execute(
        "DELETE FROM attention_episodes WHERE project = ?1",
        params![project.as_str()],
    )
    .map_err(state::sql(&state::db_path(tx)))?;
    Ok(())
}

/// The occurrence that opened `project`'s episode, while one is open.
fn open_episode_of(conn: &Connection, project: &ProjectSlug) -> Result<Option<String>, StateError> {
    conn.query_row(
        "SELECT opened_by FROM attention_episodes WHERE project = ?1",
        params![project.as_str()],
        |row| row.get(0),
    )
    .optional()
    .map_err(state::sql(&state::db_path(conn)))
}

/// The emission context of an attention event appended on this computer at
/// `now`, with the session that caused it when one did.
pub(crate) fn context(
    home: &Home,
    now: Timestamp,
    caller_session: Option<String>,
) -> Result<EmissionContext, EventLogError> {
    Ok(EmissionContext {
        source: instrukt_core::machine::machine_id(home)?.to_string(),
        time: now,
        project_owner: EventHomeOwner::Unresolved,
        caller_session,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test: failed setup fails the test")]

    use instrukt_core::state_db::StateDb;

    use super::*;
    use crate::state::RunnerState;

    fn home() -> (tempfile::TempDir, Home) {
        let dir = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        drop(StateDb::open(&home).unwrap());
        (dir, home)
    }

    fn now() -> Timestamp {
        "2026-11-25T12:00:00Z".parse().unwrap()
    }

    fn hold(source: AttentionHoldSource, at: &str) -> HoldFact {
        HoldFact {
            source,
            reason: "waiting".into(),
            placed_at: at.parse().unwrap(),
        }
    }

    fn change(todo: &str) -> RecordChange {
        RecordChange {
            project: ProjectSlug::parse("demo").unwrap(),
            todo: TodoSlug::parse(todo).unwrap(),
            hold_before: None,
            hold_after: None,
            left_active: false,
            failed_attempt: None,
            runnable_remains: true,
        }
    }

    fn failed(attempt: u64, earlier_failures: usize) -> Option<FailedAttempt> {
        Some(FailedAttempt {
            role: RoleName::parse("builder").unwrap(),
            attempt,
            reason: "crashed".into(),
            earlier_failures,
        })
    }

    /// Records `change` in a transaction of its own.
    fn apply(home: &Home, change: &RecordChange) {
        RunnerState::new(home)
            .write(|tx| record(tx, home, now(), change))
            .unwrap();
    }

    /// The logged event names, oldest first.
    fn logged(home: &Home) -> Vec<String> {
        RunnerState::new(home)
            .read(|conn, path| {
                let mut statement = conn
                    .prepare("SELECT name FROM events ORDER BY seq")
                    .map_err(state::sql(path))?;
                statement
                    .query_map([], |row| row.get(0))
                    .map_err(state::sql(path))?
                    .collect::<Result<Vec<String>, _>>()
                    .map_err(state::sql(path))
            })
            .unwrap()
            .unwrap_or_default()
    }

    fn episode(home: &Home) -> Option<String> {
        RunnerState::new(home)
            .read(|conn, _| open_episode_of(conn, &ProjectSlug::parse("demo").unwrap()))
            .unwrap()
            .flatten()
    }

    fn names(events: &[Event]) -> Vec<String> {
        events.iter().map(|event| event.name().to_owned()).collect()
    }

    fn placed() -> Event {
        Event::AttentionHoldPlaced {
            project: ProjectSlug::parse("demo").unwrap(),
            todo: TodoSlug::parse("a").unwrap(),
            source: AttentionHoldSource::Manual,
            reason: "waiting".into(),
            placed_at: "2026-11-25T11:00:00Z".parse().unwrap(),
        }
    }

    fn released() -> Event {
        Event::AttentionHoldReleased {
            project: ProjectSlug::parse("demo").unwrap(),
            todo: TodoSlug::parse("a").unwrap(),
            source: AttentionHoldSource::Manual,
            placed_at: "2026-11-25T11:00:00Z".parse().unwrap(),
        }
    }

    fn quiet() -> Event {
        Event::AttentionFleetQuiet {
            project: ProjectSlug::parse("demo").unwrap(),
            todo: TodoSlug::parse("a").unwrap(),
            opened_by: String::new(),
        }
    }

    #[test]
    fn a_placed_hold_opens_the_episode_while_other_work_runs() {
        let (_dir, home) = home();
        apply(
            &home,
            &RecordChange {
                hold_after: Some(hold(AttentionHoldSource::Manual, "2026-11-25T11:00:00Z")),
                ..change("a")
            },
        );
        assert_eq!(logged(&home), names(&[placed()]));
        assert_eq!(episode(&home), Some(placed().occurrence_id()));
    }

    #[test]
    fn a_replaced_hold_is_released_and_placed_and_an_unchanged_one_is_neither() {
        let (_dir, home) = home();
        let first = hold(AttentionHoldSource::Manual, "2026-11-25T11:00:00Z");
        let second = hold(AttentionHoldSource::Automatic, "2026-11-25T11:30:00Z");
        apply(
            &home,
            &RecordChange {
                hold_before: Some(first.clone()),
                hold_after: Some(first.clone()),
                ..change("a")
            },
        );
        assert!(logged(&home).is_empty());
        apply(
            &home,
            &RecordChange {
                hold_before: Some(first),
                hold_after: Some(second),
                ..change("a")
            },
        );
        assert_eq!(logged(&home), names(&[released(), placed()]));
    }

    #[test]
    fn a_first_failure_is_no_condition_and_a_repeated_one_is() {
        let (_dir, home) = home();
        apply(
            &home,
            &RecordChange {
                failed_attempt: failed(1, 0),
                ..change("a")
            },
        );
        assert!(logged(&home).is_empty());
        apply(
            &home,
            &RecordChange {
                failed_attempt: failed(2, 1),
                ..change("a")
            },
        );
        assert_eq!(logged(&home), ["runner.attention.attempts_failing"]);
    }

    #[test]
    fn a_condition_with_nothing_left_runnable_opens_no_episode() {
        let (_dir, home) = home();
        apply(
            &home,
            &RecordChange {
                hold_after: Some(hold(AttentionHoldSource::Manual, "2026-11-25T11:00:00Z")),
                runnable_remains: false,
                ..change("a")
            },
        );
        assert_eq!(logged(&home), names(&[placed()]));
        assert_eq!(episode(&home), None);
    }

    #[test]
    fn the_last_runnable_todo_leaving_ends_the_open_episode_once() {
        let (_dir, home) = home();
        apply(
            &home,
            &RecordChange {
                hold_after: Some(hold(AttentionHoldSource::Manual, "2026-11-25T11:00:00Z")),
                ..change("a")
            },
        );
        let leaving = RecordChange {
            left_active: true,
            runnable_remains: false,
            ..change("b")
        };
        apply(&home, &leaving);
        apply(&home, &leaving);
        assert_eq!(logged(&home), names(&[placed(), quiet()]));
        assert_eq!(episode(&home), None);
    }

    #[test]
    fn work_leaving_with_no_episode_open_appends_nothing() {
        let (_dir, home) = home();
        apply(
            &home,
            &RecordChange {
                left_active: true,
                runnable_remains: false,
                ..change("a")
            },
        );
        assert!(logged(&home).is_empty());
    }

    #[test]
    fn replaying_the_same_facts_appends_nothing_new() {
        let (_dir, home) = home();
        let repeated = RecordChange {
            failed_attempt: failed(2, 1),
            ..change("a")
        };
        apply(&home, &repeated);
        let opened = episode(&home);
        apply(&home, &repeated);
        assert_eq!(logged(&home), ["runner.attention.attempts_failing"]);
        assert_eq!(episode(&home), opened);
    }
}
