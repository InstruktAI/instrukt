//! The gate: waits per key and per scope, backoff, the time base, and the
//! two operations.
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use backon::{BackoffBuilder as _, ExponentialBuilder};
use jiff::Timestamp;

use crate::_http::Rule;
use crate::_judgment::{Attempted, Extent, Reason, Tried, Unavailable};
use crate::_key::Key;
use crate::_report::{Episode, Record, Report};

/// The host's clock: a named moment holds on it, and every moment is fixed
/// on it.
pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The monotonic source a backoff pause runs on.
pub type Monotonic = Arc<dyn Fn() -> Instant + Send + Sync>;

/// How long a pause in a stop-observing sleep lasts at most.
const PAUSE_SLICE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BackoffError {
    #[error("the first pause must be longer than nothing")]
    ZeroFirst,
    #[error("the first pause must not be longer than the longest")]
    FirstAfterLongest,
}

/// The pauses when the service named no moment: from the first, doubling to
/// the longest, starting again from the first after a success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    first: Duration,
    longest: Duration,
}

impl Backoff {
    /// The gate's backoff for a part that brings none of its own.
    pub const DEFAULT: Self = Self {
        first: Duration::from_secs(1),
        longest: Duration::from_secs(60),
    };

    pub fn new(first: Duration, longest: Duration) -> Result<Self, BackoffError> {
        if first.is_zero() {
            return Err(BackoffError::ZeroFirst);
        }
        if first > longest {
            return Err(BackoffError::FirstAfterLongest);
        }
        Ok(Self { first, longest })
    }

    /// The pause after `before` pauses in a row, computed through `backon`.
    fn pause(self, before: u32) -> Duration {
        ExponentialBuilder::default()
            .with_min_delay(self.first)
            .with_max_delay(self.longest)
            .with_factor(2.0)
            .without_max_times()
            .build()
            .nth(before.min(64) as usize)
            .unwrap_or(self.longest)
    }
}

/// A part's settings for its calls.
#[derive(Clone, Copy)]
pub struct Settings<'a> {
    pub backoff: Backoff,
    /// What counts as temporarily unavailable for this service beyond the
    /// protocol's standard, applied before the standard rules.
    pub rules: &'a [&'a Rule<'a>],
}

impl Default for Settings<'_> {
    fn default() -> Self {
        Self {
            backoff: Backoff::DEFAULT,
            rules: &[],
        }
    }
}

/// How long the caller of [`Gate::call`] waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerWait {
    UntilReturns,
    Within(Duration),
}

/// A wait the gate gave: what is waited on, why, and the moment of the next
/// possible try. Only the gate makes one; clones share it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait(Arc<Waited>);

#[derive(Debug, PartialEq, Eq)]
struct Waited {
    service: &'static str,
    level: String,
    extent: Extent,
    reason: Reason,
    named: bool,
    at: Timestamp,
    until: Instant,
}

impl Wait {
    pub fn reason(&self) -> Reason {
        self.0.reason
    }

    pub fn extent(&self) -> Extent {
        self.0.extent
    }

    /// Whether the service named the moment.
    pub fn named(&self) -> bool {
        self.0.named
    }

    /// The moment of the next try on the host's clock, fixed when the wait
    /// started; what a part displays and persists.
    pub fn at(&self) -> Timestamp {
        self.0.at
    }

    /// Where a pacing part sleeps to on the monotonic source: a pause's
    /// deadline, or for a named moment an estimate taken when the gate
    /// answered, which the next attempt checks again.
    pub fn until(&self) -> Instant {
        self.0.until
    }
}

/// The operator's rendering: the service, the key or scope, the reason and
/// the next try.
impl fmt::Display for Wait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Waited {
            service,
            level,
            reason,
            at,
            ..
        } = &*self.0;
        write!(f, "{service} {level} {reason}; next try at {at}")
    }
}

/// One key's or scope's wait.
struct Entry {
    reason: Reason,
    extent: Extent,
    /// The moment, fixed on the host clock when the wait started.
    at: Timestamp,
    /// A pause's deadline; `None` for a named moment, which holds while the
    /// host clock is before `at`.
    deadline: Option<Instant>,
    /// Pauses in a row, for the backoff.
    pauses: u32,
    /// When the episode started, for the waited duration.
    started: Instant,
}

