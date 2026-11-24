//! Schedule link work independently. Each active link has exclusive use of its
//! two targets and keeps its existing lock through native delivery.
use super::{LinkError, active_link_ids, relay_link};
use instrukt_core::{
    config::{Env, Runtime},
    home::Home,
};
use std::{
    collections::BTreeMap,
    thread::JoinHandle,
    time::{Duration, Instant},
};

const FIRST_RETRY: Duration = Duration::from_secs(1);
const MOST_RETRY: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Targets {
    links: BTreeMap<String, LinkWork>,
    failed_scan: Option<FailedScan>,
}

/// A read of which links are active that failed. It is tried again at `at`
/// for the most the source asked meanwhile, and a further failure waits
/// `delay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailedScan {
    at: Instant,
    delay: Duration,
    mark: Mark,
}

struct LinkWork {
    active: bool,
    job: Option<JoinHandle<Result<bool, LinkError>>>,
    standing: Standing,
    delay: Duration,
}

/// What the source asked of a link since its last pass started. A change asks
/// more than a request: it also cuts a failed pass's retry short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mark {
    /// The source asked its readers to read again.
    Request,
    /// The source changed.
    Change,
}

/// A link's standing between its passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    /// No failed pass awaits its retry; what the source asked since the last
    /// pass started, if anything.
    Ready(Option<Mark>),
    /// A failed pass is retried at `at`; only a source change, recorded in
    /// `changed`, starts a pass sooner.
    Retrying { at: Instant, changed: bool },
}

/// Why a link starts a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cause {
    /// The source changed.
    Change,
    /// The retry of a failed pass is due.
    Retry,
    /// The source asked for another read.
    Request,
}

impl Targets {
    pub(super) fn source_changed(&mut self, home: &Home) {
        self.wake(home, Mark::Change);
    }

    /// The source asks for another read: an earlier read may have come
    /// before a commit was readable. Each active link reads again, including
    /// one opened since the source last changed, except one that awaits the
    /// retry of a failed pass, which keeps to that retry.
    pub(super) fn read_again(&mut self, home: &Home) {
        self.wake(home, Mark::Request);
    }

    /// Reads which links are active and marks each one, when a read is due;
    /// otherwise marks the links already known.
    fn wake(&mut self, home: &Home, mark: Mark) {
        let now = Instant::now();
        let Some(mark) = self.scan_for(now, mark) else {
            self.mark_known(mark);
            return;
        };
        let active = active_link_ids(home);
        if let Err(error) = &active {
            super::_watch::record(home, error);
        }
        self.scanned(now, mark, active.ok());
    }

    /// The mark a read of which links are active carries when the source asks
    /// `mark` at `now`, or none while a failed read waits out its delay. A
    /// source change always reads. A read due after a failure carries what
    /// that failure was asked for too, so a request never takes the place of
    /// a change.
    fn scan_for(&self, now: Instant, mark: Mark) -> Option<Mark> {
        match self.failed_scan {
            None => Some(mark),
            Some(failed) if mark == Mark::Change || failed.at <= now => Some(failed.mark.max(mark)),
            Some(_) => None,
        }
    }

    /// The read of which links are active made at `now` for `mark`: the links
    /// it found, or none when it failed, which is tried again after a growing
    /// delay for the same mark.
    fn scanned(&mut self, now: Instant, mark: Mark, active: Option<Vec<String>>) {
        match active {
            Some(active) => {
                self.failed_scan = None;
                self.admit(active, mark);
            }
            None => {
                let failed = self.failed_scan;
                let delay = failed.map_or(FIRST_RETRY, |failed| failed.delay);
                self.failed_scan = Some(FailedScan {
                    at: now + delay,
                    delay: (delay * 2).min(MOST_RETRY),
                    mark: failed.map_or(mark, |failed| failed.mark.max(mark)),
                });
            }
        }
    }

    fn mark_known(&mut self, mark: Mark) {
        for work in self.links.values_mut().filter(|work| work.active) {
            work.mark(mark);
        }
    }

    /// Makes `active` the links served, marking each: a link not yet known is
    /// admitted, and one no longer active is retired once its pass ends.
    fn admit(&mut self, active: Vec<String>, mark: Mark) {
        for work in self.links.values_mut() {
            work.active = false;
        }
        for link in active {
            let work = self.links.entry(link).or_insert_with(LinkWork::new);
            work.active = true;
            work.mark(mark);
        }
    }

    pub(super) fn ready(&self) -> bool {
        let now = Instant::now();
        self.failed_scan.is_some_and(|failed| failed.at <= now)
            || self.links.values().any(|work| match &work.job {
                Some(job) => job.is_finished(),
                None => work.due(now).is_some(),
            })
    }

