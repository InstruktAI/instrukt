//! Snapshot scope collection, owning-root symlink confinement, company
//! forward-conversion delegation, and pre-staging disk headroom checks
//! (`project/instrukt/spec/feature/runner/home-backup`, Snapshot scope).

use std::path::{Path, PathBuf};

use instrukt_core::fsx;
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;

use super::BackupError;

/// Shared-machine free-space floor enforced before backup staging (`20 GiB`).
pub(super) const HEADROOM_FLOOR_BYTES: u64 = 20 * 1024 * 1024 * 1024;

/// One item collected for archival.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SnapshotEntry {
    /// Regular file streamed directly into the tar archive.
    File {
        archive_path: PathBuf,
        source_path: PathBuf,
        size_bytes: u64,
    },
    /// Live SQLite database snapshotted online and verified via `quick_check`.
    Database {
        archive_path: PathBuf,
        source_path: PathBuf,
        size_bytes: u64,
    },
}

/// Ordered collection of files and databases to snapshot from a home.
#[derive(Debug, Clone, Default)]
pub(super) struct SnapshotPlan {
    pub entries: Vec<SnapshotEntry>,
    pub max_single_db_bytes: u64,
    pub raw_snapshot_bytes: u64,
}

impl SnapshotPlan {
    fn push_file(&mut self, archive_path: PathBuf, source_path: PathBuf, size_bytes: u64) {
        self.raw_snapshot_bytes = self.raw_snapshot_bytes.saturating_add(size_bytes);
        self.entries.push(SnapshotEntry::File {
            archive_path,
            source_path,
            size_bytes,
        });
    }

    fn push_database(&mut self, archive_path: PathBuf, source_path: PathBuf, size_bytes: u64) {
        self.max_single_db_bytes = self.max_single_db_bytes.max(size_bytes);
        self.raw_snapshot_bytes = self.raw_snapshot_bytes.saturating_add(size_bytes);
        self.entries.push(SnapshotEntry::Database {
            archive_path,
            source_path,
            size_bytes,
        });
    }
}

/// Resolves an owning root (accepting a symlinked root directory such as a
/// symlinked `INSTRUKT_HOME` or `state/` mountpoint, while refusing dangling
/// symlinks).
pub(super) fn resolve_root(root: &Path) -> Result<PathBuf, BackupError> {
    match root.canonicalize() {
        Ok(canonical) => Ok(canonical),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(root) {
                Ok(meta) if meta.file_type().is_symlink() => Err(BackupError::Unconfined {
                    path: root.to_path_buf(),
                }),
                Ok(_) => Ok(root.to_path_buf()),
                Err(meta_err) if meta_err.kind() == std::io::ErrorKind::NotFound => {
                    Ok(root.to_path_buf())
                }
                Err(source) => Err(BackupError::Io {
                    path: root.to_path_buf(),
                    source,
                }),
            }
        }
        Err(source) => Err(BackupError::Io {
            path: root.to_path_buf(),
            source,
        }),
    }
}

