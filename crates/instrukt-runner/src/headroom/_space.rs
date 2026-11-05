//! Free space of the shared volume and the level it puts the guard at.

use std::path::Path;

use instrukt_core::config::HeadroomSettings;
use serde::{Deserialize, Serialize};

/// Where free space stands against the guard's levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// At or above the warn level.
    Ok,
    /// Below the warn level, at or above the floor.
    Warn,
    /// Below the floor.
    Floor,
    /// Free space could not be measured.
    Unmeasured,
}

impl Level {
    /// The level's name, as the state database stores it.
    pub fn name(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warn => "warn",
            Level::Floor => "floor",
            Level::Unmeasured => "unmeasured",
        }
    }

    /// The level a stored name names.
    pub fn from_name(name: &str) -> Option<Self> {
        [Level::Ok, Level::Warn, Level::Floor, Level::Unmeasured]
            .into_iter()
            .find(|level| level.name() == name)
    }

    /// The level `free_bytes` puts the guard at.
    pub fn of(free_bytes: u64, settings: &HeadroomSettings) -> Self {
        if free_bytes >= settings.warn_bytes {
            Level::Ok
        } else if free_bytes >= settings.floor_bytes {
            Level::Warn
        } else {
            Level::Floor
        }
    }
}

/// The bytes an unprivileged user may still write on the volume holding
/// `path`.
pub fn free_bytes(path: &Path) -> std::io::Result<u64> {
    let stat = rustix::fs::statvfs(path)?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// `bytes` for a reader: GiB, MiB or KiB with one decimal.
pub fn size_text(bytes: u64) -> String {
    const UNITS: [(&str, u32); 3] = [("GiB", 30), ("MiB", 20), ("KiB", 10)];
    for (unit, shift) in UNITS {
        let scale = 1u64 << shift;
        if bytes >= scale {
            return format!("{:.1} {unit}", bytes as f64 / scale as f64);
        }
    }
    format!("{bytes} B")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> HeadroomSettings {
        HeadroomSettings {
            warn_bytes: 30,
            floor_bytes: 10,
            roots: Vec::new(),
        }
    }

    #[test]
    fn free_space_at_the_warn_level_is_ok() {
        assert_eq!(Level::of(30, &settings()), Level::Ok);
    }

    #[test]
    fn free_space_below_the_warn_level_at_the_floor_is_warn() {
        assert_eq!(Level::of(29, &settings()), Level::Warn);
        assert_eq!(Level::of(10, &settings()), Level::Warn);
    }

    #[test]
    fn free_space_below_the_floor_is_floor() {
        assert_eq!(Level::of(9, &settings()), Level::Floor);
    }

    #[test]
    fn a_stored_level_name_reads_back_as_its_level() {
        for level in [Level::Ok, Level::Warn, Level::Floor, Level::Unmeasured] {
            assert_eq!(Level::from_name(level.name()), Some(level));
        }
    }
}
