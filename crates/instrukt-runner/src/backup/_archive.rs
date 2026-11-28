//! Streaming `tar → zstd (level 3) → age (X25519 STREAM)` archive builder with
//! owner-only (`0700`) staging and RAII cleanup
//! (`project/instrukt/spec/feature/runner/home-backup`).

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use instrukt_core::fsx;
use jiff::Timestamp;

use super::_snapshot::{SnapshotEntry, SnapshotPlan};
use super::BackupError;

/// Zstd compression level chosen by Architecture (`zstd` level 3).
pub(super) const ZSTD_LEVEL: i32 = 3;

/// Supported archive extension for home backups.
pub const ARCHIVE_EXTENSION: &str = ".tar.zst.age";

/// Formats the canonical archive filename for timestamp `now` (`UTC`):
/// `instrukt-home.<YYYYMMDDTHHMMSSZ>.tar.zst.age`.
pub fn archive_filename(now: Timestamp) -> String {
    let dt = now.to_zoned(jiff::tz::TimeZone::UTC);
    format!(
        "instrukt-home.{:04}{:02}{:02}T{:02}{:02}{:02}Z{ARCHIVE_EXTENSION}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
    )
}

/// Owner-only (`0700`) temporary staging directory cleaned up automatically on
/// drop unless explicitly preserved (for failed mid-swap rollback recovery).
pub struct StagingDir {
    path: PathBuf,
    kept: bool,
}

