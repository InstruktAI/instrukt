//! Live-home exclusive lock guard for `instrukt backup restore`
//! (`project/instrukt/spec/feature/runner/home-backup`).
//!
//! Enumerates every fixed home/company/known-issues lock plus dynamically
//! discovered active lock files, verifies leaf-inclusive symlink confinement
//! on every lock path BEFORE opening or creating any lock file, sorts
//! canonically, and acquires non-blocking exclusive locks across the entire
//! restore swap.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use instrukt_core::config::{SecretSource, document_lock_path};
use instrukt_core::fsx::LockGuard;
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;

use super::_snapshot::confine_path;
use super::BackupError;

/// Fixed lock filenames under `state/`.
const FIXED_STATE_LOCKS: &[&str] = &[
    "runner.lock",
    "writer.lock",
    "history.lock",
    "reflection.lock",
    "withdrawal.lock",
    "delivered-release.lock",
    "company-generation.lock",
    "review.lock",
    "review-publication.lock",
    "review-observation.lock",
    "requested-review.lock",
    "runtime-dispatch.lock",
    "backup.lock",
];

/// Fixed lock filenames under `locks/`.
const FIXED_HOME_LOCKS: &[&str] = &[
    "docs.lock",
    "installation.lock",
    "agents.lock",
    "secrets.lock",
    "publication-outputs.lock",
];

/// Exclusive advisory locks held across a live-home restore swap.
/// Dropping releases every held lock automatically.
#[derive(Debug)]
pub(super) struct LiveHomeLocks {
    _guards: Vec<LockGuard>,
}

