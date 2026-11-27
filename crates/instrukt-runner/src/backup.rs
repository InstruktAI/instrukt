//! Encrypted offsite home backup, guarded restore, local bookkeeping, and
//! standing query (`project/instrukt/spec/feature/runner/home-backup`).

mod _archive;
mod _snapshot;
mod _standing;
mod _state;

use std::path::PathBuf;

pub use _archive::{ARCHIVE_EXTENSION, StagedArchive, StagingDir, archive_filename};
pub use _standing::{STALENESS_THRESHOLD, Standing};
pub use _state::{BackupOutcome, BackupState, STATE_SCHEMA_VERSION};

/// Collects in-scope home files, verifies pre-staging headroom, and produces an
/// encrypted `.tar.zst.age` archive inside `state/.backup-staging-<rand>/`.
pub fn stage_backup(
    home: &instrukt_core::home::Home,
    recipient: &age::x25519::Recipient,
    now: jiff::Timestamp,
) -> Result<StagedArchive, BackupError> {
    let plan = _snapshot::collect(home)?;
    let state_dir = _snapshot::resolve_root(&home.state_dir())?;
    _snapshot::check_backup_headroom(&state_dir, &plan)?;
    _archive::create_encrypted_archive(&state_dir, &plan, recipient, now)
}

/// Persists `state` atomically to `state/backup-state.json` (`0600`).
pub fn persist_state(
    home: &instrukt_core::home::Home,
    state: &BackupState,
) -> Result<(), BackupError> {
    _state::write_state(&home.backup_state_file(), state)
}

/// Scrubs any resolved backup secret values from `text`.
pub fn scrub_secrets(text: &str, secrets: &[&str]) -> String {
    _state::redact_secrets(text, secrets)
}

/// Failure during home backup, listing, or guarded restore.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error(
        "home backup is not configured; add `backup` to ~/.instrukt/instrukt.yaml via `instrukt config patch --global`"
    )]
    Unconfigured,
    #[error("home backup is withheld by configuration findings:\n{findings}")]
    Withheld { findings: String },
    #[error(
        "{path} is a symbolic link or escapes its owning root; remove or replace it before backing up or restoring"
    )]
    Unconfined { path: PathBuf },
    #[error(
        "insufficient disk space on {path}: {available_bytes} bytes available, {required_bytes} bytes required (including 20 GiB shared-machine headroom floor)"
    )]
    InsufficientDiskSpace {
        path: PathBuf,
        available_bytes: u64,
        required_bytes: u64,
    },
    #[error("SQLite database check or snapshot failed at {path}: {detail}")]
    Database { path: PathBuf, detail: String },
    #[error("archive error: {0}")]
    Archive(String),
    #[error("encryption error: {0}")]
    Encryption(String),
    #[error("backup bookkeeping at {path}: {detail}")]
    State { path: PathBuf, detail: String },
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
    #[error(transparent)]
    Fs(#[from] instrukt_core::fsx::FsError),
}
