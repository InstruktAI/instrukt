//! Mechanical thread retention
//! (`project/instrukt/spec/feature/runner/thread-retention`;
//! `project/instrukt/spec/domain/runner`, "Mechanical thread retention"):
//! archiving the finished runtime threads of the registered projects that
//! have been quiet for the computer's retention window. A pass holds no lock,
//! keeps no durable state and emits no event; it reads eligibility again from
//! the runtime, work and the runner's standing bindings every time.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use instrukt_core::config::{Admitted, Runtime};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, SessionRef};
use jiff::{SignedDuration, Timestamp};

use crate::interfaces::{OpenWorkSessions, ProjectRef, RunnerWiring};
use crate::runtime::{ArchiveOutcome, FinishedThread, RuntimeAdapter};
use crate::state::RunnerState;

/// The most archive attempts one pass makes, whatever their outcome, so a
/// backlog drains over successive passes while the runtime keeps serving.
const ARCHIVE_ATTEMPTS_PER_PASS: usize = 20;

/// What a recurring cause is reported against.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Subject {
    /// The pass as a whole: the standing or work answer it needs.
    Pass,
    /// A project whose listing or work answer could not be read.
    Project(ProjectSlug),
    /// A finished thread whose latest activity is unknown.
    Activity(SessionRef),
    /// A thread whose archive failed.
    Archive(SessionRef),
}

/// The causes this process last logged, so a recurring cause is logged when
/// it appears or changes rather than on every pass.
#[derive(Debug, Default)]
pub struct Reported {
    causes: BTreeMap<Subject, String>,
}

impl Reported {
    /// Records `cause` for `subject`; whether it is new or changed.
    fn changed(&mut self, subject: Subject, cause: &str) -> bool {
        if self.causes.get(&subject).map(String::as_str) == Some(cause) {
            return false;
        }
        self.causes.insert(subject, cause.to_owned());
        true
    }

    fn settled(&mut self, subject: &Subject) {
        self.causes.remove(subject);
    }

    /// Forgets the threads no longer listed as finished, so what is kept
    /// stays bounded by what the runtime lists.
    fn retain_threads(&mut self, listed: &BTreeSet<&SessionRef>) {
        self.causes.retain(|subject, _| match subject {
            Subject::Activity(session) | Subject::Archive(session) => listed.contains(session),
            Subject::Pass | Subject::Project(_) => true,
        });
    }
}

/// One retention pass, prepared from an admitted generation; it runs on any
/// thread.
pub struct Pass {
    home: Home,
    runtime: Runtime,
    window: SignedDuration,
    projects: Vec<ProjectRef>,
    work: Option<Arc<dyn OpenWorkSessions>>,
}

impl Pass {
    /// The pass `admitted` asks for: `None` while its retention window is off
    /// or withheld. It covers every published project whose checkout is on
    /// this computer, whatever modules that project switches on.
    pub fn prepare(home: &Home, admitted: &Admitted, wiring: &RunnerWiring) -> Option<Self> {
        let window = admitted.thread_retention()?;
        let runtime = match admitted.global().runner_runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::warn!(event = "thread_retention_withheld", cause = %error);
                return None;
            }
        };
        Some(Self {
            home: home.clone(),
            runtime,
            window,
            projects: admitted
                .projects()
                .iter()
                .map(|project| ProjectRef {
                    slug: project.slug.clone(),
                    root: project.checkout.clone(),
                })
                .collect(),
            work: wiring
                .work
                .as_ref()
                .map(|work| Arc::clone(&work.open_sessions)),
        })
    }

    /// Archives, at `now` on the runner's clock, the longest-quiet eligible
    /// threads up to the per-pass bound, logging recurring causes in
    /// `reported` when they appear or change.
    pub fn run(self, now: Timestamp, reported: &mut Reported) {
        let adapter = crate::runtime::adapter(self.runtime);
        self.run_with(adapter.as_ref(), now, reported);
    }

    fn run_with(&self, adapter: &dyn RuntimeAdapter, now: Timestamp, reported: &mut Reported) {
        let state = RunnerState::new(&self.home);
        let standing = crate::standing::held_sessions(&state, self.runtime.name())
            .map_err(|error| error.to_string());
        let work = self
            .work
            .as_ref()
            .ok_or_else(|| "work's answers are not available in this build".to_owned());
        let (standing, work) = match (standing, work) {
            (Ok(standing), Ok(work)) => (standing, work),
            (Err(cause), _) | (_, Err(cause)) => {
                if reported.changed(Subject::Pass, &cause) {
                    tracing::warn!(event = "thread_retention_withheld", cause = %cause);
                }
                return;
            }
        };
        reported.settled(&Subject::Pass);
        let mut kept: BTreeSet<SessionRef> = standing.into_iter().collect();
        let mut observed = Vec::new();
        for project in &self.projects {
            let subject = Subject::Project(project.slug.clone());
            let answers = adapter
                .finished_threads(&project.root)
                .map_err(|error| error.to_string())
                .and_then(|finished| {
                    work.open_work_sessions(project)
                        .map(|open| (finished, open))
                        .map_err(|error| error.to_string())
                });
            match answers {
                Ok((finished, open)) => {
                    reported.settled(&subject);
                    kept.extend(open);
                    observed.extend(finished.into_iter().map(|thread| (&project.slug, thread)));
                }
                Err(cause) => {
                    if reported.changed(subject, &cause) {
                        tracing::warn!(event = "thread_retention_deferred", project = %project.slug, cause = %cause);
                    }
                }
            }
        }
        let listed: BTreeSet<&SessionRef> = observed.iter().map(|(_, t)| &t.session).collect();
        reported.retain_threads(&listed);
        for (project, thread) in &observed {
            let subject = Subject::Activity(thread.session.clone());
            if thread.latest_activity.is_some() {
                reported.settled(&subject);
            } else if reported.changed(subject, "unknown") {
                tracing::warn!(event = "thread_activity_unknown", project = %project, session = %thread.session);
            }
        }
        // A window reaching before the earliest instant leaves nothing old
        // enough.
        let Ok(cutoff) = now.checked_sub(self.window) else {
            return;
        };
        for (project, thread) in eligible(&observed, &kept, cutoff) {
            let subject = Subject::Archive(thread.session.clone());
            match adapter.archive(thread, now) {
                Ok(ArchiveOutcome::Archived) => {
                    reported.settled(&subject);
                    tracing::info!(
                        event = "thread_archived",
                        project = %project,
                        session = %thread.session,
                        finished_at = ?thread.finished_at,
                        latest_activity = ?thread.latest_activity,
                    );
                }
                Ok(outcome) => {
                    reported.settled(&subject);
                    tracing::debug!(event = "thread_archive_skipped", project = %project, session = %thread.session, outcome = ?outcome);
                }
                Err(error) => {
                    let cause = error.to_string();
                    if reported.changed(subject, &cause) {
                        tracing::warn!(event = "thread_archive_failed", project = %project, session = %thread.session, cause = %cause);
                    }
                }
            }
        }
    }
}