impl Entry {
    /// Whether the entry holds back the next call at its level: a failure
    /// of only the request that received it holds back nothing.
    fn holds(&self, now: Timestamp, monotonic: Instant) -> bool {
        if self.extent == Extent::Call {
            return false;
        }
        match self.deadline {
            Some(deadline) => monotonic < deadline,
            None => now < self.at,
        }
    }

    fn until(&self, now: Timestamp, monotonic: Instant) -> Instant {
        self.deadline.unwrap_or_else(|| {
            let left = Duration::try_from(self.at.duration_since(now)).unwrap_or_default();
            monotonic.checked_add(left).unwrap_or(monotonic)
        })
    }

    fn record(&self, key: &Key, episode: Episode) -> Record {
        Record {
            episode,
            service: key.service(),
            key: key.to_string(),
            extent: self.extent,
            reason: self.reason,
            next_try: self.at,
        }
    }

    fn wait(&self, key: &Key, now: Timestamp, monotonic: Instant) -> Wait {
        Wait(Arc::new(Waited {
            service: key.service(),
            level: key.to_string(),
            extent: self.extent,
            reason: self.reason,
            named: self.deadline.is_none(),
            at: self.at,
            until: self.until(now, monotonic),
        }))
    }
}

struct Inner {
    clock: Clock,
    monotonic: Monotonic,
    report: Report,
    entries: Mutex<HashMap<Key, Entry>>,
}

/// The outside gate of one host. Clones share its waits.
#[derive(Clone)]
pub struct Gate {
    inner: Arc<Inner>,
}

impl fmt::Debug for Gate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gate").finish_non_exhaustive()
    }
}

impl Gate {
    pub fn new(clock: Clock, monotonic: Monotonic, report: Report) -> Self {
        Self {
            inner: Arc::new(Inner {
                clock,
                monotonic,
                report,
                entries: Mutex::default(),
            }),
        }
    }

    /// The host's clock now: what a part stamps an answer's receipt with.
    pub fn now(&self) -> Timestamp {
        (self.inner.clock)()
    }

    /// One try at most, never a pause: a key or scope inside a wait is
    /// answered with the wait and not tried.
    pub fn attempt<T, E>(
        &self,
        key: &Key,
        settings: &Settings<'_>,
        try_once: impl FnOnce() -> Tried<T, E>,
    ) -> Attempted<T, E> {
        if let Some(wait) = self.waiting(key) {
            return Attempted::Wait { wait, error: None };
        }
        match try_once() {
            Tried::Done(value) => {
                self.succeeded(key);
                Attempted::Done(value)
            }
            Tried::Defect(error) => Attempted::Defect(error),
            Tried::Unavailable(unavailable, error) => Attempted::Wait {
                wait: self.failed(key, settings, unavailable),
                error: Some(error),
            },
        }
    }

    /// For a call whose repetition is safe: tries, and after a temporary
    /// unavailability pauses until the next moment and tries again. Answers
    /// the wait at once when the next moment falls after the caller's wait or
    /// `stop` is asked.
    pub fn call<T, E>(
        &self,
        key: &Key,
        settings: &Settings<'_>,
        caller_wait: CallerWait,
        stop: &dyn Fn() -> bool,
        mut try_once: impl FnMut() -> Tried<T, E>,
    ) -> Attempted<T, E> {
        let monotonic = &self.inner.monotonic;
        let bound = match caller_wait {
            CallerWait::UntilReturns => None,
            CallerWait::Within(within) => Some(monotonic().checked_add(within)),
        };
        loop {
            match self.attempt(key, settings, &mut try_once) {
                Attempted::Wait { wait, error } => {
                    let beyond = bound.is_some_and(|bound| bound.is_none_or(|b| wait.until() > b));
                    if beyond || stop() {
                        return Attempted::Wait { wait, error };
                    }
                    while monotonic() < wait.until() && !stop() {
                        let left = wait.until().saturating_duration_since(monotonic());
                        std::thread::sleep(left.min(PAUSE_SLICE));
                    }
                    if stop() {
                        return Attempted::Wait { wait, error };
                    }
                }
                answered => return answered,
            }
        }
    }