/// Collects every in-scope file and SQLite database from `home`, bringing any
/// legacy company settings document forward through `configuration` first.
pub(super) fn collect(home: &Home) -> Result<SnapshotPlan, BackupError> {
    let resolved_home = resolve_root(home.root())?;
    let resolved_state = resolve_root(&home.state_dir())?;
    let mut plan = SnapshotPlan::default();

    // Machine-root configuration and protected secrets.
    for name in ["instrukt.yaml", "agents.yaml", "secrets.env"] {
        collect_single_file(&resolved_home, Path::new(name), Path::new(name), &mut plan)?;
    }

    // Machine state: dynamic *.db / *.sqlite discovery plus durable operational state.
    if resolved_state.exists() {
        collect_state_dir(&resolved_state, &mut plan)?;
    }

    // Durable global research notes under git/ (pruning any raw .git working tree before descent).
    let git_rel = Path::new("git");
    if exists_confined_dir(&resolved_home, git_rel)? {
        walk_confined_dir(
            &resolved_home,
            git_rel,
            git_rel,
            true,
            false,
            &[],
            &mut plan,
        )?;
    }

    // Cross-project exploration reports under explore/.
    let explore_rel = Path::new("explore");
    if exists_confined_dir(&resolved_home, explore_rel)? {
        walk_confined_dir(
            &resolved_home,
            explore_rel,
            explore_rel,
            false,
            false,
            &[],
            &mut plan,
        )?;
    }

    // Company homes: confine companies/ and each companies/<name> BEFORE invoking
    // configuration's forward conversion, then collect company.yaml, secrets.env,
    // and .instrukt/ (excluding .instrukt/schemas/).
    let companies_rel = Path::new("companies");
    if exists_confined_dir(&resolved_home, companies_rel)? {
        let companies_dir = confine_path(&resolved_home, companies_rel)?;
        let mut entries: Vec<_> = std::fs::read_dir(&companies_dir)
            .map_err(|source| BackupError::Io {
                path: companies_dir.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| BackupError::Io {
                path: companies_dir.clone(),
                source,
            })?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).map_err(|source| BackupError::Io {
                path: path.clone(),
                source,
            })?;
            if meta.file_type().is_symlink() {
                return Err(BackupError::Unconfined { path });
            }
            if !meta.is_dir() {
                continue;
            }
            let folder = entry.file_name().to_string_lossy().into_owned();
            let Ok(slug) = ProjectSlug::parse(&folder) else {
                continue;
            };
            let company_rel = companies_rel.join(slug.as_str());
            let _confined_company = confine_path(&resolved_home, &company_rel)?;
            // Bring any pre-migration legacy company instrukt.yaml forward to company.yaml.
            instrukt_core::config::convert_company_document(home, &slug)?;
            for file in ["company.yaml", "secrets.env"] {
                let rel = company_rel.join(file);
                collect_single_file(&resolved_home, &rel, &rel, &mut plan)?;
            }
            let dot_instrukt_rel = company_rel.join(".instrukt");
            if exists_confined_dir(&resolved_home, &dot_instrukt_rel)? {
                walk_confined_dir(
                    &resolved_home,
                    &dot_instrukt_rel,
                    &dot_instrukt_rel,
                    false,
                    true,
                    &["schemas"],
                    &mut plan,
                )?;
            }
        }
    }

    plan.entries.sort_by(|a, b| {
        fn path_of(e: &SnapshotEntry) -> &Path {
            match e {
                SnapshotEntry::File { archive_path, .. }
                | SnapshotEntry::Database { archive_path, .. } => archive_path.as_path(),
            }
        }
        path_of(a).cmp(path_of(b))
    });
    Ok(plan)
}

/// Verifies that the filesystem hosting `state_dir` has at least
/// `HEADROOM_FLOOR_BYTES + max_single_db_bytes + raw_snapshot_bytes` available.
pub(super) fn check_backup_headroom(
    state_dir: &Path,
    plan: &SnapshotPlan,
) -> Result<(), BackupError> {
    check_backup_headroom_with_floor(state_dir, plan, HEADROOM_FLOOR_BYTES)
}

pub(super) fn check_backup_headroom_with_floor(
    state_dir: &Path,
    plan: &SnapshotPlan,
    floor_bytes: u64,
) -> Result<(), BackupError> {
    let target = existing_ancestor(state_dir)?;
    let available = available_bytes(&target)?;
    let required = floor_bytes
        .saturating_add(plan.max_single_db_bytes)
        .saturating_add(plan.raw_snapshot_bytes);
    if available < required {
        return Err(BackupError::InsufficientDiskSpace {
            path: target,
            available_bytes: available,
            required_bytes: required,
        });
    }
    Ok(())
}

/// Available unprivileged bytes on the filesystem containing `path`.
pub(super) fn available_bytes(path: &Path) -> Result<u64, BackupError> {
    let stat = rustix::fs::statvfs(path).map_err(|err| BackupError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::from(err),
    })?;
    let unit = if stat.f_frsize > 0 {
        stat.f_frsize
    } else {
        stat.f_bsize
    };
    Ok(stat.f_bavail.saturating_mul(unit))
}

