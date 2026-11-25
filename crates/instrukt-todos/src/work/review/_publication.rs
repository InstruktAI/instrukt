//! What a verdict looks like on GitHub (`github-apps`, Verdict
//! publication): the status's description, the head of a review's body,
//! the link a status carries, and which of several published verdicts was
//! decided last. Each is read back by comparing identifiers, never prose.

use instrukt_core::ids::CommitSha;

use super::{Moment, Verdict};
use crate::evidence::ContentDigest;

/// The status's context.
pub(crate) const STATUS_CONTEXT: &str = "instrukt/review";

/// The first line of a review body that carries a verdict, before its name.
const BODY_MARK: &str = "instrukt/review";

/// The longest review body this computer writes.
const BODY_LIMIT: usize = 60_000;

/// A verdict as GitHub carries it: its state, name and moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub verdict: Verdict,
    pub name: ContentDigest,
    pub moment: Moment,
}

/// The status description that carries `name` and `moment`.
pub(crate) fn description(name: &ContentDigest, moment: Moment) -> String {
    format!("{name} {moment}")
}

/// The name and moment a status description carries; none for any other
/// text.
pub(crate) fn parse_description(text: &str) -> Option<(ContentDigest, Moment)> {
    let (name, moment) = text.split_once(' ')?;
    let digest = name.strip_prefix("sha256:")?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    Some((ContentDigest::new(name), Moment::parse(moment)?))
}

/// A review body: the verdict's name and moment, a blank line, then the
/// report or the coverage, cut at a line end past the body limit with a
/// last line naming where it is given whole.
pub(crate) fn body(
    name: &ContentDigest,
    moment: Moment,
    content: &str,
    commit: &CommitSha,
) -> String {
    let head = format!("{BODY_MARK} {}\n\n", description(name, moment));
    let closing =
        format!("\n\n(shortened; `instrukt review show {commit}` gives the report whole)\n");
    shortened(&head, content, &closing)
}

/// `head` then `content`, within the body limit: when they do not fit,
/// `content` is cut at a line end so that it and `closing`, the line saying
/// where it is given whole, fit after `head`.
pub(crate) fn shortened(head: &str, content: &str, closing: &str) -> String {
    let room = BODY_LIMIT.saturating_sub(head.chars().count() + closing.chars().count());
    if content.chars().count() <= BODY_LIMIT.saturating_sub(head.chars().count()) {
        return format!("{head}{content}");
    }
    let mut kept = String::new();
    for line in content.split_inclusive('\n') {
        if kept.chars().count() + line.chars().count() > room {
            break;
        }
        kept.push_str(line);
    }
    format!("{head}{}{closing}", kept.trim_end())
}

/// The name and moment at the head of a review body; none for any other
/// body.
pub(crate) fn parse_body(text: &str) -> Option<(ContentDigest, Moment)> {
    let first = text.lines().next()?;
    parse_description(first.strip_prefix(BODY_MARK)?.strip_prefix(' ')?)
}

/// The pull request and review a status's target address names in
/// `repository`, read from its path and fragment; none for an address of
/// anything else.
pub(crate) fn parse_link(address: &str, repository: &str) -> Option<(u64, u64)> {
    let url = url::Url::parse(address).ok()?;
    let review = url
        .fragment()?
        .strip_prefix("pullrequestreview-")?
        .parse::<u64>()
        .ok()?;
    let segments: Vec<&str> = url.path_segments()?.collect();
    let [owner, name, "pull", number] = segments.as_slice() else {
        return None;
    };
    if format!("{owner}/{name}") != repository {
        return None;
    }
    Some((number.parse().ok()?, review))
}

/// Which of `published` was decided last: the later moment; at one moment
/// the refusing verdict, then the name that sorts last.
pub(crate) fn decided_last(published: &[Published]) -> Option<&Published> {
    published.iter().max_by(|a, b| {
        let refusing = |p: &Published| p.verdict == Verdict::NeedsWork;
        (a.moment, refusing(a), &a.name).cmp(&(b.moment, refusing(b), &b.name))
    })
}

