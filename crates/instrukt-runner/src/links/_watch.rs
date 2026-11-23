//! Native database notifications drive the runner's independent relay reader.
use super::{_targets::Targets, LinkError, failure, status};
use crate::{interfaces::CompanyRelayPass, runtime::conversations::SourceWake};
use instrukt_core::{
    config::{Current, Env},
    home::Home,
};
use std::{sync::mpsc, thread::JoinHandle, time::Duration};
pub(crate) struct Worker {
    stop: mpsc::SyncSender<()>,
    handle: Option<JoinHandle<Result<(), LinkError>>>,
    failure: Option<LinkError>,
    home: Home,
}
/// The company destination the relay serves beside links, through its
/// ports, and the intake's wake-ups.
#[derive(Clone)]
pub(crate) struct Destinations {
    pub(crate) company: std::sync::Arc<dyn crate::interfaces::CompanyPorts>,
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
/// The longest wait before T3's conversation source is acquired again.
const MOST_RETRY: Duration = Duration::from_secs(60);

struct RelayState {
    failing: Option<String>,
    delay: Duration,
    targets: Targets,
}

impl Worker {
    /// The relay serving links and, when given, the connector destination.
    /// It acquires T3's conversation source inside its own thread, and when
    /// that source cannot be acquired or stops notifying it waits a growing
    /// delay and acquires it again; it does not end for it.
    pub(crate) fn start(
        home: &Home,
        env: &Env,
        current: Current,
        destinations: Option<Destinations>,
        supervise: Option<mpsc::SyncSender<()>>,
    ) -> Result<Self, LinkError> {
        let owner_home = home.clone();
        let home = home.clone();
        let env = env.clone();
        let (stop, stopping) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name("conversation-relay".into())
            .spawn(move || {
                let _notice = ExitNotice(supervise);
                let mut state = RelayState {
                    failing: None,
                    delay: Duration::from_secs(1),
                    targets: Targets::default(),
                };
                loop {
                    let error = match serve(
                        &home,
                        &env,
                        &current,
                        destinations.as_ref(),
                        &stopping,
                        &mut state,
                    ) {
                        Ok(()) => return Ok(()),
                        Err(error) => error,
                    };
                    let cause = error.to_string();
                    if state.failing.as_deref() != Some(cause.as_str()) {
                        record(&home, &error);
                        if let Some(destinations) = &destinations {
                            destinations.company.relay_state(&home, Some(&cause));
                        }
                        state.failing = Some(cause);
                    }
                    if stopping.recv_timeout(state.delay).is_ok() {
                        return Ok(());
                    }
                    state.delay = (state.delay * 2).min(MOST_RETRY);
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
                    "the conversation relay ended unexpectedly; it is started again".into(),
                )),
            };
        }
        observe_stopped(&mut self.failure)
    }
}

/// Acquires T3's conversation source and serves links and the connector
/// destination from it until the worker stops (`Ok`) or the source fails.
/// A successful acquisition starts the next retry delay from its shortest.
fn serve(
    home: &Home,
    env: &Env,
    current: &Current,
    destinations: Option<&Destinations>,
    stopping: &mpsc::Receiver<()>,
    state: &mut RelayState,
) -> Result<(), LinkError> {
    let runtime = current.get().global().runner_runtime()?;
    let adapter = crate::runtime::adapter(runtime);
    let source = adapter.conversation_source(home, env)?;
    let mut notifications = source.subscribe()?;
    state.delay = Duration::from_secs(1);
    if let Some(cause) = state.failing.take() {
        tracing::info!(event = "conversation_relay_recovered", cause = %cause);
    }
    // The source is acquired: a failure recorded for the relay ends, whether
    // this process recorded it or an earlier runner did.
    if let Some(destinations) = destinations {
        destinations.company.relay_state(home, None);
    }
    let mut links_due = true;
    let mut pass = CompanyRelayPass::CatchUp;
    loop {
        let source_pass = std::mem::take(&mut links_due);
        if source_pass {
            state.targets.source_changed(home);
        }
        let link_retried = state.targets.advance(home, env, runtime);
        if let Some(destinations) = destinations
            && let Err(error) = destinations.company.relay(
                home,
                &current.get(),
                source.as_ref(),
                adapter.as_ref(),
                pass,
            )
        {
            tracing::warn!(event = "conversation_relay_failed", error = %error);
        }
        if source_pass {
            tracing::debug!(event = "conversation_relay_source_processed");
        } else if link_retried {
            tracing::debug!(event = "conversation_link_retry_processed");
        }
        // Retained connector work that is not yet done is retried within a
        // bound; an out-of-process `instrukt company reply` can also enqueue
        // pending answers while idle, so check read-only SQLite on the same
        // 2-second cadence without touching the native conversation source.
        let retry = destinations
            .is_some_and(|destinations| destinations.company.outstanding(home).unwrap_or(true));
        let mut until = std::time::Instant::now() + Duration::from_secs(2);
        pass = loop {
            if stopping.try_recv().is_ok() {
                return Ok(());
            }
            if state.targets.ready() {
                // Link completions and retries leave the native source unchanged.
                break CompanyRelayPass::Retry;
            }
            if destinations.is_some_and(|d| {
                d.arrivals
                    .lock()
                    .is_ok_and(|arrivals| arrivals.try_recv().is_ok())
            }) {
                break CompanyRelayPass::Retry;
            }
            if std::time::Instant::now() >= until {
                if retry
                    || destinations.is_some_and(|d| d.company.outstanding(home).unwrap_or(false))
                {
                    break CompanyRelayPass::Retry;
                }
                until = std::time::Instant::now() + Duration::from_secs(2);
            }
            match notifications.wait(Duration::from_millis(100)) {
                Ok(SourceWake::Changed) => {
                    links_due = true;
                    break CompanyRelayPass::CatchUp;
                }
                Ok(SourceWake::ReadAgain) => {
                    state.targets.read_again();
                    break CompanyRelayPass::Retry;
                }
                Ok(SourceWake::Timeout) => {}
                Err(error) => return Err(LinkError::Source(error)),
            }
        };
    }
}
fn observe_stopped(failure: &mut Option<LinkError>) -> Result<(), LinkError> {
    failure.take().map_or(Ok(()), Err)
}

pub(crate) fn record(home: &Home, error: &LinkError) {
    record_affected(home, error, None);
}

pub(super) fn record_link(home: &Home, link: &str, error: &LinkError) {
    record_affected(home, error, Some(link));
}

fn record_affected(home: &Home, error: &LinkError, affected: Option<&str>) {
    let message = error.to_string();
    match status(home) {
        Ok(links) => {
            let active = links
                .into_iter()
                .filter(|link| link.active && affected.is_none_or(|id| link.id == id))
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
