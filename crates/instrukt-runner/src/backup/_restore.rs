//! Guarded home backup restore: fail-fast identity/confirmation/extension
//! checks, multi-volume headroom enforcement, same-filesystem `0700` staging,
//! manual `fsx`-admitted extraction, full `PRAGMA integrity_check`, live-home
//! lock guard, leaf-inclusive destination confinement, and rollback-preserving
//! swap (`project/instrukt/spec/feature/runner/home-backup`).

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{BufRead as _, BufReader, IsTerminal as _, Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Component, Path, PathBuf};

use instrukt_core::config::BackupConfig;
use instrukt_core::fsx;
use instrukt_core::home::Home;
use instrukt_core::machine::{MachineId, existing_machine_id};
use zeroize::Zeroizing;

use super::_archive::{ARCHIVE_EXTENSION, StagingDir};
use super::_locks::acquire_live_home_locks;
use super::_snapshot::{
    HEADROOM_FLOOR_BYTES, available_bytes, confine_path, existing_ancestor, is_sqlite_filename,
};
use super::_storage::S3Storage;
use super::BackupError;

/// Re-check available disk headroom every 16 MiB written during extraction.
const EXTRACTION_HEADROOM_INTERVAL_BYTES: u64 = 16 * 1024 * 1024;
const SQLITE_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// How the operator supplies the `age` private identity for restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityInput {
    /// Path to an `age` private identity file on disk.
    Path(PathBuf),
    /// Read the `age` private identity from standard input (`--identity -`).
    Stdin,
    /// Prompt interactively with echo off (refuses fail-closed non-interactively).
    Prompt,
}

/// Parameters for `instrukt backup restore`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOptions {
    pub from: Option<String>,
    pub machine: Option<MachineId>,
    pub into: Option<PathBuf>,
    pub identity: IdentityInput,
    pub yes: bool,
}

/// Summary of a completed restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    pub generation: String,
    pub target: PathBuf,
    pub live_home: bool,
    pub restored_files: usize,
    pub verified_databases: usize,
}

struct StagedFile {
    rel: PathBuf,
    staged_path: PathBuf,
    in_state_volume: bool,
    is_db: bool,
}

enum SwapStep {
    MovedExisting { dest: PathBuf, trash: PathBuf },
    PlacedNew { dest: PathBuf },
    MovedSidecar { sidecar: PathBuf, trash: PathBuf },
}

/// Executes a guarded restore according to `options`.
pub fn restore(home: &Home, options: &RestoreOptions) -> Result<RestoreReport, BackupError> {
    let cfg = super::admitted_backup_config(home)?;
    let storage = S3Storage::new(&cfg);
    restore_with_storage(home, &cfg, &storage, options, HEADROOM_FLOOR_BYTES, None)
}

