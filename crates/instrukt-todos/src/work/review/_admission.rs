//! The decisions that admit a head verdict (`github-apps`, Admission of
//! review evidence), each a function of facts already read.

use instrukt_core::ids::{CommitSha, SessionRef};
use instrukt_runner::review::{Disposition, Request, Standing, TurnOpening};

use super::_store::Held;

/// Whose review the commit's requests make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RequestCheck {
    /// No request is live: the verdict is a one-off review's.
    Unrequested,
    /// The live request handed over to the reviewing session: it answers it.
    Answers(u64),
    /// A live request is another's, or no session holds it yet.
    Refused {
        number: u64,
        standing: Standing,
        holder: Option<SessionRef>,
    },
}

/// The Request row: while a request is queued, selected or handed over, only
/// the session it was handed over to records the commit's verdict.
pub(super) fn request_check(requests: &[Request], reviewer: &SessionRef) -> RequestCheck {
    let Some(live) = requests.iter().rev().find(|request| {
        matches!(
            request.standing,
            Standing::Queued | Standing::Selected | Standing::HandedOver
        )
    }) else {
        return RequestCheck::Unrequested;
    };
    if live.standing == Standing::HandedOver && live.session.as_ref() == Some(reviewer) {
        return RequestCheck::Answers(live.number);
    }
    RequestCheck::Refused {
        number: live.number,
        standing: live.standing,
        holder: (live.standing == Standing::HandedOver)
            .then(|| live.session.clone())
            .flatten(),
    }
}

/// What the stores say of an offered verdict, before any runtime answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Offer {
    /// No verdict of the commit is held: this is its first.
    First,
    /// A revision admitted from the stores alone.
    Revision,
    /// A revision decided by which input opened the session's turn.
    AskTurn,
    /// Settled by another session's verdict.
    SettledByAnother(SessionRef),
    /// The verdict that stood here was handed off.
    HandedOff(String),
}

/// The Settlement row and the first of its tables.
pub(super) fn settlement(held: &Held, reviewer: &SessionRef, requests: &[Request]) -> Offer {
    let current = match held {
        Held::Nothing => return Offer::First,
        Held::HandedOff { export } => return Offer::HandedOff(export.clone()),
        Held::Stands { current, .. } => current,
    };
    if &current.reviewer != reviewer {
        return Offer::SettledByAnother(current.reviewer.clone());
    }
    let reached = requests.iter().any(|request| {
        request.session.as_ref() == Some(reviewer)
            && matches!(
                request.standing,
                Standing::HandedOver
                    | Standing::Ended(Disposition::NoVerdict | Disposition::Unconfirmed)
            )
    });
    if reached || requests.iter().any(Request::awaits_handover) {
        Offer::AskTurn
    } else {
        Offer::Revision
    }
}

/// What the stores and the commit's requests make of an offered verdict,
/// read in this order: what this computer holds of the commit first. While
/// it holds no verdict the Request row decides whose review it is. Once the
/// commit is settled, or its verdict was handed off, every request of it
/// ended answered with that verdict (`pull-request-review`, One review per
/// commit), so the Request row does not apply, no version answers a request,
/// and the Settlement row alone decides.
pub(super) fn offer(
    held: &Held,
    reviewer: &SessionRef,
    requests: &[Request],
) -> (Offer, RequestCheck) {
    match held {
        Held::Nothing => (Offer::First, request_check(requests, reviewer)),
        Held::Stands { .. } | Held::HandedOff { .. } => (
            settlement(held, reviewer, requests),
            RequestCheck::Unrequested,
        ),
    }
}

/// What the runner's turn answer makes of an offered revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TurnDecision {
    Admit,
    /// A request of this commit opened the turn.
    OpenedByRequest(u64),
    /// The session has no turn in progress.
    NoTurn,
    /// Ask again within the command's wait.
    AskAgain(String),
}

/// The second table of the Settlement row.
pub(super) fn turn_decision(answer: &TurnOpening) -> TurnDecision {
    match answer {
        TurnOpening::Other => TurnDecision::Admit,
        TurnOpening::Request(number) => TurnDecision::OpenedByRequest(*number),
        TurnOpening::NoTurn => TurnDecision::NoTurn,
        TurnOpening::CannotSay(cause) => TurnDecision::AskAgain(cause.clone()),
    }
}