fn existing_ancestor(path: &Path) -> Result<PathBuf, BackupError> {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return Ok(current);
        }
        let Some(parent) = current.parent() else {
            return Err(BackupError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no existing ancestor directory",
                ),
            });
        };
        current = parent.to_path_buf();
    }
}

fn confine_path(root: &Path, rel: &Path) -> Result<PathBuf, BackupError> {
    fsx::confined(root, rel).map_err(|err| match err {
        fsx::FsError::Unconfined { path } => BackupError::Unconfined { path },
        fsx::FsError::Read { path, source } => BackupError::Io { path, source },
        other => BackupError::Fs(other),
    })
}

fn exists_confined_dir(root: &Path, rel: &Path) -> Result<bool, BackupError> {
    let confined = confine_path(root, rel)?;
    match std::fs::symlink_metadata(&confined) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(BackupError::Unconfined { path: confined })
        }
        Ok(meta) => Ok(meta.is_dir()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(BackupError::Io {
            path: confined,
            source,
        }),
    }
}

fn collect_single_file(
    root: &Path,
    rel: &Path,
    archive_path: &Path,
    plan: &mut SnapshotPlan,
) -> Result<(), BackupError> {
    let confined = confine_path(root, rel)?;
    match std::fs::symlink_metadata(&confined) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(BackupError::Unconfined { path: confined })
        }
        Ok(meta) if meta.is_file() => {
            plan.push_file(archive_path.to_path_buf(), confined, meta.len());
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(BackupError::Io {
            path: confined,
            source,
        }),
    }
}

/// Top-level `state/` entries unconditionally excluded from home backup.
const EXCLUDED_STATE_DIRS: &[&str] = &[
    "publication",
    "blueprint",
    "configuration",
    "installation",
    "hook-runs",
    "redis",
    "consumer-locks",
];

const EXCLUDED_STATE_FILES: &[&str] = &[
    "index.yaml",
    "runner-hosts.json",
    "backup-state.json",
    "backup-staleness-notified.json",
];

const INCLUDED_STATE_FILES: &[&str] = &["machine-id", "agents-deployment.json"];
const INCLUDED_STATE_DIRS: &[&str] = &["eval-fingerprint", "known-issues", "company-generation"];

fn is_excluded_state_staging(name: &str) -> bool {
    name.starts_with(".backup-staging-")
        || name.starts_with(".backup-restore-staging-")
        || name.starts_with(".backup-restore-rollback-")
}

fn is_excluded_filename(name: &str) -> bool {
    name.ends_with(".lock")
        || name.ends_with("-wal")
        || name.ends_with("-shm")
        || name.ends_with("-journal")
        || name.contains(".corrupt.")
        || name.contains(".pre-")
        || name.contains(".recovered-")
}

fn is_sqlite_filename(name: &str) -> bool {
    (name.ends_with(".db") || name.ends_with(".sqlite")) && !is_excluded_filename(name)
}