fn restore_with_storage(
    home: &Home,
    _cfg: &BackupConfig,
    storage: &S3Storage,
    options: &RestoreOptions,
    headroom_floor_bytes: u64,
    swap_fault_after: Option<usize>,
) -> Result<RestoreReport, BackupError> {
    // 1. Validate explicit `--from` archive extension before any S3 call or staging.
    if let Some(ref key) = options.from {
        validate_archive_extension(key)?;
    }

    // 2. Check confirmation capability before S3/staging so non-interactive
    //    sessions without `--yes` fail closed immediately.
    if !options.yes && !confirmation_tty_available(&options.identity) {
        return Err(BackupError::ConfirmationRequired);
    }

    // 3. Acquire and parse the age private identity before S3/staging.
    let identity = load_identity(&options.identity)?;

    // 4. Resolve target generation key.
    let selected_key = resolve_target_key(home, storage, options)?;
    validate_archive_extension(&selected_key)?;

    // 5. Canonicalize destination and compare against active INSTRUKT_HOME.
    let raw_dest = options.into.as_deref().unwrap_or(home.root());
    let canonical_home = canonical_via_existing_ancestor(home.root())?;
    let canonical_dest = canonical_via_existing_ancestor(raw_dest)?;
    let live_home = canonical_dest == canonical_home;

    // If destination already exists, it must be a directory (not a file).
    if let Ok(meta) = std::fs::symlink_metadata(&canonical_dest) {
        if meta.file_type().is_symlink() {
            return Err(BackupError::Unconfined {
                path: canonical_dest,
            });
        }
        if !meta.is_dir() {
            return Err(BackupError::RestoreInvalidTarget {
                path: canonical_dest,
                reason: "target exists and is not a directory".into(),
            });
        }
    }

    let resolved_state = resolve_state_target(home, &canonical_dest, live_home)?;
    let dest_staging_parent = if canonical_dest.exists() {
        canonical_dest.clone()
    } else {
        existing_ancestor(&canonical_dest)?
    };
    let state_existing_parent = existing_ancestor(&resolved_state)?;
    let dest_dev = device_id(&dest_staging_parent)?;
    let state_dev = device_id(&state_existing_parent)?;
    let separate_state_volume = state_dev != dest_dev;

    // 6. HEAD target object and verify headroom across every target filesystem
    //    BEFORE creating any staging directory.
    let encrypted_size = storage.head_object(&selected_key)?;
    let required_bytes = headroom_floor_bytes.saturating_add(encrypted_size.saturating_mul(4));
    check_volume_headroom(&dest_staging_parent, required_bytes)?;
    if separate_state_volume {
        check_volume_headroom(&state_existing_parent, required_bytes)?;
    }

    // 7. Create owner-only (0700) same-filesystem staging directories (RAII drop).
    let dest_staging = StagingDir::create(&dest_staging_parent, ".backup-restore-staging-")?;
    let state_staging = if separate_state_volume {
        Some(StagingDir::create(
            &state_existing_parent,
            ".backup-restore-staging-",
        )?)
    } else {
        None
    };

    // 8. Download encrypted archive into staging, decrypt, and manually extract
    //    via fsx while rejecting unsafe paths/types and enforcing headroom.
    let archive_path = dest_staging.path().join("archive.tar.zst.age");
    storage.download_object(&selected_key, &archive_path)?;

    let dest_tree_root = dest_staging.path().join("tree");
    fsx::create_dir_all(&dest_tree_root).map_err(|source| BackupError::Io {
        path: dest_tree_root.clone(),
        source,
    })?;
    let state_tree_root = if let Some(ref st_dir) = state_staging {
        let root = st_dir.path().join("tree");
        fsx::create_dir_all(&root).map_err(|source| BackupError::Io {
            path: root.clone(),
            source,
        })?;
        Some(root)
    } else {
        None
    };

    let extracted = extract_encrypted_archive(
        &archive_path,
        &identity,
        &dest_tree_root,
        state_tree_root.as_deref(),
        &dest_staging_parent,
        &state_existing_parent,
        headroom_floor_bytes,
    )?;

    // Drop downloaded ciphertext immediately to free staging space.
    let _ = fsx::remove_file(&archive_path);

    // 9. Run full PRAGMA integrity_check on EVERY restored SQLite database in
    //    staging BEFORE confirmation, lock acquisition, or destination touch.
    let mut verified_databases = 0usize;
    for file in &extracted.files {
        if file.is_db {
            instrukt_core::state_db::integrity_check(&file.staged_path).map_err(|err| {
                BackupError::Database {
                    path: file.rel.clone(),
                    detail: err.to_string(),
                }
            })?;
            verified_databases += 1;
        }
    }

    // 10. Unconditional overwrite confirmation unless `--yes`.
    if !options.yes {
        confirm_overwrite(&selected_key, &canonical_dest, &options.identity)?;
    }

    // 11. Live-home lock guard (held across the entire destination swap).
    let _live_locks = if live_home {
        Some(acquire_live_home_locks(
            home,
            &canonical_dest,
            &resolved_state,
            &extracted.companies,
            &extracted.known_issues,
        )?)
    } else {
        None
    };

    // 12. Ensure destination root exists (only now that confirmation & locks
    //     succeeded) and enforce leaf-inclusive confinement on every restored
    //     path and its SQLite sidecars AFTER locks are held.
    if !canonical_dest.exists() {
        fsx::create_dir_all(&canonical_dest).map_err(|source| BackupError::Io {
            path: canonical_dest.clone(),
            source,
        })?;
        fsx::set_permissions(&canonical_dest, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| BackupError::Io {
                path: canonical_dest.clone(),
                source,
            },
        )?;
    }
    let final_dest = canonical_dest
        .canonicalize()
        .map_err(|source| BackupError::Io {
            path: canonical_dest.clone(),
            source,
        })?;
    let final_state = if resolved_state.exists() {
        resolved_state
            .canonicalize()
            .map_err(|source| BackupError::Io {
                path: resolved_state.clone(),
                source,
            })?
    } else {
        final_dest.join("state")
    };

    let planned_swaps = plan_confined_swaps(&final_dest, &final_state, &extracted.files)?;

    // 13. Rollback-preserving swap into destination.
    execute_rollback_swap(
        &selected_key,
        &final_dest,
        &final_state,
        separate_state_volume,
        &planned_swaps,
        swap_fault_after,
    )?;

    Ok(RestoreReport {
        generation: selected_key,
        target: final_dest,
        live_home,
        restored_files: planned_swaps.len(),
        verified_databases,
    })
}

fn validate_archive_extension(key: &str) -> Result<(), BackupError> {
    if !key.ends_with(ARCHIVE_EXTENSION) {
        return Err(BackupError::UnsupportedArchiveFormat {
            key: key.to_owned(),
            expected_extension: ARCHIVE_EXTENSION,
        });
    }
    Ok(())
}

fn confirmation_tty_available(identity: &IdentityInput) -> bool {
    if *identity == IdentityInput::Stdin {
        std::io::stderr().is_terminal() && open_controlling_tty().is_ok()
    } else {
        std::io::stdin().is_terminal()
    }
}

fn open_controlling_tty() -> std::io::Result<File> {
    let file = File::open("/dev/tty")?;
    if !file.is_terminal() {
        return Err(std::io::Error::other("/dev/tty is not a terminal"));
    }
    Ok(file)
}

