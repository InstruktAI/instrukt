//! A commit's review requests and their standing, read from delivery's records.

use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, params};
use jiff::Timestamp;

use super::ReviewReadError;
use crate::consumers::{ACTIVATION_CONSUMER, REPLACEABLE};
use crate::events::review_request_id;
use crate::state::{self, RunnerState, StateError};

/// Where delivery has a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Emitted and not selected by delivery: it can still be replaced.
    Queued,
    /// Frozen by delivery for handover; no session holds it yet.
    Selected,
    /// Accepted by the runtime for a named session.
    HandedOver,
    /// Ended, with what delivery can see of why.
    Ended(Disposition),
}

/// Why a request ended, as far as delivery knows. That a verdict answered
/// it is work's to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// The turn that received it ended.
    NoVerdict,
    /// The runtime accepted the handover and no entry was seen within the
    /// bound.
    Unconfirmed,
    /// It never reached a session.
    NeverHandedOver,
}

/// One review request of a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub number: u64,
    /// The pinned base its review starts from.
    pub base: CommitSha,
    /// The pull request it is addressed to; none for a candidate.
    pub pull_request: Option<u64>,
    pub logged_at: Timestamp,
    pub standing: Standing,
    /// The session delivery handed it to or selected it for.
    pub session: Option<SessionRef>,
}

impl Request {
    /// Whether delivery will still hand it over: it is selected.
    pub fn awaits_handover(&self) -> bool {
        self.standing == Standing::Selected
    }
}

/// What delivery's records hold of one request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DeliveryRow {
    /// The activation's state, with whether it is still replaceable.
    activation: Option<(String, bool)>,
    /// The activation consumer handled the event.
    handled: bool,
    /// The runtime input's state, with whether its turn is still watched.
    input: Option<(String, bool)>,
}

/// The standing delivery's records give a request.
fn standing(row: &DeliveryRow) -> Standing {
    let Some((activation, replaceable)) = &row.activation else {
        return if row.handled {
            Standing::Ended(Disposition::NeverHandedOver)
        } else {
            Standing::Queued
        };
    };
    if *replaceable {
        return Standing::Queued;
    }
    match activation.as_str() {
        "unconfirmed" => return Standing::Ended(Disposition::Unconfirmed),
        "accepted" | "pending" | "delivered" => {}
        _ => return Standing::Ended(Disposition::NeverHandedOver),
    }
    match row
        .input
        .as_ref()
        .map(|(state, watched)| (state.as_str(), *watched))
    {
        Some(("entered", false)) => Standing::Ended(Disposition::NoVerdict),
        Some(("accepted" | "entered", _)) => Standing::HandedOver,
        Some(("unconfirmed", _)) => Standing::Ended(Disposition::Unconfirmed),
        Some(("failed" | "set_aside" | "superseded", _)) => {
            Standing::Ended(Disposition::NeverHandedOver)
        }
        // Selected for a session not started yet, or not yet accepted by
        // its runtime; a started session's handover reads as accepted.
        _ if activation == "delivered" => Standing::HandedOver,
        _ => Standing::Selected,
    }
}

/// `head`'s review requests of `project`, oldest first, from delivery's
/// records alone. Request numbers run from one without a gap, so each is
/// read by its occurrence identity until one is missing.
pub fn requests(
    state: &RunnerState,
    project: &ProjectSlug,
    head: &CommitSha,
) -> Result<Vec<Request>, ReviewReadError> {
    let read = state.read(|conn, path| {
        let mut found = Vec::new();
        for number in 1.. {
            match request(conn, path, project, head, number)? {
                Some(request) => found.push(request),
                None => break,
            }
        }
        Ok(found)
    })?;
    Ok(read.unwrap_or_default())
}

