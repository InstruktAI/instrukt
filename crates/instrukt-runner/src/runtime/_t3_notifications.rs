//! Native notifications for T3's database and WAL. A notification can arrive
//! before the commit it announces is readable: SQLite publishes a commit
//! through its shared-memory index, which raises no file event. So while both
//! files are watched, the source also asks its readers to read again, shortly
//! after each change and then on a doubling delay up to a longest one. A
//! request only becomes due here; a reader receives it at its next wait.
use super::conversations::{ConversationNotifications, SourceError, SourceWake};
use std::path::Path;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn subscribe(path: &Path) -> Result<Box<dyn ConversationNotifications>, SourceError> {
    native::subscribe(path)
}

/// No native backend watches T3's database on other systems.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn subscribe(_path: &Path) -> Result<Box<dyn ConversationNotifications>, SourceError> {
    Err(SourceError::Shape(
        "native source notifications are unsupported",
    ))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod native {
    use super::{ConversationNotifications, SourceError, SourceWake};
    use notify::{EventKind, RecursiveMode, Watcher};
    use std::{
        path::{Path, PathBuf},
        sync::mpsc::{self, Receiver, TrySendError},
        time::{Duration, Instant},
    };

    /// The backend that watches T3's database. kqueue hears writes made through
    /// a file another process keeps open. It must only ever be given files: given
    /// a folder, it takes a descriptor for every entry of that folder and its
    /// subfolders as they change. It is named here rather than taken from
    /// `notify::RecommendedWatcher`, which turns into a backend that does not
    /// hear writes through an open file when notify's `macos_kqueue` feature is
    /// not enabled: with this type named, that does not compile.
    #[cfg(target_os = "macos")]
    type SourceWatcher = notify::KqueueWatcher;
    /// The backend that watches T3's database: inotify, which also hears writes
    /// made through a file another process keeps open.
    #[cfg(target_os = "linux")]
    type SourceWatcher = notify::INotifyWatcher;

    /// How long a file that is not registered waits for its next registration
    /// attempt: the two seconds of history's freshness contract.
    const REGISTRATION_RETRY: Duration = Duration::from_secs(2);
    /// How long after a change the first request to read again becomes due.
    /// It exceeds the time SQLite usually takes to publish a commit after the
    /// file write that announced it, so one request usually reaches the commit.
    const FIRST_REQUEST: Duration = Duration::from_millis(250);
    /// The longest delay between two requests to read again: the most a
    /// reader waits to be asked to read a commit that became readable without
    /// a file event.
    const LONGEST_REQUEST: Duration = Duration::from_secs(2);

    /// When readers are next asked to read again, and the delay that brought
    /// that time. It never ends by itself; it is dropped only while a file is
    /// not registered.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Requests {
        due: Instant,
        delay: Duration,
    }
    impl Requests {
        /// The schedule after a change at `now`: its first request is due
        /// after the shortest delay.
        fn start(now: Instant) -> Self {
            Self {
                due: now + FIRST_REQUEST,
                delay: FIRST_REQUEST,
            }
        }
        /// A change at `now`: the next request is due after the shortest
        /// delay again, whatever the delay had grown to.
        fn restart(&mut self, now: Instant) {
            *self = Self::start(now);
        }
        /// Whether a request is due at `now`. A due request is reported once,
        /// however long ago it became due, and the next is scheduled from
        /// `now` after double the delay, up to the longest.
        fn take(&mut self, now: Instant) -> bool {
            if now < self.due {
                return false;
            }
            self.delay = (self.delay * 2).min(LONGEST_REQUEST);
            self.due = now + self.delay;
            true
        }
    }

    /// One of the two files watched: T3's database or its WAL.
    struct Watched {
        path: PathBuf,
        registered: bool,
        /// Its registration failed, and has not succeeded since.
        failing: bool,
    }

    /// What the producer waits for besides native notifications.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Schedule {
        /// A file is not registered: the files not registered are tried again
        /// at this time, and no reader is asked to read again.
        Registering(Instant),
        /// Both files are registered: readers are asked to read again.
        Requesting(Requests),
    }

    struct Notifications {
        receive: Receiver<Result<(), SourceError>>,
        watcher: SourceWatcher,
        files: [Watched; 2],
        schedule: Schedule,
    }
    impl Notifications {
        /// Registers each file again: one replaced under its name is followed,
        /// and one gone is tried again later.
        fn register_all(&mut self) -> Result<(), SourceError> {
            for file in &mut self.files {
                register(&mut self.watcher, file)?;
            }
            Ok(())
        }
        /// Tries the files not registered, when their time has come. Whether one
        /// of them is registered now, which counts as a change.
        fn attempt_due(&mut self) -> Result<bool, SourceError> {
            let Schedule::Registering(at) = self.schedule else {
                return Ok(false);
            };
            if Instant::now() < at {
                return Ok(false);
            }
            let mut established = false;
            for file in self.files.iter_mut().filter(|file| !file.registered) {
                established |= register(&mut self.watcher, file)?;
            }
            if !established {
                self.schedule = Schedule::Registering(Instant::now() + REGISTRATION_RETRY);
            }
            Ok(established)
        }
        /// A source change: requests to read again start from the shortest
        /// delay when both files are registered; otherwise the files not
        /// registered are tried again later.
        fn changed(&mut self) -> SourceWake {
            let now = Instant::now();
            self.schedule = if self.files.iter().any(|file| !file.registered) {
                Schedule::Registering(now + REGISTRATION_RETRY)
            } else {
                match self.schedule {
                    Schedule::Requesting(mut requests) => {
                        requests.restart(now);
                        Schedule::Requesting(requests)
                    }
                    Schedule::Registering(_) => Schedule::Requesting(Requests::start(now)),
                }
            };
            SourceWake::Changed
        }
    }
    impl ConversationNotifications for Notifications {
        fn wait(&mut self, timeout: Duration) -> Result<SourceWake, SourceError> {
            if self.attempt_due()? {
                return Ok(self.changed());
            }
            let at = match self.schedule {
                Schedule::Registering(at) => at,
                Schedule::Requesting(requests) => requests.due,
            };
            let bound = timeout.min(at.saturating_duration_since(Instant::now()));
            match self.receive.recv_timeout(bound) {
                Ok(result) => {
                    result?;
                    self.register_all()?;
                    Ok(self.changed())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self.attempt_due()? {
                        Ok(self.changed())
                    } else if let Schedule::Requesting(requests) = &mut self.schedule
                        && requests.take(Instant::now())
                    {
                        Ok(SourceWake::ReadAgain)
                    } else {
                        Ok(SourceWake::Timeout)
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    Err(SourceError::Shape("source notification channel closed"))
                }
            }
        }
    }
    pub(super) fn subscribe(
        path: &Path,
    ) -> Result<Box<dyn ConversationNotifications>, SourceError> {
        let (send, receive) = mpsc::sync_channel(1);
        let target = path.to_path_buf();
        let wal = PathBuf::from(format!("{}-wal", path.display()));
        let (matched_target, matched_wal) = (target.clone(), wal.clone());
        let watcher = SourceWatcher::new(
            move |event: notify::Result<notify::Event>| {
                let change = match event {
                    Ok(event)
                        if !matches!(event.kind, EventKind::Access(_))
                            && event
                                .paths
                                .iter()
                                .any(|p| p == &matched_target || p == &matched_wal) =>
                    {
                        Some(Ok(()))
                    }
                    Ok(_) => None,
                    Err(error) => Some(Err(SourceError::Watch(error))),
                };
                if let Some(change) = change {
                    match send.try_send(change) {
                        Ok(()) => {}
                        Err(TrySendError::Full(result)) => {
                            // An already queued change catches up all durable rows; errors remain logged.
                            if let Err(error) = result {
                                tracing::error!(event="t3_source_watch_failed",error=%error);
                            }
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            tracing::debug!(event = "t3_source_notifications_closed")
                        }
                    }
                }
            },
            notify::Config::default(),
        )?;
        let file = |path: PathBuf| Watched {
            path,
            registered: false,
            failing: false,
        };
        let mut notifications = Notifications {
            receive,
            watcher,
            files: [file(target), file(wal)],
            // Replaced by the change below, once registration is known.
            schedule: Schedule::Registering(Instant::now()),
        };
        notifications.register_all()?;
        notifications.changed();
        Ok(Box::new(notifications))
    }
    /// Registers `file`, the one place a path is given to the backend. Whether
    /// it was not registered before and is now. A folder is refused. A file
    /// that does not exist, or whose registration fails, stays unregistered and
    /// is tried again; a failure is reported when it starts and when the
    /// registration succeeds again.
    fn register(watcher: &mut SourceWatcher, file: &mut Watched) -> Result<bool, SourceError> {
        let was = file.registered;
        file.registered = false;
        match std::fs::metadata(&file.path) {
            Ok(meta) if meta.is_dir() => {
                return Err(SourceError::Shape(
                    "a folder is never watched for T3's source",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                failing(file, &error);
                return Ok(false);
            }
        }
        retire(watcher, &file.path)?;
        match watcher.watch(&file.path, RecursiveMode::NonRecursive) {
            Ok(()) => {
                if file.failing {
                    file.failing = false;
                    tracing::info!(event = "t3_source_watch_recovered", path = %file.path.display());
                }
                file.registered = true;
                Ok(!was)
            }
            // The file vanished between the check and the call.
            #[cfg(target_os = "linux")]
            Err(error) if matches!(error.kind, notify::ErrorKind::PathNotFound) => Ok(false),
            Err(error) => {
                failing(file, &error);
                Ok(false)
            }
        }
    }
    /// Marks `file` failing, reporting it once when it starts failing.
    fn failing(file: &mut Watched, error: &dyn std::fmt::Display) {
        if !file.failing {
            file.failing = true;
            tracing::warn!(event = "t3_source_watch_failing", path = %file.path.display(), error = %error);
        }
    }
    /// Retires the registration `path` may still hold, before it is registered
    /// again: inotify keeps the watch of a file renamed away.
    #[cfg(target_os = "linux")]
    fn retire(watcher: &mut SourceWatcher, path: &Path) -> Result<(), SourceError> {
        match watcher.unwatch(path) {
            Ok(()) => Ok(()),
            Err(error) if matches!(error.kind, notify::ErrorKind::WatchNotFound) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    /// Nothing to retire: kqueue closes a name's previous descriptor when the
    /// name is registered again.
    #[cfg(target_os = "macos")]
    fn retire(_watcher: &mut SourceWatcher, _path: &Path) -> Result<(), SourceError> {
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn requests_double_from_the_shortest_delay_up_to_the_longest() {
            let change = Instant::now();
            let mut requests = Requests::start(change);
            assert!(!requests.take(change + FIRST_REQUEST - Duration::from_millis(1)));
            let mut now = change + FIRST_REQUEST;
            let mut delays = Vec::new();
            for _ in 0..6 {
                assert!(requests.take(now));
                delays.push(requests.due - now);
                assert!(!requests.take(requests.due - Duration::from_millis(1)));
                now = requests.due;
            }
            assert_eq!(
                delays,
                [500, 1000, 2000, 2000, 2000, 2000].map(Duration::from_millis)
            );
        }

        #[test]
        fn a_change_returns_requests_to_the_shortest_delay() {
            let change = Instant::now();
            let mut requests = Requests::start(change);
            let mut now = change;
            for _ in 0..4 {
                now = requests.due;
                assert!(requests.take(now));
            }
            assert_eq!(requests.delay, LONGEST_REQUEST);
            let again = now + Duration::from_millis(10);
            requests.restart(again);
            assert_eq!(
                requests,
                Requests {
                    due: again + FIRST_REQUEST,
                    delay: FIRST_REQUEST,
                }
            );
        }

        #[test]
        fn a_request_due_long_ago_is_reported_once_and_the_next_follows_the_report() {
            let change = Instant::now();
            let mut requests = Requests::start(change);
            let late = change + Duration::from_secs(60);
            assert!(requests.take(late));
            assert!(!requests.take(late));
            assert_eq!(requests.due, late + FIRST_REQUEST * 2);
        }
    }
}
