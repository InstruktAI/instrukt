//! Where the next read of an issue's people's controls starts
//! (`github-record`, Reading GitHub): the comment and event positions a read
//! leaves. A position never passes a control that was read and not yet
//! applied or answered, so a restart or a retry reads it again; controls
//! applied before it are recognized by their identities and never applied
//! twice.

use jiff::Timestamp;

/// One event read from an issue: (page, identity, outstanding).
pub(crate) type EventRead = (u64, u64, bool);

/// How many events one page of an issue's events holds.
pub(crate) const EVENTS_PAGE: u64 = 100;

/// The comment position a read leaves: the latest update time among the
/// comments read, when every control among them was applied or answered;
/// otherwise the latest update time before the first control still
/// outstanding. Never earlier than `current`.
///
/// `read` holds each comment read as (update time, outstanding).
pub(crate) fn next_comments_since(
    current: Option<Timestamp>,
    read: &[(Timestamp, bool)],
) -> Option<Timestamp> {
    let first_outstanding = read
        .iter()
        .filter(|(_, outstanding)| *outstanding)
        .map(|(at, _)| *at)
        .min();
    let passed = read
        .iter()
        .map(|(at, _)| *at)
        .filter(|at| first_outstanding.is_none_or(|first| *at < first))
        .max();
    match (current, passed) {
        (Some(current), Some(passed)) => Some(current.max(passed)),
        (current, passed) => current.or(passed),
    }
}

/// Whether a read of the events from the stored `page` resumes there:
/// that page still holds `last`, the last event read, or it is the first
/// page, where a full read begins anyway. Otherwise the events are read
/// once in full from the first page.
pub(crate) fn resumes_at(page: u64, ids: &[u64], last: Option<u64>) -> bool {
    page <= 1 || last.is_none_or(|last| ids.contains(&last))
}

/// The event position a read leaves, as (page, event): the last event read
/// when no control among them is outstanding, otherwise the last event
/// before the first outstanding one. `current` stays when nothing is
/// passed.
///
/// `read` holds each event read, in order, as (page, identity,
/// outstanding).
pub(crate) fn next_events_position(
    current: (Option<u64>, Option<u64>),
    read: &[EventRead],
) -> (Option<u64>, Option<u64>) {
    read.iter()
        .take_while(|(_, _, outstanding)| !outstanding)
        .last()
        .map_or(current, |(page, id, _)| (Some(*page), Some(*id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn comments_read_with_nothing_outstanding_move_the_position_to_the_latest() {
        let read = [
            (at("2026-11-01T10:00:00Z"), false),
            (at("2026-11-01T12:00:00Z"), false),
        ];
        assert_eq!(
            next_comments_since(Some(at("2026-11-01T09:00:00Z")), &read),
            Some(at("2026-11-01T12:00:00Z"))
        );
    }

    #[test]
    fn an_outstanding_command_stops_the_comment_position_before_it() {
        let read = [
            (at("2026-11-01T10:00:00Z"), false),
            (at("2026-11-01T11:00:00Z"), true),
            (at("2026-11-01T12:00:00Z"), false),
        ];
        assert_eq!(
            next_comments_since(None, &read),
            Some(at("2026-11-01T10:00:00Z"))
        );
        let first = [(at("2026-11-01T11:00:00Z"), true)];
        assert_eq!(
            next_comments_since(Some(at("2026-11-01T09:00:00Z")), &first),
            Some(at("2026-11-01T09:00:00Z"))
        );
        assert_eq!(next_comments_since(None, &first), None);
    }

    #[test]
    fn events_resume_at_a_page_that_still_holds_the_last_event_read() {
        assert!(resumes_at(3, &[300, 301], Some(301)));
        assert!(!resumes_at(3, &[302, 303], Some(301)));
        assert!(resumes_at(1, &[5], Some(4)));
        assert!(resumes_at(2, &[200], None));
    }

    #[test]
    fn the_event_position_stops_before_the_first_outstanding_event() {
        let read = [
            (1, 10, false),
            (1, 11, false),
            (2, 12, true),
            (2, 13, false),
        ];
        assert_eq!(
            next_events_position((None, None), &read),
            (Some(1), Some(11))
        );
        let done = [(1, 10, false), (2, 12, false)];
        assert_eq!(
            next_events_position((None, None), &done),
            (Some(2), Some(12))
        );
        let blocked = [(1, 10, true)];
        assert_eq!(
            next_events_position((Some(1), Some(9)), &blocked),
            (Some(1), Some(9))
        );
    }
}
