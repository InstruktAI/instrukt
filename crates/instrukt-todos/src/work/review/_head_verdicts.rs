//! Runner-facing [`instrukt_runner::interfaces::HeadVerdicts`] port adapter
//! over work's review evidence.

use instrukt_core::home::Home;
use instrukt_core::ids::CommitSha;
use instrukt_runner::interfaces::{HeadVerdict, HeadVerdictValue, ProjectRef, VerdictStanding};
use instrukt_runner::state::RunnerState;
use jiff::Timestamp;

use super::{Held, ReviewError, ReviewProject, Verdict, held_here, pinned_base, retention_cutoff};

/// Work's answers a requested review is admitted on
/// (`project/instrukt/spec/feature/runner/requested-review`, Kinds of ask):
/// a head's verdict standing, its pinned base and the review history
/// retention cutoff, read from this home alone, without a lock.
pub struct HeadVerdictWork {
    home: Home,
}

impl HeadVerdictWork {
    pub fn new(home: Home) -> Self {
        Self { home }
    }

    fn answer(&self, project: &ProjectRef, head: &CommitSha) -> Result<HeadVerdict, ReviewError> {
        let state = RunnerState::new(&self.home);
        let requests = instrukt_runner::review::requests(&state, &project.slug, head)?;
        let held = held_here(&self.home, &project.slug, head)?;
        let review_project = ReviewProject {
            slug: project.slug.clone(),
            root: project.root.clone(),
        };
        let (_, pinned_base) = pinned_base(&review_project, head, &held, &requests)?;
        let standing = match held {
            Held::Nothing => VerdictStanding::Nothing,
            Held::Stands { current, .. } => VerdictStanding::Stands {
                verdict: match current.verdict {
                    Verdict::Approve => HeadVerdictValue::Approve,
                    Verdict::NeedsWork => HeadVerdictValue::NeedsWork,
                },
                recorder: current.reviewer.clone(),
            },
            Held::HandedOff { export } => VerdictStanding::HandedOff { export },
        };
        Ok(HeadVerdict {
            standing,
            pinned_base,
        })
    }
}

impl instrukt_runner::interfaces::HeadVerdicts for HeadVerdictWork {
    fn head_verdict(&self, project: &ProjectRef, head: &CommitSha) -> Result<HeadVerdict, String> {
        self.answer(project, head)
            .map_err(|error| error.to_string())
    }

    fn history_cutoff(&self, now: Timestamp) -> Timestamp {
        retention_cutoff(now)
    }
}
