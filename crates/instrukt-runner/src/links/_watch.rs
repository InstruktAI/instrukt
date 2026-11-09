//! Native database notifications drive the runner's independent relay reader.
use super::{LinkError, failure, relay, status};
use instrukt_core::{
    config::{Configuration, Env},
    home::Home,
};
use std::{sync::mpsc, thread::JoinHandle, time::Duration};
pub(crate) struct Worker {
    stop: mpsc::SyncSender<()>,
    handle: Option<JoinHandle<()>>,
}
impl Worker {
    pub(crate) fn start(home: &Home, env: &Env) -> Result<Self, LinkError> {
        let runtime = Configuration::load_global(home)?
            .runner_runtime()?
            .ok_or(crate::sessions::SessionsError::NoRuntime)?;
        let adapter = crate::runtime::adapter(runtime);
        let source = adapter.conversation_source(home, env)?;
        let mut notifications = source.subscribe()?;
        let home = home.clone();
        let (stop, stopping) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name("conversation-relay".into())
            .spawn(move || {
                loop {
                    if let Err(error) = relay(&home, source.as_ref(), adapter.as_ref()) {
                        record(&home, &error);
                    }
                    loop {
                        if stopping.try_recv().is_ok() {
                            return;
                        }
                        match notifications.wait(Duration::from_millis(100)) {
                            Ok(true) => break,
                            Ok(false) => {}
                            Err(error) => {
                                record(&home, &LinkError::Source(error));
                                return;
                            }
                        }
                    }
                }
            })
            .map_err(|e| LinkError::Refused(format!("relay worker could not start: {e}")))?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
    pub(crate) fn check(&self) -> Result<(), LinkError> {
        if self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            return Err(LinkError::Refused(
                "native conversation notification reader stopped; restart instrukt run".into(),
            ));
        }
        Ok(())
    }
}
pub(crate) fn record(home: &Home, error: &LinkError) {
    tracing::error!(event="conversation_source_failed",error=%error);
    match status(home) {
        Ok(links) => {
            for link in links.into_iter().filter(|link| link.active) {
                if let Err(error) = failure(home, &link.id, None, &error.to_string()) {
                    tracing::error!(event="conversation_failure_record_failed",%error);
                }
            }
        }
        Err(error) => tracing::error!(event="conversation_status_failed",%error),
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Err(error) = self.stop.send(()) {
            tracing::debug!(event="conversation_reader_already_stopped",%error);
        }
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::error!(event = "conversation_reader_panicked");
        }
    }
}
