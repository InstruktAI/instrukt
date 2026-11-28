//! Explicit live-home schema migration and automatic pre-migration snapshot
//! backup (`project/instrukt/spec/feature/runner/module-installation`,
//! `project/instrukt/spec/feature/work/state-database`,
//! `project/instrukt/spec/feature/configuration/owned-files`).
//!
//! `instrukt install` is the sole boundary that migrates machine-owned
//! configuration (`instrukt.yaml`, `agents.yaml`, company documents) and
//! SQLite state stores. Before any migration writes to disk, this module takes
//! an owner-only (`0700`/`0600`) snapshot under
//! `<home>/backups/pre-migrate-<timestamp>/` using SQLite's online backup API
//! (`quick_check` verified) and plain copies of present owned configuration
//! files, aborting before any mutation if backup creation fails.

use std::collections::BTreeSet;
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use instrukt_core::config::{self, Agents, AgentsMigration, Layer};
use instrukt_core::fsx;
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::{self, StateDb, StateDbError, Store, StoreDb, StoreError};
use jiff::Timestamp;

use super::{InstallationError, Report, Request};

struct PendingLayer {
    path: PathBuf,
    notice: String,
}

struct PendingCompany {
    name: ProjectSlug,
    path: PathBuf,
    notice: String,
}

struct Pending {
    global: Option<PendingLayer>,
    agents: Option<(AgentsMigration, String)>,
    companies: Vec<PendingCompany>,
    stores: Vec<(Store, u32, u32)>,
    newer_stores: Vec<(String, PathBuf, String)>,
}

impl Pending {
    fn has_migrations(&self) -> bool {
        self.global.is_some()
            || self.agents.is_some()
            || !self.companies.is_empty()
            || !self.stores.is_empty()
    }
}

/// Every SQLite store owned by this instrukt home: core state, runner stores,
/// and caller-supplied module stores (review, review publication, knowledge
/// proposals), deduplicated by file path.
pub(crate) fn collect_stores(home: &Home, extra_stores: &[Store]) -> Vec<Store> {
    let mut stores = Vec::with_capacity(5 + extra_stores.len());
    let mut seen = BTreeSet::new();
    let base = [
        Store::state(home),
        crate::history::store(home),
        crate::requested_review::store(home),
        crate::review::observation_store(home),
        crate::transport::reachability_store(home),
    ];
    for store in base.into_iter().chain(extra_stores.iter().cloned()) {
        if seen.insert(store.file.clone()) {
            stores.push(store);
        }
    }
    stores
}

/// Read-only inspection of every pending config and SQLite migration under
/// `home`. Takes no locks and writes nothing.
fn inspect_pending(home: &Home, extra_stores: &[Store]) -> Result<Pending, InstallationError> {
    let inspection = home.for_inspection();
    let global = match config::inspect_layer(&inspection, &Layer::Global) {
        Ok(view) => view.migration.map(|notice| PendingLayer {
            path: view.path,
            notice,
        }),
        Err(error) => return Err(InstallationError::Configuration(error)),
    };
    let agents = match Agents::pending_migration(&inspection)? {
        Some(kind) => {
            let notice = Agents::inspection_notice(&inspection)?.unwrap_or_else(|| {
                format!(
                    "{} needs migration; run `instrukt install`",
                    home.agents_file().display()
                )
            });
            Some((kind, notice))
        }
        None => None,
    };
    let mut companies = Vec::new();
    for (name, unreadable) in config::company_folders(&inspection)? {
        if unreadable.is_some() {
            continue;
        }
        if let Ok(view) = config::inspect_layer(&inspection, &Layer::Company(name.clone()))
            && let Some(notice) = view.migration
        {
            companies.push(PendingCompany {
                name,
                path: view.path,
                notice,
            });
        }
    }
    let mut stores = Vec::new();
    let mut newer_stores = Vec::new();
    for store in collect_stores(&inspection, extra_stores) {
        match store.needs_migration() {
            Ok(Some((found, known))) => stores.push((store, found, known)),
            Ok(None) => {}
            Err(error @ StoreError::Newer { .. }) => {
                newer_stores.push((store.name.to_owned(), store.file.clone(), error.to_string()));
            }
            Err(error) => {
                return Err(InstallationError::State(StateDbError::from(error).into()));
            }
        }
    }
    Ok(Pending {
        global,
        agents,
        companies,
        stores,
        newer_stores,
    })
}

