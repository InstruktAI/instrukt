//! A held integration candidate at the boundary between workflow's decision
//! and the runner's acts (`project/instrukt/spec/feature/runner/pull-request-review`,
//! Asking a held candidate's authors): the runner's answer whether the
//! selected runtime holds a named session, and what workflow decides from
//! those answers for each commit the candidate adds that is not covered.

use instrukt_core::ids::{CommitSha, SessionRef};

/// Whether the selected runtime holds a named session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceAnswer {
    /// The runtime reports it present, and it is a thread instrukt sends
    /// input to: an author of ours who can decide.
    Held,
    /// No author of ours can decide through it: no session reference,
    /// another runtime, gone, archived or deleted, or a thread delivery
    /// sends nothing.
    NotHeld,
    /// The runtime cannot be reached or asked, or delivery holds the
    /// thread's input until its project is restored.
    CannotSay,
}

/// The runner's fact workflow consults for each session an uncovered commit
/// names.
pub trait SessionPresence {
    /// The answer for `session`, a value a commit's `Session:` trailer
    /// carries.
    fn presence(&self, session: &str) -> PresenceAnswer;
}

/// A commit no row covers, with each session it names and the answer for
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldCommit {
    pub commit: CommitSha,
    pub sessions: Vec<(String, PresenceAnswer)>,
}

/// Workflow's decision on a held candidate (`integration-gate`, Review
/// coverage of a candidate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldCandidate {
    /// Every commit not covered, in the order coverage met them.
    pub commits: Vec<HeldCommit>,
    /// Each commit to be asked of each session that can decide for it.
    pub asks: Vec<(CommitSha, SessionRef)>,
    /// Whether the runtime answered for every session the commits name.
    pub answered: bool,
}

impl HeldCandidate {
    /// Whether the candidate owes its own review: only once every session
    /// is answered and no uncovered commit has an author of ours who can
    /// decide.
    pub fn owes_review(&self) -> bool {
        self.answered && self.asks.is_empty()
    }

    /// The sessions the runtime could not say about.
    pub fn unanswered(&self) -> impl Iterator<Item = (&CommitSha, &str)> {
        self.commits.iter().flat_map(|held| {
            held.sessions
                .iter()
                .filter(|(_, answer)| *answer == PresenceAnswer::CannotSay)
                .map(move |(session, _)| (&held.commit, session.as_str()))
        })
    }
}