    pub(super) fn advance(&mut self, home: &Home, env: &Env, runtime: Runtime) -> bool {
        let now = Instant::now();
        let mut retried = false;
        if let Some(failed) = self.failed_scan
            && failed.at <= now
        {
            retried = true;
            self.wake(home, failed.mark);
        }
        self.links.retain(|link, work| {
            if work.job.as_ref().is_some_and(|job| job.is_finished())
                && let Some(job) = work.job.take()
            {
                let failed = match job.join() {
                    Ok(Ok(failed)) => failed,
                    Ok(Err(error)) => {
                        super::_watch::record_link(home, link, &error);
                        true
                    }
                    Err(_) => {
                        let error = LinkError::Refused("link relay worker panicked".into());
                        super::_watch::record_link(home, link, &error);
                        true
                    }
                };
                work.completed(now, failed);
            }
            if let Some(cause) = work.due(now) {
                retried |= cause == Cause::Retry;
                work.started();
                let job = {
                    let home = home.clone();
                    let env = env.clone();
                    let link = link.clone();
                    std::thread::Builder::new()
                        .name("conversation-link".into())
                        .spawn(move || {
                            let adapter = crate::runtime::adapter(runtime);
                            let source = adapter.conversation_source(&home, &env)?;
                            relay_link(&home, source.as_ref(), adapter.as_ref(), &link)
                        })
                };
                match job {
                    Ok(job) => work.job = Some(job),
                    Err(error) => {
                        let error = LinkError::Refused(format!(
                            "link relay worker could not start: {error}"
                        ));
                        super::_watch::record_link(home, link, &error);
                        work.completed(now, true);
                    }
                }
            }
            work.active || work.job.is_some()
        });
        retried
    }
}

