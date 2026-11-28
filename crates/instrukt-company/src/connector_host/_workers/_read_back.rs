//! Per-connector read-back scheduling on the host and monotonic clocks.
use std::time::{Duration, Instant};

use jiff::Timestamp;
use tokio::sync::{Notify, watch};
use tokio::task::block_in_place;

use crate::conversations::read_back;

use super::super::{Connection, LOG_TARGET, after_failure, connector_policy, sleep_until, stopped};

/// Whether the host clock has reached a stored moment.
fn due(until: Timestamp, now: Timestamp) -> bool {
    now >= until
}

pub(super) async fn worker(
    connection: &Connection,
    mut stop: watch::Receiver<bool>,
    scans: &Notify,
    scan_nudge: &Notify,
) {
    let Connection {
        home, company, key, ..
    } = connection;
    if let Err(error) =
        block_in_place(|| read_back::observe(home, company, key, connection.connectors.now()))
    {
        tracing::warn!(target: LOG_TARGET, event = "connector_read_back_unrecorded", company = %company, error = %error);
    }
    // A scan runs when a connection wakes it, when its recheck is
    // due, or when the stored moment of its wait passes on the host
    // clock, the clock that moment was stored on. A live pause the
    // service named no moment for is a recheck on the monotonic
    // source; the scan after it skips that stored moment.
    let mut recheck: Option<Instant> = None;
    let mut waiting_until: Option<Timestamp> = None;
    let mut live_pause: Option<(Timestamp, Instant)> = None;
    while !*stop.borrow() {
        tokio::select! {
            biased;
            () = stopped(&mut stop) => break,
            () = scans.notified() => {}
            () = scan_deadline(connection, recheck, waiting_until) => {}
        }
        let waited_out = live_pause
            .filter(|(_, pause)| Instant::now() >= *pause)
            .map(|(at, _)| at);
        let admitted_gen = connection.current.get();
        let prepared = connector_policy(&admitted_gen, company, key).and_then(|policy| {
            connection
                .connectors
                .open(&admitted_gen, company, key)
                .map(|connector| (policy, connector))
        });
        let result = match prepared {
            Ok(((role, _, session_scope), connector)) => {
                read_back::scan(
                    home,
                    company,
                    key,
                    &role,
                    session_scope,
                    connector.as_ref(),
                    &|| connection.connectors.now(),
                    &|| *stop.borrow(),
                    &|| scan_nudge.notify_one(),
                    waited_out,
                )
                .await
            }
            Err(error) => Err(error),
        };
        match result {
            Err(error) => {
                if error.wait.is_none() {
                    tracing::warn!(target: LOG_TARGET, event = "connector_read_back_failed", company = %company, connector = key, error = %error);
                }
                recheck = Some(after_failure(error.wait.as_ref(), Instant::now()));
                waiting_until = None;
                live_pause = None;
            }
            Ok(read_back::Scanned::Done { catch_up }) => {
                recheck = catch_up.then(Instant::now);
                waiting_until = None;
                live_pause = None;
            }
            Ok(read_back::Scanned::Waiting {
                until,
                pause: Some(pause),
            }) => {
                recheck = Some(pause);
                waiting_until = None;
                live_pause = Some((until, pause));
            }
            Ok(read_back::Scanned::Waiting { until, pause: None }) => {
                // Woken during its own live pause, the read keeps it.
                match live_pause.filter(|(at, _)| *at == until) {
                    Some((_, pause)) => {
                        recheck = Some(pause);
                        waiting_until = None;
                    }
                    None => {
                        recheck = None;
                        waiting_until = Some(until);
                        live_pause = None;
                    }
                }
            }
        }
    }
}

/// Stored provider deadlines use the host clock, which can be advanced
/// independently of Tokio's clock. Re-read it only while a deadline is owed.
async fn scan_deadline(
    connection: &Connection,
    recheck: Option<Instant>,
    waiting_until: Option<Timestamp>,
) {
    const CLOCK_RECHECK: Duration = Duration::from_millis(200);
    if let Some(at) = recheck {
        sleep_until(at).await;
    } else if let Some(at) = waiting_until {
        loop {
            let now = connection.connectors.now();
            if due(at, now) {
                break;
            }
            sleep_until(Instant::now() + CLOCK_RECHECK).await;
        }
    } else {
        std::future::pending().await
    }
}

#[cfg(test)]
mod tests {
    use super::{Timestamp, due};

    #[test]
    fn a_stored_moment_is_due_once_the_host_clock_reaches_it() {
        let until: Timestamp = "2026-11-20T10:02:00Z".parse().unwrap();
        let micro = jiff::SignedDuration::from_micros(1);
        assert!(due(until, until));
        assert!(due(until, until + micro));
        assert!(!due(until, until - micro));
    }
}
