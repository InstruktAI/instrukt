//! The facts of one record change that fleet attention reads
//! (`fleet-attention`, "The fleet-attention contract"): a todo's hold before
//! and after, whether it left active work, and an attempt it newly records
//! as failed. Which of them are conditions is the runner's.

use instrukt_runner::events::AttentionHoldSource;
use instrukt_runner::fleet_attention::{FailedAttempt, HoldFact, RecordChange};

use super::TodoRecord;
use crate::evidence::{HoldRecord, HoldSource, TodoStatus};

/// What changed in one todo's status, hold or attempts, short of whether
/// runnable work remains in its project, which the transaction answers once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Facts {
    change: RecordChange,
}

impl Facts {
    /// The change, with whether the project has an active todo that is
    /// unheld and not a structural holder after it.
    pub(super) fn with_runnable(mut self, runnable_remains: bool) -> RecordChange {
        self.change.runnable_remains = runnable_remains;
        self.change
    }
}

/// The facts of the change from `before` (`None` for a todo the change
/// creates) to `after`, when its hold, its departure from active work or a
/// newly failed attempt changed; otherwise `None`. Every condition concerns
/// active work, so a change to a todo that is active neither before nor
/// after it has no facts.
pub(super) fn facts(before: Option<&TodoRecord>, after: &TodoRecord) -> Option<Facts> {
    let was_active = before.is_some_and(|record| record.status == TodoStatus::Active);
    if !was_active && after.status != TodoStatus::Active {
        return None;
    }
    let hold_before = before.and_then(|record| record.hold.as_ref()).map(hold);
    let hold_after = after.hold.as_ref().map(hold);
    let left_active = was_active && after.status != TodoStatus::Active;
    let failed_attempt = newly_failed(before, after);
    if hold_before == hold_after && !left_active && failed_attempt.is_none() {
        return None;
    }
    Some(Facts {
        change: RecordChange {
            project: after.project.clone(),
            todo: after.slug.clone(),
            todo_id: after.id.clone(),
            hold_before,
            hold_after,
            left_active,
            failed_attempt,
            runnable_remains: false,
        },
    })
}

fn hold(record: &HoldRecord) -> HoldFact {
    HoldFact {
        source: match record.source {
            HoldSource::Manual => AttentionHoldSource::Manual,
            HoldSource::Automatic => AttentionHoldSource::Automatic,
            HoldSource::GitHub => AttentionHoldSource::GitHub,
        },
        reason: record.reason.clone(),
        placed_at: record.at,
    }
}

