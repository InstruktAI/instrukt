//! Where each agent runtime instrukt deploys to loads its user-scope
//! instructions, in a home, and, for those that need one, the settings key
//! that makes it load project `AGENTS.md` files. Constants of the
//! implementation, each from the runtime vendor's documentation
//! (`project/instrukt/spec/feature/publication/agent-entrypoints`,
//! "Destinations"); not configuration.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::PublicationError;
use crate::interfaces::{ConfiguredInstance, InstructionAssurance, InstructionHomeSource};

/// A settings key that makes an agent runtime load `AGENTS.md` files.
#[derive(Debug, Clone, Copy)]
pub(super) struct LoadingKey {
    /// The JSON settings file, relative to the runtime's home.
    pub settings: &'static str,
    /// The key's path of object members.
    pub key: &'static [&'static str],
    /// The value it takes.
    pub value: fn() -> Value,
}

/// One agent runtime's destination.
#[derive(Debug, Clone, Copy)]
pub(super) struct Destination {
    /// The runtime, as its adapter and `agents` name it.
    pub cli: &'static str,
    /// The instruction file it loads, relative to its home.
    pub file: &'static str,
    /// The setting it needs to load project instruction files; `None` when
    /// it loads them already.
    pub loading: Option<LoadingKey>,
}

/// Native file metadata for established loader families, independent of name admission.
pub(super) const DESTINATIONS: [Destination; 9] = [
    Destination {
        cli: "claude",
        file: "CLAUDE.md",
        loading: Some(LoadingKey {
            settings: "settings.json",
            key: &[
                "pluginConfigs",
                "agents-md@builtin",
                "options",
                "instructionFiles",
            ],
            value: || Value::String("claude-md-and-agents-md".to_owned()),
        }),
    },
    Destination {
        cli: "codex",
        file: "AGENTS.md",
        loading: None,
    },
    Destination {
        cli: "pi",
        file: "AGENTS.md",
        loading: None,
    },
    Destination {
        cli: "opencode",
        file: "AGENTS.md",
        loading: None,
    },
    Destination {
        cli: "kiro",
        file: "steering/instrukt.md",
        loading: None,
    },
    Destination {
        cli: "antigravity",
        file: "AGENTS.md",
        loading: None,
    },
    Destination {
        cli: "cursor",
        file: "rules/instrukt.mdc",
        loading: None,
    },
    Destination {
        cli: "grok",
        file: "AGENTS.md",
        loading: None,
    },
    Destination {
        cli: "fx",
        file: "AGENTS.md",
        loading: None,
    },
];

/// Native metadata for an adapter-established loader family; never provider discovery.
pub(super) fn destination_of(runtime: &str) -> Option<&'static Destination> {
    DESTINATIONS
        .iter()
        .find(|destination| destination.cli == runtime)
}

/// One global agent file the configured instances need: its runtime, the
/// home that holds it and every instance it serves.
#[derive(Debug, Clone)]
pub(super) struct TargetFile {
    pub destination: &'static Destination,
    pub runtime: String,
    /// The home that holds the file, as reported or defaulted.
    pub home: PathBuf,
    /// The instances this file serves, in the order the runtime reported
    /// them.
    pub instances: Vec<String>,
}

impl TargetFile {
    /// The file's path: its home joined with the runtime's file.
    pub fn path(&self) -> PathBuf {
        self.home.join(self.destination.file)
    }
}

/// A link to an instance's file that must exist in one of its link folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TargetLink {
    /// Where the link stands: the folder joined with the file's path in its
    /// home.
    pub link: PathBuf,
    /// The file it points at, by its absolute path, as the runtime makes it.
    pub target: PathBuf,
}

