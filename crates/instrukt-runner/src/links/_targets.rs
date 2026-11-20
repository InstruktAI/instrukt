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
    changed: bool,
    job: Option<JoinHandle<Result<bool, LinkError>>>,
    retry: Option<Instant>,
    delay: Duration,
}

impl Targets {
    pub(super) fn source_changed(&mut self, home: &Home) {
        let active = match active_link_ids(home) {
            Ok(active) => active,
            Err(error) => {
                super::_watch::record(home, &error);
                let delay = self.next_scan.map_or(FIRST_RETRY, |(_, delay)| delay);
                self.next_scan = Some((Instant::now() + delay, (delay * 2).min(MOST_RETRY)));
                return;
            }
        };
        self.next_scan = None;
        for work in self.links.values_mut() {
            work.active = false;
        }
        for link in active {
            let work = self.links.entry(link).or_insert(LinkWork {
                active: true,
                changed: false,
                job: None,
                retry: None,
                delay: FIRST_RETRY,
            });
            work.active = true;
            work.changed = true;
        }
    }

    pub(super) fn ready(&self) -> bool {
        let now = Instant::now();
        self.next_scan.is_some_and(|(at, _)| at <= now)
            || self.links.values().any(|work| match &work.job {
                Some(job) => job.is_finished(),
                None => work.active && (work.changed || work.retry.is_some_and(|at| at <= now)),
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
            if work.job.is_none()
                && work.active
                && (work.changed || work.retry.is_some_and(|at| at <= now))
            {
                retried |= work.retry.is_some_and(|at| at <= now);
                work.changed = false;
                work.retry = None;
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
    fn completed(&mut self, now: Instant, failed: bool) {
        self.retry = failed.then(|| now + self.delay);
        self.delay = if failed {
            (self.delay * 2).min(MOST_RETRY)
        } else {
            FIRST_RETRY
        };
    }
}