impl StagingDir {
    /// Creates a new `0700` staging directory inside `parent` with `prefix`.
    pub fn create(parent: &Path, prefix: &str) -> Result<Self, BackupError> {
        fsx::create_dir_all(parent).map_err(|source| BackupError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        let mut builder = tempfile::Builder::new();
        builder.prefix(prefix);
        let temp = fsx::temp_dir_in(&builder, parent).map_err(|source| BackupError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        let path = temp.keep();
        if let Err(source) = fsx::set_permissions(&path, std::fs::Permissions::from_mode(0o700)) {
            let _ = fsx::remove_dir_all(&path);
            return Err(BackupError::Io { path, source });
        }
        Ok(Self { path, kept: false })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Leaves this staging directory on disk when dropped (used only if a
    /// mid-swap rollback fails so rollback trash stays recoverable).
    pub fn keep(&mut self) {
        self.kept = true;
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        if let Err(error) = fsx::remove_dir_all(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                event = "backup_staging_cleanup_failed",
                path = %self.path.display(),
                %error,
                "failed to remove temporary backup staging directory"
            );
        }
    }
}

/// Encrypted home backup archive held inside an RAII-cleaned staging directory.
pub struct StagedArchive {
    _staging: StagingDir,
    pub path: PathBuf,
    pub filename: String,
    pub size_bytes: u64,
}

/// Builds a compressed, client-side `age`-encrypted archive from `plan` inside
/// `state_dir/.backup-staging-<rand>/`.
///
/// Each SQLite database is snapshotted online into staging, verified with
/// `PRAGMA quick_check`, streamed immediately into the live archive, and
/// unlinked before the next database is snapshotted so peak disk stays bounded
/// to `max_single_db + archive_size`.
pub(super) fn create_encrypted_archive(
    state_dir: &Path,
    plan: &SnapshotPlan,
    recipient: &age::x25519::Recipient,
    now: Timestamp,
) -> Result<StagedArchive, BackupError> {
    let staging = StagingDir::create(state_dir, ".backup-staging-")?;
    let filename = archive_filename(now);
    let archive_path = staging.path().join(&filename);
    let mut open_opts = OpenOptions::new();
    open_opts.create_new(true).write(true).mode(0o600);
    let file = fsx::open(&open_opts, &archive_path).map_err(|source| BackupError::Io {
        path: archive_path.clone(),
        source,
    })?;

    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))
            .map_err(|err| BackupError::Encryption(err.to_string()))?;
    let age_writer = encryptor
        .wrap_output(file)
        .map_err(|err| BackupError::Encryption(err.to_string()))?;
    let zstd_writer = zstd::stream::write::Encoder::new(age_writer, ZSTD_LEVEL)
        .map_err(|err| BackupError::Archive(format!("failed to initialize zstd encoder: {err}")))?;
    let mut tar_builder = tar::Builder::new(zstd_writer);

    let db_snap_path = staging.path().join("db-snapshot.sqlite");
    for entry in &plan.entries {
        match entry {
            SnapshotEntry::File {
                archive_path: entry_rel,
                source_path,
                ..
            } => {
                append_regular_file(&mut tar_builder, entry_rel, source_path)?;
            }
            SnapshotEntry::Database {
                archive_path: entry_rel,
                source_path,
                ..
            } => {
                // Remove any prior leftover just in case, snapshot online, verify, stream, unlink.
                if db_snap_path.exists() {
                    fsx::remove_file(&db_snap_path).map_err(|source| BackupError::Io {
                        path: db_snap_path.clone(),
                        source,
                    })?;
                }
                instrukt_core::state_db::online_backup(source_path, &db_snap_path).map_err(
                    |err| BackupError::Database {
                        path: source_path.clone(),
                        detail: err.to_string(),
                    },
                )?;
                instrukt_core::state_db::quick_check(&db_snap_path).map_err(|err| {
                    BackupError::Database {
                        path: source_path.clone(),
                        detail: err.to_string(),
                    }
                })?;
                append_regular_file(&mut tar_builder, entry_rel, &db_snap_path)?;
                fsx::remove_file(&db_snap_path).map_err(|source| BackupError::Io {
                    path: db_snap_path.clone(),
                    source,
                })?;
            }
        }
    }

    let zstd_writer = tar_builder
        .into_inner()
        .map_err(|err| BackupError::Archive(format!("failed to finalize tar archive: {err}")))?;
    let mut age_writer = zstd_writer
        .finish()
        .map_err(|err| BackupError::Archive(format!("failed to finalize zstd stream: {err}")))?;
    age_writer
        .flush()
        .map_err(|err| BackupError::Encryption(format!("failed to flush age stream: {err}")))?;
    let file = age_writer
        .finish()
        .map_err(|err| BackupError::Encryption(format!("failed to finalize age stream: {err}")))?;
    file.sync_all().map_err(|source| BackupError::Io {
        path: archive_path.clone(),
        source,
    })?;
    let size_bytes = std::fs::metadata(&archive_path)
        .map_err(|source| BackupError::Io {
            path: archive_path.clone(),
            source,
        })?
        .len();

    Ok(StagedArchive {
        _staging: staging,
        path: archive_path,
        filename,
        size_bytes,
    })
}

fn append_regular_file<W: std::io::Write>(
    tar_builder: &mut tar::Builder<W>,
    archive_rel: &Path,
    source_path: &Path,
) -> Result<(), BackupError> {
    let mut file = File::open(source_path).map_err(|source| BackupError::Io {
        path: source_path.to_path_buf(),
        source,
    })?;
    let meta = file.metadata().map_err(|source| BackupError::Io {
        path: source_path.to_path_buf(),
        source,
    })?;
    let mut header = tar::Header::new_gnu();
    header.set_size(meta.len());
    header.set_mode(0o600);
    header.set_entry_type(tar::EntryType::Regular);
    if let Ok(modified) = meta.modified()
        && let Ok(secs) = modified.duration_since(std::time::UNIX_EPOCH)
    {
        header.set_mtime(secs.as_secs());
    }
    header.set_cksum();
    tar_builder
        .append_data(&mut header, archive_rel, &mut file)
        .map_err(|err| {
            BackupError::Archive(format!(
                "failed to write {} into archive: {err}",
                archive_rel.display()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::_snapshot::collect;
    use instrukt_core::home::Home;
    use instrukt_core::state_db::rusqlite::Connection;
    use std::collections::BTreeMap;
    use std::io::Read as _;

    #[test]
    fn streaming_tar_zstd_age_round_trip_and_corrupt_db_abort() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf()).for_writing();
        std::fs::create_dir_all(home.state_dir()).unwrap();
        std::fs::write(home.global_config(), "schema_version: 8\n").unwrap();
        std::fs::write(home.secrets_file(), "TOP_SECRET=value123\n").unwrap();
        let db_path = home.state_dir().join("instrukt.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute("CREATE TABLE notes (id INT PRIMARY KEY, body TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO notes VALUES (1, 'hello backup')", [])
            .unwrap();
        drop(conn);

        let identity = age::x25519::Identity::generate();
        let recipient = identity.to_public();
        let now: Timestamp = "2030-01-06T03:00:00Z".parse().unwrap();

        let plan = collect(&home).unwrap();
        let staged = create_encrypted_archive(&home.state_dir(), &plan, &recipient, now).unwrap();
        assert_eq!(
            staged.filename,
            "instrukt-home.20300106T030000Z.tar.zst.age"
        );
        assert!(staged.size_bytes > 0);
        // Raw ciphertext does not leak plaintext secret.
        let ciphertext = std::fs::read(&staged.path).unwrap();
        assert!(
            !ciphertext
                .windows(b"TOP_SECRET=value123".len())
                .any(|w| w == b"TOP_SECRET=value123")
        );

        // Decrypt with wrong identity fails.
        let wrong_id = age::x25519::Identity::generate();
        let dec = age::Decryptor::new(File::open(&staged.path).unwrap()).unwrap();
        assert!(
            dec.decrypt(std::iter::once(&wrong_id as &dyn age::Identity))
                .is_err()
        );

        // Decrypt with matching identity restores all entries cleanly.
        let dec = age::Decryptor::new(File::open(&staged.path).unwrap()).unwrap();
        let age_reader = dec
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .unwrap();
        let zstd_reader = zstd::stream::read::Decoder::new(age_reader).unwrap();
        let mut archive = tar::Archive::new(zstd_reader);
        let mut extracted: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().to_string_lossy().into_owned();
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).unwrap();
            extracted.insert(path, buf);
        }
        assert_eq!(
            extracted.get("secrets.env").map(Vec::as_slice),
            Some(b"TOP_SECRET=value123\n".as_slice())
        );
        let restored_db = dir.path().join("restored.db");
        std::fs::write(&restored_db, extracted.get("state/instrukt.db").unwrap()).unwrap();
        instrukt_core::state_db::integrity_check(&restored_db).unwrap();

        // Staging dir is removed on drop.
        let staging_path = staged.path.parent().unwrap().to_path_buf();
        drop(staged);
        assert!(!staging_path.exists());

        // Corrupt DB aborts archive creation and cleans up staging.
        std::fs::write(&db_path, b"not a valid sqlite database").unwrap();
        let plan_corrupt = collect(&home).unwrap();
        let err = create_encrypted_archive(&home.state_dir(), &plan_corrupt, &recipient, now);
        assert!(matches!(err, Err(BackupError::Database { .. })));
        let leftovers: Vec<_> = std::fs::read_dir(home.state_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".backup-staging-")
            })
            .collect();
        assert!(leftovers.is_empty());
    }
}
