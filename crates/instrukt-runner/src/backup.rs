//! Encrypted offsite home backup, guarded restore, local bookkeeping, and
//! standing query (`project/instrukt/spec/feature/runner/home-backup`).

mod _archive;
mod _locks;
mod _restore;
mod _snapshot;
mod _standing;
mod _state;
mod _storage;

use std::path::PathBuf;
use std::time::Instant;

use instrukt_core::config::{
    BackupAdmission, BackupConfig, Configuration, Env, inspect_global_backup,
};
use instrukt_core::fsx::LockGuard;
use instrukt_core::home::Home;
use instrukt_core::machine::{MachineId, existing_machine_id, machine_id};
use jiff::Timestamp;

pub use _archive::{ARCHIVE_EXTENSION, StagedArchive, StagingDir, archive_filename};
pub use _restore::{IdentityInput, RestoreOptions, RestoreReport, restore};
pub use _standing::{STALENESS_THRESHOLD, Standing, check_staleness_on_wake, events_status_json};
pub use _state::{BackupOutcome, BackupState, STATE_SCHEMA_VERSION};
pub use _storage::{
    BackupGeneration, ORPHAN_CLEANUP_AGE, PruneReport, S3Storage, TransferThresholds,
};

/// Result of a completed `instrukt backup run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRunReport {
    pub generation: String,
    pub size_bytes: u64,
    pub pruned_generations: Vec<String>,
    pub deleted_orphans: Vec<String>,
    pub aborted_uploads: Vec<String>,
}

/// Validates and extracts admitted global `backup` configuration before any
/// lock acquisition or remote I/O.
pub(crate) fn admitted_backup_config(home: &Home) -> Result<BackupConfig, BackupError> {
    match inspect_global_backup(home)? {
        BackupAdmission::Unconfigured => Err(BackupError::Unconfigured),
        BackupAdmission::Withheld(findings) => {
            let formatted = findings
                .iter()
                .map(|f| format!("- {}: {}", f.key, f.message))
                .collect::<Vec<_>>()
                .join("\n");
            Err(BackupError::Withheld {
                findings: formatted,
            })
        }
        BackupAdmission::Admitted(cfg) => Ok(*cfg),
    }
}

/// Lists remote backup generations for this computer's `machine-id`, newest
/// first. Read-only: never creates `state/machine-id` or `state/backup.lock`.
pub fn list(home: &Home) -> Result<Vec<BackupGeneration>, BackupError> {
    let cfg = admitted_backup_config(home)?;
    let Some(mid) = existing_machine_id(home)? else {
        return Ok(Vec::new());
    };
    let storage = S3Storage::new(&cfg);
    storage.list_generations(&mid)
}

/// Runs a complete encrypted home backup: validates config, acquires
/// `state/backup.lock`, stages and verifies the archive, uploads via staged
/// verify-promote-validate, prunes excess generations and >24h orphans, updates
/// `state/backup-state.json`, and raises `runner.notify` on failure.
pub fn run(home: &Home, env: &Env, now: Timestamp) -> Result<BackupRunReport, BackupError> {
    let cfg = admitted_backup_config(home)?;
    let storage = S3Storage::new(&cfg);
    run_with_storage(home, env, &cfg, &storage, now)
}

