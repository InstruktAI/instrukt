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
    next_scan: Option<(Instant, Duration)>,
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
        self.reconcile(home, Mark::Change);
    }

    /// The source asks for another read: an earlier read may have come
    /// before a commit was readable. Each active link reads again, including
    /// one opened since the source last changed, except one that awaits the
    /// retry of a failed pass, which keeps to that retry.
    pub(super) fn read_again(&mut self, home: &Home) {
        self.reconcile(home, Mark::Request);
    }

    /// Reads which links are active and marks each one. A failed read is
    /// recorded and the scan is tried again after a growing delay.
    fn reconcile(&mut self, home: &Home, mark: Mark) {
        match active_link_ids(home) {
            Ok(active) => {
                self.next_scan = None;
                self.admit(active, mark);
            }
            Err(error) => {
                super::_watch::record(home, &error);
                let delay = self.next_scan.map_or(FIRST_RETRY, |(_, delay)| delay);
                self.next_scan = Some((Instant::now() + delay, (delay * 2).min(MOST_RETRY)));
            }
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
        self.next_scan.is_some_and(|(at, _)| at <= now)
            || self.links.values().any(|work| match &work.job {
                Some(job) => job.is_finished(),
                None => work.due(now).is_some(),
            })
    }

    pub(super) fn advance(&mut self, home: &Home, env: &Env, runtime: Runtime) -> bool {
        let now = Instant::now();
        let mut retried = self.next_scan.is_some_and(|(at, _)| at <= now);
        if retried {
            self.source_changed(home);
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
