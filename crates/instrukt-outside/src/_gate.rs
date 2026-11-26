//! The gate: waits per key and per scope, backoff, the time base, and the
//! two operations.
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

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
    /// The gate's default: one second doubling to a minute.
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

    /// The pause after `before` pauses in a row.
    fn pause(self, before: u32) -> Duration {
        self.first
            .saturating_mul(1_u32.checked_shl(before).unwrap_or(u32::MAX))
            .min(self.longest)
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
    fn holds(&self, now: Timestamp, monotonic: Instant) -> bool {
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
            Extent::Key => key.clone(),
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

    /// A try at `key` succeeded: its wait and its scope's end.
    fn succeeded(&self, key: &Key) {
        let monotonic = (self.inner.monotonic)();
        let scope = key.scope_key();
        let ended: Vec<Record> = {
            let mut entries = self.entries();
            [scope, key.clone()]
                .into_iter()
                .filter_map(|level| {
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

        fn attempt(&self, key: &Key, outcome: Outcome) -> Attempted<(), ()> {
            self.gate
                .attempt(key, &Settings::default(), || outcome.tried())
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
        let mut tries = 0;
        let started = Instant::now();
        let named = Named::After {
            delay: Duration::from_secs(3_600),
            received_at: f.wall(),
        };
        let answer = f.gate.call(
            &scope().item("a"),
            &Settings::default(),
            CallerWait::Within(Duration::from_secs(5)),
            &|| false,
            || {
                tries += 1;
                Outcome::Down(Extent::Key, Some(named)).tried()
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
        assert!(started.elapsed() < Duration::from_secs(1));
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
            next_try: start + jiff::SignedDuration::from_secs(1),
        };
        let ended = Record {
            episode: Episode::Ended {
                waited: Duration::from_secs(1 + 2 + 4),
            },
            next_try: start + jiff::SignedDuration::from_secs(4),
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
}