fn confirm_overwrite(
    generation: &str,
    dest: &Path,
    identity: &IdentityInput,
) -> Result<(), BackupError> {
    let prompt = format!(
        "Restore `{generation}` into `{}`? Existing files may be replaced. [y/N]: ",
        dest.display()
    );
    let mut stderr = std::io::stderr();
    stderr
        .write_all(prompt.as_bytes())
        .and_then(|()| stderr.flush())
        .map_err(|source| BackupError::Io {
            path: PathBuf::from("stderr"),
            source,
        })?;
    let answer = if *identity == IdentityInput::Stdin || !std::io::stdin().is_terminal() {
        let tty = open_controlling_tty().map_err(|_| BackupError::ConfirmationRequired)?;
        let mut reader = BufReader::new(tty);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|source| BackupError::Io {
                path: PathBuf::from("/dev/tty"),
                source,
            })?;
        line
    } else {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|source| BackupError::Io {
                path: PathBuf::from("stdin"),
                source,
            })?;
        line
    };
    let trimmed = answer.trim();
    if trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err(BackupError::RestoreDeclined)
    }
}

fn load_identity(input: &IdentityInput) -> Result<age::x25519::Identity, BackupError> {
    let raw: Zeroizing<String> = match input {
        IdentityInput::Path(path) => {
            if path.to_string_lossy().contains("AGE-SECRET-KEY-") {
                return Err(BackupError::InvalidIdentity(
                    "raw age secret keys are not accepted as command-line arguments; pass a path to an identity file via `--identity <path>` or pipe it via `--identity -`".into(),
                ));
            }
            let content = std::fs::read_to_string(path).map_err(|source| BackupError::Io {
                path: path.clone(),
                source,
            })?;
            Zeroizing::new(content)
        }
        IdentityInput::Stdin => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|source| BackupError::Io {
                    path: PathBuf::from("stdin"),
                    source,
                })?;
            Zeroizing::new(buf)
        }
        IdentityInput::Prompt => {
            if !std::io::stdin().is_terminal() {
                return Err(BackupError::InvalidIdentity(
                    "cannot prompt for age identity in a non-interactive session; pass `--identity <path>` or `--identity -`".into(),
                ));
            }
            let entered = rpassword::prompt_password("age private identity: ").map_err(|err| {
                BackupError::InvalidIdentity(format!("failed to read age identity from tty: {err}"))
            })?;
            Zeroizing::new(entered)
        }
    };
    parse_age_identity(&raw)
}

fn parse_age_identity(raw: &str) -> Result<age::x25519::Identity, BackupError> {
    let mut found: Option<age::x25519::Identity> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parsed = trimmed.parse::<age::x25519::Identity>().map_err(|_| {
            BackupError::InvalidIdentity(
                "identity input does not contain a valid age X25519 secret key".into(),
            )
        })?;
        if found.is_some() {
            return Err(BackupError::InvalidIdentity(
                "identity input contains multiple keys; supply one X25519 identity".into(),
            ));
        }
        found = Some(parsed);
    }
    found.ok_or_else(|| {
        BackupError::InvalidIdentity(
            "identity input did not contain an age X25519 secret key".into(),
        )
    })
}

fn resolve_target_key(
    home: &Home,
    storage: &S3Storage,
    options: &RestoreOptions,
) -> Result<String, BackupError> {
    if let Some(ref key) = options.from {
        return Ok(key.clone());
    }
    let machine = match options.machine {
        Some(ref mid) => mid.clone(),
        None => existing_machine_id(home)?.ok_or_else(|| {
            BackupError::NoGenerationFound(
                "this home has no state/machine-id yet; pass `--machine <id>` or `--from <key>`"
                    .into(),
            )
        })?,
    };
    let generations = storage.list_generations(&machine)?;
    if let Some(newest_validated) = generations.iter().find(|g| g.validated) {
        return Ok(newest_validated.key.clone());
    }
    if generations.is_empty() {
        return Err(BackupError::NoGenerationFound(format!(
            "no backup generations found for machine `{machine}`"
        )));
    }
    Err(BackupError::NoGenerationFound(format!(
        "no validated backup generation found for machine `{machine}`; pass `--from <key>` to restore an unvalidated generation explicitly"
    )))
}

