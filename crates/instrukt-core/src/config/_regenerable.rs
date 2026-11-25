//! The project layer's `regenerable` declaration
//! (`project/instrukt/spec/feature/configuration/layered-configuration`,
//! "Regenerable output declaration"): the locations inside a checkout that
//! the project rebuilds from its committed sources. An entry that reaches
//! outside the checkout is a finding at the key, and no reader ever holds it
//! as a [`RegenerablePath`].

use std::path::{Component, Path, PathBuf};

use serde_yaml_ng::Value;

use super::_error::{Finding, FindingKind};
use super::_schema::KeyFinding;

/// The project-layer key that declares a project's regenerable output.
pub const REGENERABLE_KEY: &str = "regenerable";

/// One declared location, relative to a checkout's root, that names neither
/// the root itself nor anything outside it nor the repository's `.git`: a
/// path of plain names only, at least one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegenerablePath(PathBuf);

/// Why an entry declares no location inside a checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryRefusal {
    Empty,
    Absolute,
    StepsToParent,
    NamesTheRoot,
    NamesGitDirectory,
}

impl EntryRefusal {
    fn reason(self) -> &'static str {
        match self {
            Self::Empty => "is empty",
            Self::Absolute => "is absolute",
            Self::StepsToParent => "steps to a parent directory",
            Self::NamesTheRoot => "names the checkout's root itself",
            Self::NamesGitDirectory => "names the repository's `.git`",
        }
    }
}

impl RegenerablePath {
    /// The location `entry` declares, or why it declares none inside a
    /// checkout. `.git` is matched without regard to case: the volumes
    /// instrukt runs on include case-insensitive ones.
    pub fn parse(entry: &str) -> Result<Self, EntryRefusal> {
        if entry.is_empty() {
            return Err(EntryRefusal::Empty);
        }
        let path = Path::new(entry);
        let mut names = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => return Err(EntryRefusal::Absolute),
                Component::ParentDir => return Err(EntryRefusal::StepsToParent),
                Component::CurDir => {}
                Component::Normal(name) => names.push(name),
            }
        }
        match names.components().next() {
            None => Err(EntryRefusal::NamesTheRoot),
            Some(first) if first.as_os_str().eq_ignore_ascii_case(".git") => {
                Err(EntryRefusal::NamesGitDirectory)
            }
            Some(_) => Ok(Self(names)),
        }
    }

    /// The location, relative to a checkout's root.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Display for RegenerablePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

/// Whether `finding` withholds worktree reclamation for the project whose
/// layer it is of: it stands at `regenerable`.
pub fn withholds_reclamation(finding: &Finding) -> bool {
    finding.key == REGENERABLE_KEY || finding.key.starts_with(&format!("{REGENERABLE_KEY}."))
}

/// The findings of the `regenerable` value `value` at `path`: a value that
/// is not a list of strings, and each entry that declares no location
/// inside a checkout, named.
pub(super) fn check(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(entries) = value.as_sequence() else {
        out.push(KeyFinding::new(
            path,
            FindingKind::InvalidValue,
            "expected a list of paths inside the checkout",
        ));
        return;
    };
    for entry in entries {
        let Some(text) = entry.as_str() else {
            out.push(KeyFinding::new(
                path,
                FindingKind::InvalidValue,
                "expected a list of paths inside the checkout; an entry is not a string",
            ));
            continue;
        };
        if let Err(refusal) = RegenerablePath::parse(text) {
            out.push(KeyFinding::new(
                path,
                FindingKind::InvalidValue,
                format!(
                    "the entry `{text}` {}; an entry names a location inside the checkout",
                    refusal.reason()
                ),
            ));
        }
    }
}

/// The declared locations of the `regenerable` value `value`, as validation
/// admits them; `None` when the value is not a list of entries that each
/// declare a location inside a checkout.
pub(super) fn parse(value: &Value) -> Option<Vec<RegenerablePath>> {
    value
        .as_sequence()?
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .and_then(|text| RegenerablePath::parse(text).ok())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_location_is_declared_with_its_current_directory_steps_and_trailing_slash_dropped()
    {
        assert_eq!(
            RegenerablePath::parse("./build/out/").map(|p| p.as_path().to_path_buf()),
            Ok(PathBuf::from("build/out"))
        );
    }

    #[test]
    fn an_empty_entry_declares_nothing() {
        assert_eq!(RegenerablePath::parse(""), Err(EntryRefusal::Empty));
    }

    #[test]
    fn an_absolute_entry_reaches_outside_the_checkout() {
        assert_eq!(
            RegenerablePath::parse("/tmp/out"),
            Err(EntryRefusal::Absolute)
        );
    }

    #[test]
    fn an_entry_stepping_to_a_parent_reaches_outside_the_checkout() {
        assert_eq!(
            RegenerablePath::parse("target/../../elsewhere"),
            Err(EntryRefusal::StepsToParent)
        );
    }

    #[test]
    fn an_entry_naming_only_the_current_directory_names_the_root() {
        assert_eq!(
            RegenerablePath::parse("./."),
            Err(EntryRefusal::NamesTheRoot)
        );
    }

    #[test]
    fn an_entry_under_the_repository_git_directory_is_refused_whatever_its_case() {
        assert_eq!(
            RegenerablePath::parse("./.GIT/objects"),
            Err(EntryRefusal::NamesGitDirectory)
        );
    }
}
