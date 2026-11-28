//! Session-addressed keyed wake (`session-continuity`, Keyed wake): looks up
//! the current runtime binding of a session and delivers input under a fixed
//! command identity without starting a new session or advancing generations.

use std::fmt;
use std::path::PathBuf;

use instrukt_core::ids::SessionRef;
use jiff::Timestamp;

use crate::runtime::{Presence, RuntimeAdapter, RuntimeError};
use crate::sessions::{SessionState, SessionsError};
use crate::state::RunnerState;

use super::KeyGuard;

/// The current binding resolved for a session that may be woken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeTarget {
    pub key: String,
    pub generation: u64,
    pub recorded_project: bool,
    pub checkout: Option<PathBuf>,
}

/// Outcome of resolving a session's current keyed binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeTargetLookup {
    Found(WakeTarget),
    NotBound,
    DifferentGeneration {
        key: String,
        current_generation: u64,
    },
}

/// Resolves the current runtime binding for `session` on `runtime`.
///
/// Refuses when `runtime_bindings` holds more than one binding for `session`.
pub fn wake_target(
    state: &RunnerState,
    runtime: &str,
    session: &SessionRef,
) -> Result<WakeTargetLookup, SessionsError> {
    let mut rows = super::bindings_for_session(state, runtime, session)?;
    match rows.len() {
        0 => Ok(WakeTargetLookup::NotBound),
        1 => {
            let row = rows.remove(0);
            let Some(current) = super::bound(state, runtime, &row.key)? else {
                return Ok(WakeTargetLookup::NotBound);
            };
            if current.generation != row.generation || &current.session != session {
                return Ok(WakeTargetLookup::DifferentGeneration {
                    key: row.key,
                    current_generation: current.generation,
                });
            }
            let checkout = row.checkout_path();
            Ok(WakeTargetLookup::Found(WakeTarget {
                key: row.key,
                generation: row.generation,
                recorded_project: row.project_id.is_some(),
                checkout,
            }))
        }
        _ => {
            let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
            Err(SessionsError::Refused(format!(
                "session {session} is bound to multiple semantic keys: {}",
                keys.join(", ")
            )))
        }
    }
}

/// Why a keyed wake did not deliver its input.
#[derive(Debug)]
pub enum NotWoken {
    NotBound {
        session: SessionRef,
    },
    DifferentGeneration {
        session: SessionRef,
        current_generation: u64,
    },
    Gone {
        session: SessionRef,
    },
    Misplaced {
        session: SessionRef,
        cause: String,
    },
    TurnInProgress {
        session: SessionRef,
    },
    ReadinessUnknown {
        session: SessionRef,
    },
    NotAccepted {
        session: SessionRef,
        error: SessionsError,
    },
}

impl NotWoken {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotBound { .. } => "not_bound",
            Self::DifferentGeneration { .. } => "different_generation",
            Self::Gone { .. } => "gone",
            Self::Misplaced { .. } => "misplaced",
            Self::TurnInProgress { .. } => "turn_in_progress",
            Self::ReadinessUnknown { .. } => "readiness_unknown",
            Self::NotAccepted { .. } => "not_accepted",
        }
    }
}

impl fmt::Display for NotWoken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotBound { session } => write!(f, "session {session} is not bound"),
            Self::DifferentGeneration {
                session,
                current_generation,
            } => write!(
                f,
                "session {session} is no longer the current generation (now generation {current_generation})"
            ),
            Self::Gone { session } => write!(f, "session {session} was archived or deleted"),
            Self::Misplaced { session, cause } => {
                write!(f, "session {session} is misplaced: {cause}")
            }
            Self::TurnInProgress { session } => {
                write!(f, "session {session} has a turn in progress")
            }
            Self::ReadinessUnknown { session } => {
                write!(f, "readiness of session {session} is unknown")
            }
            Self::NotAccepted { session, error } => {
                write!(
                    f,
                    "runtime did not accept wake of session {session}: {error}"
                )
            }
        }
    }
}

/// Outcome of a keyed wake attempt.
#[derive(Debug)]
pub enum WakeOutcome {
    Woken,
    Refused(NotWoken),
}

fn refuse(key: &str, session: &SessionRef, reason: NotWoken) -> WakeOutcome {
    tracing::warn!(
        event = "keyed_wake_refused",
        key = %key,
        session = %session,
        reason = reason.kind(),
        detail = %reason
    );
    WakeOutcome::Refused(reason)
}

fn check_eligibility(
    guard: &KeyGuard,
    adapter: &dyn RuntimeAdapter,
    target: &WakeTarget,
    session: &SessionRef,
) -> Result<Result<(), (NotWoken, Option<SessionsError>)>, SessionsError> {
    if guard.key != target.key {
        return Err(SessionsError::Refused(format!(
            "key guard for `{}` cannot wake target key `{}`",
            guard.key, target.key
        )));
    }
    let Some(current) = super::bound(&guard.state, &guard.runtime, &target.key)? else {
        return Ok(Err((
            NotWoken::NotBound {
                session: session.clone(),
            },
            None,
        )));
    };
    if current.generation != target.generation || &current.session != session {
        return Ok(Err((
            NotWoken::DifferentGeneration {
                session: session.clone(),
                current_generation: current.generation,
            },
            None,
        )));
    }
    let thread = match adapter.read_state(session) {
        Ok(thread) => thread,
        Err(RuntimeError::ThreadNotFound { .. }) => {
            return Ok(Err((
                NotWoken::Gone {
                    session: session.clone(),
                },
                None,
            )));
        }
        Err(error) => return Err(error.into()),
    };
    if thread.presence != Presence::Present || thread.state == SessionState::Gone {
        return Ok(Err((
            NotWoken::Gone {
                session: session.clone(),
            },
            None,
        )));
    }
    let placement = (|| -> Result<(), SessionsError> {
        if target.recorded_project {
            super::verify_recorded_project(&guard.state, &target.key, &thread)?;
        }
        let checkout = target
            .checkout
            .as_deref()
            .ok_or_else(|| super::unverified_project(&target.key, session))?;
        adapter.verify_thread_project(checkout, &thread, Some(&target.key))?;
        Ok(())
    })();
    if let Err(error) = placement {
        return Ok(Err((
            NotWoken::Misplaced {
                session: session.clone(),
                cause: error.to_string(),
            },
            Some(error),
        )));
    }
    match thread.state {
        SessionState::Running | SessionState::NeedsInput => Ok(Err((
            NotWoken::TurnInProgress {
                session: session.clone(),
            },
            None,
        ))),
        SessionState::Gone => Ok(Err((
            NotWoken::Gone {
                session: session.clone(),
            },
            None,
        ))),
        SessionState::InputReady | SessionState::EndedFailed => Ok(Ok(())),
    }
}

