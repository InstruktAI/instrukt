//! Native notifications for T3's database and WAL; no periodic source reads.
use super::conversations::{ConversationNotifications, SourceError};
use notify::{EventKind, RecursiveMode, Watcher};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TrySendError},
    time::Duration,
};

struct Notifications {
    receive: Receiver<Result<(), SourceError>>,
    watcher: notify::RecommendedWatcher,
    watched: PathBuf,
    target: PathBuf,
    wal: PathBuf,
}
impl ConversationNotifications for Notifications {
    fn wait(&mut self, timeout: Duration) -> Result<bool, SourceError> {
        match self.receive.recv_timeout(timeout) {
            Ok(result) => {
                result?;
                let directory = watch_directory(&self.target)?;
                if directory != self.watched {
                    self.watcher
                        .watch(&directory, RecursiveMode::NonRecursive)?;
                    if self.watched.exists() {
                        self.watcher.unwatch(&self.watched)?;
                    }
                    self.watched = directory;
                }
                watch_file(&mut self.watcher, &self.target)?;
                watch_file(&mut self.watcher, &self.wal)?;
                Ok(true)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(SourceError::Shape("source notification channel closed"))
            }
        }
    }
}
pub(super) fn subscribe(path: &Path) -> Result<Box<dyn ConversationNotifications>, SourceError> {
    if matches!(
        notify::RecommendedWatcher::kind(),
        notify::WatcherKind::PollWatcher | notify::WatcherKind::NullWatcher
    ) {
        return Err(SourceError::Shape(
            "native source notifications are unsupported",
        ));
    }
    let (send, receive) = mpsc::sync_channel(1);
    let target = path.to_path_buf();
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    let watched_wal = wal.clone();
    let mut watcher = notify::RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            let change = match event {
                Ok(event)
                    if !matches!(event.kind, EventKind::Access(_))
                        && event
                            .paths
                            .iter()
                            .any(|p| p == &target || p == &wal || target.starts_with(p)) =>
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
    let directory = watch_directory(path)?;
    watcher.watch(&directory, RecursiveMode::NonRecursive)?;
    watch_file(&mut watcher, path)?;
    watch_file(&mut watcher, &watched_wal)?;
    Ok(Box::new(Notifications {
        receive,
        watcher,
        watched: directory,
        target: path.to_path_buf(),
        wal: watched_wal,
    }))
}
fn watch_file(watcher: &mut notify::RecommendedWatcher, path: &Path) -> Result<(), SourceError> {
    if path.exists() {
        match watcher.watch(path, RecursiveMode::NonRecursive) {
            Ok(()) => {}
            // The parent subscription observes replacement if a file disappears during registration.
            Err(error) if matches!(error.kind, notify::ErrorKind::PathNotFound) => {
                tracing::debug!(event = "t3_source_watch_replaced")
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
fn watch_directory(path: &Path) -> Result<PathBuf, SourceError> {
    let mut directory = path
        .parent()
        .ok_or(SourceError::Shape("database has no parent"))?
        .to_path_buf();
    while !directory.exists() {
        if !directory.pop() {
            return Err(SourceError::Shape("watch directory unavailable"));
        }
    }
    Ok(directory)
}
