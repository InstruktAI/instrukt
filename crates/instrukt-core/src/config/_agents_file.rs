//! The agents file of an instrukt home (`Home::agents_file`) on disk: what
//! the next writing command does to it and every write to it under its one
//! lock.
use super::_agents::{Agents, VERSION};
use super::ConfigError;
use crate::{
    fsx::{LockGuard, atomic_write},
    home::Home,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const DEPLOYMENT_VERSION: u64 = 1;

/// How long a writer of the agents file waits for another.
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// The lock every writer of the agents file holds for its read-modify-write.
pub(super) fn agents_lock(home: &Home) -> PathBuf {
    home.locks_dir().join("agents.lock")
}

impl Agents {
    /// What a writing command does to `agents.yaml` before reading it. A full
    /// copy of shipped defaults the deployment record recognises as unedited
    /// is removed, and then the record; any other file is kept and has a due
    /// shape conversion persisted. Removal comes first, so a recognised copy
    /// is never converted and kept.
    pub fn converge_file(home: &Home) -> Result<(), ConfigError> {
        let path = home.agents_file();
        let state = home.agents_deployment_file();
        let Some(before) = read_existing(&path)? else {
            return Ok(());
        };
        if next_write(&before, Deployment::load(&state).as_ref())? == NextWrite::Keep {
            return Ok(());
        }
        let _lock = LockGuard::acquire(&agents_lock(home), LOCK_TIMEOUT)?;
        let Some(current) = read_existing(&path)? else {
            return Ok(());
        };
        match next_write(&current, Deployment::load(&state).as_ref())? {
            NextWrite::Keep => {}
            NextWrite::Remove => remove_copy(&path, &state)?,
            NextWrite::Convert(converted) => {
                atomic_write(&path, &converted.bytes)?;
                tracing::info!(
                    event = "agents_shape_migrated",
                    from_version = converted.from,
                    to_version = VERSION
                );
            }
        }
        Ok(())
    }

    /// What a reading command reports about `agents.yaml`: the change the
    /// next writing command makes to it, if any, without making it.
    pub fn inspection_notice(home: &Home) -> Result<Option<String>, ConfigError> {
        let path = home.agents_file();
        let Some(bytes) = read_existing(&path)? else {
            return Ok(None);
        };
        let state = home.agents_deployment_file();
        Ok(
            match next_write(&bytes, Deployment::load(&state).as_ref())? {
                NextWrite::Keep => None,
                NextWrite::Remove => Some(format!(
                    "{}: an unedited copy of earlier shipped defaults; the next writing command removes it and the shipped defaults apply",
                    path.display()
                )),
                NextWrite::Convert(_) => Some(format!(
                    "{}: selection configuration inspected in memory; file migrates on the next writing command",
                    path.display()
                )),
            },
        )
    }
}

/// What the next writing command does to `agents.yaml`.
#[derive(Debug, PartialEq, Eq)]
enum NextWrite {
    /// Nothing for this operation: the file is not a recognised copy and no
    /// shape conversion applies. Whether it is valid is for [`Agents::parse`]
    /// to decide.
    Keep,
    /// Remove the file and then the deployment record: the record recognises
    /// the file as a copy an earlier release wrote and nobody edited.
    Remove,
    /// Persist the file's own bytes at the current shape.
    Convert(super::_agents_migrate::Converted),
}

/// The decision for `file` beside `record`, the readable deployment record
/// (`None` when there is none, or it is unreadable). Removal comes first, so
/// a recognised copy is never converted and kept.
fn next_write(file: &[u8], record: Option<&Deployment>) -> Result<NextWrite, ConfigError> {
    if record.is_some_and(|record| record.recognises(file)) {
        return Ok(NextWrite::Remove);
    }
    Ok(match super::_agents_migrate::current(file)? {
        Some(converted) => NextWrite::Convert(converted),
        None => NextWrite::Keep,
    })
}

/// Removes a recognised copy at `path`, then its record at `state`.
fn remove_copy(path: &Path, state: &Path) -> Result<(), ConfigError> {
    for removed in [path, state] {
        match crate::fsx::remove_file(removed) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(crate::fsx::FsError::Write {
                    path: removed.to_path_buf(),
                    source,
                }
                .into());
            }
        }
    }
    tracing::info!(event = "agents_defaults_copy_removed");
    Ok(())
}

