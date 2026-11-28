//! Runner's local-only, lock-free `HomeBackupStanding` implementation
//! (`project/instrukt/spec/feature/runner/home-backup`).

use std::os::unix::fs::PermissionsExt as _;

use instrukt_core::config::{BackupAdmission, Configuration, Env, inspect_global_backup};
use instrukt_core::fsx;
use instrukt_core::home::Home;
use instrukt_core::interfaces::{
    BackupStanding, BackupStandingError, BackupSuccess, HomeBackupStanding,
};
use instrukt_core::machine::existing_machine_id;
use jiff::{SignedDuration, Timestamp};

use super::_state::{BackupOutcome, read_state};

/// Grace period past the daily 24-hour backup cadence before a configured home
/// backup is considered stale (36 hours).
pub const STALENESS_THRESHOLD: SignedDuration = SignedDuration::from_hours(36);

/// Runner's implementation of [`HomeBackupStanding`], wired into
/// `publication::status` by `instrukt-cli`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Standing;

impl HomeBackupStanding for Standing {
    fn standing(&self, home: &Home, now: Timestamp) -> Result<BackupStanding, BackupStandingError> {
        let admitted = match inspect_global_backup(home) {
            Ok(BackupAdmission::Unconfigured) => return Ok(BackupStanding::Unconfigured),
            Ok(BackupAdmission::Admitted(_)) => true,
            Ok(BackupAdmission::Withheld(_)) => false,
            Err(err) => {
                return Err(BackupStandingError {
                    path: home.global_config(),
                    reason: err.to_string(),
                });
            }
        };
        let state_path = home.backup_state_file();
        let Some(state) = read_state(&state_path).map_err(|err| BackupStandingError {
            path: state_path.clone(),
            reason: err.to_string(),
        })?
        else {
            return Ok(BackupStanding::NeverRun { admitted });
        };
        let anchor = state.last_success_at.unwrap_or(state.first_attempt_at);
        let is_stale = now.duration_since(anchor) > STALENESS_THRESHOLD;
        match state.last_outcome {
            BackupOutcome::Ok => {
                let (Some(at), Some(generation)) = (state.last_success_at, state.last_generation)
                else {
                    return Err(BackupStandingError {
                        path: state_path,
                        reason:
                            "backup state records ok without last_success_at and last_generation"
                                .into(),
                    });
                };
                let last_success = BackupSuccess { at, generation };
                if is_stale {
                    Ok(BackupStanding::Stale {
                        admitted,
                        last_attempt_at: state.last_attempt_at,
                        last_success,
                    })
                } else {
                    Ok(BackupStanding::Ok {
                        admitted,
                        last_attempt_at: state.last_attempt_at,
                        last_success,
                    })
                }
            }
            BackupOutcome::Failed => {
                let last_success = state
                    .last_success_at
                    .zip(state.last_generation)
                    .map(|(at, generation)| BackupSuccess { at, generation });
                Ok(BackupStanding::Failed {
                    admitted,
                    stale: is_stale,
                    last_attempt_at: state.last_attempt_at,
                    last_success,
                    last_error: state.last_error.unwrap_or_default(),
                })
            }
        }
    }
}

/// Builds the `backup` section of `instrukt events status`, including the
/// applicable repair door for every active failure, staleness, or unreadable
/// local-bookkeeping condition without exposing secret material.
pub fn events_status_json(home: &Home, now: Timestamp) -> serde_json::Value {
    match HomeBackupStanding::standing(&Standing, home, now) {
        Ok(standing) => {
            let repair_door: Option<&'static str> = match &standing {
                BackupStanding::Stale { admitted: true, .. } => Some(
                    "run `instrukt backup run` to publish a fresh generation, and run `instrukt install --check` to verify the daily backup timer",
                ),
                BackupStanding::Stale {
                    admitted: false, ..
                } => Some(
                    "repair the global `backup` configuration in `~/.instrukt/instrukt.yaml` and `~/.instrukt/secrets.env`, then run `instrukt backup run` and `instrukt install`",
                ),
                BackupStanding::Failed { admitted: true, .. } => Some(
                    "resolve the error in `last_error` and run `instrukt backup run` to publish a fresh generation; run `instrukt install --check` if scheduled runs are failing",
                ),
                BackupStanding::Failed {
                    admitted: false, ..
                } => Some(
                    "repair the global `backup` configuration in `~/.instrukt/instrukt.yaml` and `~/.instrukt/secrets.env`, then run `instrukt backup run` and `instrukt install`",
                ),
                BackupStanding::Unconfigured
                | BackupStanding::NeverRun { .. }
                | BackupStanding::Ok { .. } => None,
            };
            let mut value = standing.to_json();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("repair_door".into(), serde_json::json!(repair_door));
            }
            value
        }
        Err(err) => serde_json::json!({
            "errors": [{"path": err.path, "reason": err.reason}],
            "repair_door": "inspect and repair or move aside the unreadable backup file above, then run `instrukt backup run`",
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StalenessMark {
    episode_anchor: Timestamp,
}

fn remove_staleness_mark(path: &std::path::Path) {
    if let Err(error) = fsx::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            event = "backup_staleness_mark_remove_failed",
            path = %path.display(),
            %error,
            "failed to remove backup staleness notification mark"
        );
    }
}