/// Inspects whether `session` under `target` is currently eligible for a keyed
/// wake while holding `guard`, without delivering input or recording a
/// placement note.
pub fn check_wake(
    guard: &KeyGuard,
    adapter: &dyn RuntimeAdapter,
    target: &WakeTarget,
    session: &SessionRef,
) -> Result<Option<NotWoken>, SessionsError> {
    match check_eligibility(guard, adapter, target, session)? {
        Ok(()) => Ok(None),
        Err((reason, _)) => Ok(Some(reason)),
    }
}

/// Sends `text` to `session` under `command_id` while holding `guard` for
/// `target.key`. Never starts a session and never advances a generation.
pub fn wake(
    guard: &KeyGuard,
    adapter: &dyn RuntimeAdapter,
    target: &WakeTarget,
    session: &SessionRef,
    command_id: &str,
    text: &str,
    now: Timestamp,
) -> Result<WakeOutcome, SessionsError> {
    if let Err((reason, misplaced)) = check_eligibility(guard, adapter, target, session)? {
        if let Some(error) = misplaced {
            super::note(
                &guard.state,
                adapter.runtime_name(),
                &target.key,
                now,
                Err(&error),
            );
        }
        return Ok(refuse(&target.key, session, reason));
    }
    match adapter.dispatch_send(session, command_id, text, &mut || Ok(())) {
        Ok(_) => Ok(WakeOutcome::Woken),
        Err(error) => Ok(refuse(
            &target.key,
            session,
            NotWoken::NotAccepted {
                session: session.clone(),
                error: error.into_inner().into(),
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{self, StateError};
    use instrukt_core::home::Home;
    use instrukt_core::ids::ProjectSlug;
    use instrukt_core::state_db::rusqlite::params;

    #[test]
    fn is_review_key_for_project_is_prefix_free() {
        let foo = ProjectSlug::parse("foo").unwrap();
        let foo_bar = ProjectSlug::parse("foo-bar").unwrap();
        let head = "a".repeat(40);
        let foo_key = super::super::review_key("foo", &head);
        let foo_bar_key = super::super::review_key("foo-bar", &head);
        let todo = super::super::todo_key("foo", "slug", "builder", 1);
        let standing = super::super::standing_key("proc", "foo");

        assert!(super::super::is_review_key(&foo_key));
        assert!(super::super::is_review_key_for_project(&foo_key, &foo));
        assert!(!super::super::is_review_key_for_project(&foo_bar_key, &foo));
        assert!(super::super::is_review_key_for_project(
            &foo_bar_key,
            &foo_bar
        ));
        assert!(!super::super::is_review_key_for_project(&todo, &foo));
        assert!(!super::super::is_review_key_for_project(&standing, &foo));
    }

    #[test]
    fn wake_target_handles_zero_one_and_multiple_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().join("home"));
        let state = RunnerState::new(&home);
        let session = SessionRef::parse("t3:rev-1").unwrap();

        assert_eq!(
            wake_target(&state, "t3", &session).unwrap(),
            WakeTargetLookup::NotBound
        );

        let key1 = super::super::review_key("demo", &"1".repeat(40));
        state
            .write::<_, StateError>(|tx| {
                tx.execute(
                    "INSERT INTO runtime_bindings(runtime,semantic_key,kind,project,subject,session,display_title,project_id,checkout,accepted,generation)
                     VALUES ('t3',?1,'review','demo',?1,?2,'review','proj-1','/tmp/demo',1,2)",
                    params![key1, session.to_string()],
                )
                .map_err(state::sql(&state::db_path(tx)))?;
                Ok(())
            })
            .unwrap();

        assert_eq!(
            wake_target(&state, "t3", &session).unwrap(),
            WakeTargetLookup::Found(WakeTarget {
                key: key1.clone(),
                generation: 2,
                recorded_project: true,
                checkout: Some(PathBuf::from("/tmp/demo")),
            })
        );

        // A second binding for the same session refuses with conflict.
        let key2 = super::super::review_key("demo", &"2".repeat(40));
        state
            .write::<_, StateError>(|tx| {
                tx.execute(
                    "INSERT INTO runtime_bindings(runtime,semantic_key,kind,project,subject,session,display_title,project_id,checkout,accepted,generation)
                     VALUES ('t3',?1,'review','demo',?1,?2,'review','proj-1','/tmp/demo',1,1)",
                    params![key2, session.to_string()],
                )
                .map_err(state::sql(&state::db_path(tx)))?;
                Ok(())
            })
            .unwrap();

        assert!(matches!(
            wake_target(&state, "t3", &session),
            Err(SessionsError::Refused(_))
        ));
    }
}