fn run_with_storage(
    home: &Home,
    env: &Env,
    cfg: &BackupConfig,
    storage: &S3Storage,
    now: Timestamp,
) -> Result<BackupRunReport, BackupError> {
    let lock_path = home.backup_lock_file();
    let Some(_lock) = LockGuard::try_acquire(&lock_path)? else {
        return Err(BackupError::Busy { lock: lock_path });
    };

    // Refuse to clobber unreadable or newer-schema bookkeeping.
    let prior_state = _state::read_state(&home.backup_state_file())?;
    let mid = machine_id(home)?;
    let global_cfg = Configuration::load_global(home).ok();
    let started = Instant::now();
    let secrets = [cfg.access_key_id.expose(), cfg.secret_access_key.expose()];

    let StagedBackupOutcome { published, result } =
        execute_staged_backup(home, cfg, &mid, storage, now);
    let first_attempt_at = prior_state.as_ref().map_or(now, |s| s.first_attempt_at);

    match result {
        Ok(report) => {
            let next_state = BackupState {
                schema_version: STATE_SCHEMA_VERSION,
                first_attempt_at,
                last_attempt_at: now,
                last_outcome: BackupOutcome::Ok,
                last_success_at: Some(now),
                last_generation: Some(report.generation.clone()),
                last_error: None,
            };
            if let Err(bookkeeping_err) = persist_state(home, &next_state) {
                let redacted = scrub_secrets(&bookkeeping_err.to_string(), &secrets);
                tracing::error!(
                    event = "backup_failed",
                    generation = %report.generation,
                    error = %redacted,
                    "home backup published remotely, but persisting local backup state failed"
                );
                if let Some(ref config) = global_cfg {
                    let reference = format!("backup:failed:{mid}");
                    crate::anomaly::notify(config, env, &reference, None);
                }
                return Err(BackupError::Failed(redacted));
            }
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            tracing::debug!(
                event = "backup_completed",
                generation = %report.generation,
                size_bytes = report.size_bytes,
                elapsed_ms,
                pruned = report.pruned_generations.len(),
                orphans_deleted = report.deleted_orphans.len(),
                uploads_aborted = report.aborted_uploads.len(),
                "home backup completed"
            );
            Ok(report)
        }
        Err(primary_err) => {
            let redacted_error = scrub_secrets(&primary_err.to_string(), &secrets);
            let (last_success_at, last_generation) = match published {
                Some(gen_key) => (Some(now), Some(gen_key)),
                None => (
                    prior_state.as_ref().and_then(|s| s.last_success_at),
                    prior_state.as_ref().and_then(|s| s.last_generation.clone()),
                ),
            };
            let next_state = BackupState {
                schema_version: STATE_SCHEMA_VERSION,
                first_attempt_at,
                last_attempt_at: now,
                last_outcome: BackupOutcome::Failed,
                last_success_at,
                last_generation,
                last_error: Some(redacted_error.clone()),
            };
            let state_err = persist_state(home, &next_state).err();

            tracing::error!(
                event = "backup_failed",
                error = %redacted_error,
                "home backup failed"
            );
            if let Some(ref config) = global_cfg {
                let reference = format!("backup:failed:{mid}");
                crate::anomaly::notify(config, env, &reference, None);
            }

            if let Some(bookkeeping_err) = state_err {
                let bookkeeping_msg = scrub_secrets(&bookkeeping_err.to_string(), &secrets);
                return Err(BackupError::FailedWithBookkeepingError {
                    primary: redacted_error,
                    bookkeeping: bookkeeping_msg,
                });
            }
            Err(BackupError::Failed(redacted_error))
        }
    }
}

struct StagedBackupOutcome {
    published: Option<String>,
    result: Result<BackupRunReport, BackupError>,
}

/// Runs snapshot staging, upload, verification, promotion, validation marker
/// write, and post-validation cleanup/pruning.
///
/// Sets `published = Some(live_key)` once `_validated/<live_key>` has been
/// written, so post-validation cleanup or pruning failures still advance
/// `last_success_at` and `last_generation` in local bookkeeping.
fn execute_staged_backup(
    home: &Home,
    cfg: &BackupConfig,
    mid: &MachineId,
    storage: &S3Storage,
    now: Timestamp,
) -> StagedBackupOutcome {
    let staged = match stage_backup(home, &cfg.age_recipient, now) {
        Ok(staged) => staged,
        Err(err) => {
            return StagedBackupOutcome {
                published: None,
                result: Err(err),
            };
        }
    };
    let live_key = _storage::generation_key(cfg.prefix.as_deref(), mid, &staged.filename);
    let stg_key = _storage::staging_key(&live_key);
    let size_bytes = staged.size_bytes;

    let pre_validation = (|| -> Result<(), BackupError> {
        storage.upload_to_staging(&staged.path, size_bytes, &stg_key)?;
        storage.verify_staging(&stg_key, size_bytes)?;
        storage.promote_staging(&stg_key, &live_key, size_bytes)?;
        storage.write_validation_marker(&live_key)?;
        Ok(())
    })();
    if let Err(err) = pre_validation {
        // Best-effort remote staging cleanup on every pre-validation failure
        // (including upload errors and validation-marker failure) while
        // preserving the primary error and leaving validated generations intact.
        let _ = storage.delete_object(&stg_key);
        return StagedBackupOutcome {
            published: None,
            result: Err(err),
        };
    }

    // Validation marker is durably written: drop local staging early and mark
    // this generation as published even if post-validation steps fail.
    drop(staged);
    let published = Some(live_key.clone());

    if let Err(err) = storage.delete_object(&stg_key) {
        return StagedBackupOutcome {
            published,
            result: Err(err),
        };
    }
    let prune = match storage.prune_and_clean_orphans(mid, cfg.keep, now) {
        Ok(prune) => prune,
        Err(err) => {
            return StagedBackupOutcome {
                published,
                result: Err(err),
            };
        }
    };
    StagedBackupOutcome {
        published,
        result: Ok(BackupRunReport {
            generation: live_key,
            size_bytes,
            pruned_generations: prune.pruned_generations,
            deleted_orphans: prune.deleted_orphans,
            aborted_uploads: prune.aborted_uploads,
        }),
    }
}

