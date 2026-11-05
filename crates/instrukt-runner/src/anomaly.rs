//! Anomaly notification (`anomaly-notification`): a failure waiting in no
//! thread is reported through `runner.notify` by reference only.
//!
//! The notification command receives the reference to where the anomaly
//! lives (a session `runtime:id`, or the work item when no session could
//! start) as its first argument, and a deep link as its second when one can
//! be built. Nothing of the anomaly's content is passed, logged or
//! retried; a failed notification leaves the anomaly where it lives.

use instrukt_core::config::{Configuration, Env, Notify, RuntimeEntry, VerbName};

use crate::sessions::{RUNTIME_COMMAND_TIMEOUT, Sessions};

/// Which command reports an anomaly, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route<'a> {
    /// `runner.notify: off`, or no notification capability.
    Silent,
    /// The configured `runner.notify` command.
    Configured(&'a str),
    /// The runner runtime's own `notify` template.
    Runtime(&'a RuntimeEntry),
}

/// The route `notify` and the runner's runtime give.
pub fn route<'a>(notify: &'a Notify, runtime: Option<&'a RuntimeEntry>) -> Route<'a> {
    match notify {
        Notify::Off => Route::Silent,
        Notify::Command(command) => Route::Configured(command),
        Notify::RuntimeDefault => match runtime {
            Some(entry) if entry.command(VerbName::Notify).is_some() => Route::Runtime(entry),
            _ => Route::Silent,
        },
    }
}

/// Reports an anomaly living at `reference`. Never fails: an undeliverable
/// notification is logged and dropped.
pub fn notify(config: &Configuration, env: &Env, reference: &str, deep_link: Option<&str>) {
    let notify = match config.runner_notify() {
        Ok(notify) => notify,
        Err(err) => {
            tracing::warn!(event = "anomaly_notify_unconfigured", error = %err);
            return;
        }
    };
    let sessions = Sessions::new(config, env);
    let runtime = sessions.runner_runtime().ok();
    let mut operands = vec![reference];
    operands.extend(deep_link);
    let invocation = match route(&notify, runtime) {
        Route::Silent => {
            tracing::info!(event = "anomaly_not_notified", reference);
            return;
        }
        Route::Configured(command) => config
            .expand(command, env)
            .map(|cmd| crate::process::Invocation::shell(&cmd, &operands))
            .map_err(|err| err.to_string()),
        Route::Runtime(entry) => entry
            .command(VerbName::Notify)
            .ok_or_else(|| "the runtime has no notify template".to_owned())
            .and_then(|template| {
                sessions
                    .invocation(entry, template, None, &operands)
                    .map_err(|err| err.to_string())
            }),
    };
    let outcome = invocation.and_then(|inv| {
        inv.run(RUNTIME_COMMAND_TIMEOUT)
            .map_err(|err| err.to_string())
            .and_then(|done| {
                if done.success {
                    Ok(())
                } else {
                    Err(done.stderr.trim().to_owned())
                }
            })
    });
    match outcome {
        Ok(()) => tracing::info!(event = "anomaly_notified", reference),
        Err(error) => tracing::warn!(event = "anomaly_notification_failed", reference, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_is_silent_and_a_configured_command_wins() {
        let command = String::from("notify-me");
        assert_eq!(route(&Notify::Off, None), Route::Silent);
        assert_eq!(
            route(&Notify::Command(command.clone()), None),
            Route::Configured(command.as_str())
        );
        assert_eq!(route(&Notify::RuntimeDefault, None), Route::Silent);
    }
}
