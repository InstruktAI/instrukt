//! Native notifications for T3's database and WAL; no periodic source reads.
use super::conversations::{ConversationNotifications, SourceError};
use std::path::Path;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn subscribe(path: &Path) -> Result<Box<dyn ConversationNotifications>, SourceError> {
    #[cfg(debug_assertions)]
    if instrukt_core::fsx::fence_active()
        && let Some(path) = std::env::var_os("INSTRUKT_TEST_T3_WAKES")
    {
        return fixture::subscribe(path.into());
    }
    native::subscribe(path)
}

/// No native backend watches T3's database on other systems.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn subscribe(_path: &Path) -> Result<Box<dyn ConversationNotifications>, SourceError> {
    Err(SourceError::Shape(
        "native source notifications are unsupported",
    ))
}

/// A notification source supplied by the out-of-process T3 fixture. Each
/// revision is consumed once, independently of SQLite's filesystem activity.
#[cfg(all(debug_assertions, any(target_os = "macos", target_os = "linux")))]
mod fixture {
    use super::{ConversationNotifications, SourceError};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    struct Fixture {
        path: PathBuf,
        revision: u64,
    }

    pub(super) fn subscribe(
        path: PathBuf,
    ) -> Result<Box<dyn ConversationNotifications>, SourceError> {
        Ok(Box::new(Fixture {
            revision: revision(&path)?,
            path,
        }))
    }

    fn revision(path: &Path) -> Result<u64, SourceError> {
        std::fs::read_to_string(path)
            .map_err(|source| SourceError::Io {
                path: path.to_owned(),
                source,
            })?
            .trim()
            .parse()
            .map_err(|_| SourceError::Shape("invalid fixture notification revision"))
    }

    impl ConversationNotifications for Fixture {
        fn wait(&mut self, timeout: Duration) -> Result<bool, SourceError> {
            std::thread::sleep(timeout);
            let revision = revision(&self.path)?;
            if revision <= self.revision {
                return Ok(false);
            }
            self.revision = revision;
            tracing::debug!(event = "t3_fixture_wake", revision);
            Ok(true)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod native {
    use super::{ConversationNotifications, SourceError};
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

    /// One of the two files watched: T3's database or its WAL.
    struct Watched {
        path: PathBuf,
        registered: bool,
        /// Its registration failed, and has not succeeded since.
        failing: bool,
    }

    struct Notifications {
        receive: Receiver<Result<(), SourceError>>,
        watcher: SourceWatcher,
        files: [Watched; 2],
        /// When the files not registered are tried again; none while both are.
        next_attempt: Option<Instant>,
    }
    impl Notifications {
        /// Registers each file again: one replaced under its name is followed,
        /// and one gone is tried again later.
        fn register_all(&mut self) -> Result<(), SourceError> {
            for file in &mut self.files {
                register(&mut self.watcher, file)?;
            }
            self.schedule();
            Ok(())
        }
        /// Tries the files not registered, when their time has come. Whether one
        /// of them is registered now, which counts as a change.
        fn attempt_due(&mut self) -> Result<bool, SourceError> {
            if self.next_attempt.is_none_or(|at| Instant::now() < at) {
                return Ok(false);
            }
            let mut established = false;
            for file in self.files.iter_mut().filter(|file| !file.registered) {
                established |= register(&mut self.watcher, file)?;
            }
            self.schedule();
            Ok(established)
        }
        fn schedule(&mut self) {
            self.next_attempt = self
                .files
                .iter()
                .any(|file| !file.registered)
                .then(|| Instant::now() + REGISTRATION_RETRY);
        }
    }
    impl ConversationNotifications for Notifications {
        fn wait(&mut self, timeout: Duration) -> Result<bool, SourceError> {
            if self.attempt_due()? {
                return Ok(true);
            }
            let bound = self.next_attempt.map_or(timeout, |at| {
                timeout.min(at.saturating_duration_since(Instant::now()))
            });
            match self.receive.recv_timeout(bound) {
                Ok(result) => {
                    result?;
                    self.register_all()?;
                    Ok(true)
                }
                Err(mpsc::RecvTimeoutError::Timeout) => self.attempt_due(),
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
            next_attempt: None,
        };
        notifications.register_all()?;
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
}