    /// One async try at most, never a pause: a key or scope inside a wait is
    /// answered with the wait and not tried.
    pub async fn attempt_async<T, E, Fut>(
        &self,
        key: &Key,
        settings: &Settings<'_>,
        try_once: impl FnOnce() -> Fut,
    ) -> Attempted<T, E>
    where
        Fut: Future<Output = Tried<T, E>>,
    {
        if let Some(wait) = self.waiting(key) {
            return Attempted::Wait { wait, error: None };
        }
        match try_once().await {
            Tried::Done(value) => {
                self.succeeded(key);
                Attempted::Done(value)
            }
            Tried::Defect(error) => Attempted::Defect(error),
            Tried::Unavailable(unavailable, error) => Attempted::Wait {
                wait: self.failed(key, settings, unavailable),
                error: Some(error),
            },
        }
    }

    /// Async counterpart to [`Gate::call`]: pauses with `tokio::time::sleep`
    /// between tries without blocking a Tokio worker thread, and bounds each
    /// in-flight try when [`CallerWait::Within`] is given.
    pub async fn call_async<T, E, Fut>(
        &self,
        key: &Key,
        settings: &Settings<'_>,
        caller_wait: CallerWait,
        stop: &(dyn Fn() -> bool + Sync),
        mut try_once: impl FnMut() -> Fut,
    ) -> Attempted<T, E>
    where
        Fut: Future<Output = Tried<T, E>>,
    {
        let monotonic = &self.inner.monotonic;
        let bound = match caller_wait {
            CallerWait::UntilReturns => None,
            CallerWait::Within(within) => Some((within, monotonic().checked_add(within))),
        };
        loop {
            let outcome = match bound {
                None => self.attempt_async(key, settings, &mut try_once).await,
                Some((within, _)) if within.is_zero() => {
                    // A zero caller-wait budget performs a single try without
                    // pausing afterward (`project/instrukt/policy/checks-and-guards`).
                    return self.attempt_async(key, settings, &mut try_once).await;
                }
                Some((_, deadline)) => {
                    let remaining = deadline
                        .map_or(Duration::ZERO, |b| b.saturating_duration_since(monotonic()));
                    if remaining.is_zero() {
                        if let Some(wait) = self.waiting(key) {
                            return Attempted::Wait { wait, error: None };
                        }
                        let wait = self.failed(
                            key,
                            settings,
                            Unavailable {
                                reason: Reason::TimedOut,
                                extent: Extent::Call,
                                named: None,
                            },
                        );
                        return Attempted::Wait { wait, error: None };
                    }
                    match tokio::time::timeout(
                        remaining,
                        self.attempt_async(key, settings, &mut try_once),
                    )
                    .await
                    {
                        Ok(outcome) => outcome,
                        Err(_) => {
                            let wait = self.failed(
                                key,
                                settings,
                                Unavailable {
                                    reason: Reason::TimedOut,
                                    extent: Extent::Call,
                                    named: None,
                                },
                            );
                            return Attempted::Wait { wait, error: None };
                        }
                    }
                }
            };
            match outcome {
                Attempted::Wait { wait, error } => {
                    let beyond = bound
                        .is_some_and(|(_, deadline)| deadline.is_none_or(|b| wait.until() > b));
                    if beyond || stop() {
                        return Attempted::Wait { wait, error };
                    }
                    while monotonic() < wait.until() && !stop() {
                        let left = wait.until().saturating_duration_since(monotonic());
                        tokio::time::sleep(left.min(PAUSE_SLICE)).await;
                    }
                    if stop() {
                        return Attempted::Wait { wait, error };
                    }
                }
                answered => return answered,
            }
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<Key, Entry>> {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The wait holding `key` or its scope, the later one when both hold.
    fn waiting(&self, key: &Key) -> Option<Wait> {
        let now = (self.inner.clock)();
        let monotonic = (self.inner.monotonic)();
        let scope = key.scope_key();
        let entries = self.entries();
        [&scope, key]
            .into_iter()
            .filter_map(|level| {
                entries
                    .get(level)
                    .filter(|entry| entry.holds(now, monotonic))
                    .map(|entry| entry.wait(level, now, monotonic))
            })
            .max_by_key(Wait::until)
    }

    fn failed(&self, key: &Key, settings: &Settings<'_>, unavailable: Unavailable) -> Wait {
        let now = (self.inner.clock)();
        let monotonic = (self.inner.monotonic)();
        let level = match unavailable.extent {
            Extent::Key | Extent::Call => key.clone(),
            Extent::Scope => key.scope_key(),
        };
        let (wait, started) = {
            let mut entries = self.entries();
            let started = !entries.contains_key(&level);
            let entry = entries.entry(level.clone()).or_insert(Entry {
                reason: unavailable.reason,
                extent: unavailable.extent,
                at: now,
                deadline: None,
                pauses: 0,
                started: monotonic,
            });
            entry.reason = unavailable.reason;
            entry.extent = unavailable.extent;
            match unavailable.named {
                Some(named) => {
                    entry.at = named.moment();
                    entry.deadline = None;
                }
                None => {
                    let pause = settings.backoff.pause(entry.pauses);
                    entry.pauses = entry.pauses.saturating_add(1);
                    entry.at = jiff::SignedDuration::try_from(pause)
                        .ok()
                        .and_then(|pause| now.checked_add(pause).ok())
                        .unwrap_or(Timestamp::MAX);
                    entry.deadline = Some(monotonic.checked_add(pause).unwrap_or(monotonic));
                }
            }
            let record = started.then(|| entry.record(&level, Episode::Started));
            (entry.wait(&level, now, monotonic), record)
        };
        if let Some(record) = started {
            self.inner.report.emit(&record);
        }
        wait
    }

    /// A try at `key` succeeded: its wait and its scope's end once they no
    /// longer hold. A wait that still holds was recorded after this try was
    /// admitted, by a call that ran beside it, and keeps its moment.
    fn succeeded(&self, key: &Key) {
        let now = (self.inner.clock)();
        let monotonic = (self.inner.monotonic)();
        let scope = key.scope_key();
        let ended: Vec<Record> = {
            let mut entries = self.entries();
            [scope, key.clone()]
                .into_iter()
                .filter_map(|level| {
                    if entries.get(&level)?.holds(now, monotonic) {
                        return None;
                    }
                    let entry = entries.remove(&level)?;
                    Some(entry.record(
                        &level,
                        Episode::Ended {
                            waited: monotonic.saturating_duration_since(entry.started),
                        },
                    ))
                })
                .collect()
        };
        for record in &ended {
            self.inner.report.emit(record);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Backoff, BackoffError, CallerWait, Gate, Settings};
    use crate::_judgment::{Attempted, Extent, Named, Reason, Tried, Unavailable};
    use crate::_key::Key;
    use crate::_report::{Episode, Record, Report};
    use jiff::Timestamp;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A gate on a settable wall clock and monotonic source, reporting to a
    /// collecting sink.
    struct Fixture {
        gate: Gate,
        now: Arc<Mutex<Timestamp>>,
        mono: Arc<Mutex<Instant>>,
        records: Arc<Mutex<Vec<Record>>>,
    }

    impl Fixture {
        fn new() -> Self {
            let now = Arc::new(Mutex::new(
                "2026-11-20T10:00:00Z".parse::<Timestamp>().unwrap(),
            ));
            let mono = Arc::new(Mutex::new(Instant::now()));
            let records = Arc::new(Mutex::new(Vec::new()));
            let (clock, monotonic, sink) = (now.clone(), mono.clone(), records.clone());
            let gate = Gate::new(
                Arc::new(move || *clock.lock().unwrap()),
                Arc::new(move || *monotonic.lock().unwrap()),
                Report::to(move |record| sink.lock().unwrap().push(record.clone())),
            );
            Self {
                gate,
                now,
                mono,
                records,
            }
        }

        fn wall(&self) -> Timestamp {
            *self.now.lock().unwrap()
        }

        fn advance_mono(&self, by: Duration) {
            let mut mono = self.mono.lock().unwrap();
            *mono += by;
        }

        /// Moves the monotonic source to `until`.
        fn reach(&self, until: Instant) {
            let mut mono = self.mono.lock().unwrap();
            *mono = (*mono).max(until);
        }

        fn advance_wall(&self, seconds: i64) {
            let mut now = self.now.lock().unwrap();
            *now += jiff::SignedDuration::from_secs(seconds);
        }

        /// One attempt under the fixture's own backoff ([`backoff`]).
        fn attempt(&self, key: &Key, outcome: Outcome) -> Attempted<(), ()> {
            let settings = Settings {
                backoff: backoff(),
                ..Settings::default()
            };
            self.gate.attempt(key, &settings, || outcome.tried())
        }
    }

    #[derive(Clone, Copy)]
    enum Outcome {
        Done,
        Down(Extent, Option<Named>),
    }

    impl Outcome {
        fn tried(self) -> Tried<(), ()> {
            match self {
                Self::Done => Tried::Done(()),
                Self::Down(extent, named) => Tried::Unavailable(
                    Unavailable {
                        reason: Reason::Unavailable { status: None },
                        extent,
                        named,
                    },
                    (),
                ),
            }
        }
    }

    /// The fixture's backoff: three seconds doubling to twenty.
    fn backoff() -> Backoff {
        Backoff::new(Duration::from_secs(3), Duration::from_secs(20)).unwrap()
    }

    fn scope() -> Key {
        Key::new("svc", ["acme", "one"])
    }

    fn waits(answer: &Attempted<(), ()>) -> bool {
        matches!(answer, Attempted::Wait { .. })
    }

    #[test]
    fn a_scope_wait_holds_every_key_of_it_and_a_key_wait_only_its_own() {
        let f = Fixture::new();
        let (a, b) = (scope().item("a"), scope().item("b"));
        assert!(waits(&f.attempt(&a, Outcome::Down(Extent::Key, None))));
        assert!(matches!(f.attempt(&b, Outcome::Done), Attempted::Done(())));
        assert!(waits(&f.attempt(&b, Outcome::Down(Extent::Scope, None))));
        let other = Key::new("svc", ["acme", "two"]).item("a");
        assert!(matches!(
            f.attempt(&other, Outcome::Done),
            Attempted::Done(())
        ));
        assert!(matches!(
            f.attempt(&scope().item("c"), Outcome::Done),
            Attempted::Wait { error: None, .. }
        ));
    }

    #[test]
    fn a_success_admitted_before_a_named_wait_does_not_end_it() {
        let f = Fixture::new();
        let key = scope().item("a");
        let named = Named::After {
            delay: Duration::from_secs(30),
            received_at: f.wall(),
        };
        // A call beside this one is answered with a named wait while this
        // one is in flight; this one then succeeds.
        let answer = f.gate.attempt(&key, &Settings::default(), || {
            f.attempt(&key, Outcome::Down(Extent::Key, Some(named)));
            Tried::<(), ()>::Done(())
        });
        assert!(matches!(answer, Attempted::Done(())));
        let mut tried = false;
        let next = f.gate.attempt(&key, &Settings::default(), || {
            tried = true;
            Tried::<(), ()>::Done(())
        });
        assert!(!tried);
        assert!(matches!(next, Attempted::Wait { error: None, .. }));
    }

    #[test]
    fn a_failed_call_holds_back_no_call_and_follows_the_backoff() {
        let f = Fixture::new();
        let (a, b) = (scope().item("a"), scope().item("b"));
        let start = *f.mono.lock().unwrap();
        let Attempted::Wait { wait, .. } = f.attempt(&a, Outcome::Down(Extent::Call, None)) else {
            panic!("a wait");
        };
        assert_eq!(wait.until() - start, backoff().first);
        for key in [&a, &b] {
            let mut tried = false;
            let answer = f.gate.attempt(key, &Settings::default(), || {
                tried = true;
                Tried::<(), ()>::Done(())
            });
            assert!(tried);
            assert!(matches!(answer, Attempted::Done(())));
        }
    }

    #[test]
    fn a_failed_call_is_reported_once_on_its_key_with_the_call_extent() {
        let f = Fixture::new();
        let key = scope().item("a");
        let start = f.wall();
        f.attempt(&key, Outcome::Down(Extent::Call, None));
        assert!(matches!(
            f.attempt(&key, Outcome::Done),
            Attempted::Done(())
        ));
        let started = Record {
            episode: Episode::Started,
            service: "svc",
            key: "acme/one a".into(),
            extent: Extent::Call,
            reason: Reason::Unavailable { status: None },
            next_try: start + jiff::SignedDuration::from_secs(3),
        };
        let ended = Record {
            episode: Episode::Ended {
                waited: Duration::ZERO,
            },
            ..started.clone()
        };
        assert_eq!(*f.records.lock().unwrap(), [started, ended]);
    }

    #[test]
    fn backoff_doubles_to_the_longest_and_starts_again_after_a_success() {
        let f = Fixture::new();
        let key = scope().item("a");
        let backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(4)).unwrap();
        let settings = Settings {
            backoff,
            ..Settings::default()
        };
        let mut pauses = Vec::new();
        for outcome in [
            Outcome::Down(Extent::Key, None),
            Outcome::Down(Extent::Key, None),
            Outcome::Down(Extent::Key, None),
            Outcome::Down(Extent::Key, None),
            Outcome::Done,
            Outcome::Down(Extent::Key, None),
        ] {
            let start = *f.mono.lock().unwrap();
            if let Attempted::Wait { wait, .. } =
                f.gate.attempt(&key, &settings, || outcome.tried())
            {
                pauses.push(wait.until() - start);
                f.advance_mono(wait.until() - start);
            }
        }
        let secs = Duration::from_secs;
        assert_eq!(pauses, [secs(1), secs(2), secs(4), secs(4), secs(1)]);
    }

    #[test]
    fn an_attempt_inside_a_wait_does_not_try() {
        let f = Fixture::new();
        let key = scope().item("a");
        f.attempt(&key, Outcome::Down(Extent::Key, None));
        let mut tried = false;
        let answer = f.gate.attempt(&key, &Settings::default(), || {
            tried = true;
            Tried::<(), ()>::Done(())
        });
        assert!(!tried);
        assert!(matches!(answer, Attempted::Wait { error: None, .. }));
    }

    #[test]
    fn a_call_answers_a_wait_past_its_bound_without_pausing() {
        let f = Fixture::new();
        let settings = Settings {
            backoff: Backoff::new(Duration::from_secs(10), Duration::from_secs(10)).unwrap(),
            ..Settings::default()
        };
        // Whenever the call asks whether to stop, the monotonic source moves
        // past any pause and the answer is no: a call that paused would try
        // again at once and succeed, so only the caller's bound answers the
        // wait after one try.
        let far = *f.mono.lock().unwrap() + Duration::from_secs(86_400);
        let mut tries = 0;
        let answer = f.gate.call(
            &scope().item("a"),
            &settings,
            CallerWait::Within(Duration::from_secs(5)),
            &|| {
                f.reach(far);
                false
            },
            || {
                tries += 1;
                if tries == 1 {
                    Outcome::Down(Extent::Key, None).tried()
                } else {
                    Outcome::Done.tried()
                }
            },
        );
        assert!(matches!(
            answer,
            Attempted::Wait {
                error: Some(()),
                ..
            }
        ));
        assert_eq!(tries, 1);
    }

    #[test]
    fn a_call_returns_when_stop_is_asked() {
        let f = Fixture::new();
        let mut tries = 0;
        let answer = f.gate.call(
            &scope().item("a"),
            &Settings::default(),
            CallerWait::UntilReturns,
            &|| true,
            || {
                tries += 1;
                Outcome::Down(Extent::Key, None).tried()
            },
        );
        assert!(matches!(answer, Attempted::Wait { .. }));
        assert_eq!(tries, 1);
    }

    #[test]
    fn a_wait_is_reported_once_when_it_starts_and_once_when_it_ends() {
        let f = Fixture::new();
        let key = scope().item("a");
        let start = f.wall();
        for _ in 0..3 {
            if let Attempted::Wait { wait, .. } =
                f.attempt(&key, Outcome::Down(Extent::Scope, None))
            {
                f.reach(wait.until());
            }
        }
        assert!(matches!(
            f.attempt(&key, Outcome::Done),
            Attempted::Done(())
        ));
        let records = f.records.lock().unwrap().clone();
        let started = Record {
            episode: Episode::Started,
            service: "svc",
            key: "acme/one".into(),
            extent: Extent::Scope,
            reason: Reason::Unavailable { status: None },
            next_try: start + jiff::SignedDuration::from_secs(3),
        };
        let ended = Record {
            episode: Episode::Ended {
                waited: Duration::from_secs(3 + 6 + 12),
            },
            next_try: start + jiff::SignedDuration::from_secs(12),
            ..started.clone()
        };
        assert_eq!(records, [started, ended]);
    }

    #[test]
    fn a_frozen_clock_never_holds_a_pause_open() {
        let f = Fixture::new();
        let key = scope().item("a");
        let Attempted::Wait { wait, .. } = f.attempt(&key, Outcome::Down(Extent::Key, None)) else {
            panic!("a wait");
        };
        f.reach(wait.until());
        assert!(matches!(
            f.attempt(&key, Outcome::Done),
            Attempted::Done(())
        ));
    }

    #[test]
    fn a_named_wait_holds_on_the_host_clock() {
        let f = Fixture::new();
        let key = scope().item("a");
        let named = Named::After {
            delay: Duration::from_secs(30),
            received_at: f.wall(),
        };
        f.attempt(&key, Outcome::Down(Extent::Key, Some(named)));
        f.advance_mono(Duration::from_secs(3_600));
        assert!(waits(&f.attempt(&key, Outcome::Done)));
        f.advance_wall(30);
        assert!(matches!(
            f.attempt(&key, Outcome::Done),
            Attempted::Done(())
        ));
    }

    #[test]
    fn a_named_moment_is_receipt_plus_the_named_seconds() {
        let f = Fixture::new();
        let received_at = f.wall();
        f.advance_wall(7);
        let named = Named::After {
            delay: Duration::from_secs(120),
            received_at,
        };
        let Attempted::Wait { wait, .. } =
            f.attempt(&scope().item("a"), Outcome::Down(Extent::Key, Some(named)))
        else {
            panic!("a wait");
        };
        assert_eq!(
            wait.at(),
            received_at + jiff::SignedDuration::from_secs(120)
        );
        assert!(wait.named());
    }

    #[test]
    fn a_backoff_refuses_a_first_pause_after_the_longest() {
        let secs = Duration::from_secs;
        assert_eq!(
            Backoff::new(secs(5), secs(1)),
            Err(BackoffError::FirstAfterLongest)
        );
        assert_eq!(
            Backoff::new(Duration::ZERO, secs(1)),
            Err(BackoffError::ZeroFirst)
        );
    }

    #[test]
    fn async_gate_retries_and_reports_through_shared_runtime_bridge() {
        let f = Fixture::new();
        let key = scope().item("async-op");
        let settings = Settings {
            backoff: Backoff::new(Duration::from_millis(1), Duration::from_millis(4)).unwrap(),
            ..Settings::default()
        };
        let tries = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let tries_clone = tries.clone();
        let answer = crate::runtime::block_on(f.gate.call_async(
            &key,
            &settings,
            CallerWait::Within(Duration::from_secs(5)),
            &|| {
                f.advance_mono(Duration::from_millis(2));
                false
            },
            || {
                let n = tries_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    if n == 0 {
                        Outcome::Down(Extent::Key, None).tried()
                    } else {
                        Outcome::Done.tried()
                    }
                }
            },
        ))
        .unwrap();
        assert!(matches!(answer, Attempted::Done(())));
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(f.records.lock().unwrap().len(), 2);
    }

    #[test]
    fn call_async_bounds_in_flight_attempt_by_caller_wait() {
        let f = Fixture::new();
        let key = scope().item("slow-op");
        let answer = crate::runtime::block_on(f.gate.call_async(
            &key,
            &Settings::default(),
            CallerWait::Within(Duration::from_millis(20)),
            &|| false,
            || async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Tried::<(), ()>::Done(())
            },
        ))
        .unwrap();
        assert!(matches!(
            answer,
            Attempted::Wait {
                ref wait,
                error: None
            } if wait.reason() == Reason::TimedOut
        ));
    }

    #[test]
    fn call_async_with_zero_caller_wait_tries_once_and_does_not_pause() {
        let f = Fixture::new();
        let key = scope().item("single-try");
        let tries = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let tries_clone = tries.clone();
        let done = crate::runtime::block_on(f.gate.call_async(
            &key,
            &Settings::default(),
            CallerWait::Within(Duration::ZERO),
            &|| false,
            || {
                tries_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { Tried::<(), ()>::Done(()) }
            },
        ))
        .unwrap();
        assert!(matches!(done, Attempted::Done(())));
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 1);

        let wait_key = scope().item("single-try-down");
        let wait_tries = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let wait_tries_clone = wait_tries.clone();
        let waited = crate::runtime::block_on(f.gate.call_async(
            &wait_key,
            &Settings::default(),
            CallerWait::Within(Duration::ZERO),
            &|| false,
            || {
                wait_tries_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { Outcome::Down(Extent::Key, None).tried() }
            },
        ))
        .unwrap();
        assert!(matches!(
            waited,
            Attempted::Wait {
                error: Some(()),
                ..
            }
        ));
        assert_eq!(wait_tries.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