/// The attempt `after` newly carries as failed, with the earlier failed
/// attempts of its role on the todo.
fn newly_failed(before: Option<&TodoRecord>, after: &TodoRecord) -> Option<FailedAttempt> {
    let failed_before = |binding: &super::Binding| {
        before.is_some_and(|record| {
            record.bindings.iter().any(|earlier| {
                earlier.role == binding.role
                    && earlier.attempt.number == binding.attempt.number
                    && earlier.attempt.failed_at.is_some()
            })
        })
    };
    let failed = after
        .bindings
        .iter()
        .find(|binding| binding.attempt.failed_at.is_some() && !failed_before(binding))?;
    let earlier_failures = after
        .bindings
        .iter()
        .filter(|binding| {
            binding.role == failed.role
                && binding.attempt.number < failed.attempt.number
                && binding.attempt.failed_at.is_some()
        })
        .count();
    Some(FailedAttempt {
        role: failed.role.clone(),
        attempt: failed.attempt.number,
        reason: failed.attempt.failure_reason.clone().unwrap_or_default(),
        earlier_failures,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test: failed setup fails the test")]

    use instrukt_core::ids::{ProjectSlug, RoleName, TodoSlug};
    use instrukt_runner::interfaces::{CatchUp, TodoKind};

    use super::*;
    use crate::evidence::Actor;
    use crate::work::{Attempt, Binding, TodoId};

    fn record() -> TodoRecord {
        TodoRecord {
            schema: super::super::_model::SCHEMA,
            id: TodoId::parse(&"ab".repeat(16)).unwrap(),
            project: ProjectSlug::parse("demo").unwrap(),
            slug: TodoSlug::parse("parent").unwrap(),
            prior_slugs: Vec::new(),
            kind: TodoKind::Change,
            status: TodoStatus::Active,
            created: "2026-11-03T10:00:00Z".parse().unwrap(),
            created_by: Actor::Person { name: "dev".into() },
            human_involved: None,
            revision: 3,
            sequence: 0,
            artifacts: Vec::new(),
            stamps: Vec::new(),
            claim: None,
            bindings: Vec::new(),
            hold: None,
            hold_history: Vec::new(),
            label_actions: Vec::new(),
            after: Vec::new(),
            group: None,
            schedule: None,
            catch_up: CatchUp::Once,
            agent: None,
            branch: None,
            delivery: None,
            removal: None,
        }
    }

    fn held(source: HoldSource) -> TodoRecord {
        TodoRecord {
            hold: Some(HoldRecord {
                source,
                reason: "waiting".into(),
                by: Actor::Person { name: "dev".into() },
                at: "2026-11-03T11:00:00Z".parse().unwrap(),
            }),
            ..record()
        }
    }

    fn attempt(role: &str, number: u64, failed: bool) -> Binding {
        Binding {
            role: RoleName::parse(role).unwrap(),
            session: None,
            worktree: None,
            attempt: Attempt {
                number,
                failure_reason: failed.then(|| "crashed".to_owned()),
                failed_at: failed.then(|| "2026-11-03T12:00:00Z".parse().unwrap()),
                ..Attempt::default()
            },
        }
    }

    fn with(bindings: Vec<Binding>) -> TodoRecord {
        TodoRecord {
            bindings,
            ..record()
        }
    }

    #[test]
    fn a_held_parent_that_becomes_a_structural_holder_is_released_and_leaves_active_work() {
        let before = held(HoldSource::GitHub);
        let after = TodoRecord {
            status: TodoStatus::StructuralHolder,
            ..record()
        };
        let change = facts(Some(&before), &after).unwrap().with_runnable(false);
        assert_eq!(
            change.hold_before.map(|hold| hold.source),
            Some(AttentionHoldSource::GitHub)
        );
        assert_eq!(change.hold_after, None);
        assert!(change.left_active);
    }

    #[test]
    fn an_unheld_active_todo_that_becomes_a_structural_holder_leaves_active_work() {
        let after = TodoRecord {
            status: TodoStatus::StructuralHolder,
            ..record()
        };
        let change = facts(Some(&record()), &after).unwrap().with_runnable(true);
        assert!(change.left_active);
        assert_eq!((change.hold_before, change.hold_after), (None, None));
    }

    #[test]
    fn a_newly_failed_attempt_counts_only_earlier_failures_of_its_role() {
        let before = with(vec![
            attempt("builder", 1, true),
            attempt("reviewer", 1, true),
            attempt("builder", 2, false),
        ]);
        let after = with(vec![
            attempt("builder", 1, true),
            attempt("reviewer", 1, true),
            attempt("builder", 2, true),
        ]);
        let failed = facts(Some(&before), &after)
            .unwrap()
            .with_runnable(true)
            .failed_attempt
            .unwrap();
        assert_eq!(
            (
                failed.role.as_str(),
                failed.attempt,
                failed.earlier_failures
            ),
            ("builder", 2, 1)
        );
    }

    #[test]
    fn a_change_that_touches_no_hold_status_or_failure_has_no_facts() {
        let before = with(vec![attempt("builder", 1, true)]);
        let after = TodoRecord {
            revision: 4,
            ..before.clone()
        };
        assert_eq!(facts(Some(&before), &after), None);
    }

    #[test]
    fn a_todo_created_active_and_held_places_its_hold() {
        let change = facts(None, &held(HoldSource::Manual))
            .unwrap()
            .with_runnable(false);
        assert_eq!(change.hold_before, None);
        assert_eq!(
            change.hold_after.map(|hold| hold.source),
            Some(AttentionHoldSource::Manual)
        );
        assert!(!change.left_active);
    }

    #[test]
    fn a_hold_placed_or_released_on_an_iceboxed_todo_has_no_facts() {
        let iceboxed = TodoRecord {
            status: TodoStatus::Iceboxed,
            ..record()
        };
        let held = TodoRecord {
            status: TodoStatus::Iceboxed,
            ..held(HoldSource::Manual)
        };
        assert_eq!(facts(Some(&iceboxed), &held), None);
        assert_eq!(facts(Some(&held), &iceboxed), None);
    }
}
