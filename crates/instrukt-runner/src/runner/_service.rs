//! The runner service's identity: what it executes and with which
//! configuration. The service publishes it, with its process id, as soon as
//! it starts and before it waits for the runner lock; installation computes
//! the desired identity the same way and reuses a running service only
//! when the two agree.
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::Path;

use instrukt_core::home::Home;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::RunnerError;

/// What a started runner service published about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRecord {
    pub pid: u32,
    pub identity: String,
}

/// The identity of a runner service executing `executable` with this
/// computer's configuration: the executable's path and file, the global
/// layer, the agent profiles and every company home's document, and the
/// size and time of each company's secrets file, never its content.
pub fn service_identity(home: &Home, executable: &Path) -> std::io::Result<String> {
    let mut digest = Sha256::new();
    let mut file = |label: &str, path: &Path, content: bool| -> std::io::Result<()> {
        digest.update(label.as_bytes());
        digest.update(path.as_os_str().as_encoded_bytes());
        match std::fs::metadata(path) {
            Ok(meta) => {
                digest.update(
                    format!(
                        "{} {} {} {} {}",
                        meta.dev(),
                        meta.ino(),
                        meta.len(),
                        meta.mtime(),
                        meta.mtime_nsec()
                    )
                    .as_bytes(),
                );
                if content {
                    digest.update(std::fs::read(path)?);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => digest.update(b"absent"),
            Err(error) => return Err(error),
        }
        Ok(())
    };
    file("executable", executable, false)?;
    file("global", &home.global_config(), true)?;
    file("agents", &home.agents_file(), true)?;
    let mut companies = match std::fs::read_dir(home.companies_dir()) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    companies.sort();
    for company in companies {
        file("company", &company.join("instrukt.yaml"), true)?;
        file("company secrets", &company.join("secrets.env"), false)?;
    }
    Ok(hex::encode(digest.finalize()))
}

/// Publishes this process as the runner service, before it waits for the
/// runner lock.
pub(super) fn publish(home: &Home) -> Result<(), RunnerError> {
    let unpublished =
        |error: &dyn std::fmt::Display| RunnerError::ServiceUnpublished(error.to_string());
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| unpublished(&e))?;
    let record = ServiceRecord {
        pid: std::process::id(),
        identity: service_identity(home, &executable).map_err(|e| unpublished(&e))?,
    };
    let bytes = serde_json::to_vec(&record).map_err(|e| unpublished(&e))?;
    instrukt_core::fsx::atomic_write_with_permissions(
        &home.runner_service_file(),
        &bytes,
        std::fs::Permissions::from_mode(0o600),
    )
    .map_err(|e| unpublished(&e))
}

/// What the runner service last published, if anything readable.
pub fn published_service(home: &Home) -> Option<ServiceRecord> {
    serde_json::from_slice(&std::fs::read(home.runner_service_file()).ok()?).ok()
}