/// `agents.yaml`'s bytes, or `None` when there is no file.
pub(super) fn read_existing(path: &Path) -> Result<Option<Vec<u8>>, ConfigError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// What an ignored deployment record leaves, and how an operator returns to
/// the shipped defaults: told whether the record could not be read or could
/// not be understood.
const IGNORED_RECORD_RECOVERY: &str = "the record is ignored: agents.yaml stays as the user's overrides, still converted to the current shape; to return to the shipped defaults, remove agents.yaml together with this record once you have confirmed it holds no edits you want";

#[derive(Deserialize)]
struct Deployment {
    schema_version: u64,
    digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<String>,
}

impl Deployment {
    /// The record an earlier release kept of the defaults it deployed; `None`
    /// when there is none. A record that cannot be read or is in an unknown
    /// shape recognises nothing: the file beside it is kept and the command
    /// proceeds, because the record is only bookkeeping about instrukt's own
    /// earlier write and the person's file is never removed on a guess.
    fn load(state: &Path) -> Option<Self> {
        let bytes = match std::fs::read(state) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(
                    event = "agents_deployment_record_unreadable",
                    record = %state.display(),
                    %error,
                    "{IGNORED_RECORD_RECOVERY}"
                );
                return None;
            }
        };
        match Self::parse(&bytes) {
            Ok(record) => Some(record),
            Err(cause) => {
                tracing::warn!(
                    event = "agents_deployment_record_invalid",
                    record = %state.display(),
                    %cause,
                    "{IGNORED_RECORD_RECOVERY}"
                );
                None
            }
        }
    }

    /// A deployment record from its bytes, or why it recognises nothing.
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let record: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if record.schema_version != DEPLOYMENT_VERSION {
            return Err(format!(
                "schema_version {} is not the supported {DEPLOYMENT_VERSION}",
                record.schema_version
            ));
        }
        Ok(record)
    }

    /// Whether `bytes` are what an earlier release itself wrote: the deployed
    /// defaults, or a write it recorded before making it.
    fn recognises(&self, bytes: &[u8]) -> bool {
        let found = hex::encode(Sha256::digest(bytes));
        self.digest == found || self.pending.as_deref() == Some(found.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::super::SHIPPED_AGENTS;

    /// The deployment record an earlier release kept of `bytes`.
    fn record_of(bytes: &str) -> super::Deployment {
        use sha2::Digest as _;
        let record = serde_json::json!({
            "schema_version": 1,
            "digest": hex::encode(sha2::Sha256::digest(bytes.as_bytes())),
        });
        super::Deployment::parse(record.to_string().as_bytes()).unwrap()
    }

    /// The shipped defaults in the shape an earlier release wrote: the split
    /// orchestrator role the current shape retires.
    fn older_shape(text: &str) -> String {
        text.replacen("  todo-orchestrator:", "  work-orchestrator:", 1)
    }

    #[test]
    fn a_recognised_older_copy_is_removed_rather_than_converted() {
        let older = older_shape(SHIPPED_AGENTS);
        assert_eq!(
            super::next_write(older.as_bytes(), Some(&record_of(&older))).unwrap(),
            super::NextWrite::Remove
        );
    }

    #[test]
    fn an_edited_older_file_is_converted_and_kept() {
        let edited = format!("{}# my own note\n", older_shape(SHIPPED_AGENTS));
        let decided = super::next_write(
            edited.as_bytes(),
            Some(&record_of(&older_shape(SHIPPED_AGENTS))),
        )
        .unwrap();
        assert!(matches!(decided, super::NextWrite::Convert(_)));
    }

    #[test]
    fn an_edited_current_file_is_kept_as_it_is() {
        let edited = format!("{SHIPPED_AGENTS}# my own note\n");
        assert_eq!(
            super::next_write(edited.as_bytes(), Some(&record_of(SHIPPED_AGENTS))).unwrap(),
            super::NextWrite::Keep
        );
    }

    #[test]
    fn a_record_in_an_unknown_shape_recognises_nothing() {
        use sha2::Digest as _;
        let record = serde_json::json!({
            "schema_version": 2,
            "digest": hex::encode(sha2::Sha256::digest(SHIPPED_AGENTS.as_bytes())),
        });
        assert!(super::Deployment::parse(record.to_string().as_bytes()).is_err());
        assert!(super::Deployment::parse(b"{not a record").is_err());
    }
}