/// Inspects pending live-home schema migrations.
///
/// - When `request.check` is true, records every pending migration (and any
///   newer store) as an unmet finding on `report` without mutating anything.
/// - When `request.check` is false and at least one migration is owed, writes
///   an owner-only pre-migration snapshot backup under
///   `<home>/backups/pre-migrate-<timestamp>/` (aborting before any mutation if
///   backup fails), executes every pending config and SQLite migration under
///   its own lock, records notices on `report`, and returns `Ok(true)`.
pub(crate) fn run(request: &Request<'_>, report: &mut Report) -> Result<bool, InstallationError> {
    let pending = inspect_pending(request.home, &request.extra_stores)?;
    if request.check {
        if let Some(global) = &pending.global {
            report.finding(
                format!("configuration {}", global.path.display()),
                &global.notice,
            );
        }
        if let Some((_, notice)) = &pending.agents {
            report.finding(
                format!("configuration {}", request.home.agents_file().display()),
                notice,
            );
        }
        for company in &pending.companies {
            report.finding(
                format!(
                    "company {} configuration {}",
                    company.name,
                    company.path.display()
                ),
                &company.notice,
            );
        }
        for (store, found, known) in &pending.stores {
            report.finding(
                format!("{} ({})", store.name, store.file.display()),
                format!(
                    "schema version {found} is older than this build's {known}; run `instrukt install` to back it up and migrate it"
                ),
            );
        }
        for (name, path, detail) in &pending.newer_stores {
            report.finding(format!("{name} ({})", path.display()), detail);
        }
        return Ok(false);
    }

    if let Some((_, _, detail)) = pending.newer_stores.first() {
        return Err(InstallationError::Migration(detail.clone()));
    }
    if !pending.has_migrations() {
        return Ok(false);
    }

    let backup_dir = take_pre_migration_backup(request.home, request.now, &pending)?;
    report.notices.push(format!(
        "pre-migration backup written to {}",
        backup_dir.display()
    ));

    apply_pending(request.home, &pending, Some(report))?;
    Ok(true)
}

/// Explicitly migrates any older machine-owned configuration and SQLite stores
/// restored into the live home by `instrukt backup restore`. Skips absent and
/// uninitialized (`user_version == 0`) SQLite files.
pub(crate) fn migrate_restored(
    home: &Home,
    extra_stores: &[Store],
) -> Result<(), InstallationError> {
    let pending = inspect_pending(home, extra_stores)?;
    if let Some((_, _, detail)) = pending.newer_stores.first() {
        return Err(InstallationError::Migration(detail.clone()));
    }
    if !pending.has_migrations() {
        return Ok(());
    }
    let backup = take_pre_migration_backup(home, Timestamp::now(), &pending)?;
    tracing::info!(event = "pre_migration_backup", path = %backup.display());
    apply_pending(home, &pending, None)
}