fn canonical_via_existing_ancestor(path: &Path) -> Result<PathBuf, BackupError> {
    match path.canonicalize() {
        Ok(canon) => Ok(canon),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // Reject a dangling symlink at `path` itself.
            if let Ok(meta) = std::fs::symlink_metadata(path)
                && meta.file_type().is_symlink()
            {
                return Err(BackupError::Unconfined {
                    path: path.to_path_buf(),
                });
            }
            let mut tail = Vec::new();
            let mut cursor = path.to_path_buf();
            while !cursor.exists() {
                if let Ok(meta) = std::fs::symlink_metadata(&cursor)
                    && meta.file_type().is_symlink()
                {
                    return Err(BackupError::Unconfined { path: cursor });
                }
                let Some(name) = cursor.file_name() else {
                    return Err(BackupError::Io {
                        path: path.to_path_buf(),
                        source: err,
                    });
                };
                tail.push(name.to_os_string());
                let Some(parent) = cursor.parent() else {
                    return Err(BackupError::Io {
                        path: path.to_path_buf(),
                        source: err,
                    });
                };
                cursor = parent.to_path_buf();
            }
            let mut base = cursor.canonicalize().map_err(|source| BackupError::Io {
                path: cursor,
                source,
            })?;
            for part in tail.into_iter().rev() {
                base.push(part);
            }
            Ok(base)
        }
        Err(source) => Err(BackupError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn resolve_state_target(
    home: &Home,
    canonical_dest: &Path,
    live_home: bool,
) -> Result<PathBuf, BackupError> {
    let candidate = if live_home {
        home.state_dir()
    } else {
        canonical_dest.join("state")
    };
    canonical_via_existing_ancestor(&candidate)
}

fn device_id(path: &Path) -> Result<u64, BackupError> {
    let meta = std::fs::metadata(path).map_err(|source| BackupError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(meta.dev())
}

fn check_volume_headroom(volume_path: &Path, required_bytes: u64) -> Result<(), BackupError> {
    let available = available_bytes(volume_path)?;
    if available < required_bytes {
        return Err(BackupError::InsufficientDiskSpace {
            path: volume_path.to_path_buf(),
            available_bytes: available,
            required_bytes,
        });
    }
    Ok(())
}

struct ExtractedTree {
    files: Vec<StagedFile>,
    companies: BTreeSet<String>,
    known_issues: BTreeSet<String>,
}

fn extract_encrypted_archive(
    archive_path: &Path,
    identity: &age::x25519::Identity,
    dest_tree_root: &Path,
    state_tree_root: Option<&Path>,
    dest_volume: &Path,
    state_volume: &Path,
    headroom_floor_bytes: u64,
) -> Result<ExtractedTree, BackupError> {
    let file = File::open(archive_path).map_err(|source| BackupError::Io {
        path: archive_path.to_path_buf(),
        source,
    })?;
    let decryptor = age::Decryptor::new(BufReader::new(file))
        .map_err(|err| BackupError::Encryption(format!("invalid age archive header: {err}")))?;
    let age_reader = decryptor
        .decrypt(std::iter::once(identity as &dyn age::Identity))
        .map_err(|err| BackupError::Encryption(format!("age decryption failed: {err}")))?;
    let zstd_reader = zstd::stream::read::Decoder::new(age_reader)
        .map_err(|err| BackupError::Archive(format!("zstd decompression init failed: {err}")))?;
    let mut tar_archive = tar::Archive::new(zstd_reader);

    let mut files = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut companies = BTreeSet::new();
    let mut known_issues = BTreeSet::new();
    let mut bytes_since_headroom_check = 0u64;

    let entries = tar_archive
        .entries()
        .map_err(|err| BackupError::Archive(format!("failed reading tar entries: {err}")))?;
    for entry_res in entries {
        let mut entry =
            entry_res.map_err(|err| BackupError::Archive(format!("malformed tar entry: {err}")))?;
        let entry_type = entry.header().entry_type();
        if !matches!(
            entry_type,
            tar::EntryType::Regular | tar::EntryType::Directory
        ) {
            return Err(BackupError::UnsafeArchiveEntry {
                path: entry
                    .path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "<unreadable>".into()),
                reason: format!("disallowed archive entry type `{entry_type:?}`"),
            });
        }

        let raw_path = entry
            .path()
            .map_err(|err| BackupError::Archive(format!("unreadable tar entry path: {err}")))?
            .into_owned();
        let rel = validate_archive_rel_path(&raw_path)?;

        record_staged_scopes(&rel, &mut companies, &mut known_issues);

        let (staged_path, in_state_volume) = if let Some(st_root) = state_tree_root
            && let Ok(sub) = rel.strip_prefix("state")
        {
            (st_root.join(sub), true)
        } else {
            (dest_tree_root.join(&rel), false)
        };

        if entry_type == tar::EntryType::Directory {
            fsx::create_dir_all(&staged_path).map_err(|source| BackupError::Io {
                path: staged_path.clone(),
                source,
            })?;
            continue;
        }

        if !seen_paths.insert(rel.clone()) {
            return Err(BackupError::UnsafeArchiveEntry {
                path: rel.display().to_string(),
                reason: "duplicate file entry in archive".into(),
            });
        }

        if let Some(parent) = staged_path.parent() {
            fsx::create_dir_all(parent).map_err(|source| BackupError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut open_opts = OpenOptions::new();
        open_opts.create_new(true).write(true).mode(0o600);
        let mut out_file =
            fsx::open(&open_opts, &staged_path).map_err(|source| BackupError::Io {
                path: staged_path.clone(),
                source,
            })?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.read(&mut buf).map_err(|err| {
                BackupError::Archive(format!("failed extracting `{}`: {err}", rel.display()))
            })?;
            if n == 0 {
                break;
            }
            out_file
                .write_all(&buf[..n])
                .map_err(|source| BackupError::Io {
                    path: staged_path.clone(),
                    source,
                })?;
            bytes_since_headroom_check += n as u64;
            if bytes_since_headroom_check >= EXTRACTION_HEADROOM_INTERVAL_BYTES {
                bytes_since_headroom_check = 0;
                let vol = if in_state_volume {
                    state_volume
                } else {
                    dest_volume
                };
                check_volume_headroom(vol, headroom_floor_bytes)?;
            }
        }
        out_file.sync_all().map_err(|source| BackupError::Io {
            path: staged_path.clone(),
            source,
        })?;
        fsx::set_permissions(&staged_path, std::fs::Permissions::from_mode(0o600)).map_err(
            |source| BackupError::Io {
                path: staged_path.clone(),
                source,
            },
        )?;

        let is_db = rel
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_sqlite_filename);
        files.push(StagedFile {
            rel,
            staged_path,
            in_state_volume,
            is_db,
        });
    }

    Ok(ExtractedTree {
        files,
        companies,
        known_issues,
    })
}

fn validate_archive_rel_path(raw: &Path) -> Result<PathBuf, BackupError> {
    if raw.is_absolute() {
        return Err(BackupError::UnsafeArchiveEntry {
            path: raw.display().to_string(),
            reason: "absolute paths are forbidden in backup archives".into(),
        });
    }
    let mut normalized = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BackupError::UnsafeArchiveEntry {
                    path: raw.display().to_string(),
                    reason: "path traversal or root component is forbidden in backup archives"
                        .into(),
                });
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(BackupError::UnsafeArchiveEntry {
            path: raw.display().to_string(),
            reason: "empty path in backup archive".into(),
        });
    }
    Ok(normalized)
}