/// Verifies confinement on and acquires every fixed and dynamically discovered
/// live-home lock in canonical sorted order.
pub(super) fn acquire_live_home_locks(
    home: &Home,
    resolved_home: &Path,
    resolved_state: &Path,
    staged_companies: &BTreeSet<String>,
    staged_known_issues: &BTreeSet<String>,
) -> Result<LiveHomeLocks, BackupError> {
    let mut state_rels: BTreeSet<PathBuf> = FIXED_STATE_LOCKS.iter().map(PathBuf::from).collect();
    let mut home_rels: BTreeSet<PathBuf> = FIXED_HOME_LOCKS
        .iter()
        .map(|name| Path::new("locks").join(name))
        .collect();

    // Global config document lock (`locks/config-<digest>.lock`).
    if let Some(name) = document_lock_path(home, &home.global_config()).file_name() {
        home_rels.insert(Path::new("locks").join(name));
    }

    // Union of existing and staged company homes.
    let mut companies: BTreeSet<String> = staged_companies.clone();
    let companies_dir = confine_path(resolved_home, Path::new("companies"))?;
    if companies_dir.is_dir() {
        for entry in std::fs::read_dir(&companies_dir).map_err(|source| BackupError::Io {
            path: companies_dir.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| BackupError::Io {
                path: companies_dir.clone(),
                source,
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // Confine each company entry before inspecting its children so a
            // symlinked company folder fails closed immediately.
            let company_path = confine_path(resolved_home, &Path::new("companies").join(&name))?;
            if company_path.is_dir() {
                companies.insert(name);
            }
        }
    }

    for name in &companies {
        let company_rel = Path::new("companies").join(name);
        // Always confine relative to resolved_home (never pass company dir as root).
        let _ = confine_path(resolved_home, &company_rel)?;
        home_rels.insert(
            company_rel
                .join(".instrukt")
                .join("knowledge-proposals.lock"),
        );

        if let Ok(slug) = ProjectSlug::parse(name) {
            if let Some(lock_name) = SecretSource::Company(slug.clone())
                .lock_path(home)
                .file_name()
            {
                home_rels.insert(Path::new("locks").join(lock_name));
            }
            for doc in ["company.yaml", "instrukt.yaml"] {
                let doc_path = home.company_dir(&slug).join(doc);
                if let Some(lock_name) = document_lock_path(home, &doc_path).file_name() {
                    home_rels.insert(Path::new("locks").join(lock_name));
                }
            }
        }

        // Dynamic company `.instrukt/*.lock` discovery.
        let dot_instrukt_rel = company_rel.join(".instrukt");
        let dot_instrukt = confine_path(resolved_home, &dot_instrukt_rel)?;
        if dot_instrukt.is_dir() {
            collect_dir_locks(&dot_instrukt, &dot_instrukt_rel, true, &mut home_rels)?;
        }
    }

    // Known-issues project locks (staged + existing).
    for project in staged_known_issues {
        state_rels.insert(Path::new("known-issues").join(format!("{project}.lock")));
    }

    // Dynamic state/ lock discovery (`state/*.lock`, `state/known-issues/*.lock`,
    // `state/consumer-locks/*`).
    if resolved_state.is_dir() {
        collect_dir_locks(resolved_state, Path::new(""), true, &mut state_rels)?;

        let ki_rel = Path::new("known-issues");
        let ki_dir = confine_path(resolved_state, ki_rel)?;
        if ki_dir.is_dir() {
            collect_dir_locks(&ki_dir, ki_rel, true, &mut state_rels)?;
        }

        let cl_rel = Path::new("consumer-locks");
        let cl_dir = confine_path(resolved_state, cl_rel)?;
        if cl_dir.is_dir() {
            collect_dir_locks(&cl_dir, cl_rel, false, &mut state_rels)?;
        }
    }

    // Dynamic `locks/*.lock` discovery.
    let locks_rel = Path::new("locks");
    let locks_dir = confine_path(resolved_home, locks_rel)?;
    if locks_dir.is_dir() {
        collect_dir_locks(&locks_dir, locks_rel, true, &mut home_rels)?;
    }

    // Confine EVERY lock path (leaf-inclusive) BEFORE opening or creating any lock file.
    let mut lock_paths: BTreeSet<PathBuf> = BTreeSet::new();
    for rel in &state_rels {
        lock_paths.insert(confine_path(resolved_state, rel)?);
    }
    for rel in &home_rels {
        lock_paths.insert(confine_path(resolved_home, rel)?);
    }

    // Acquire non-blocking exclusive locks in canonical sorted order.
    let mut guards = Vec::with_capacity(lock_paths.len());
    for lock_path in lock_paths {
        match LockGuard::try_acquire(&lock_path)? {
            Some(guard) => guards.push(guard),
            None => {
                let holder = read_lock_holder(&lock_path);
                return Err(BackupError::LiveHomeBusy {
                    lock: lock_path,
                    holder,
                });
            }
        }
    }

    Ok(LiveHomeLocks { _guards: guards })
}

fn collect_dir_locks(
    dir: &Path,
    rel_prefix: &Path,
    require_lock_ext: bool,
    out: &mut BTreeSet<PathBuf>,
) -> Result<(), BackupError> {
    for entry in std::fs::read_dir(dir).map_err(|source| BackupError::Io {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| BackupError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if require_lock_ext && !name.ends_with(".lock") {
            continue;
        }
        let rel = if rel_prefix.as_os_str().is_empty() {
            PathBuf::from(name)
        } else {
            rel_prefix.join(name)
        };
        out.insert(rel);
    }
    Ok(())
}

fn read_lock_holder(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let clipped: String = line.chars().take(120).collect();
    (!clipped.is_empty()).then_some(clipped)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn refuses_symlink_at_lock_path_before_opening_lock() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("target.lock");
        std::fs::write(&outside_file, b"sentinel").unwrap();

        let home = Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.locks_dir()).unwrap();
        std::fs::create_dir_all(home.state_dir()).unwrap();
        symlink(&outside_file, home.locks_dir().join("docs.lock")).unwrap();

        let resolved_home = home.root().canonicalize().unwrap();
        let resolved_state = home.state_dir().canonicalize().unwrap();
        let err = acquire_live_home_locks(
            &home,
            &resolved_home,
            &resolved_state,
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .unwrap_err();
        assert!(matches!(err, BackupError::Unconfined { .. }), "{err:?}");
    }

    #[test]
    fn refuses_held_fixed_and_dynamic_locks_with_doors() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.state_dir().join("consumer-locks")).unwrap();
        let resolved_home = home.root().canonicalize().unwrap();
        let resolved_state = home.state_dir().canonicalize().unwrap();

        let dyn_lock = resolved_state.join("consumer-locks/slack.lock");
        let _held = LockGuard::try_acquire(&dyn_lock).unwrap().unwrap();

        let err = acquire_live_home_locks(
            &home,
            &resolved_home,
            &resolved_state,
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, BackupError::LiveHomeBusy { .. }), "{msg}");
        assert!(msg.contains("--into"), "{msg}");
    }
}
