//! Hosts an adapter's source notifications in the existing runner.
use super::{HistoryError, extract};
use crate::runtime::conversations::ConversationNotifications;
use instrukt_core::{config::Env, home::Home};
use std::{
    sync::mpsc::{self, SyncSender},
    thread::JoinHandle,
    time::Duration,
};

pub(crate) struct Worker {
    stop: SyncSender<()>,
    handle: Option<JoinHandle<()>>,
}
impl Worker {
    pub(super) fn start(home: &Home, env: &Env) -> Result<Self, HistoryError> {
        let source = super::source(home, env)?;
        let notifications = source.subscribe()?;
        Self::spawn(home, env, notifications)
    }
    fn spawn(
        home: &Home,
        env: &Env,
        mut notifications: Box<dyn ConversationNotifications>,
    ) -> Result<Self, HistoryError> {
        let home = home.clone();
        let env = env.clone();
        let (stop, stopping) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name("history-extraction".into())
            .spawn(move || {
                loop {
                    match super::enabled(&home) {
                        Ok(true) => {}
                        Ok(false) => return,
                        Err(error) => {
                            tracing::error!(event="history_enablement_failed", %error);
                            return;
                        }
                    }
                    if let Err(error) = extract(&home, &env) {
                        tracing::error!(event="history_extract_failed",error=%error);
                    }
                    loop {
                        if stopping.try_recv().is_ok() {
                            return;
                        }
                        match notifications.wait(Duration::from_millis(100)) {
                            Ok(true) => break,
                            Ok(false) => {}
                            Err(error) => {
                                tracing::error!(event="history_source_watch_failed",error=%error);
                                return;
                            }
                        }
                    }
                }
            })
            .map_err(|source| HistoryError::Io {
                path: home_path(),
                source,
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
    pub(crate) fn check(&self) -> Result<(), HistoryError> {
        if self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            return Err(HistoryError::Source(
                "history notification worker stopped; restart the runner".into(),
            ));
        }
        Ok(())
    }
}
fn home_path() -> std::path::PathBuf {
    std::path::PathBuf::from("history worker")
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Err(error) = self.stop.send(()) {
            tracing::debug!(event="history_worker_already_stopped",%error);
        }
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::error!(event = "history_worker_panicked");
        }
    }
}
