//! Local backup bookkeeping (`state/backup-state.json`) and secret redaction
//! (`project/instrukt/spec/feature/runner/home-backup`).

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use jiff::Timestamp;

use super::BackupError;

/// Current schema version of `state/backup-state.json`.
pub const STATE_SCHEMA_VERSION: u32 = 1;

/// Outcome of the most recent backup run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupOutcome {
    Ok,
    Failed,
}

/// Durable local bookkeeping persisted atomically under `state/backup.lock`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackupState {
    pub schema_version: u32,
    pub first_attempt_at: Timestamp,
    pub last_attempt_at: Timestamp,
    pub last_outcome: BackupOutcome,
    pub last_success_at: Option<Timestamp>,
    pub last_generation: Option<String>,
    pub last_error: Option<String>,
}

/// Lock-free read of `state/backup-state.json`; returns `Ok(None)` when absent.
/// Refuses symlinks, newer schema versions, and contradictory state records.
pub(super) fn read_state(path: &Path) -> Result<Option<BackupState>, BackupError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(BackupError::State {
                path: path.to_path_buf(),
                detail: "refusing to read backup state through a symbolic link".into(),
            });
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(BackupError::State {
                path: path.to_path_buf(),
                detail: source.to_string(),
            });
        }
    }
    let bytes = std::fs::read(path).map_err(|source| BackupError::State {
        path: path.to_path_buf(),
        detail: source.to_string(),
    })?;
    let state: BackupState = serde_json::from_slice(&bytes).map_err(|err| BackupError::State {
        path: path.to_path_buf(),
        detail: format!("malformed backup state JSON: {err}"),
    })?;
    if state.schema_version == 0 {
        return Err(BackupError::State {
            path: path.to_path_buf(),
            detail: "backup state schema_version must be positive".into(),
        });
    }
    if state.schema_version > STATE_SCHEMA_VERSION {
        return Err(BackupError::State {
            path: path.to_path_buf(),
            detail: format!(
                "backup state schema_version {} is newer than this instrukt supports ({}); upgrade with `{}`",
                state.schema_version,
                STATE_SCHEMA_VERSION,
                instrukt_core::INSTALLER_COMMAND
            ),
        });
    }
    match state.last_outcome {
        BackupOutcome::Ok => {
            if state.last_success_at.is_none() || state.last_generation.is_none() {
                return Err(BackupError::State {
                    path: path.to_path_buf(),
                    detail: "backup state records last_outcome=ok without last_success_at and last_generation".into(),
                });
            }
        }
        BackupOutcome::Failed => {
            if state.last_error.as_deref().is_none_or(str::is_empty) {
                return Err(BackupError::State {
                    path: path.to_path_buf(),
                    detail: "backup state records last_outcome=failed without last_error".into(),
                });
            }
        }
    }
    Ok(Some(state))
}

/// Atomically writes `state/backup-state.json` with `0600` permissions.
pub(super) fn write_state(path: &Path, state: &BackupState) -> Result<(), BackupError> {
    let mut bytes = serde_json::to_vec_pretty(state).map_err(|err| BackupError::State {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })?;
    bytes.push(b'\n');
    instrukt_core::fsx::atomic_write_with_permissions(
        path,
        &bytes,
        std::fs::Permissions::from_mode(0o600),
    )
    .map_err(|err| BackupError::State {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })
}

/// Scrubs every non-empty secret value from `text`.
pub(super) fn redact_secrets(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            out = out.replace(secret, "<redacted>");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_and_refuses_newer_schema_and_malformed_ok() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup-state.json");
        assert!(read_state(&path).unwrap().is_none());

        let ts: Timestamp = "2030-01-06T03:00:00Z".parse().unwrap();
        let state = BackupState {
            schema_version: STATE_SCHEMA_VERSION,
            first_attempt_at: ts,
            last_attempt_at: ts,
            last_outcome: BackupOutcome::Ok,
            last_success_at: Some(ts),
            last_generation: Some("m1/instrukt-home.20300106T030000Z.tar.zst.age".into()),
            last_error: None,
        };
        write_state(&path, &state).unwrap();
        assert_eq!(read_state(&path).unwrap(), Some(state));

        // Newer schema version is refused.
        std::fs::write(
            &path,
            r#"{"schema_version":99,"first_attempt_at":"2030-01-06T03:00:00Z","last_attempt_at":"2030-01-06T03:00:00Z","last_outcome":"ok","last_success_at":"2030-01-06T03:00:00Z","last_generation":"g","last_error":null}"#,
        )
        .unwrap();
        assert!(read_state(&path).is_err());

        // `ok` without `last_success_at` is refused as malformed.
        std::fs::write(
            &path,
            r#"{"schema_version":1,"first_attempt_at":"2030-01-06T03:00:00Z","last_attempt_at":"2030-01-06T03:00:00Z","last_outcome":"ok","last_success_at":null,"last_generation":null,"last_error":null}"#,
        )
        .unwrap();
        assert!(read_state(&path).is_err());
    }

    #[test]
    fn redact_secrets_replaces_resolved_credentials() {
        let msg = "S3 failed for key AKIA_SECRET_1 and token SUPER_SECRET_2";
        let scrubbed = redact_secrets(msg, &["AKIA_SECRET_1", "SUPER_SECRET_2", ""]);
        assert!(!scrubbed.contains("AKIA_SECRET_1"));
        assert!(!scrubbed.contains("SUPER_SECRET_2"));
        assert!(scrubbed.contains("<redacted>"));
    }
}