fn read_staleness_mark(path: &std::path::Path) -> Result<Option<StalenessMark>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err("staleness mark is a symbolic link".into());
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mark: StalenessMark = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(Some(mark))
}

/// Evaluates local backup staleness on every runner wake (under `runner.lock`)
/// without network calls, sending at most one `backup:stale:<machine-id>`
/// notification per stale episode and clearing the mark once backup recovers.
pub fn check_staleness_on_wake(home: &Home, config: &Configuration, env: &Env, now: Timestamp) {
    let mark_path = home.backup_staleness_notified_file();
    match inspect_global_backup(home) {
        Ok(BackupAdmission::Unconfigured) => {
            remove_staleness_mark(&mark_path);
            return;
        }
        Ok(BackupAdmission::Admitted(_) | BackupAdmission::Withheld(_)) => {}
        Err(error) => {
            tracing::warn!(
                event = "backup_staleness_check_skipped",
                %error,
                "could not inspect global backup configuration during wake"
            );
            return;
        }
    }

    let state_path = home.backup_state_file();
    let state = match read_state(&state_path) {
        Ok(Some(state)) => state,
        Ok(None) => {
            remove_staleness_mark(&mark_path);
            return;
        }
        Err(error) => {
            tracing::warn!(
                event = "backup_staleness_state_unreadable",
                path = %state_path.display(),
                %error,
                "skipping wake-time backup staleness notification because bookkeeping is unreadable"
            );
            return;
        }
    };

    let anchor = state.last_success_at.unwrap_or(state.first_attempt_at);
    if now.duration_since(anchor) <= STALENESS_THRESHOLD {
        remove_staleness_mark(&mark_path);
        return;
    }

    match read_staleness_mark(&mark_path) {
        Ok(Some(existing)) if existing.episode_anchor == anchor => return,
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(
                event = "backup_staleness_mark_unreadable",
                path = %mark_path.display(),
                %error,
                "skipping wake-time backup staleness notification because deduplication mark is unreadable"
            );
            return;
        }
    }

    let machine_id = match existing_machine_id(home) {
        Ok(Some(mid)) => mid,
        Ok(None) => {
            tracing::warn!(
                event = "backup_staleness_machine_id_missing",
                "skipping wake-time backup staleness notification because state/machine-id is absent"
            );
            return;
        }
        Err(error) => {
            tracing::warn!(
                event = "backup_staleness_machine_id_unreadable",
                %error,
                "skipping wake-time backup staleness notification because state/machine-id is unreadable"
            );
            return;
        }
    };

    // Persist deduplication mark BEFORE notifying so a mark write failure
    // never turns into per-minute notification spam.
    let mark_bytes = match serde_json::to_vec(&StalenessMark {
        episode_anchor: anchor,
    }) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                event = "backup_staleness_mark_serialize_failed",
                %error,
                "skipping wake-time backup staleness notification"
            );
            return;
        }
    };
    if let Err(error) = fsx::atomic_write_with_permissions(
        &mark_path,
        &mark_bytes,
        std::fs::Permissions::from_mode(0o600),
    ) {
        tracing::warn!(
            event = "backup_staleness_mark_write_failed",
            path = %mark_path.display(),
            %error,
            "skipping wake-time backup staleness notification because deduplication mark could not be persisted"
        );
        return;
    }

    let reference = format!("backup:stale:{machine_id}");
    crate::anomaly::notify(config, env, &reference, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::_state::{BackupState, STATE_SCHEMA_VERSION, write_state};
    use instrukt_core::config::{GLOBAL_SCHEMA_VERSION, set_secret};

    fn configure_home(with_secrets: bool) -> (tempfile::TempDir, Home) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf()).for_writing();
        let recipient = age::x25519::Identity::generate().to_public().to_string();
        let yaml = [
            format!("schema_version: {GLOBAL_SCHEMA_VERSION}"),
            "backup:".to_owned(),
            "  endpoint: https://s3.example.test".to_owned(),
            "  region: us-east-1".to_owned(),
            "  bucket: test".to_owned(),
            "  access_key_id: ${BK_KEY}".to_owned(),
            "  secret_access_key: ${BK_SECRET}".to_owned(),
            format!("  age_recipient: {recipient}"),
            String::new(),
        ]
        .join("\n");
        instrukt_core::fsx::atomic_write(&home.global_config(), yaml.as_bytes()).unwrap();
        if with_secrets {
            set_secret(&home, "BK_KEY", "ak").unwrap();
            set_secret(&home, "BK_SECRET", "sk").unwrap();
        }
        (dir, home)
    }

    #[test]
    fn standing_evaluates_36h_staleness_across_ok_failed_withheld_and_never_succeeded() {
        let t0: Timestamp = "2030-01-06T03:00:00Z".parse().unwrap();
        let t_fresh: Timestamp = "2030-01-07T15:00:00Z".parse().unwrap(); // exactly 36h
        let t_stale: Timestamp = "2030-01-07T15:00:01Z".parse().unwrap(); // 36h + 1s

        let dir_empty = tempfile::tempdir().unwrap();
        let home_empty = Home::at_user_home(dir_empty.path().to_path_buf());
        assert_eq!(
            Standing.standing(&home_empty, t0).unwrap(),
            BackupStanding::Unconfigured
        );

        let (_dir, home) = configure_home(true);
        assert_eq!(
            Standing.standing(&home, t0).unwrap(),
            BackupStanding::NeverRun { admitted: true }
        );

        // Successful run: fresh at 36h, stale at 36h + 1s.
        write_state(
            &home.backup_state_file(),
            &BackupState {
                schema_version: STATE_SCHEMA_VERSION,
                first_attempt_at: t0,
                last_attempt_at: t0,
                last_outcome: BackupOutcome::Ok,
                last_success_at: Some(t0),
                last_generation: Some("gen1".into()),
                last_error: None,
            },
        )
        .unwrap();
        assert!(matches!(
            Standing.standing(&home, t_fresh).unwrap(),
            BackupStanding::Ok { admitted: true, .. }
        ));
        assert!(matches!(
            Standing.standing(&home, t_stale).unwrap(),
            BackupStanding::Stale { admitted: true, .. }
        ));

        // Failed run with no prior success anchors staleness to first_attempt_at.
        write_state(
            &home.backup_state_file(),
            &BackupState {
                schema_version: STATE_SCHEMA_VERSION,
                first_attempt_at: t0,
                last_attempt_at: t_fresh,
                last_outcome: BackupOutcome::Failed,
                last_success_at: None,
                last_generation: None,
                last_error: Some("upload failed".into()),
            },
        )
        .unwrap();
        assert!(matches!(
            Standing.standing(&home, t_fresh).unwrap(),
            BackupStanding::Failed {
                admitted: true,
                stale: false,
                ..
            }
        ));
        assert!(matches!(
            Standing.standing(&home, t_stale).unwrap(),
            BackupStanding::Failed {
                admitted: true,
                stale: true,
                ..
            }
        ));

        // Withheld config still evaluates staleness without hiding as unconfigured.
        let (_dir2, withheld_home) = configure_home(false);
        write_state(
            &withheld_home.backup_state_file(),
            &BackupState {
                schema_version: STATE_SCHEMA_VERSION,
                first_attempt_at: t0,
                last_attempt_at: t0,
                last_outcome: BackupOutcome::Ok,
                last_success_at: Some(t0),
                last_generation: Some("gen1".into()),
                last_error: None,
            },
        )
        .unwrap();
        assert!(matches!(
            Standing.standing(&withheld_home, t_stale).unwrap(),
            BackupStanding::Stale {
                admitted: false,
                ..
            }
        ));
    }

    #[test]
    fn check_staleness_on_wake_deduplicates_clears_on_recovery_and_handles_withheld() {
        let (_dir, home) = configure_home(false); // withheld config still alerts on staleness
        let mid = instrukt_core::machine::machine_id(&home).unwrap();
        let log_file = home.root().join("notify.log");
        let notify_script = home.root().join("notify.sh");
        std::fs::write(
            &notify_script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$1\" >> {}\n",
                log_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&notify_script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let existing_yaml = std::fs::read_to_string(home.global_config()).unwrap();
        let with_notify = format!(
            "{existing_yaml}runner:\n  notify: {}\n",
            notify_script.display()
        );
        fsx::atomic_write(&home.global_config(), with_notify.as_bytes()).unwrap();
        let config = Configuration::load_global(&home).unwrap();

        let t0: Timestamp = "2030-01-06T03:00:00Z".parse().unwrap();
        let t_stale: Timestamp = "2030-01-07T15:00:01Z".parse().unwrap();
        let t_recovered: Timestamp = "2030-01-07T16:00:00Z".parse().unwrap();
        let t_stale2: Timestamp = "2030-01-09T04:00:01Z".parse().unwrap();

        write_state(
            &home.backup_state_file(),
            &BackupState {
                schema_version: STATE_SCHEMA_VERSION,
                first_attempt_at: t0,
                last_attempt_at: t0,
                last_outcome: BackupOutcome::Ok,
                last_success_at: Some(t0),
                last_generation: Some("gen1".into()),
                last_error: None,
            },
        )
        .unwrap();

        // First stale wake notifies once and writes mark.
        check_staleness_on_wake(&home, &config, &Env::default(), t_stale);
        // Second stale wake for the same episode deduplicates.
        check_staleness_on_wake(&home, &config, &Env::default(), t_recovered);
        let lines: Vec<String> = std::fs::read_to_string(&log_file)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(lines, vec![format!("backup:stale:{mid}")]);
        assert!(home.backup_staleness_notified_file().exists());

        // Recovery clears the mark.
        write_state(
            &home.backup_state_file(),
            &BackupState {
                schema_version: STATE_SCHEMA_VERSION,
                first_attempt_at: t0,
                last_attempt_at: t_recovered,
                last_outcome: BackupOutcome::Ok,
                last_success_at: Some(t_recovered),
                last_generation: Some("gen2".into()),
                last_error: None,
            },
        )
        .unwrap();
        check_staleness_on_wake(&home, &config, &Env::default(), t_recovered);
        assert!(!home.backup_staleness_notified_file().exists());

        // A later distinct stale episode notifies again.
        check_staleness_on_wake(&home, &config, &Env::default(), t_stale2);
        let lines2: Vec<String> = std::fs::read_to_string(&log_file)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(
            lines2,
            vec![format!("backup:stale:{mid}"), format!("backup:stale:{mid}")]
        );
    }

    #[test]
    fn events_status_json_includes_repair_doors_for_active_conditions_only() {
        let (_dir, home) = configure_home(true);
        let t0: Timestamp = "2030-01-06T03:00:00Z".parse().unwrap();
        let t_ok: Timestamp = "2030-01-06T12:00:00Z".parse().unwrap();
        let t_stale: Timestamp = "2030-01-07T15:00:01Z".parse().unwrap();

        // NeverRun: no repair door.
        assert_eq!(
            events_status_json(&home, t0)["repair_door"],
            serde_json::Value::Null
        );

        // Ok: no repair door.
        write_state(
            &home.backup_state_file(),
            &BackupState {
                schema_version: STATE_SCHEMA_VERSION,
                first_attempt_at: t0,
                last_attempt_at: t0,
                last_outcome: BackupOutcome::Ok,
                last_success_at: Some(t0),
                last_generation: Some("gen1".into()),
                last_error: None,
            },
        )
        .unwrap();
        assert_eq!(
            events_status_json(&home, t_ok)["repair_door"],
            serde_json::Value::Null
        );

        // Stale (admitted): non-empty repair door.
        let stale_json = events_status_json(&home, t_stale);
        assert_eq!(stale_json["standing"], "stale");
        assert!(
            stale_json["repair_door"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );

        // Failed (withheld): non-empty config-repair door, zero secret leakage.
        let (_dir2, withheld_home) = configure_home(false);
        write_state(
            &withheld_home.backup_state_file(),
            &BackupState {
                schema_version: STATE_SCHEMA_VERSION,
                first_attempt_at: t0,
                last_attempt_at: t0,
                last_outcome: BackupOutcome::Failed,
                last_success_at: None,
                last_generation: None,
                last_error: Some("config withheld".into()),
            },
        )
        .unwrap();
        let withheld_failed = events_status_json(&withheld_home, t_ok);
        assert_eq!(withheld_failed["standing"], "failed");
        assert_eq!(withheld_failed["admitted"], false);
        assert!(
            withheld_failed["repair_door"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );

        // Unreadable bookkeeping: errors + repair door.
        std::fs::write(home.backup_state_file(), b"{not-json").unwrap();
        let err_json = events_status_json(&home, t_ok);
        assert!(err_json["errors"].is_array());
        assert!(
            err_json["repair_door"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );
    }
}
