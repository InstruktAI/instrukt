//! Runner's local-only, lock-free `HomeBackupStanding` implementation
//! (`project/instrukt/spec/feature/runner/home-backup`).

use instrukt_core::config::{BackupAdmission, inspect_global_backup};
use instrukt_core::home::Home;
use instrukt_core::interfaces::{
    BackupStanding, BackupStandingError, BackupSuccess, HomeBackupStanding,
};
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
}