/// The pinned base the Range row fixes: a revision keeps its predecessor's;
/// otherwise the commit's request carries it (the live one, else the latest);
/// a commit that has had no request is pinned at its merge base with the
/// canonical branch.
pub(super) fn pinned_base(
    held: &Held,
    requests: &[Request],
    merge_base: Option<&CommitSha>,
) -> Option<CommitSha> {
    if let Held::Stands { current, .. } = held {
        return Some(current.base.clone());
    }
    requests
        .iter()
        .rev()
        .find(|request| {
            matches!(
                request.standing,
                Standing::Queued | Standing::Selected | Standing::HandedOver
            )
        })
        .or_else(|| requests.last())
        .map(|request| request.base.clone())
        .or_else(|| merge_base.cloned())
}

/// The approving verdict line a review report ends in
/// (`sdlc/procedure/lifecycle/review`, Report format).
const APPROVING_LINE: &str = "Verdict: APPROVE";

/// Whether `report`'s last non-empty line is the approving verdict line.
pub(super) fn approving(report: &str) -> bool {
    report.lines().map(str::trim).rfind(|line| !line.is_empty()) == Some(APPROVING_LINE)
}

/// The hexadecimal runs of `text`: the forms a commit id or a digest is
/// named in, whatever surrounds them.
fn hex_runs(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .filter(|run| !run.is_empty())
}

/// Whether `text` names `commit` as a full commit id.
pub(super) fn names_commit(text: &str, commit: &CommitSha) -> bool {
    hex_runs(text).any(|run| run.eq_ignore_ascii_case(commit.as_str()))
}

/// Whether one message names `head` together with the SHA-256 `digest`
/// (`sha256:<hex>`) of a report.
pub(super) fn names_head_and_digest(message: &str, head: &CommitSha, digest: &str) -> bool {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    names_commit(message, head) && hex_runs(message).any(|run| run.eq_ignore_ascii_case(hex))
}

#[cfg(test)]
mod tests {
    use super::super::{Moment, Verdict, Version};
    use super::*;
    use crate::evidence::ContentDigest;

    fn session(id: &str) -> SessionRef {
        SessionRef::parse(&format!("t3:{id}")).unwrap()
    }

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    fn request(number: u64, standing: Standing, holder: Option<&str>) -> Request {
        Request {
            number,
            base: sha('b'),
            pull_request: Some(1),
            logged_at: jiff::Timestamp::UNIX_EPOCH,
            standing,
            session: holder.map(session),
        }
    }

    fn stands(reviewer: &str) -> Held {
        Held::Stands {
            settlement: 1,
            current: Box::new(Version {
                number: 1,
                verdict: Verdict::Approve,
                name: ContentDigest::of("report"),
                report: "report".into(),
                reviewer: session(reviewer),
                entered_by: None,
                base: sha('c'),
                base_on_canonical: true,
                request: None,
                unattributed: Vec::new(),
                moment: Moment::at(jiff::Timestamp::UNIX_EPOCH),
                recorded_at: jiff::Timestamp::UNIX_EPOCH,
                recorded_on: "here".into(),
            }),
        }
    }

    #[test]
    fn a_commit_without_a_live_request_is_a_one_off_review_whoever_reviews_it() {
        let ended = [request(
            1,
            Standing::Ended(Disposition::NoVerdict),
            Some("other"),
        )];
        assert_eq!(
            request_check(&ended, &session("me")),
            RequestCheck::Unrequested
        );
    }

    #[test]
    fn the_session_a_live_request_was_handed_over_to_answers_it() {
        let live = [request(2, Standing::HandedOver, Some("me"))];
        assert_eq!(
            request_check(&live, &session("me")),
            RequestCheck::Answers(2)
        );
    }

    #[test]
    fn a_queued_request_refuses_every_caller() {
        let live = [request(1, Standing::Queued, None)];
        assert!(matches!(
            request_check(&live, &session("me")),
            RequestCheck::Refused { number: 1, .. }
        ));
    }

    #[test]
    fn a_commit_without_a_verdict_takes_its_first() {
        assert_eq!(
            settlement(&Held::Nothing, &session("me"), &[]),
            Offer::First
        );
    }

    #[test]
    fn a_verdict_settled_by_another_session_refuses_the_offer() {
        assert_eq!(
            settlement(&stands("other"), &session("me"), &[]),
            Offer::SettledByAnother(session("other"))
        );
    }

    #[test]
    fn a_handed_off_verdict_refuses_a_further_version() {
        let held = Held::HandedOff {
            export: "sha256:file".into(),
        };
        assert!(matches!(
            settlement(&held, &session("me"), &[]),
            Offer::HandedOff(_)
        ));
    }

    #[test]
    fn no_request_reached_the_settling_session_and_none_awaits_handover_admits_from_the_stores() {
        let other = [request(
            1,
            Standing::Ended(Disposition::NoVerdict),
            Some("other"),
        )];
        assert_eq!(
            settlement(&stands("me"), &session("me"), &other),
            Offer::Revision
        );
    }