fn request(
    conn: &Connection,
    path: &std::path::Path,
    project: &ProjectSlug,
    head: &CommitSha,
    number: u64,
) -> Result<Option<Request>, StateError> {
    let sql = state::sql(path);
    let id = review_request_id(project, head, number);
    let Some((payload, logged_at)) = conn
        .query_row(
            "SELECT payload, logged_at FROM events WHERE id = ?1",
            params![id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(&sql)?
    else {
        return Ok(None);
    };
    let payload: serde_json::Value =
        serde_json::from_str(&payload).map_err(|e| state::invalid(path, e))?;
    let base = payload["base"]
        .as_str()
        .and_then(|base| CommitSha::parse(base).ok())
        .ok_or_else(|| state::invalid(path, format!("review request {id} has no pinned base")))?;
    let logged_at = logged_at
        .parse::<Timestamp>()
        .map_err(|e| state::invalid(path, e))?;
    let row = conn
        .query_row(
            &format!(
                "SELECT a.state, ({REPLACEABLE}), i.state, i.watch_run, d.target
                 FROM procedure_activations a
                 LEFT JOIN standing_deliveries d ON d.event_id=a.event_id AND d.procedure=a.procedure AND d.exact_home=a.exact_home
                 LEFT JOIN runtime_inputs i ON i.input_id=d.input_id AND (i.runtime || ':' || i.session)=d.target
                 WHERE a.event_id=?1 AND a.procedure=?2 ORDER BY a.rowid LIMIT 1"
            ),
            params![id, instrukt_core::config::REVIEW_PROCEDURE],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<bool>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(&sql)?;
    let handled = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM handled_events WHERE consumer=?1 AND event_id=?2)",
            params![ACTIVATION_CONSUMER, id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(&sql)?;
    let (delivery, session) = match row {
        Some((activation, replaceable, input, watched, target)) => (
            DeliveryRow {
                activation: Some((activation, replaceable)),
                handled,
                input: input.map(|state| (state, watched.unwrap_or(false))),
            },
            target.and_then(|target| SessionRef::parse(&target).ok()),
        ),
        None => (
            DeliveryRow {
                handled,
                ..DeliveryRow::default()
            },
            None,
        ),
    };
    Ok(Some(Request {
        number,
        base,
        pull_request: payload["pull_request"].as_u64(),
        logged_at,
        standing: standing(&delivery),
        session,
    }))
}

/// How long the second request of a commit waits after the first; each
/// further one waits twice as long as the one before.
const FIRST_RETRY: jiff::SignedDuration = jiff::SignedDuration::from_mins(10);

/// The longest a further request waits.
const LONGEST_RETRY: jiff::SignedDuration = jiff::SignedDuration::from_hours(4);

/// Whether the next request of a commit whose requests are `previous` may
/// be emitted at `now`: at once for the first, otherwise once the wait the
/// last one's number gives has passed.
pub(crate) fn due(previous: &[Request], now: Timestamp) -> bool {
    let Some(last) = previous.last() else {
        return true;
    };
    let doublings = u32::try_from(last.number.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(16);
    let wait = FIRST_RETRY
        .checked_mul(1 << doublings)
        .unwrap_or(LONGEST_RETRY)
        .min(LONGEST_RETRY);
    last.logged_at.checked_add(wait).is_ok_and(|at| at <= now)
}

/// Whether a request is live: queued, selected or handed over.
pub(crate) fn live(request: &Request) -> bool {
    matches!(
        request.standing,
        Standing::Queued | Standing::Selected | Standing::HandedOver
    )
}

/// Ends every queued pull-request review request of `project` whose commit
/// is not in `owed`: it is never handed over.
pub(crate) fn end_unowed(
    state: &RunnerState,
    project: &ProjectSlug,
    owed: &std::collections::BTreeSet<CommitSha>,
) -> Result<(), StateError> {
    end_queued(state, project, true, owed)
}

/// Ends every queued candidate review request of `project` for a commit not
/// in `owed`.
pub(crate) fn end_unowed_candidates(
    state: &RunnerState,
    project: &ProjectSlug,
    owed: &std::collections::BTreeSet<CommitSha>,
) -> Result<(), StateError> {
    end_queued(state, project, false, owed)
}

fn end_queued(
    state: &RunnerState,
    project: &ProjectSlug,
    pulls: bool,
    owed: &std::collections::BTreeSet<CommitSha>,
) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let sql = state::sql(&path);
        let mut statement = tx
            .prepare(&format!(
                "SELECT a.event_id, a.procedure, a.exact_home, json_extract(e.payload, '$.head')
                 FROM procedure_activations a JOIN events e ON e.id = a.event_id
                 WHERE a.event_type = ?1 AND a.exact_home = ?2
                   AND (json_extract(e.payload, '$.pull_request') IS NOT NULL) = ?3 AND {REPLACEABLE}"
            ))
            .map_err(&sql)?;
        let queued = statement
            .query_map(
                params![
                    instrukt_core::config::REVIEW_NEEDED_EVENT,
                    format!("project:{project}"),
                    pulls
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .map_err(&sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(&sql)?;
        drop(statement);
        for (event, procedure, home, head) in queued {
            if head
                .as_deref()
                .and_then(|head| CommitSha::parse(head).ok())
                .is_some_and(|head| owed.contains(&head))
            {
                continue;
            }
            crate::consumers::end_replaceable(
                tx,
                &event,
                &procedure,
                &home,
                "its commit stopped being owed a review before delivery selected it",
            )?;
            tracing::info!(event = "review_request_ended", project = %project, request = %event, "commit no longer owed");
        }
        Ok(())
    })
}

/// Whether the runner observed canonical head `head` of `project` red: it
/// logged `runner.main_failing` for it.
pub fn observed_red(
    state: &RunnerState,
    project: &ProjectSlug,
    head: &CommitSha,
) -> Result<bool, ReviewReadError> {
    Ok(state
        .read(|conn, path| {
            conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM events WHERE name='runner.main_failing' AND project=?1 AND json_extract(payload, '$.head')=?2)",
                params![project.as_str(), head.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(state::sql(path))
        })?
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        activation: Option<(&str, bool)>,
        handled: bool,
        input: Option<(&str, bool)>,
    ) -> DeliveryRow {
        DeliveryRow {
            activation: activation.map(|(s, r)| (s.to_owned(), r)),
            handled,
            input: input.map(|(s, w)| (s.to_owned(), w)),
        }
    }

    #[test]
    fn an_emitted_request_no_responsibility_has_taken_yet_is_queued() {
        assert_eq!(standing(&row(None, false, None)), Standing::Queued);
    }

    #[test]
    fn a_replaceable_activation_is_queued() {
        assert_eq!(
            standing(&row(
                Some(("pending", true)),
                false,
                Some(("pending", true))
            )),
            Standing::Queued
        );
    }

    #[test]
    fn a_frozen_activation_whose_input_the_runtime_has_not_accepted_is_selected() {
        assert_eq!(
            standing(&row(
                Some(("accepted", false)),
                true,
                Some(("selected", true))
            )),
            Standing::Selected
        );
    }

    #[test]
    fn an_input_the_runtime_accepted_is_handed_over() {
        assert_eq!(
            standing(&row(
                Some(("delivered", false)),
                true,
                Some(("accepted", true))
            )),
            Standing::HandedOver
        );
    }

    #[test]
    fn an_entered_input_whose_turn_ended_has_no_verdict() {
        assert_eq!(
            standing(&row(
                Some(("delivered", false)),
                true,
                Some(("entered", false))
            )),
            Standing::Ended(Disposition::NoVerdict)
        );
    }

    #[test]
    fn an_unconfirmed_handover_ends_unconfirmed() {
        assert_eq!(
            standing(&row(
                Some(("unconfirmed", false)),
                true,
                Some(("unconfirmed", false))
            )),
            Standing::Ended(Disposition::Unconfirmed)
        );
    }

    #[test]
    fn a_superseded_activation_was_never_handed_over() {
        assert_eq!(
            standing(&row(Some(("superseded", false)), true, None)),
            Standing::Ended(Disposition::NeverHandedOver)
        );
    }

    #[test]
    fn an_event_no_responsibility_accepted_was_never_handed_over() {
        assert_eq!(
            standing(&row(None, true, None)),
            Standing::Ended(Disposition::NeverHandedOver)
        );
    }

    fn requested(number: u64, logged_at: &str) -> Request {
        Request {
            number,
            base: CommitSha::parse(&"a".repeat(40)).unwrap(),
            pull_request: Some(1),
            logged_at: logged_at.parse().unwrap(),
            standing: Standing::Ended(Disposition::NoVerdict),
            session: None,
        }
    }

    #[test]
    fn a_commit_never_asked_for_is_asked_for_at_once() {
        assert!(due(&[], "2026-11-24T10:00:00Z".parse().unwrap()));
    }

    #[test]
    fn each_further_request_waits_twice_as_long_as_the_one_before() {
        let second = [
            requested(1, "2026-11-24T10:00:00Z"),
            requested(2, "2026-11-24T10:10:00Z"),
        ];
        assert!(!due(&second, "2026-11-24T10:29:59Z".parse().unwrap()));
        assert!(due(&second, "2026-11-24T10:30:00Z".parse().unwrap()));
    }

    #[test]
    fn no_request_waits_longer_than_the_bound() {
        let late = [requested(9, "2026-11-24T10:00:00Z")];
        assert!(due(&late, "2026-11-24T14:00:00Z".parse().unwrap()));
    }
}