/// Current per-instance target facts, shared by sync, init and status.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstructionTarget {
    pub instance: String,
    pub runtime: String,
    pub registry_agent: Option<String>,
    pub unknown_home: Option<PathBuf>,
    pub conflict_path: Option<PathBuf>,
    pub mechanism: Option<String>,
    pub target: Option<PathBuf>,
    pub outcome: InstructionOutcome,
    pub reason: Option<NonDelivery>,
    pub home_source: InstructionHomeSource,
    pub assurance: Vec<InstructionAssurance>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionOutcome {
    FilePresent,
    FileMissing,
    NotDelivered,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NonDelivery {
    MechanismUnestablished,
    HomeUnknown,
    NativeOverride,
    TargetConflict,
}
impl NonDelivery {
    pub fn name(self) -> &'static str {
        match self {
            Self::MechanismUnestablished => "mechanism_unestablished",
            Self::HomeUnknown => "home_unknown",
            Self::NativeOverride => "native_override",
            Self::TargetConflict => "target_conflict",
        }
    }
}
#[derive(Debug, Clone, Default)]
pub(super) struct Targets {
    pub files: Vec<TargetFile>,
    pub links: Vec<TargetLink>,
    pub instances: Vec<InstructionTarget>,
}
fn io(path: &Path, source: std::io::Error) -> PublicationError {
    PublicationError::Io {
        path: path.to_path_buf(),
        source,
    }
}
fn present(path: &Path) -> Result<bool, PublicationError> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(meta.is_file()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io(path, e)),
    }
}
/// Identity of the entry publication replaces: resolve its directory, never its final link.
fn physical(path: &Path) -> Result<PathBuf, PublicationError> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(PublicationError::Unconfined {
            path: path.to_path_buf(),
        });
    };
    Ok(physical_directory(parent)?.join(name))
}
fn physical_directory(path: &Path) -> Result<PathBuf, PublicationError> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                let mut resolved = physical_directory(parent)?;
                if path.components().next_back() == Some(std::path::Component::ParentDir) {
                    resolved.pop();
                    return Ok(resolved);
                }
                if let Some(name) = path.file_name() {
                    return Ok(resolved.join(name));
                }
            }
            Ok(path.to_path_buf())
        }
        Err(e) => Err(io(path, e)),
    }
}
/// Resolve observed instances only. Home precedence/defaults belong to the adapter.
pub(super) fn targets(instances: &[ConfiguredInstance]) -> Result<Targets, PublicationError> {
    let mut result = Targets::default();
    for instance in instances {
        let mut fact = InstructionTarget {
            instance: instance.instance.to_string(),
            runtime: instance.runtime.clone(),
            registry_agent: instance.registry_agent.clone(),
            unknown_home: instance.unknown_home.clone(),
            conflict_path: None,
            mechanism: None,
            target: None,
            outcome: InstructionOutcome::NotDelivered,
            reason: None,
            home_source: instance.home_source,
            assurance: instance.assurance.clone(),
        };
        let destination = instance.native_runtime.as_deref().and_then(destination_of);
        if let Some(destination) = destination {
            fact.mechanism = Some(destination.file.to_owned());
        }
        fact.reason = if destination.is_none() {
            Some(NonDelivery::MechanismUnestablished)
        } else if instance.home_source == InstructionHomeSource::Unknown
            || instance.unknown_home.is_some()
            || instance.home.as_ref().is_none_or(|p| !p.is_absolute())
            || instance.link_folders.iter().any(|p| !p.is_absolute())
        {
            Some(NonDelivery::HomeUnknown)
        } else {
            None
        };
        if let (None, Some(destination), Some(home)) = (fact.reason, destination, &instance.home) {
            let path = home.join(destination.file);
            fact.target = Some(path.clone());
            if matches!(destination.cli, "codex" | "pi") {
                for folder in std::iter::once(home).chain(&instance.link_folders) {
                    let override_path = folder.join("AGENTS.override.md");
                    match std::fs::read(&override_path) {
                        Ok(bytes) if !bytes.is_empty() => {
                            fact.reason = Some(NonDelivery::NativeOverride)
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(io(&override_path, e)),
                    }
                }
            }
            if fact.reason.is_none() {
                fact.outcome = if present(&path)? {
                    InstructionOutcome::FilePresent
                } else {
                    InstructionOutcome::FileMissing
                };
            }
        }
        result.instances.push(fact);
    }
    // Include link entries: a shadow link must never replace another selection's file,
    // nor point the same entry at two different homes.
    #[derive(Clone, PartialEq, Eq)]
    enum Intent {
        File(String),
        Link { selection: String, target: PathBuf },
    }
    let mut owners = std::collections::BTreeMap::<PathBuf, Intent>::new();
    let mut conflicts = std::collections::BTreeSet::new();
    let mut addresses = Vec::with_capacity(instances.len());
    for (instance, fact) in instances.iter().zip(&result.instances) {
        let mut claimed = Vec::new();
        if fact.reason.is_none()
            && let Some(path) = &fact.target
        {
            let key = physical(path)?;
            claimed.push((key, Intent::File(fact.runtime.clone())));
            if let Some(destination) = instance.native_runtime.as_deref().and_then(destination_of) {
                for folder in &instance.link_folders {
                    claimed.push((
                        physical(&folder.join(destination.file))?,
                        Intent::Link {
                            selection: fact.runtime.clone(),
                            target: path.clone(),
                        },
                    ));
                }
            }
        }
        for (key, intent) in &claimed {
            if let Some(previous) = owners.insert(key.clone(), intent.clone())
                && previous != *intent
            {
                conflicts.insert(key.clone());
            }
        }
        addresses.push(claimed);
    }
    for ((instance, fact), claimed) in instances.iter().zip(&mut result.instances).zip(addresses) {
        if let Some((path, _)) = claimed.iter().find(|(path, _)| conflicts.contains(path)) {
            fact.conflict_path = Some(path.clone());
            fact.reason = Some(NonDelivery::TargetConflict);
            fact.outcome = InstructionOutcome::NotDelivered;
        }
        if fact.reason.is_some() {
            continue;
        }
        if let (Some(destination), Some(home)) = (
            instance.native_runtime.as_deref().and_then(destination_of),
            &instance.home,
        ) {
            let key = physical(&home.join(destination.file))?;
            let mut existing = None;
            for (n, file) in result.files.iter().enumerate() {
                if physical(&file.path())? == key {
                    existing = Some(n);
                    break;
                }
            }
            if let Some(n) = existing {
                result.files[n].instances.push(fact.instance.clone());
            } else {
                result.files.push(TargetFile {
                    destination,
                    runtime: instance.runtime.clone(),
                    home: home.clone(),
                    instances: vec![fact.instance.clone()],
                });
            }
            for folder in &instance.link_folders {
                let link = TargetLink {
                    link: folder.join(destination.file),
                    target: home.join(destination.file),
                };
                let key = physical(&link.link)?;
                let mut duplicate = false;
                for existing in &result.links {
                    if physical(&existing.link)? == key {
                        duplicate = true;
                        break;
                    }
                }
                if !duplicate {
                    result.links.push(link);
                }
            }
        }
    }
    Ok(result)
}

/// Why a settings file cannot take the loading key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MergeRefusal {
    /// The file is not a JSON object.
    NotAnObject,
    /// A member on the key's path holds something other than an object.
    Blocked { member: String },
}

