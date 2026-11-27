//! The recorded request and pinned base associated with a commit verdict.

use instrukt_core::ids::{CommitSha, SessionRef};
use instrukt_runner::review::{Request, Standing};

use super::_store::Held;

/// The matching request whose number accompanies a first verdict.
pub(super) fn answered_request(requests: &[Request], reviewer: &SessionRef) -> Option<u64> {
    requests.iter().rev().find_map(|request| {
        (request.standing == Standing::HandedOver && request.session.as_ref() == Some(reviewer))
            .then_some(request.number)
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::ContentDigest;
    use crate::work::review::{Moment, Verdict, Version};

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