    #[test]
    fn a_request_handed_over_to_the_settling_session_asks_the_turn() {
        let reached = [request(
            1,
            Standing::Ended(Disposition::NoVerdict),
            Some("me"),
        )];
        assert_eq!(
            settlement(&stands("me"), &session("me"), &reached),
            Offer::AskTurn
        );
    }

    #[test]
    fn a_selected_request_still_to_be_handed_over_asks_the_turn() {
        let selected = [request(1, Standing::Selected, None)];
        assert_eq!(
            settlement(&stands("me"), &session("me"), &selected),
            Offer::AskTurn
        );
    }

    #[test]
    fn a_settled_commits_selected_request_leaves_the_settling_session_its_turn_question() {
        let selected = [request(1, Standing::Selected, None)];
        assert_eq!(
            offer(&stands("me"), &session("me"), &selected),
            (Offer::AskTurn, RequestCheck::Unrequested)
        );
    }

    #[test]
    fn a_revision_answers_no_request_the_settled_verdict_already_answered() {
        let handed = [request(1, Standing::HandedOver, Some("me"))];
        assert_eq!(
            offer(&stands("me"), &session("me"), &handed),
            (Offer::AskTurn, RequestCheck::Unrequested)
        );
    }

    #[test]
    fn a_handed_off_verdict_is_refused_with_its_export_whatever_requests_stand() {
        let held = Held::HandedOff {
            export: "sha256:file".into(),
        };
        let queued = [request(1, Standing::Queued, None)];
        assert_eq!(
            offer(&held, &session("me"), &queued),
            (
                Offer::HandedOff("sha256:file".into()),
                RequestCheck::Unrequested
            )
        );
    }

    #[test]
    fn a_commit_without_a_verdict_is_its_live_requests_review() {
        let queued = [request(1, Standing::Queued, None)];
        assert!(matches!(
            offer(&Held::Nothing, &session("me"), &queued),
            (Offer::First, RequestCheck::Refused { number: 1, .. })
        ));
    }

    #[test]
    fn a_turn_something_else_opened_admits_the_revision() {
        assert_eq!(turn_decision(&TurnOpening::Other), TurnDecision::Admit);
    }

    #[test]
    fn a_turn_a_request_of_the_commit_opened_refuses_the_revision() {
        assert_eq!(
            turn_decision(&TurnOpening::Request(1)),
            TurnDecision::OpenedByRequest(1)
        );
    }

    #[test]
    fn a_session_without_a_turn_refuses_the_revision() {
        assert_eq!(turn_decision(&TurnOpening::NoTurn), TurnDecision::NoTurn);
    }

    #[test]
    fn a_runtime_that_cannot_say_is_asked_again() {
        assert!(matches!(
            turn_decision(&TurnOpening::CannotSay("down".into())),
            TurnDecision::AskAgain(_)
        ));
    }

    #[test]
    fn a_report_ending_in_the_approving_verdict_line_approves() {
        assert!(approving("REVIEW COMPLETE: x\n\nVerdict: APPROVE\n\n"));
        assert!(!approving("Verdict: APPROVE\nmore after it\n"));
        assert!(!approving("Verdict: REQUEST CHANGES\n"));
    }

    #[test]
    fn one_message_naming_head_and_digest_together_returns_the_report() {
        let digest = crate::evidence::ContentDigest::of("report");
        let message = format!("Reviewed {}: report {digest}, verdict approve.", sha('a'));
        assert!(names_head_and_digest(&message, &sha('a'), digest.as_str()));
    }

    #[test]
    fn a_message_naming_only_the_head_does_not_return_the_report() {
        let digest = crate::evidence::ContentDigest::of("report");
        let apart = format!("Reviewed {}.", sha('a'));
        assert!(!names_head_and_digest(&apart, &sha('a'), digest.as_str()));
    }

    #[test]
    fn a_revision_keeps_the_pinned_base_of_the_version_it_replaces() {
        let requests = [request(1, Standing::HandedOver, Some("me"))];
        assert_eq!(
            pinned_base(&stands("me"), &requests, Some(&sha('d'))),
            Some(sha('c'))
        );
    }

    #[test]
    fn a_requested_commit_is_pinned_where_its_request_was() {
        let requests = [request(1, Standing::HandedOver, Some("me"))];
        assert_eq!(
            pinned_base(&Held::Nothing, &requests, Some(&sha('d'))),
            Some(sha('b'))
        );
    }

    #[test]
    fn an_unrequested_commit_is_pinned_at_its_merge_base_with_the_canonical_branch() {
        assert_eq!(
            pinned_base(&Held::Nothing, &[], Some(&sha('d'))),
            Some(sha('d'))
        );
    }
}