fn record_staged_scopes(
    rel: &Path,
    companies: &mut BTreeSet<String>,
    known_issues: &mut BTreeSet<String>,
) {
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    if parts.first() == Some(&"companies")
        && let Some(company) = parts.get(1)
    {
        companies.insert((*company).to_owned());
    }
    if parts.first() == Some(&"state")
        && parts.get(1) == Some(&"known-issues")
        && let Some(file) = parts.get(2)
        && let Some(stem) = Path::new(file).file_stem().and_then(|s| s.to_str())
        && !stem.is_empty()
    {
        known_issues.insert(stem.to_owned());
    }
}

struct PlannedSwap<'a> {
    staged: &'a StagedFile,
    dest: PathBuf,
    sidecars: Vec<PathBuf>,
}

fn plan_confined_swaps<'a>(
    final_dest: &Path,
    final_state: &Path,
    files: &'a [StagedFile],
) -> Result<Vec<PlannedSwap<'a>>, BackupError> {
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let (owning_root, sub_rel) = match file.rel.strip_prefix("state") {
            Ok(sub) if !sub.as_os_str().is_empty() => (final_state, sub),
            _ => (final_dest, file.rel.as_path()),
        };
        let dest = confine_path(owning_root, sub_rel)?;
        let mut sidecars = Vec::new();
        if file.is_db {
            let base_name = sub_rel
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| BackupError::Unconfined { path: dest.clone() })?;
            let sub_parent = sub_rel.parent().unwrap_or(Path::new(""));
            for suffix in SQLITE_SIDECAR_SUFFIXES {
                let sidecar_rel = sub_parent.join(format!("{base_name}{suffix}"));
                let confined_sidecar = confine_path(owning_root, &sidecar_rel)?;
                sidecars.push(confined_sidecar);
            }
        }
        out.push(PlannedSwap {
            staged: file,
            dest,
            sidecars,
        });
    }
    Ok(out)
}