/// The threads of `observed` a pass attempts to archive, longest quiet
/// first, ties by native id, at most [`ARCHIVE_ATTEMPTS_PER_PASS`]: quiet,
/// not pinned, not in `kept`, and both finished and last active at or before
/// `cutoff`. A thread with an unknown time is not eligible.
fn eligible<'a, P>(
    observed: &'a [(P, FinishedThread)],
    kept: &BTreeSet<SessionRef>,
    cutoff: Timestamp,
) -> Vec<&'a (P, FinishedThread)> {
    let mut eligible: Vec<_> = observed
        .iter()
        .filter(|(_, thread)| {
            thread.quiet
                && !thread.pinned
                && !kept.contains(&thread.session)
                && thread.finished_at.is_some_and(|at| at <= cutoff)
                && thread.latest_activity.is_some_and(|at| at <= cutoff)
        })
        .collect();
    eligible.sort_by(|(_, a), (_, b)| {
        a.latest_activity
            .cmp(&b.latest_activity)
            .then_with(|| a.session.id.as_str().cmp(b.session.id.as_str()))
    });
    eligible.truncate(ARCHIVE_ATTEMPTS_PER_PASS);
    eligible
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn thread(id: &str, finished: &str, active: &str) -> ((), FinishedThread) {
        (
            (),
            FinishedThread {
                session: SessionRef::parse(&format!("t3:{id}")).unwrap(),
                finished_at: Some(at(finished)),
                latest_activity: Some(at(active)),
                quiet: true,
                pinned: false,
            },
        )
    }

    fn ids(chosen: &[&((), FinishedThread)]) -> Vec<String> {
        chosen
            .iter()
            .map(|(_, thread)| thread.session.id.to_string())
            .collect()
    }

    const CUTOFF: &str = "2026-11-01T00:00:00Z";

    #[test]
    fn the_longest_quiet_go_first_and_equal_quiet_by_native_id() {
        let observed = [
            thread("c", "2026-10-01T00:00:00Z", "2026-10-03T00:00:00Z"),
            thread("b", "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z"),
            thread("a", "2026-10-01T00:00:00Z", "2026-10-03T00:00:00Z"),
        ];
        let chosen = eligible(&observed, &BTreeSet::new(), at(CUTOFF));
        assert_eq!(ids(&chosen), ["b", "a", "c"]);
    }

    #[test]
    fn a_pass_attempts_no_more_than_its_bound() {
        let observed: Vec<_> = (0..ARCHIVE_ATTEMPTS_PER_PASS + 5)
            .map(|n| {
                thread(
                    &format!("t{n:03}"),
                    "2026-10-01T00:00:00Z",
                    "2026-10-02T00:00:00Z",
                )
            })
            .collect();
        let chosen = eligible(&observed, &BTreeSet::new(), at(CUTOFF));
        assert_eq!(chosen.len(), ARCHIVE_ATTEMPTS_PER_PASS);
    }

    #[test]
    fn a_thread_with_an_unknown_time_is_not_eligible() {
        let (_, mut unknown_activity) = thread("a", "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z");
        unknown_activity.latest_activity = None;
        let (_, mut unknown_finish) = thread("b", "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z");
        unknown_finish.finished_at = None;
        let observed = [((), unknown_activity), ((), unknown_finish)];
        assert!(eligible(&observed, &BTreeSet::new(), at(CUTOFF)).is_empty());
    }

    #[test]
    fn a_thread_active_exactly_at_the_cutoff_is_eligible_and_one_after_it_is_not() {
        let observed = [
            thread("at", "2026-10-01T00:00:00Z", CUTOFF),
            thread("after", "2026-10-01T00:00:00Z", "2026-11-01T00:00:01Z"),
        ];
        let chosen = eligible(&observed, &BTreeSet::new(), at(CUTOFF));
        assert_eq!(ids(&chosen), ["at"]);
    }
}
