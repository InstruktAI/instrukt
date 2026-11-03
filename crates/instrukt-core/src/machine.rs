//! This computer's identity: the id in `~/.instrukt/state/machine-id`,
//! created once, that holds the computer's execution claims
//! (architecture overview, "The home"; `work-dispatch`, "Execution
//! contract").
//!
//! Also the random source every durable instrukt identity is minted from.

use std::fmt;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::home::Home;

/// The operating system's random source.
const RANDOM_SOURCE: &str = "/dev/urandom";

/// How many random bytes a machine id carries.
const MACHINE_ID_BYTES: usize = 16;

/// The machine id could not be read or created.
#[derive(Debug, thiserror::Error)]
pub enum MachineError {
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot create {}: {source}", path.display())]
    Create { path: PathBuf, source: io::Error },
    #[error("{} is not a machine id; delete it to mint a new one", path.display())]
    Malformed { path: PathBuf },
    #[error("{} is a symbolic link; instrukt does not follow links in its home", path.display())]
    Symlink { path: PathBuf },
}

/// A computer's id: lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MachineId(String);

impl MachineId {
    /// Validates `value` as a machine id: 16 to 64 lowercase hex characters.
    pub fn parse(value: &str) -> Option<Self> {
        let ok = (16..=64).contains(&value.len())
            && value
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        ok.then(|| Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MachineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for MachineId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for MachineId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| serde::de::Error::custom("not a machine id"))
    }
}

/// This computer's id, created once in `state/machine-id` on first use.
/// Two processes creating it at once agree on one id: the file is created
/// without replacing an existing one, and the loser reads the winner's.
pub fn machine_id(home: &Home) -> Result<MachineId, MachineError> {
    let path = home.machine_id_file();
    if let Some(id) = read(&path)? {
        return Ok(id);
    }
    let created = random_hex(MACHINE_ID_BYTES).map_err(|source| MachineError::Create {
        path: path.clone(),
        source,
    })?;
    create_once(&path, &created)?;
    read(&path)?.ok_or(MachineError::Malformed { path })
}

/// `bytes` random bytes from the operating system, as lowercase hex.
pub fn random_hex(bytes: usize) -> io::Result<String> {
    let mut buffer = vec![0u8; bytes];
    std::fs::File::open(RANDOM_SOURCE)?.read_exact(&mut buffer)?;
    Ok(hex::encode(buffer))
}

fn read(path: &Path) -> Result<Option<MachineId>, MachineError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(MachineError::Symlink {
                path: path.to_path_buf(),
            });
        }
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(MachineError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    let text = std::fs::read_to_string(path).map_err(|source| MachineError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    MachineId::parse(text.trim())
        .map(Some)
        .ok_or_else(|| MachineError::Malformed {
            path: path.to_path_buf(),
        })
}

fn create_once(path: &Path, id: &str) -> Result<(), MachineError> {
    let wrap = |source: io::Error| MachineError::Create {
        path: path.to_path_buf(),
        source,
    };
    let parent = path.parent().ok_or_else(|| {
        wrap(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the machine-id path has no parent",
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(wrap)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(wrap)?;
    io::Write::write_all(&mut temp, format!("{id}\n").as_bytes()).map_err(wrap)?;
    temp.as_file().sync_all().map_err(wrap)?;
    match temp.persist_noclobber(path) {
        Ok(_) => Ok(()),
        // Another process created it first; its id stands.
        Err(err) if err.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(wrap(err.error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_created_once_and_then_kept() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let first = machine_id(&home).unwrap();
        let second = machine_id(&home).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.as_str().len(), MACHINE_ID_BYTES * 2);
    }

    #[test]
    fn a_malformed_id_is_refused_rather_than_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.state_dir()).unwrap();
        std::fs::write(home.machine_id_file(), "not hex\n").unwrap();
        assert!(matches!(
            machine_id(&home),
            Err(MachineError::Malformed { .. })
        ));
    }

    #[test]
    fn a_linked_id_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.state_dir()).unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::write(&elsewhere, "0123456789abcdef0123456789abcdef\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.machine_id_file()).unwrap();
        assert!(matches!(
            machine_id(&home),
            Err(MachineError::Symlink { .. })
        ));
    }
}
