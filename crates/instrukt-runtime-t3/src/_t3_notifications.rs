//! Native file notifications for T3's cold text mirror (`history` only; never live operational paths).
use std::path::Path;

use instrukt_runtime::cold_text::{ColdTextNotifications, SourceError, SourceWake};

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn subscribe(path: &Path) -> Result<Box<dyn ColdTextNotifications>, SourceError> {
    native::subscribe(path)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn subscribe(_path: &Path) -> Result<Box<dyn ColdTextNotifications>, SourceError> {
    Err(SourceError::Shape(
        "native source notifications are unsupported",
    ))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod native {
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, Receiver, TrySendError};
    use std::time::{Duration, Instant};

    use notify::{EventKind, RecursiveMode, Watcher};

    use super::{ColdTextNotifications, SourceError, SourceWake};

    #[cfg(target_os = "macos")]
    type SourceWatcher = notify::KqueueWatcher;
    #[cfg(target_os = "linux")]
    type SourceWatcher = notify::INotifyWatcher;

    const REGISTRATION_RETRY: Duration = Duration::from_secs(2);
    const FIRST_REQUEST: Duration = Duration::from_millis(250);
    const LONGEST_REQUEST: Duration = Duration::from_secs(2);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Requests {
        due: Instant,
        delay: Duration,
    }

    impl Requests {
        fn start(now: Instant) -> Self {
            Self {
                due: now + FIRST_REQUEST,
                delay: FIRST_REQUEST,
            }
        }

        fn restart(&mut self, now: Instant) {
            *self = Self::start(now);
        }

        fn take(&mut self, now: Instant) -> bool {
            if now < self.due {
                return false;
            }
            self.delay = (self.delay * 2).min(LONGEST_REQUEST);
            self.due = now + self.delay;
            true
        }
    }

    struct Watched {
        path: PathBuf,
        registered: bool,
        failing: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Schedule {
        Registering(Instant),
        Requesting(Requests),
    }

    struct Notifications {
        receive: Receiver<Result<(), SourceError>>,
        watcher: SourceWatcher,
        files: [Watched; 2],
        schedule: Schedule,
    }

    impl Notifications {
        fn register_all(&mut self) -> Result<(), SourceError> {
            for file in &mut self.files {
                register(&mut self.watcher, file)?;
            }
            Ok(())
        }

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

    impl ColdTextNotifications for Notifications {
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

    pub(super) fn subscribe(path: &Path) -> Result<Box<dyn ColdTextNotifications>, SourceError> {
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
            schedule: Schedule::Registering(Instant::now()),
        };
        notifications.register_all()?;
        notifications.changed();
        Ok(Box::new(notifications))
    }

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
            #[cfg(target_os = "linux")]
            Err(error) if matches!(error.kind, notify::ErrorKind::PathNotFound) => Ok(false),
            Err(error) => {
                failing(file, &error);
                Ok(false)
            }
        }
    }

    fn failing(file: &mut Watched, error: &dyn std::fmt::Display) {
        if !file.failing {
            file.failing = true;
            tracing::warn!(event = "t3_source_watch_failing", path = %file.path.display(), error = %error);
        }
    }

    #[cfg(target_os = "linux")]
    fn retire(watcher: &mut SourceWatcher, path: &Path) -> Result<(), SourceError> {
        match watcher.unwatch(path) {
            Ok(()) => Ok(()),
            Err(error) if matches!(error.kind, notify::ErrorKind::WatchNotFound) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

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