fn apply_pending(
    home: &Home,
    pending: &Pending,
    mut report: Option<&mut Report>,
) -> Result<(), InstallationError> {
    if let Some((kind, _)) = &pending.agents {
        Agents::migrate_file(home)?;
        let message = match kind {
            AgentsMigration::RemoveDefaultsCopy => format!(
                "removed redundant defaults copy {}",
                home.agents_file().display()
            ),
            AgentsMigration::Convert { from, to } => format!(
                "migrated {} from schema_version {from} to {to}",
                home.agents_file().display()
            ),
        };
        tracing::info!(event = "live_home_migrated", target = "agents", %message);
        if let Some(report) = report.as_deref_mut() {
            report.notices.push(message);
        }
    }
    if let Some(global) = &pending.global {
        config::migrate(home, &Layer::Global)?;
        let message = format!(
            "migrated {} to schema_version {}",
            global.path.display(),
            config::GLOBAL_SCHEMA_VERSION
        );
        tracing::info!(event = "live_home_migrated", target = "global_config", %message);
        if let Some(report) = report.as_deref_mut() {
            report.notices.push(message);
        }
    }
    for company in &pending.companies {
        config::migrate(home, &Layer::Company(company.name.clone()))?;
        let message = format!(
            "migrated company `{}` configuration to {} (schema_version {})",
            company.name,
            Layer::Company(company.name.clone()).path(home).display(),
            config::COMPANY_SCHEMA_VERSION
        );
        tracing::info!(
            event = "live_home_migrated",
            target = "company_config",
            company = %company.name,
            %message
        );
        if let Some(report) = report.as_deref_mut() {
            report.notices.push(message);
        }
    }
    let state_db_path = home.state_db_file();
    let history_db_path = home.history_db_file();
    for (store, found, known) in &pending.stores {
        if store.file == state_db_path {
            StateDb::migrate(home).map_err(crate::state::StateError::from)?;
        } else if store.file == history_db_path {
            crate::history::migrate(home)
                .map_err(|error| InstallationError::Migration(error.to_string()))?;
        } else {
            drop(
                StoreDb::open_migrating(store)
                    .map_err(|error| InstallationError::State(StateDbError::from(error).into()))?,
            );
        }
        let message = format!(
            "migrated {} ({}) from schema version {found} to {known}",
            store.name,
            store.file.display()
        );
        tracing::info!(
            event = "live_home_migrated",
            target = store.name,
            from = *found,
            to = *known,
            %message
        );
        if let Some(report) = report.as_deref_mut() {
            report.notices.push(message);
        }
    }
    Ok(())
}