/// The status state of a verdict.
pub(crate) fn status_state(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Approve => "success",
        Verdict::NeedsWork => "failure",
    }
}

/// The verdict a status state carries.
pub(crate) fn status_verdict(state: &str) -> Option<Verdict> {
    match state {
        "success" => Some(Verdict::Approve),
        "failure" => Some(Verdict::NeedsWork),
        _ => None,
    }
}

/// The review event that delivers a verdict.
pub(crate) fn review_event(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Approve => "APPROVE",
        Verdict::NeedsWork => "REQUEST_CHANGES",
    }
}

/// The verdict a review's native state carries.
pub(crate) fn review_verdict(state: &str) -> Option<Verdict> {
    match state {
        "APPROVED" => Some(Verdict::Approve),
        "CHANGES_REQUESTED" => Some(Verdict::NeedsWork),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moment(at: &str) -> Moment {
        Moment::at(at.parse().unwrap())
    }

    fn published(verdict: Verdict, report: &str, at: &str) -> Published {
        Published {
            verdict,
            name: ContentDigest::of(report),
            moment: moment(at),
        }
    }

    #[test]
    fn the_later_moment_was_decided_last() {
        let all = [
            published(Verdict::NeedsWork, "a", "2026-11-24T10:00:00Z"),
            published(Verdict::Approve, "b", "2026-11-24T11:00:00Z"),
        ];
        assert_eq!(decided_last(&all), Some(&all[1]));
    }

    #[test]
    fn at_one_moment_the_refusing_verdict_was_decided_last() {
        let all = [
            published(Verdict::NeedsWork, "a", "2026-11-24T10:00:00Z"),
            published(Verdict::Approve, "b", "2026-11-24T10:00:00Z"),
        ];
        assert_eq!(decided_last(&all), Some(&all[0]));
    }

    #[test]
    fn at_one_moment_and_state_the_name_that_sorts_last_was_decided_last() {
        let a = published(Verdict::Approve, "a", "2026-11-24T10:00:00Z");
        let b = published(Verdict::Approve, "b", "2026-11-24T10:00:00Z");
        let last = if a.name > b.name { &a } else { &b };
        let both = [a.clone(), b.clone()];
        assert_eq!(decided_last(&both), Some(last));
    }

    #[test]
    fn a_description_and_a_body_head_carry_the_name_and_moment_back() {
        let name = ContentDigest::of("report");
        let at = moment("2026-11-24T10:00:00.007Z");
        let commit = CommitSha::parse(&"a".repeat(40)).unwrap();
        assert_eq!(
            parse_description(&description(&name, at)),
            Some((name.clone(), at))
        );
        assert_eq!(
            parse_body(&body(&name, at, "the report", &commit)),
            Some((name, at))
        );
    }

    #[test]
    fn a_status_description_stays_within_githubs_limit() {
        let name = ContentDigest::of("report");
        assert!(description(&name, moment("2026-11-24T10:00:00Z")).len() <= 140);
    }

    #[test]
    fn a_long_report_is_shortened_within_the_body_limit() {
        let report = "a line of findings\n".repeat(10_000);
        let commit = CommitSha::parse(&"a".repeat(40)).unwrap();
        let body = body(
            &ContentDigest::of(&report),
            moment("2026-11-24T10:00:00Z"),
            &report,
            &commit,
        );
        assert!(body.chars().count() <= BODY_LIMIT);
    }

    #[test]
    fn a_link_names_the_pull_request_and_review_of_its_repository() {
        assert_eq!(
            parse_link(
                "https://github.test/acme/demo/pull/7#pullrequestreview-1234",
                "acme/demo"
            ),
            Some((7, 1234))
        );
    }

    #[test]
    fn a_link_into_another_repository_names_nothing() {
        assert_eq!(
            parse_link(
                "https://github.test/other/demo/pull/7#pullrequestreview-1234",
                "acme/demo"
            ),
            None
        );
    }
}
