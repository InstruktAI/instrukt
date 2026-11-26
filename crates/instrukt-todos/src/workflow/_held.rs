//! What follows a commit the active candidate adds that no row covers
//! (`project/instrukt/spec/feature/workflow/integration-gate`, Review
//! coverage of a candidate): each session that can decide for it is asked,
//! and the candidate owes its own review only when the runtime answered for
//! every named session and no uncovered commit has an author of ours who can
//! decide. Derived from the runner's answers; stored nowhere.

use instrukt_core::ids::SessionRef;
use instrukt_runner::review::{HeldCandidate, HeldCommit, PresenceAnswer, SessionPresence};

use super::UncoveredCommit;

/// The held candidate for `uncovered`, from the runner's answer on each
/// session those commits name. A commit naming no session has no author of
/// ours who can decide.
pub fn held(uncovered: &[UncoveredCommit], presence: &dyn SessionPresence) -> HeldCandidate {
    let mut asks = Vec::new();
    let mut answered = true;
    let commits = uncovered
        .iter()
        .map(|found| {
            let sessions = found
                .sessions
                .iter()
                .map(|session| {
                    let answer = match presence.presence(session) {
                        PresenceAnswer::Held => match SessionRef::parse(session) {
                            Ok(parsed) => {
                                asks.push((found.commit.clone(), parsed));
                                PresenceAnswer::Held
                            }
                            Err(_) => PresenceAnswer::NotHeld,
                        },
                        PresenceAnswer::CannotSay => {
                            answered = false;
                            PresenceAnswer::CannotSay
                        }
                        PresenceAnswer::NotHeld => PresenceAnswer::NotHeld,
                    };
                    (session.clone(), answer)
                })
                .collect();
            HeldCommit {
                commit: found.commit.clone(),
                sessions,
            }
        })
        .collect();
    HeldCandidate {
        commits,
        asks,
        answered,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use instrukt_core::ids::CommitSha;

    use super::*;

    struct Answers(BTreeMap<&'static str, PresenceAnswer>);

    impl SessionPresence for Answers {
        fn presence(&self, session: &str) -> PresenceAnswer {
            self.0
                .get(session)
                .copied()
                .unwrap_or(PresenceAnswer::NotHeld)
        }
    }

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    fn found(c: char, sessions: &[&str]) -> UncoveredCommit {
        UncoveredCommit {
            commit: sha(c),
            sessions: sessions.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn answers(pairs: &[(&'static str, PresenceAnswer)]) -> Answers {
        Answers(pairs.iter().copied().collect())
    }

    #[test]
    fn a_held_author_is_asked_and_the_candidate_owes_no_review() {
        use PresenceAnswer::{Held, NotHeld};
        let held = held(
            &[found('a', &["t3:author"]), found('b', &["t3:gone"])],
            &answers(&[("t3:author", Held), ("t3:gone", NotHeld)]),
        );
        assert_eq!(
            held.asks,
            [(sha('a'), SessionRef::parse("t3:author").unwrap())]
        );
        assert!(held.answered);
        assert!(!held.owes_review());
    }

    #[test]
    fn with_no_author_of_ours_who_can_decide_the_candidate_owes_its_review() {
        use PresenceAnswer::NotHeld;
        let held = held(
            &[found('a', &[]), found('b', &["t3:gone", "not a session"])],
            &answers(&[("t3:gone", NotHeld)]),
        );
        assert!(held.asks.is_empty());
        assert!(held.answered);
        assert!(held.owes_review());
        assert_eq!(held.commits[0].sessions, []);
    }

    #[test]
    fn nothing_is_owed_while_a_session_is_unanswered() {
        use PresenceAnswer::{CannotSay, NotHeld};
        let held = held(
            &[found('a', &["t3:busy-runtime"]), found('b', &["t3:gone"])],
            &answers(&[("t3:busy-runtime", CannotSay), ("t3:gone", NotHeld)]),
        );
        assert!(!held.answered);
        assert!(!held.owes_review());
        assert_eq!(
            held.unanswered().collect::<Vec<_>>(),
            [(&sha('a'), "t3:busy-runtime")]
        );
    }

    #[test]
    fn each_held_session_of_a_commit_is_asked_while_others_are_unanswered() {
        use PresenceAnswer::{CannotSay, Held};
        let held = held(
            &[found('a', &["t3:one", "t3:two", "t3:three"])],
            &answers(&[("t3:one", Held), ("t3:two", Held), ("t3:three", CannotSay)]),
        );
        assert_eq!(
            held.asks,
            [
                (sha('a'), SessionRef::parse("t3:one").unwrap()),
                (sha('a'), SessionRef::parse("t3:two").unwrap())
            ]
        );
        assert!(!held.answered);
        assert!(!held.owes_review());
    }
}