/// Collects in-scope home files, verifies pre-staging headroom, and produces an
/// encrypted `.tar.zst.age` archive inside `state/.backup-staging-<rand>/`.
pub fn stage_backup(
    home: &Home,
    recipient: &age::x25519::Recipient,
    now: Timestamp,
) -> Result<StagedArchive, BackupError> {
    let plan = _snapshot::collect(home)?;
    let state_dir = _snapshot::resolve_root(&home.state_dir())?;
    _snapshot::check_backup_headroom(&state_dir, &plan)?;
    _archive::create_encrypted_archive(&state_dir, &plan, recipient, now)
}

/// Persists `state` atomically to `state/backup-state.json` (`0600`).
pub fn persist_state(home: &Home, state: &BackupState) -> Result<(), BackupError> {
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
    #[error("another home backup is already running ({lock} is held)")]
    Busy { lock: PathBuf },
    #[error(
        "cannot restore into live instrukt home while {lock} is held{}; stop instrukt services first, or pass `--into <dir>` to extract beside the live home for inspection or selective recovery",
        holder.as_ref().map(|h| format!(" (holder: {h})")).unwrap_or_default()
    )]
    LiveHomeBusy {
        lock: PathBuf,
        holder: Option<String>,
    },
    #[error("restore requires confirmation in a non-interactive session; re-run with `--yes`")]
    ConfirmationRequired,
    #[error("restore aborted by operator")]
    RestoreDeclined,
    #[error("invalid age identity: {0}")]
    InvalidIdentity(String),
    #[error("{0}")]
    NoGenerationFound(String),
    #[error("unsupported backup archive format `{key}`; expected extension `{expected_extension}`")]
    UnsupportedArchiveFormat {
        key: String,
        expected_extension: &'static str,
    },
    #[error("unsafe archive entry `{path}`: {reason}")]
    UnsafeArchiveEntry { path: String, reason: String },
    #[error("invalid restore target {path}: {reason}")]
    RestoreInvalidTarget { path: PathBuf, reason: String },
    #[error("restore swap failed for `{key}`: {cause} ({rollback_hint})")]
    RestoreSwapFailed {
        cause: String,
        rolled_back: bool,
        key: String,
        rollback_hint: String,
    },
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
    #[error("storage error: {0}")]
    Storage(String),
    #[error("{0}")]
    Failed(String),
    #[error("{primary} (additionally failed to write backup bookkeeping: {bookkeeping})")]
    FailedWithBookkeepingError {
        primary: String,
        bookkeeping: String,
    },
    #[error("backup bookkeeping at {path}: {detail}")]
    State { path: PathBuf, detail: String },
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Machine(#[from] instrukt_core::machine::MachineError),
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
    #[error(transparent)]
    Fs(#[from] instrukt_core::fsx::FsError),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU32;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use instrukt_core::config::{BackupTime, SecretValue};

    use super::*;

    #[test]
    fn unconfigured_run_and_list_refuse_without_creating_backup_lock() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.root()).unwrap();
        let now: Timestamp = "2030-01-06T03:00:00Z".parse().unwrap();

        assert!(matches!(
            run(&home, &Env::default(), now),
            Err(BackupError::Unconfigured)
        ));
        assert!(matches!(list(&home), Err(BackupError::Unconfigured)));
        assert!(!home.backup_lock_file().exists());
        assert!(!home.machine_id_file().exists());
    }

    #[test]
    fn pre_and_post_validation_failures_update_bookkeeping_and_notify() {
        #[derive(Default)]
        struct FaultS3 {
            objects: BTreeMap<String, Vec<u8>>,
            delete_requests: Vec<String>,
            fail_delete_staging: bool,
            fail_put_staging: bool,
            fail_put_validated: bool,
        }

        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let st = Arc::new(Mutex::new(FaultS3::default()));
        let srv = Arc::clone(&server);
        let st_bg = Arc::clone(&st);
        let handle = thread::spawn(move || {
            for mut req in srv.incoming_requests() {
                let method = req.method().as_str().to_owned();
                let url = req.url().to_owned();
                let mut body = Vec::new();
                let _ = req.as_reader().read_to_end(&mut body);
                let path = url.split('?').next().unwrap_or("");
                let key = path
                    .trim_start_matches('/')
                    .split_once('/')
                    .map(|(_, k)| k.to_owned())
                    .unwrap_or_default();
                let copy_src = req
                    .headers()
                    .iter()
                    .find(|h| {
                        h.field
                            .as_str()
                            .as_str()
                            .eq_ignore_ascii_case("x-amz-copy-source")
                    })
                    .map(|h| h.value.as_str().to_owned());

                let mut guard = st_bg.lock().unwrap();
                match method.as_str() {
                    "PUT" if copy_src.is_some() => {
                        let src_key = copy_src
                            .unwrap()
                            .trim_start_matches('/')
                            .split_once('/')
                            .map(|(_, k)| k.to_owned())
                            .unwrap_or_default();
                        if let Some(data) = guard.objects.get(&src_key).cloned() {
                            guard.objects.insert(key, data);
                            let _ = req.respond(tiny_http::Response::from_string(
                                "<CopyObjectResult><ETag>\"ok\"</ETag></CopyObjectResult>",
                            ));
                        } else {
                            let _ = req.respond(tiny_http::Response::empty(404));
                        }
                    }
                    "PUT" => {
                        if guard.fail_put_staging && key.starts_with("_staging/") {
                            let _ = req.respond(
                                tiny_http::Response::from_string("AKIA_SECRET_LEAK")
                                    .with_status_code(403),
                            );
                        } else if guard.fail_put_validated && key.starts_with("_validated/") {
                            let _ = req.respond(
                                tiny_http::Response::from_string("validation marker refused")
                                    .with_status_code(403),
                            );
                        } else {
                            guard.objects.insert(key, body);
                            let etag = tiny_http::Header::from_bytes("ETag", "\"ok\"").unwrap();
                            let _ = req.respond(tiny_http::Response::empty(200).with_header(etag));
                        }
                    }
                    "HEAD" => {
                        if let Some(data) = guard.objects.get(&key) {
                            let hdr = tiny_http::Header::from_bytes(
                                "Content-Length",
                                data.len().to_string(),
                            )
                            .unwrap();
                            let _ = req.respond(tiny_http::Response::empty(200).with_header(hdr));
                        } else {
                            let _ = req.respond(tiny_http::Response::empty(404));
                        }
                    }
                    "DELETE" => {
                        guard.delete_requests.push(key.clone());
                        if guard.fail_delete_staging && key.starts_with("_staging/") {
                            let _ = req.respond(tiny_http::Response::empty(500));
                        } else {
                            guard.objects.remove(&key);
                            let _ = req.respond(tiny_http::Response::empty(204));
                        }
                    }
                    "GET" => {
                        let xml = if url.contains("uploads") {
                            "<ListMultipartUploadsResult><IsTruncated>false</IsTruncated></ListMultipartUploadsResult>"
                        } else {
                            "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"
                        };
                        let _ = req.respond(tiny_http::Response::from_string(xml));
                    }
                    _ => {
                        let _ = req.respond(tiny_http::Response::empty(400));
                    }
                }
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.root()).unwrap();
        let notify_log = dir.path().join("notified.txt");
        let global_yaml = [
            "schema_version: 8".to_owned(),
            "runner:".to_owned(),
            format!("  notify: 'echo \"$1\" >> {}'", notify_log.display()),
        ]
        .join("\n");
        std::fs::write(home.global_config(), global_yaml).unwrap();

        let identity = age::x25519::Identity::generate();
        let cfg = BackupConfig {
            endpoint: url::Url::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
            region: "auto".into(),
            bucket: "bkt".into(),
            access_key_id: SecretValue::new("AKIA_SECRET_LEAK".into()),
            secret_access_key: SecretValue::new("TOP_SECRET_VALUE".into()),
            age_recipient: identity.to_public(),
            prefix: None,
            keep: NonZeroU32::new(4).unwrap(),
            time: BackupTime::default(),
        };
        let storage = S3Storage::new(&cfg);

        // Seed an existing validated generation so we confirm pre-validation
        // failures leave validated generations untouched.
        let existing_gen = "existing-validated.tar.zst.age".to_owned();
        let existing_val = format!("_validated/{existing_gen}");
        {
            let mut guard = st.lock().unwrap();
            guard
                .objects
                .insert(existing_gen.clone(), b"validated-bytes".to_vec());
            guard.objects.insert(existing_val.clone(), Vec::new());
        }

        // 1a. Pre-validation upload failure: attempts best-effort staging cleanup,
        // preserves no prior success, sets first_attempt_at, scrubs secrets, notifies.
        st.lock().unwrap().fail_put_staging = true;
        let t1: Timestamp = "2030-01-06T03:00:00Z".parse().unwrap();
        let err1 = run_with_storage(&home, &Env::default(), &cfg, &storage, t1).unwrap_err();
        assert!(!err1.to_string().contains("AKIA_SECRET_LEAK"));
        {
            let guard = st.lock().unwrap();
            assert!(
                guard
                    .delete_requests
                    .iter()
                    .any(|k| k.starts_with("_staging/")),
                "upload failure must attempt best-effort staging cleanup"
            );
            assert!(guard.objects.contains_key(&existing_gen));
            assert!(guard.objects.contains_key(&existing_val));
        }
        let state1 = _state::read_state(&home.backup_state_file())
            .unwrap()
            .unwrap();
        assert_eq!(state1.first_attempt_at, t1);
        assert_eq!(state1.last_outcome, BackupOutcome::Failed);
        assert!(state1.last_success_at.is_none());
        assert!(
            !state1
                .last_error
                .as_deref()
                .unwrap()
                .contains("AKIA_SECRET_LEAK")
        );

        // 1b. Pre-validation validation-marker failure: deletes the remote
        // staging object, preserves the primary error, leaves validated
        // generations untouched, and does not advance last_success_at.
        {
            let mut guard = st.lock().unwrap();
            guard.fail_put_staging = false;
            guard.fail_put_validated = true;
            guard.delete_requests.clear();
        }
        let t1b: Timestamp = "2030-01-06T12:00:00Z".parse().unwrap();
        assert!(run_with_storage(&home, &Env::default(), &cfg, &storage, t1b).is_err());
        {
            let guard = st.lock().unwrap();
            assert!(
                guard
                    .delete_requests
                    .iter()
                    .any(|k| k.starts_with("_staging/")),
                "validation-marker failure must delete remote staging object"
            );
            assert!(
                !guard.objects.keys().any(|k| k.starts_with("_staging/")),
                "staging object must be gone after validation-marker failure"
            );
            assert!(guard.objects.contains_key(&existing_gen));
            assert!(guard.objects.contains_key(&existing_val));
        }
        let state1b = _state::read_state(&home.backup_state_file())
            .unwrap()
            .unwrap();
        assert_eq!(state1b.first_attempt_at, t1);
        assert_eq!(state1b.last_outcome, BackupOutcome::Failed);
        assert!(state1b.last_success_at.is_none());

        // 2. Post-validation failure (DELETE _staging fails AFTER _validated/ written):
        // advances last_success_at + last_generation while keeping last_outcome=failed and notifying.
        {
            let mut guard = st.lock().unwrap();
            guard.fail_put_staging = false;
            guard.fail_put_validated = false;
            guard.fail_delete_staging = true;
        }
        let t2: Timestamp = "2030-01-07T03:00:00Z".parse().unwrap();
        assert!(run_with_storage(&home, &Env::default(), &cfg, &storage, t2).is_err());
        let state2 = _state::read_state(&home.backup_state_file())
            .unwrap()
            .unwrap();
        assert_eq!(state2.first_attempt_at, t1);
        assert_eq!(state2.last_attempt_at, t2);
        assert_eq!(state2.last_outcome, BackupOutcome::Failed);
        assert_eq!(state2.last_success_at, Some(t2));
        assert!(state2.last_generation.is_some());

        let notified = std::fs::read_to_string(&notify_log).unwrap();
        let lines: Vec<&str> = notified.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|line| line.starts_with("backup:failed:")));

        server.unblock();
        let _ = handle.join();
    }
}
