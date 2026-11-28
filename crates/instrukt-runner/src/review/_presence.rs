//! The runner's answer whether the selected runtime holds a named session
//! (`project/instrukt/spec/feature/runner/pull-request-review`, Asking a
//! held candidate's authors): one bounded read of the runtime per distinct
//! session and derivation, then delivery's own decision whether it sends
//! that thread anything. No placement rule is restated here.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;

use instrukt_core::config::Configuration;
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, SessionRef};
use jiff::Timestamp;

use super::{PresenceAnswer, SessionPresence};
use crate::interfaces::SessionBindings;
use crate::runtime::{Presence, RuntimeAdapter};
use crate::sessions::{SessionState, SessionsError};
use crate::state::RunnerState;

/// Presence over the selected runtime's adapter, memoized for one
/// derivation.
pub(crate) struct RuntimePresence<'a> {
    home: &'a Home,
    state: &'a RunnerState,
    work: Option<&'a dyn SessionBindings>,
    project: &'a ProjectSlug,
    now: Timestamp,
    adapter: OnceCell<Result<Box<dyn RuntimeAdapter>, String>>,
    answers: RefCell<HashMap<String, PresenceAnswer>>,
}

impl<'a> RuntimePresence<'a> {
    pub(crate) fn new(
        home: &'a Home,
        state: &'a RunnerState,
        work: Option<&'a dyn SessionBindings>,
        project: &'a ProjectSlug,
        now: Timestamp,
    ) -> Self {
        Self {
            home,
            state,
            work,
            project,
            now,
            adapter: OnceCell::new(),
            answers: RefCell::new(HashMap::new()),
        }
    }

    /// The selected runtime's adapter, as the tick obtains it.
    fn adapter(&self) -> Result<&dyn RuntimeAdapter, &str> {
        self.adapter
            .get_or_init(|| {
                Configuration::load_global(self.home)
                    .and_then(|configuration| configuration.runner_runtime())
                    .map(crate::runtime::adapter)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map(|adapter| adapter.as_ref())
            .map_err(String::as_str)
    }

    fn answer(&self, session: &str) -> PresenceAnswer {
        let Ok(session) = SessionRef::parse(session) else {
            return PresenceAnswer::NotHeld;
        };
        let adapter = match self.adapter() {
            Ok(adapter) => adapter,
            Err(cause) => {
                tracing::warn!(event = "review_presence_runtime_unavailable", project = %self.project, session = %session, error = %cause);
                return PresenceAnswer::CannotSay;
            }
        };
        if session.runtime.as_str() != adapter.runtime().name() {
            return PresenceAnswer::NotHeld;
        }
        let thread = match adapter.read_state(&session) {
            Ok(thread) => thread,
            Err(SessionsError::Gone(_)) => return PresenceAnswer::NotHeld,
            Err(error) => {
                tracing::warn!(event = "review_presence_unreadable", project = %self.project, session = %session, error = %error);
                return PresenceAnswer::CannotSay;
            }
        };
        if thread.state == SessionState::Gone || thread.presence != Presence::Present {
            return PresenceAnswer::NotHeld;
        }
        match crate::keyed::verify_delivery_project(
            self.state, adapter, self.work, &thread, self.now,
        ) {
            Ok(()) => PresenceAnswer::Held,
            Err(SessionsError::ThreadMisplaced { .. }) => PresenceAnswer::NotHeld,
            // Delivery holds the input until the operator restores the
            // project, and has recorded the finding for the operator.
            Err(error) => {
                tracing::debug!(event = "review_presence_delivery_held", project = %self.project, session = %session, error = %error);
                PresenceAnswer::CannotSay
            }
        }
    }
}

impl SessionPresence for RuntimePresence<'_> {
    fn presence(&self, session: &str) -> PresenceAnswer {
        if let Some(answer) = self.answers.borrow().get(session) {
            return *answer;
        }
        let answer = self.answer(session);
        self.answers.borrow_mut().insert(session.to_owned(), answer);
        answer
    }
}
