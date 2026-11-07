//! Anomaly notification (`anomaly-notification`): a failure waiting in no
//! thread is reported through `runner.notify` by reference only.
//!
//! The notification command receives the reference to where the anomaly
//! lives (a session `runtime:id`, or the work item when no session could
//! start) as its first argument, and a deep link as its second when one can
//! be built. Nothing of the anomaly's content is passed, logged or
//! retried; a failed notification leaves the anomaly where it lives.

use crate::sessions::RUNTIME_COMMAND_TIMEOUT;
use instrukt_core::config::{Configuration, Env, Notify};

/// An explicit command is the only notification route on T3's published CLI.
pub fn notify(config: &Configuration, env: &Env, reference: &str, deep_link: Option<&str>) {
    let command = match config.runner_notify() {
        Ok(Notify::Command(command)) => command,
        Ok(Notify::Off | Notify::RuntimeDefault) => {
            tracing::info!(
                event = "anomaly_not_notified",
                reference,
                "runtime notification is unsupported; configure runner.notify"
            );
            return;
        }
        Err(error) => {
            tracing::warn!(event="anomaly_notify_unconfigured", error=%error);
            return;
        }
    };
    let mut operands = vec![reference];
    operands.extend(deep_link);
    let outcome = config
        .expand(&command, env)
        .map_err(|e| e.to_string())
        .and_then(|command| {
            crate::process::Invocation::shell(&command, &operands)
                .run(RUNTIME_COMMAND_TIMEOUT)
                .map_err(|e| e.to_string())
        });
    match outcome {
        Ok(done) if done.success => tracing::info!(event = "anomaly_notified", reference),
        Ok(done) => {
            tracing::warn!(event="anomaly_notification_failed",reference,error=%done.stderr)
        }
        Err(error) => tracing::warn!(event = "anomaly_notification_failed", reference, error),
    }
}