/// `settings` (absent: `None`) with `key` set to `value` and nothing else
/// changed; `Ok(None)` when it already holds that value.
pub(super) fn merge_key(
    settings: Option<&[u8]>,
    key: &[&str],
    value: Value,
) -> Result<Option<Vec<u8>>, MergeRefusal> {
    let mut root = match settings {
        None => Value::Object(Map::new()),
        Some(bytes) => {
            serde_json::from_slice::<Value>(bytes).map_err(|_| MergeRefusal::NotAnObject)?
        }
    };
    let Value::Object(object) = &mut root else {
        return Err(MergeRefusal::NotAnObject);
    };
    let Some((last, parents)) = key.split_last() else {
        return Ok(None);
    };
    let mut current = object;
    for member in parents {
        let next = current
            .entry((*member).to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        current = match next {
            Value::Object(map) => map,
            _ => {
                return Err(MergeRefusal::Blocked {
                    member: (*member).to_owned(),
                });
            }
        };
    }
    if current.get(*last) == Some(&value) {
        return Ok(None);
    }
    current.insert((*last).to_owned(), value);
    let mut text = serde_json::to_vec_pretty(&root).map_err(|_| MergeRefusal::NotAnObject)?;
    text.push(b'\n');
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Value {
        serde_json::from_slice(bytes).unwrap()
    }

    #[test]
    fn the_key_is_set_and_every_other_member_is_kept() {
        let before =
            br#"{"theme":"dark","context":{"includeDirectories":["/"],"fileName":"X.md"}}"#;
        let merged = merge_key(
            Some(before),
            &["context", "fileName"],
            Value::String("AGENTS.md".into()),
        )
        .unwrap()
        .unwrap();
        let mut expected = parse(before);
        expected["context"]["fileName"] = Value::String("AGENTS.md".into());
        assert_eq!(parse(&merged), expected);
    }

    #[test]
    fn a_missing_file_holds_only_the_key_and_a_set_key_is_unchanged() {
        let merged = merge_key(None, &["a", "b"], Value::Bool(true))
            .unwrap()
            .unwrap();
        assert_eq!(parse(&merged), serde_json::json!({"a": {"b": true}}));
        assert_eq!(
            merge_key(Some(&merged), &["a", "b"], Value::Bool(true)),
            Ok(None)
        );
    }

    #[test]
    fn unparseable_or_blocking_settings_are_refused() {
        assert_eq!(
            merge_key(Some(b"{not json"), &["a"], Value::Null),
            Err(MergeRefusal::NotAnObject)
        );
        assert_eq!(
            merge_key(Some(br#"{"a": 1}"#), &["a", "b"], Value::Null),
            Err(MergeRefusal::Blocked { member: "a".into() })
        );
    }
}