/// Writes an owner-only (`0700`/`0600`) pre-migration snapshot of present
/// machine-owned configuration files and SQLite databases (always including
/// `state/instrukt.db` when present, plus every migrating store) under
/// `<home>/backups/pre-migrate-<timestamp>/`.
fn take_pre_migration_backup(
    home: &Home,
    now: Timestamp,
    pending: &Pending,
) -> Result<PathBuf, InstallationError> {
    let backups_root = home.root().join("backups");
    ensure_owner_dir(&backups_root)?;

    let stamp = now.strftime("%Y%m%dT%H%M%SZ").to_string();
    let backup_dir = allocate_backup_dir(&backups_root, &stamp)?;

    // Copy present machine-owned configuration files.
    copy_if_present(
        &home.global_config(),
        &backup_dir.join(config::CONFIG_FILE_NAME),
    )?;
    copy_if_present(&home.agents_file(), &backup_dir.join("agents.yaml"))?;
    copy_if_present(
        &home.agents_deployment_file(),
        &backup_dir.join("state/agents-deployment.json"),
    )?;
    for (name, unreadable) in config::company_folders(home)? {
        if unreadable.is_some() {
            continue;
        }
        let company_dir = home.company_dir(&name);
        let backup_company_dir = backup_dir.join("companies").join(name.as_str());
        copy_if_present(
            &company_dir.join("company.yaml"),
            &backup_company_dir.join("company.yaml"),
        )?;
        copy_if_present(
            &company_dir.join(config::CONFIG_FILE_NAME),
            &backup_company_dir.join(config::CONFIG_FILE_NAME),
        )?;
    }

    // Online-backup `state/instrukt.db` (whenever present) and every migrating SQLite store.
    let mut db_targets: Vec<(&'static str, PathBuf)> = Vec::new();
    let mut seen_dbs = BTreeSet::new();
    let state_store = Store::state(home);
    if path_exists_or_symlink(&state_store.file) && seen_dbs.insert(state_store.file.clone()) {
        db_targets.push((state_store.name, state_store.file));
    }
    for (store, _, _) in &pending.stores {
        if path_exists_or_symlink(&store.file) && seen_dbs.insert(store.file.clone()) {
            db_targets.push((store.name, store.file.clone()));
        }
    }
    for (store_name, src) in db_targets {
        state_db::refuse_link(store_name, &src)
            .map_err(|error| InstallationError::Migration(error.to_string()))?;
        let dest = backup_destination(home, &backup_dir, &src);
        if let Some(parent) = dest.parent() {
            ensure_owner_dir(parent)?;
        }
        state_db::online_backup(&src, &dest).map_err(|error| {
            InstallationError::Migration(format!(
                "pre-migration backup of {} failed: {error}",
                src.display()
            ))
        })?;
        state_db::quick_check(&dest).map_err(|error| {
            InstallationError::Migration(format!(
                "pre-migration backup verification of {} failed: {error}",
                dest.display()
            ))
        })?;
        fsx::set_permissions(&dest, Permissions::from_mode(0o600)).map_err(|error| {
            InstallationError::Migration(format!(
                "cannot tighten permissions on {}: {error}",
                dest.display()
            ))
        })?;
    }

    Ok(backup_dir)
}

fn allocate_backup_dir(backups_root: &Path, stamp: &str) -> Result<PathBuf, InstallationError> {
    for attempt in 1u32..=1000 {
        let name = if attempt == 1 {
            format!("pre-migrate-{stamp}")
        } else {
            format!("pre-migrate-{stamp}-{attempt}")
        };
        let candidate = backups_root.join(name);
        if std::fs::symlink_metadata(&candidate).is_err() {
            ensure_owner_dir(&candidate)?;
            return Ok(candidate);
        }
    }
    Err(InstallationError::Migration(format!(
        "cannot allocate a unique pre-migration backup directory under {}",
        backups_root.display()
    )))
}

fn ensure_owner_dir(dir: &Path) -> Result<(), InstallationError> {
    if let Ok(meta) = std::fs::symlink_metadata(dir) {
        if meta.file_type().is_symlink() {
            return Err(InstallationError::Migration(format!(
                "refusing pre-migration backup at {}: path is a symbolic link",
                dir.display()
            )));
        }
        if !meta.is_dir() {
            return Err(InstallationError::Migration(format!(
                "refusing pre-migration backup at {}: path exists and is not a directory",
                dir.display()
            )));
        }
    } else {
        fsx::create_dir_all(dir).map_err(|error| {
            InstallationError::Migration(format!(
                "cannot create pre-migration backup directory {}: {error}",
                dir.display()
            ))
        })?;
    }
    fsx::set_permissions(dir, Permissions::from_mode(0o700)).map_err(|error| {
        InstallationError::Migration(format!(
            "cannot set permissions on pre-migration backup directory {}: {error}",
            dir.display()
        ))
    })
}

fn copy_if_present(src: &Path, dst: &Path) -> Result<(), InstallationError> {
    let meta = match std::fs::symlink_metadata(src) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(InstallationError::Migration(format!(
                "cannot stat {} for pre-migration backup: {error}",
                src.display()
            )));
        }
    };
    if meta.file_type().is_symlink() {
        return Err(InstallationError::Migration(format!(
            "refusing pre-migration backup of {}: path is a symbolic link",
            src.display()
        )));
    }
    if let Some(parent) = dst.parent() {
        ensure_owner_dir(parent)?;
    }
    let bytes = std::fs::read(src).map_err(|error| {
        InstallationError::Migration(format!(
            "cannot read {} for pre-migration backup: {error}",
            src.display()
        ))
    })?;
    fsx::atomic_write_with_permissions(dst, &bytes, Permissions::from_mode(0o600)).map_err(
        |error| {
            InstallationError::Migration(format!(
                "cannot write pre-migration backup {}: {error}",
                dst.display()
            ))
        },
    )
}

fn path_exists_or_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn backup_destination(home: &Home, backup_dir: &Path, src: &Path) -> PathBuf {
    if let Ok(rel) = src.strip_prefix(home.root()) {
        return backup_dir.join(rel);
    }
    if let Ok(rel) = src.strip_prefix(home.state_dir()) {
        return backup_dir.join("state").join(rel);
    }
    let name = src
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("store.db"));
    backup_dir.join("state").join(name)
}