fn execute_rollback_swap(
    generation_key: &str,
    final_dest: &Path,
    final_state: &Path,
    separate_state_volume: bool,
    swaps: &[PlannedSwap<'_>],
    fault_after_step: Option<usize>,
) -> Result<(), BackupError> {
    let mut dest_rollback = StagingDir::create(final_dest, ".backup-restore-rollback-")?;
    let mut state_rollback = if separate_state_volume {
        if !final_state.exists() {
            fsx::create_dir_all(final_state).map_err(|source| BackupError::Io {
                path: final_state.to_path_buf(),
                source,
            })?;
        }
        Some(StagingDir::create(
            final_state,
            ".backup-restore-rollback-",
        )?)
    } else {
        None
    };

    let mut journal: Vec<SwapStep> = Vec::new();
    let swap_result: Result<(), BackupError> = (|| {
        for (idx, item) in swaps.iter().enumerate() {
            if let Some(limit) = fault_after_step
                && idx >= limit
            {
                return Err(BackupError::RestoreSwapFailed {
                    cause: format!("injected swap failure at entry {idx}"),
                    rolled_back: false,
                    key: generation_key.to_owned(),
                    rollback_hint: String::new(),
                });
            }

            if let Some(parent) = item.dest.parent() {
                fsx::create_dir_all(parent).map_err(|source| BackupError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
                if is_private_dir(&item.staged.rel) {
                    let _ = fsx::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
                }
            }

            let trash_root = if item.staged.in_state_volume {
                state_rollback
                    .as_ref()
                    .map_or(dest_rollback.path(), StagingDir::path)
            } else {
                dest_rollback.path()
            };

            if item.dest.exists() {
                let trash = trash_root.join(format!("old-{idx}"));
                fsx::rename(&item.dest, &trash).map_err(|source| BackupError::Io {
                    path: item.dest.clone(),
                    source,
                })?;
                journal.push(SwapStep::MovedExisting {
                    dest: item.dest.clone(),
                    trash,
                });
            }

            fsx::rename(&item.staged.staged_path, &item.dest).map_err(|source| {
                BackupError::Io {
                    path: item.dest.clone(),
                    source,
                }
            })?;
            journal.push(SwapStep::PlacedNew {
                dest: item.dest.clone(),
            });

            fsx::set_permissions(&item.dest, std::fs::Permissions::from_mode(0o600)).map_err(
                |source| BackupError::Io {
                    path: item.dest.clone(),
                    source,
                },
            )?;

            // Move pre-existing SQLite sidecars into same-filesystem rollback
            // trash AFTER the main DB rename succeeded (never unlink mid-swap).
            for (sidx, sidecar) in item.sidecars.iter().enumerate() {
                if sidecar.exists() {
                    let sidecar_trash = trash_root.join(format!("sidecar-{idx}-{sidx}"));
                    fsx::rename(sidecar, &sidecar_trash).map_err(|source| BackupError::Io {
                        path: sidecar.clone(),
                        source,
                    })?;
                    journal.push(SwapStep::MovedSidecar {
                        sidecar: sidecar.clone(),
                        trash: sidecar_trash,
                    });
                }
            }
        }
        Ok(())
    })();

    if let Err(err) = swap_result {
        let mut rollback_errors = Vec::new();
        for step in journal.into_iter().rev() {
            match step {
                SwapStep::MovedSidecar { sidecar, trash } => {
                    if let Err(rb_err) = fsx::rename(&trash, &sidecar) {
                        rollback_errors.push(format!(
                            "restore sidecar {} failed: {rb_err}",
                            sidecar.display()
                        ));
                    }
                }
                SwapStep::PlacedNew { dest } => {
                    if let Err(rb_err) = fsx::remove_file(&dest)
                        && rb_err.kind() != std::io::ErrorKind::NotFound
                    {
                        rollback_errors
                            .push(format!("remove staged {} failed: {rb_err}", dest.display()));
                    }
                }
                SwapStep::MovedExisting { dest, trash } => {
                    if let Err(rb_err) = fsx::rename(&trash, &dest) {
                        rollback_errors.push(format!(
                            "restore original {} failed: {rb_err}",
                            dest.display()
                        ));
                    }
                }
            }
        }

        let cause = match err {
            BackupError::RestoreSwapFailed { cause, .. } => cause,
            other => other.to_string(),
        };
        if rollback_errors.is_empty() {
            return Err(BackupError::RestoreSwapFailed {
                cause,
                rolled_back: true,
                key: generation_key.to_owned(),
                rollback_hint: "all destination files and SQLite sidecars were rolled back cleanly"
                    .into(),
            });
        }

        // Preserve rollback trash directories on disk so nothing is lost.
        dest_rollback.keep();
        let mut preserved = vec![dest_rollback.path().display().to_string()];
        if let Some(ref mut st_rb) = state_rollback {
            st_rb.keep();
            preserved.push(st_rb.path().display().to_string());
        }
        return Err(BackupError::RestoreSwapFailed {
            cause: format!("{cause}; rollback errors: {}", rollback_errors.join("; ")),
            rolled_back: false,
            key: generation_key.to_owned(),
            rollback_hint: format!(
                "partial state remains; rollback staging preserved at [{}]; re-run `instrukt backup restore --from {generation_key}` or extract into an alternate directory with `--into <dir>`",
                preserved.join(", ")
            ),
        });
    }

    // Swap committed cleanly: dropping rollback `StagingDir`s removes trash
    // and moved SQLite sidecars only now.
    Ok(())
}

fn is_private_dir(rel: &Path) -> bool {
    rel.starts_with("state") || rel.starts_with("companies")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU32;
    use std::os::unix::fs::symlink;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use instrukt_core::config::{BackupTime, SecretValue};
    use instrukt_core::fsx::LockGuard;
    use jiff::Timestamp;

    use super::*;
    use crate::backup::{_storage::validated_key, stage_backup};

    struct SimpleS3 {
        endpoint: String,
        objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        server: Arc<tiny_http::Server>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl SimpleS3 {
        fn start() -> Self {
            let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
            let port = server.server_addr().to_ip().unwrap().port();
            let endpoint = format!("http://127.0.0.1:{port}");
            let objects = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
            let srv = Arc::clone(&server);
            let objs = Arc::clone(&objects);
            let handle = thread::spawn(move || {
                for req in srv.incoming_requests() {
                    let method = req.method().as_str().to_owned();
                    let url = req.url().to_owned();
                    let path = url.split('?').next().unwrap_or("");
                    let key = path
                        .trim_start_matches('/')
                        .split_once('/')
                        .map(|(_, k)| k.to_owned())
                        .unwrap_or_default();
                    let guard = objs.lock().unwrap();
                    match method.as_str() {
                        "GET" if url.contains("list-type=2") => {
                            let prefix = url
                                .split('?')
                                .nth(1)
                                .and_then(|q| {
                                    form_urlencoded::parse(q.as_bytes())
                                        .find(|(k, _)| k == "prefix")
                                        .map(|(_, v)| v.into_owned())
                                })
                                .unwrap_or_default();
                            let mut xml =
                                String::from("<ListBucketResult><IsTruncated>false</IsTruncated>");
                            for (k, v) in guard.iter() {
                                if k.starts_with(&prefix) {
                                    xml.push_str(&format!(
                                        "<Contents><Key>{k}</Key><Size>{}</Size><LastModified>2030-01-06T03:00:00Z</LastModified></Contents>",
                                        v.len()
                                    ));
                                }
                            }
                            xml.push_str("</ListBucketResult>");
                            let _ = req.respond(tiny_http::Response::from_string(xml));
                        }
                        "HEAD" => {
                            if let Some(bytes) = guard.get(&key) {
                                let hdr = tiny_http::Header::from_bytes(
                                    "Content-Length",
                                    bytes.len().to_string(),
                                )
                                .unwrap();
                                let _ =
                                    req.respond(tiny_http::Response::empty(200).with_header(hdr));
                            } else {
                                let _ = req.respond(tiny_http::Response::empty(404));
                            }
                        }
                        "GET" => {
                            if let Some(bytes) = guard.get(&key).cloned() {
                                let _ = req.respond(tiny_http::Response::from_data(bytes));
                            } else {
                                let _ = req.respond(tiny_http::Response::empty(404));
                            }
                        }
                        _ => {
                            let _ = req.respond(tiny_http::Response::empty(400));
                        }
                    }
                }
            });
            Self {
                endpoint,
                objects,
                server,
                handle: Some(handle),
            }
        }

        fn config(&self, recipient: age::x25519::Recipient) -> BackupConfig {
            BackupConfig {
                endpoint: url::Url::parse(&self.endpoint).unwrap(),
                region: "auto".into(),
                bucket: "bkt".into(),
                access_key_id: SecretValue::new("AKIA_TEST".into()),
                secret_access_key: SecretValue::new("SECRET_TEST".into()),
                age_recipient: recipient,
                prefix: None,
                keep: NonZeroU32::new(5).unwrap(),
                time: BackupTime::default(),
            }
        }
    }

    impl Drop for SimpleS3 {
        fn drop(&mut self) {
            self.server.unblock();
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn encrypt_tar_bytes(tar_bytes: &[u8], recipient: &age::x25519::Recipient) -> Vec<u8> {
        let mut encrypted = Vec::new();
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))
                .unwrap();
        let age_writer = encryptor.wrap_output(&mut encrypted).unwrap();
        let mut zstd_writer = zstd::stream::write::Encoder::new(age_writer, 3).unwrap();
        zstd_writer.write_all(tar_bytes).unwrap();
        let age_writer = zstd_writer.finish().unwrap();
        age_writer.finish().unwrap();
        encrypted
    }

    #[test]
    fn rejects_raw_secret_key_without_echo_and_unsupported_extension() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.root()).unwrap();
        let identity = age::x25519::Identity::generate();
        let s3 = SimpleS3::start();
        let cfg = s3.config(identity.to_public());
        let storage = S3Storage::new(&cfg);

        // Unsupported extension rejected before HEAD/staging.
        let bad_ext = RestoreOptions {
            from: Some("0123456789abcdef/archive.tar.gz.age".into()),
            machine: None,
            into: Some(dir.path().join("out")),
            identity: IdentityInput::Path(dir.path().join("id.txt")),
            yes: true,
        };
        assert!(matches!(
            restore_with_storage(&home, &cfg, &storage, &bad_ext, 0, None),
            Err(BackupError::UnsupportedArchiveFormat { .. })
        ));

        // Raw AGE-SECRET-KEY- passed as path rejected without echoing key.
        let raw_key = "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ";
        let raw_opts = RestoreOptions {
            from: Some("0123456789abcdef/instrukt-home.20300106T030000Z.tar.zst.age".into()),
            machine: None,
            into: Some(dir.path().join("out")),
            identity: IdentityInput::Path(PathBuf::from(raw_key)),
            yes: true,
        };
        let err = restore_with_storage(&home, &cfg, &storage, &raw_opts, 0, None).unwrap_err();
        assert!(!err.to_string().contains(raw_key));
        assert!(matches!(err, BackupError::InvalidIdentity(_)));
    }

    #[test]
    fn rejects_symlink_entry_traversal_and_corrupt_sqlite_without_touching_dest() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.root()).unwrap();
        let identity = age::x25519::Identity::generate();
        let id_file = dir.path().join("identity.txt");
        std::fs::write(
            &id_file,
            format!("{}\n", {
                use age::secrecy::ExposeSecret as _;
                identity.to_string().expose_secret().to_owned()
            }),
        )
        .unwrap();

        let s3 = SimpleS3::start();
        let cfg = s3.config(identity.to_public());
        let storage = S3Storage::new(&cfg);

        // 1. Archive containing a symlink entry.
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_cksum();
            builder
                .append_link(&mut header, "evil-link", "/etc/passwd")
                .unwrap();
            builder.finish().unwrap();
        }
        let key_sym = "0123456789abcdef/instrukt-home.20300106T030000Z.tar.zst.age";
        s3.objects.lock().unwrap().insert(
            key_sym.into(),
            encrypt_tar_bytes(&tar_buf, &identity.to_public()),
        );

        let dest = dir.path().join("alt-dest");
        let opts = RestoreOptions {
            from: Some(key_sym.into()),
            machine: None,
            into: Some(dest.clone()),
            identity: IdentityInput::Path(id_file.clone()),
            yes: true,
        };
        assert!(matches!(
            restore_with_storage(&home, &cfg, &storage, &opts, 0, None),
            Err(BackupError::UnsafeArchiveEntry { .. })
        ));
        // Aborted restore must not leave an empty destination directory behind.
        assert!(!dest.exists());

        // 2. Archive containing a corrupt SQLite database.
        let mut corrupt_tar = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut corrupt_tar);
            let data = b"not a sqlite database";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            builder
                .append_data(&mut header, "state/instrukt.db", &data[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let key_bad_db = "0123456789abcdef/instrukt-home.20300107T030000Z.tar.zst.age";
        s3.objects.lock().unwrap().insert(
            key_bad_db.into(),
            encrypt_tar_bytes(&corrupt_tar, &identity.to_public()),
        );
        let opts_db = RestoreOptions {
            from: Some(key_bad_db.into()),
            machine: None,
            into: Some(dest.clone()),
            identity: IdentityInput::Path(id_file),
            yes: true,
        };
        assert!(matches!(
            restore_with_storage(&home, &cfg, &storage, &opts_db, 0, None),
            Err(BackupError::Database { .. })
        ));
        assert!(!dest.exists());
    }

    #[test]
    fn full_round_trip_live_home_guard_and_mid_swap_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.state_dir()).unwrap();
        std::fs::write(home.global_config(), "schema_version: 8\n").unwrap();
        std::fs::write(home.secrets_file(), "TOKEN=original\n").unwrap();
        let mid = instrukt_core::machine::machine_id(&home).unwrap();

        let db_path = home.state_db_file();
        {
            let _db = instrukt_core::state_db::StateDb::open(&home).unwrap();
        }

        let identity = age::x25519::Identity::generate();
        let id_file = dir.path().join("id.txt");
        std::fs::write(
            &id_file,
            format!("{}\n", {
                use age::secrecy::ExposeSecret as _;
                identity.to_string().expose_secret().to_owned()
            }),
        )
        .unwrap();

        let now: Timestamp = "2030-01-06T03:00:00Z".parse().unwrap();
        let staged = stage_backup(&home, &identity.to_public(), now).unwrap();
        let archive_bytes = std::fs::read(&staged.path).unwrap();
        let gen_key = format!("{}/{}", mid.as_str(), staged.filename);

        let s3 = SimpleS3::start();
        {
            let mut objs = s3.objects.lock().unwrap();
            objs.insert(gen_key.clone(), archive_bytes);
            objs.insert(validated_key(&gen_key), Vec::new());
        }
        let cfg = s3.config(identity.to_public());
        let storage = S3Storage::new(&cfg);

        // Mutate live home after backup, including a live WAL sidecar.
        std::fs::write(home.secrets_file(), "TOKEN=mutated\n").unwrap();
        let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
        std::fs::write(&wal_path, b"stale-wal-bytes").unwrap();

        // 1. Symlinked alias to live home triggers live-home lock guard when writer.lock is held.
        let alias = dir.path().join("home-alias");
        symlink(home.root(), &alias).unwrap();
        {
            let _held = LockGuard::try_acquire(&home.writer_lock_file())
                .unwrap()
                .unwrap();
            let opts_alias = RestoreOptions {
                from: None,
                machine: None,
                into: Some(alias.clone()),
                identity: IdentityInput::Path(id_file.clone()),
                yes: true,
            };
            assert!(matches!(
                restore_with_storage(&home, &cfg, &storage, &opts_alias, 0, None),
                Err(BackupError::LiveHomeBusy { .. })
            ));
        }

        // 2. Mid-swap failure rolls back original files and pre-existing WAL sidecar intact.
        let opts_live = RestoreOptions {
            from: None,
            machine: None,
            into: None,
            identity: IdentityInput::Path(id_file.clone()),
            yes: true,
        };
        let rb_err =
            restore_with_storage(&home, &cfg, &storage, &opts_live, 0, Some(2)).unwrap_err();
        assert!(matches!(
            rb_err,
            BackupError::RestoreSwapFailed {
                rolled_back: true,
                ..
            }
        ));
        assert_eq!(
            std::fs::read_to_string(home.secrets_file()).unwrap(),
            "TOKEN=mutated\n"
        );
        assert_eq!(std::fs::read(&wal_path).unwrap(), b"stale-wal-bytes");

        // 3. Clean live-home restore commits, removes stale WAL sidecar, and restores 0600 files.
        let report = restore_with_storage(&home, &cfg, &storage, &opts_live, 0, None).unwrap();
        assert!(report.live_home);
        assert_eq!(report.verified_databases, 1);
        assert_eq!(
            std::fs::read_to_string(home.secrets_file()).unwrap(),
            "TOKEN=original\n"
        );
        assert!(!wal_path.exists());
        assert_eq!(
            std::fs::metadata(home.secrets_file())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        // 4. Alternate target (`--into <dir>`) restores cleanly even while writer.lock is held.
        let alt = dir.path().join("extracted");
        let _held = LockGuard::try_acquire(&home.writer_lock_file())
            .unwrap()
            .unwrap();
        let opts_alt = RestoreOptions {
            from: None,
            machine: None,
            into: Some(alt.clone()),
            identity: IdentityInput::Path(id_file),
            yes: true,
        };
        let alt_report = restore_with_storage(&home, &cfg, &storage, &opts_alt, 0, None).unwrap();
        assert!(!alt_report.live_home);
        assert_eq!(
            std::fs::read_to_string(alt.join("secrets.env")).unwrap(),
            "TOKEN=original\n"
        );
    }
}
