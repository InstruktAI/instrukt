//! Anomaly notification (`anomaly-notification`): a failure waiting in no
//! thread is reported through `runner.notify` by reference only.
//!
//! The notification command receives the reference to where the anomaly
//! lives (a session `runtime:id`, or the work item when no session could
//! start) as its first argument, and a deep link as its second when one can
//! be built. Nothing of the anomaly's content is passed, logged or
//! retried; a failed notification leaves the anomaly where it lives.

use instrukt_core::config::{Configuration, Env, Notify, RuntimeEntry};

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
            Some(entry) if entry.field("notify").is_some() => Route::Runtime(entry),
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
            .field("notify")
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

    fn runtime(with_notify: bool) -> Configuration {
        let dir = tempfile::tempdir().unwrap();
        let home = instrukt_core::home::Home::at_user_home(dir.path().to_path_buf());
        let notify = if with_notify {
            ", notify: 'rt notify'"
        } else {
            ""
        };
        std::fs::create_dir_all(home.root()).unwrap();
        std::fs::write(
            home.global_config(),
            format!(
                "schema_version: 1\nruntimes:\n  - {{name: rt, session_id_env: RT, start: s, send: s, alive: s, conversation: s{notify}}}\n"
            ),
        )
        .unwrap();
        Configuration::load_global(&home).unwrap()
    }

    #[test]
    fn off_is_silent_a_command_wins_and_the_default_uses_the_runtimes_capability() {
        let with = runtime(true);
        let without = runtime(false);
        let entry = with.runtimes().first();
        assert_eq!(route(&Notify::Off, entry), Route::Silent);
        assert_eq!(
            route(&Notify::Command("n".into()), entry),
            Route::Configured("n")
        );
        assert!(
            matches!(route(&Notify::RuntimeDefault, entry), Route::Runtime(e) if Some(e) == entry)
        );
        assert_eq!(
            route(&Notify::RuntimeDefault, without.runtimes().first()),
            Route::Silent
        );
        assert_eq!(route(&Notify::RuntimeDefault, None), Route::Silent);
    }
}