impl LinkWork {
    fn new() -> Self {
        Self {
            active: true,
            job: None,
            standing: Standing::Ready(None),
            delay: FIRST_RETRY,
        }
    }
    /// Why this link starts a pass at `now`, if it does. A source change
    /// starts one whatever the link awaits; a request starts none while the
    /// link awaits the retry of a failed pass.
    fn due(&self, now: Instant) -> Option<Cause> {
        if self.job.is_some() || !self.active {
            return None;
        }
        match self.standing {
            Standing::Ready(Some(Mark::Change)) | Standing::Retrying { changed: true, .. } => {
                Some(Cause::Change)
            }
            Standing::Retrying { at, .. } if at <= now => Some(Cause::Retry),
            Standing::Ready(Some(Mark::Request)) => Some(Cause::Request),
            Standing::Ready(None) | Standing::Retrying { .. } => None,
        }
    }
    /// The source asks something of this link. A running pass keeps the mark
    /// for when it ends. A link awaiting a failed pass's retry keeps its
    /// deadline and takes only a change.
    fn mark(&mut self, mark: Mark) {
        match &mut self.standing {
            Standing::Ready(held) => *held = Some(held.map_or(mark, |held| held.max(mark))),
            Standing::Retrying { changed, .. } => *changed |= mark == Mark::Change,
        }
    }
    /// A pass starts: it reads everything the marks asked for.
    fn started(&mut self) {
        self.standing = Standing::Ready(None);
    }
    /// A pass ended. A failed or uncertain one is retried after its delay; it
    /// keeps a change received while it ran and drops a request.
    fn completed(&mut self, now: Instant, failed: bool) {
        if failed {
            self.standing = Standing::Retrying {
                at: now + self.delay,
                changed: self.standing == Standing::Ready(Some(Mark::Change)),
            };
            self.delay = (self.delay * 2).min(MOST_RETRY);
        } else {
            self.delay = FIRST_RETRY;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_starts_a_pass_for_an_idle_link() {
        let now = Instant::now();
        let mut work = LinkWork::new();
        assert_eq!(work.due(now), None);
        work.mark(Mark::Request);
        assert_eq!(work.due(now), Some(Cause::Request));
    }

    #[test]
    fn a_request_starts_none_for_a_link_awaiting_its_retry_and_keeps_the_deadline() {
        let now = Instant::now();
        let mut work = LinkWork::new();
        work.started();
        work.completed(now, true);
        let standing = work.standing;
        work.mark(Mark::Request);
        assert_eq!(work.due(now), None);
        assert_eq!(work.standing, standing);
        assert_eq!(work.due(now + FIRST_RETRY), Some(Cause::Retry));
    }

    #[test]
    fn a_request_received_during_a_pass_is_kept_when_that_pass_succeeds() {
        let now = Instant::now();
        let mut work = LinkWork::new();
        work.mark(Mark::Request);
        work.started();
        work.mark(Mark::Request);
        work.completed(now, false);
        assert_eq!(work.due(now), Some(Cause::Request));
    }

    #[test]
    fn a_request_received_during_a_pass_is_dropped_when_that_pass_fails() {
        let now = Instant::now();
        let mut work = LinkWork::new();
        work.mark(Mark::Request);
        work.started();
        work.mark(Mark::Request);
        work.completed(now, true);
        assert_eq!(work.due(now), None);
        assert_eq!(work.due(now + FIRST_RETRY), Some(Cause::Retry));
    }

    #[test]
    fn a_request_admits_a_link_opened_since_the_source_last_changed() {
        let now = Instant::now();
        let mut targets = Targets::default();
        targets.admit(vec!["known".into()], Mark::Change);
        targets.links.get_mut("known").unwrap().started();
        targets.admit(vec!["known".into(), "opened".into()], Mark::Request);
        assert_eq!(targets.links["opened"].due(now), Some(Cause::Request));
        assert_eq!(targets.links["known"].due(now), Some(Cause::Request));
    }

    #[test]
    fn a_request_retires_a_closed_link_and_keeps_a_failed_links_deadline() {
        let now = Instant::now();
        let mut targets = Targets::default();
        targets.admit(vec!["failed".into(), "closed".into()], Mark::Change);
        for work in targets.links.values_mut() {
            work.started();
        }
        targets
            .links
            .get_mut("failed")
            .unwrap()
            .completed(now, true);
        let standing = targets.links["failed"].standing;
        targets.admit(vec!["failed".into()], Mark::Request);
        assert_eq!(targets.links["closed"].due(now), None);
        assert!(!targets.links["closed"].active);
        assert_eq!(targets.links["failed"].due(now), None);
        assert_eq!(targets.links["failed"].standing, standing);
        assert_eq!(
            targets.links["failed"].due(now + FIRST_RETRY),
            Some(Cause::Retry)
        );
    }

    fn retrying_until(at: Instant) -> LinkWork {
        let mut work = LinkWork::new();
        work.standing = Standing::Retrying { at, changed: false };
        work
    }

    #[test]
    fn a_failed_request_scan_is_retried_as_a_request_and_keeps_a_failed_links_deadline() {
        let now = Instant::now();
        let mut targets = Targets::default();
        targets
            .links
            .insert("failed".into(), retrying_until(now + MOST_RETRY));
        targets.scanned(now, Mark::Request, None);
        let at = targets.failed_scan.unwrap().at;
        let mark = targets.scan_for(at, Mark::Request).unwrap();
        assert_eq!(mark, Mark::Request);
        targets.scanned(at, mark, Some(vec!["failed".into()]));
        assert_eq!(targets.failed_scan, None);
        assert_eq!(targets.links["failed"].due(at), None);
        assert_eq!(
            targets.links["failed"].standing,
            Standing::Retrying {
                at: now + MOST_RETRY,
                changed: false
            }
        );
    }

    #[test]
    fn a_request_never_takes_the_place_of_a_failed_change_scan() {
        let now = Instant::now();
        let mut targets = Targets::default();
        targets
            .links
            .insert("failed".into(), retrying_until(now + MOST_RETRY));
        targets.scanned(now, Mark::Change, None);
        let at = targets.failed_scan.unwrap().at;
        assert_eq!(
            targets.scan_for(at - Duration::from_millis(1), Mark::Request),
            None
        );
        let mark = targets.scan_for(at, Mark::Request).unwrap();
        targets.scanned(at, mark, Some(vec!["failed".into()]));
        assert_eq!(targets.links["failed"].due(at), Some(Cause::Change));
    }

    #[test]
    fn requests_wait_out_the_scan_delay_while_reads_of_active_links_fail() {
        let mut now = Instant::now();
        let mut targets = Targets::default();
        let mut delays = Vec::new();
        for _ in 0..8 {
            targets.scanned(now, Mark::Request, None);
            let at = targets.failed_scan.unwrap().at;
            assert_eq!(
                targets.scan_for(at - Duration::from_millis(1), Mark::Request),
                None
            );
            delays.push((at - now).as_secs());
            now = at;
        }
        assert_eq!(delays, [1, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(targets.scan_for(now, Mark::Request), Some(Mark::Request));
    }

    #[test]
    fn a_due_retry_starts_a_pass_with_no_mark_set() {
        let now = Instant::now();
        let mut work = LinkWork::new();
        work.started();
        work.completed(now, true);
        assert!(matches!(
            work.standing,
            Standing::Retrying { changed: false, .. }
        ));
        assert_eq!(work.due(now + FIRST_RETRY), Some(Cause::Retry));
    }
}
