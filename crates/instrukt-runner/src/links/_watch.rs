//! Native database notifications drive the runner's independent relay reader.
use super::{LinkError, failure, relay, status};
use instrukt_core::{
    config::{Configuration, Env},
    home::Home,
};
use std::{sync::mpsc, thread::JoinHandle, time::Duration};
pub(crate) struct Worker {
    stop: mpsc::SyncSender<()>,
    handle: Option<JoinHandle<Result<(), LinkError>>>,
    failure: Option<LinkError>,
    home: Home,
}
/// The connector destination the relay serves beside links: its endpoint,
/// the intake's wake-ups, and the run loop's supervision notice.
#[derive(Clone)]
pub(crate) struct Destinations {
    pub(crate) slack: crate::connectors::SlackEndpoint,
    pub(crate) arrivals: std::sync::Arc<std::sync::Mutex<mpsc::Receiver<()>>>,
}
/// Wakes the run loop's supervision when the relay ends, whether it
/// returned or unwound.
struct ExitNotice(Option<mpsc::SyncSender<()>>);
impl Drop for ExitNotice {
    fn drop(&mut self) {
        if let Some(notice) = &self.0 {
            let _ = notice.try_send(());
        }
    }
}
impl Worker {
    /// The relay serving links and, when given, the connector destination.
    pub(crate) fn start(
        home: &Home,
        env: &Env,
        destinations: Option<Destinations>,
        supervise: Option<mpsc::SyncSender<()>>,
    ) -> Result<Self, LinkError> {
        let runtime = Configuration::load_global(home)?
            .runner_runtime()?
            .ok_or(crate::sessions::SessionsError::NoRuntime)?;
        let adapter = crate::runtime::adapter(runtime);
        let source = adapter.conversation_source(home, env)?;
        let mut notifications = source.subscribe()?;
        let owner_home = home.clone();
        let home = home.clone();
        let (stop, stopping) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name("conversation-relay".into())
            .spawn(move || {
                let _notice = ExitNotice(supervise);
                // Links catch up at startup and on native source changes only;
                // connector arrivals and the retry bound drive conversation
                // work alone.
                let mut links_due = true;
                loop {
                    if links_due {
                        if let Err(error) = relay(&home, source.as_ref(), adapter.as_ref()) {
                            record(&home, &error);
                        }
                        links_due = false;
                    }
                    if let Some(destinations) = &destinations
                        && let Err(error) = crate::conversations::relay(
                            &home,
                            source.as_ref(),
                            adapter.as_ref(),
                            &destinations.slack,
                        )
                    {
                        tracing::warn!(event = "conversation_relay_failed", error = %error);
                    }
                    // Retained connector work that is not yet done is retried
                    // within a bound; otherwise only a native change or new
                    // intake wakes the relay.
                    let retry = destinations.is_some()
                        && crate::conversations::outstanding(&home).unwrap_or(true);
                    let until = std::time::Instant::now() + Duration::from_secs(2);
                    loop {
                        if stopping.try_recv().is_ok() {
                            return Ok(());
                        }
                        if destinations.as_ref().is_some_and(|d| {
                            d.arrivals
                                .lock()
                                .is_ok_and(|arrivals| arrivals.try_recv().is_ok())
                        }) || (retry && std::time::Instant::now() >= until)
                        {
                            break;
                        }
                        match notifications.wait(Duration::from_millis(100)) {
                            Ok(true) => {
                                links_due = true;
                                break;
                            }
                            Ok(false) => {}
                            Err(error) => {
                                return Err(LinkError::Source(error));
                            }
                        }
                    }
                }
            })
            .map_err(|e| LinkError::Refused(format!("relay worker could not start: {e}")))?;
        Ok(Self {
            stop,
            handle: Some(handle),
            failure: None,
            home: owner_home,
        })
    }
    pub(crate) fn check(&mut self) -> Result<(), LinkError> {
        if self.handle.as_ref().is_some_and(|h| h.is_finished())
            && let Some(handle) = self.handle.take()
        {
            self.failure = match handle.join() {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(LinkError::Stopped(Box::new(error))),
                Err(_) => Some(LinkError::Refused(
                    "native conversation notification reader stopped; restart instrukt run".into(),
                )),
            };
        }
        observe_stopped(&mut self.failure)
    }
}
fn observe_stopped(failure: &mut Option<LinkError>) -> Result<(), LinkError> {
    failure.take().map_or(Ok(()), Err)
}

pub(crate) fn record(home: &Home, error: &LinkError) {
    let message = error.to_string();
    match status(home) {
        Ok(links) => {
            let active = links
                .into_iter()
                .filter(|link| link.active)
                .collect::<Vec<_>>();
            if active.is_empty() {
                tracing::error!(event="conversation_source_failed",error=%error);
            }
            for link in active
                .into_iter()
                .filter(|link| link.failure.as_deref() != Some(&message))
            {
                for direction in &link.directions {
                    tracing::error!(event="conversation_source_failed",link=%link.id,source=%direction.source,target=%direction.target,turn=?direction.retained_turn,position=%direction.observed.reference(),error=%error);
                }
                if let Err(error) = failure(home, &link.id, None, &message) {
                    tracing::error!(event="conversation_failure_record_failed",link=%link.id,error=%error);
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
        if let Some(handle) = self.handle.take() {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => record(&self.home, &LinkError::Stopped(Box::new(error))),
                Err(_) => record(
                    &self.home,
                    &LinkError::Refused(
                        "native conversation notification reader panicked; restart instrukt run"
                            .into(),
                    ),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "pure failure-state observation")]
    use super::*;
    #[test]
    fn stopped_failure_observation_is_consumed_once_without_losing_its_cause() {
        let mut failure = Some(LinkError::Refused(
            "native notification channel closed; restart instrukt run".into(),
        ));
        insta::assert_snapshot!(
            "stopped-reader-failure",
            observe_stopped(&mut failure).unwrap_err().to_string()
        );
        for _ in 0..100 {
            assert!(observe_stopped(&mut failure).is_ok());
        }
    }
}