fn collect_state_dir(resolved_state: &Path, plan: &mut SnapshotPlan) -> Result<(), BackupError> {
    let mut entries: Vec<_> = std::fs::read_dir(resolved_state)
        .map_err(|source| BackupError::Io {
            path: resolved_state.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| BackupError::Io {
            path: resolved_state.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_excluded_state_staging(&name)
            || EXCLUDED_STATE_DIRS.contains(&name.as_str())
            || EXCLUDED_STATE_FILES.contains(&name.as_str())
            || is_excluded_filename(&name)
        {
            continue;
        }
        let rel = PathBuf::from(&name);
        let confined = confine_path(resolved_state, &rel)?;
        let meta = std::fs::symlink_metadata(&confined).map_err(|source| BackupError::Io {
            path: confined.clone(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            return Err(BackupError::Unconfined { path: confined });
        }
        let archive_rel = Path::new("state").join(&rel);
        if meta.is_file() {
            if is_sqlite_filename(&name) {
                let total_bytes = db_size_with_wal(&confined, meta.len())?;
                plan.push_database(archive_rel, confined, total_bytes);
            } else if INCLUDED_STATE_FILES.contains(&name.as_str()) {
                plan.push_file(archive_rel, confined, meta.len());
            }
        } else if meta.is_dir() {
            if INCLUDED_STATE_DIRS.contains(&name.as_str()) {
                walk_confined_dir(resolved_state, &rel, &archive_rel, false, true, &[], plan)?;
            } else {
                // Dynamically discover any *.db / *.sqlite files inside non-excluded subdirectories of state/.
                walk_sqlite_only(resolved_state, &rel, &archive_rel, plan)?;
            }
        }
    }
    Ok(())
}

fn db_size_with_wal(db_path: &Path, db_bytes: u64) -> Result<u64, BackupError> {
    let mut wal_name = db_path.as_os_str().to_os_string();
    wal_name.push("-wal");
    let wal_path = PathBuf::from(wal_name);
    let wal_bytes = match std::fs::symlink_metadata(&wal_path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(BackupError::Unconfined { path: wal_path });
        }
        Ok(meta) => meta.len(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
        Err(source) => {
            return Err(BackupError::Io {
                path: wal_path,
                source,
            });
        }
    };
    Ok(db_bytes.saturating_add(wal_bytes))
}

fn walk_sqlite_only(
    root: &Path,
    rel_dir: &Path,
    archive_prefix: &Path,
    plan: &mut SnapshotPlan,
) -> Result<(), BackupError> {
    let dir = confine_path(root, rel_dir)?;
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|source| BackupError::Io {
            path: dir.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| BackupError::Io {
            path: dir.clone(),
            source,
        })?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_excluded_filename(&name) || is_excluded_state_staging(&name) {
            continue;
        }
        let child_rel = rel_dir.join(&name);
        let confined = confine_path(root, &child_rel)?;
        let meta = std::fs::symlink_metadata(&confined).map_err(|source| BackupError::Io {
            path: confined.clone(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            return Err(BackupError::Unconfined { path: confined });
        }
        let child_archive = archive_prefix.join(&name);
        if meta.is_dir() {
            walk_sqlite_only(root, &child_rel, &child_archive, plan)?;
        } else if meta.is_file() && is_sqlite_filename(&name) {
            let total_bytes = db_size_with_wal(&confined, meta.len())?;
            plan.push_database(child_archive, confined, total_bytes);
        }
    }
    Ok(())
}

fn walk_confined_dir(
    root: &Path,
    rel_dir: &Path,
    archive_prefix: &Path,
    prune_git_checkouts: bool,
    discover_sqlite: bool,
    excluded_top_dirs: &[&str],
    plan: &mut SnapshotPlan,
) -> Result<(), BackupError> {
    let dir = confine_path(root, rel_dir)?;
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|source| BackupError::Io {
            path: dir.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| BackupError::Io {
            path: dir.clone(),
            source,
        })?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_excluded_filename(&name) {
            continue;
        }
        if excluded_top_dirs.contains(&name.as_str()) {
            continue;
        }
        let child_rel = rel_dir.join(&name);
        let confined = confine_path(root, &child_rel)?;
        let meta = std::fs::symlink_metadata(&confined).map_err(|source| BackupError::Io {
            path: confined.clone(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            return Err(BackupError::Unconfined { path: confined });
        }
        let child_archive = archive_prefix.join(&name);
        if meta.is_dir() {
            if prune_git_checkouts && dir_contains_dot_git(&confined)? {
                continue;
            }
            walk_confined_dir(
                root,
                &child_rel,
                &child_archive,
                prune_git_checkouts,
                discover_sqlite,
                &[],
                plan,
            )?;
        } else if meta.is_file() {
            if discover_sqlite && is_sqlite_filename(&name) {
                let total_bytes = db_size_with_wal(&confined, meta.len())?;
                plan.push_database(child_archive, confined, total_bytes);
            } else {
                plan.push_file(child_archive, confined, meta.len());
            }
        }
    }
    Ok(())
}

fn dir_contains_dot_git(dir: &Path) -> Result<bool, BackupError> {
    let dot_git = dir.join(".git");
    match std::fs::symlink_metadata(&dot_git) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(BackupError::Io {
            path: dot_git,
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_home() -> (tempfile::TempDir, Home) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf()).for_writing();
        std::fs::create_dir_all(home.state_dir()).unwrap();
        (dir, home)
    }

    #[test]
    fn collect_includes_irreplaceable_scope_and_excludes_regenerable_and_git_checkouts() {
        let (_dir, home) = make_home();
        std::fs::write(home.global_config(), "schema_version: 8\n").unwrap();
        std::fs::write(home.agents_file(), "schema_version: 4\n").unwrap();
        std::fs::write(home.secrets_file(), "KEY=val\n").unwrap();
        std::fs::write(home.machine_id_file(), "0123456789abcdef\n").unwrap();
        std::fs::write(home.state_dir().join("instrukt.db"), b"db1").unwrap();
        std::fs::write(home.state_dir().join("instrukt.db-wal"), b"wal-bytes").unwrap();
        std::fs::write(home.state_dir().join("custom.sqlite"), b"db2").unwrap();
        std::fs::create_dir_all(home.state_dir().join("known-issues")).unwrap();
        std::fs::write(home.state_dir().join("known-issues/proj.json"), b"{}").unwrap();
        std::fs::write(home.state_dir().join("known-issues/proj.lock"), b"").unwrap();
        std::fs::create_dir_all(home.state_dir().join("company-generation")).unwrap();
        std::fs::write(home.state_dir().join("company-generation/acme.json"), b"{}").unwrap();
        // Excluded state artifacts
        std::fs::write(home.state_dir().join("backup-state.json"), b"{}").unwrap();
        std::fs::write(
            home.state_dir().join("backup-staleness-notified.json"),
            b"{}",
        )
        .unwrap();
        std::fs::create_dir_all(home.state_dir().join(".backup-restore-staging-123")).unwrap();
        std::fs::write(
            home.state_dir()
                .join(".backup-restore-staging-123/leaked.db"),
            b"bad",
        )
        .unwrap();
        std::fs::create_dir_all(home.state_dir().join("publication")).unwrap();
        std::fs::write(home.state_dir().join("publication/p.json"), b"{}").unwrap();
        std::fs::create_dir_all(home.root().join("cache/git")).unwrap();
        std::fs::write(home.root().join("cache/git/cached.md"), b"cache").unwrap();
        std::fs::create_dir_all(home.root().join("docs/general")).unwrap();
        std::fs::write(home.root().join("docs/general/a.md"), b"doc").unwrap();
        // Research notes under git/ beside a raw .git checkout
        std::fs::create_dir_all(home.git_dir().join("raw-repo/.git")).unwrap();
        std::fs::write(home.git_dir().join("raw-repo/README.md"), b"checkout").unwrap();
        std::fs::write(home.git_dir().join("index.md"), b"notes").unwrap();
        std::fs::create_dir_all(home.root().join("explore")).unwrap();
        std::fs::write(home.root().join("explore/report.md"), b"explore").unwrap();
        // Company home with legacy instrukt.yaml, secrets, .instrukt/schemas, and DB
        let acme = ProjectSlug::parse("acme").unwrap();
        let acme_dir = home.company_dir(&acme);
        std::fs::create_dir_all(acme_dir.join(".instrukt/schemas")).unwrap();
        std::fs::create_dir_all(acme_dir.join("projects/sub")).unwrap();
        std::fs::write(
            acme_dir.join("instrukt.yaml"),
            "schema_version: 6\ncompany: acme\n",
        )
        .unwrap();
        std::fs::write(acme_dir.join("secrets.env"), "S=1\n").unwrap();
        std::fs::write(acme_dir.join("tracked.md"), b"tracked").unwrap();
        std::fs::write(acme_dir.join("projects/sub/a.txt"), b"proj").unwrap();
        std::fs::write(acme_dir.join(".instrukt/schemas/s.json"), b"{}").unwrap();
        std::fs::write(
            acme_dir.join(".instrukt/knowledge-proposals.db"),
            b"company-db",
        )
        .unwrap();

        let plan = collect(&home).unwrap();
        let paths: Vec<String> = plan
            .entries
            .iter()
            .map(|e| match e {
                SnapshotEntry::File { archive_path, .. } => {
                    format!("F:{}", archive_path.display())
                }
                SnapshotEntry::Database { archive_path, .. } => {
                    format!("D:{}", archive_path.display())
                }
            })
            .collect();

        for expected in [
            "F:instrukt.yaml",
            "F:agents.yaml",
            "F:secrets.env",
            "F:state/machine-id",
            "D:state/instrukt.db",
            "D:state/custom.sqlite",
            "F:state/known-issues/proj.json",
            "F:state/company-generation/acme.json",
            "F:git/index.md",
            "F:explore/report.md",
            "F:companies/acme/company.yaml",
            "F:companies/acme/secrets.env",
            "D:companies/acme/.instrukt/knowledge-proposals.db",
        ] {
            assert!(
                paths.iter().any(|p| p == expected),
                "missing {expected} in {paths:?}"
            );
        }
        // Excluded items must not appear.
        for forbidden in [
            "backup-state.json",
            "backup-staleness-notified.json",
            ".backup-restore-staging",
            "publication/p.json",
            "cache/git",
            "docs/general",
            "raw-repo",
            "proj.lock",
            "instrukt.db-wal",
            "tracked.md",
            "projects/sub",
            "schemas/s.json",
        ] {
            assert!(
                !paths.iter().any(|p| p.contains(forbidden)),
                "unexpected {forbidden} in {paths:?}"
            );
        }
        // Legacy company instrukt.yaml was converted forward to company.yaml.
        assert!(acme_dir.join("company.yaml").exists());
        assert!(!acme_dir.join("instrukt.yaml").exists());
    }

    #[test]
    fn collect_refuses_escaping_symlink_company_before_conversion_and_supports_symlinked_state() {
        let outside = tempfile::tempdir().unwrap();
        let escaped_company = outside.path().join("escaped");
        std::fs::create_dir_all(&escaped_company).unwrap();
        std::fs::write(
            escaped_company.join("instrukt.yaml"),
            "schema_version: 6\ncompany: acme\n",
        )
        .unwrap();

        let (_dir, home) = make_home();
        std::fs::create_dir_all(home.companies_dir()).unwrap();
        std::os::unix::fs::symlink(&escaped_company, home.companies_dir().join("acme")).unwrap();
        assert!(matches!(
            collect(&home),
            Err(BackupError::Unconfined { .. })
        ));
        // Conversion never ran on the escaped target.
        assert!(!escaped_company.join("company.yaml").exists());

        // Symlinked state/ mountpoint is supported as an owning root.
        let root2 = tempfile::tempdir().unwrap();
        let external_state = tempfile::tempdir().unwrap();
        std::fs::write(
            external_state.path().join("machine-id"),
            "0123456789abcdef\n",
        )
        .unwrap();
        let home_root = root2.path().join(".instrukt");
        std::fs::create_dir_all(&home_root).unwrap();
        std::os::unix::fs::symlink(external_state.path(), home_root.join("state")).unwrap();
        let home2 = Home::at_user_home(root2.path().to_path_buf()).for_writing();
        let plan = collect(&home2).unwrap();
        assert!(plan.entries.iter().any(|e| matches!(
            e,
            SnapshotEntry::File { archive_path, .. } if archive_path == Path::new("state/machine-id")
        )));
    }

    #[test]
    fn headroom_check_refuses_when_available_space_falls_below_floor() {
        let (_dir, home) = make_home();
        let plan = SnapshotPlan {
            entries: vec![],
            max_single_db_bytes: 1024,
            raw_snapshot_bytes: 2048,
        };
        assert!(matches!(
            check_backup_headroom_with_floor(&home.state_dir(), &plan, u64::MAX - 10),
            Err(BackupError::InsufficientDiskSpace { .. })
        ));
    }
}
