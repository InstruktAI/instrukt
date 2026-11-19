//! Where each agent runtime instrukt deploys to loads its user-scope
//! instructions, in a home, and, for those that need one, the settings key
//! that makes it load project `AGENTS.md` files. Constants of the
//! implementation, each from the runtime vendor's documentation
//! (`project/instrukt/spec/feature/publication/agent-entrypoints`,
//! "Destinations"); not configuration.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::docs::AgentCli;
use crate::interfaces::ConfiguredInstance;

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
    pub cli: AgentCli,
    /// Its default home, relative to the user's home directory: where an
    /// instance that reports no home of its own reads.
    pub default_home: &'static str,
    /// The instruction file it loads, relative to its home.
    pub file: &'static str,
    /// The setting it needs to load project instruction files; `None` when
    /// it loads them already.
    pub loading: Option<LoadingKey>,
}

/// One destination per runtime instrukt deploys to.
pub(super) const DESTINATIONS: [Destination; 7] = [
    Destination {
        cli: AgentCli::Claude,
        default_home: ".claude",
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
        cli: AgentCli::Codex,
        default_home: ".codex",
        file: "AGENTS.md",
        loading: None,
    },
    Destination {
        cli: AgentCli::Pi,
        default_home: ".pi/agent",
        file: "AGENTS.md",
        loading: None,
    },
    Destination {
        cli: AgentCli::Opencode,
        default_home: ".config/opencode",
        file: "AGENTS.md",
        loading: None,
    },
    Destination {
        cli: AgentCli::Kiro,
        default_home: ".kiro",
        file: "steering/instrukt.md",
        loading: None,
    },
    Destination {
        cli: AgentCli::Antigravity,
        default_home: ".gemini",
        file: "AGENTS.md",
        loading: None,
    },
    Destination {
        cli: AgentCli::Fx,
        default_home: ".fx",
        file: "AGENTS.md",
        loading: None,
    },
];

/// The destination of the runtime named `runtime`; `None` for a runtime
/// instrukt does not deploy to.
pub(super) fn destination_of(runtime: &str) -> Option<&'static Destination> {
    let cli = AgentCli::named(runtime)?;
    DESTINATIONS
        .iter()
        .find(|destination| destination.cli == cli)
}

/// One global agent file the configured instances need: its runtime, the
/// home that holds it and every instance it serves.
#[derive(Debug, Clone)]
pub(super) struct TargetFile {
    pub destination: &'static Destination,
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

/// Where the configured instances read: the files, the links to them, and
/// each instance that reported no home and was given its runtime's default.
#[derive(Debug, Clone, Default)]
pub(super) struct Targets {
    pub files: Vec<TargetFile>,
    pub links: Vec<TargetLink>,
    /// `(instance, runtime, default home)` of each instance that reported
    /// no home.
    pub defaulted: Vec<(String, &'static str, PathBuf)>,
    /// `(instance, runtime, path)` of each instance whose home cannot be
    /// known: one named relative to the runtime's own working directory.
    /// Nothing is written for it, never relative to publication's own.
    pub unknown: Vec<(String, &'static str, PathBuf)>,
}

/// Where `instances`, as the selected runtime reported them, read, for a
/// user whose home directory is `user_home`. An instance of a runtime
/// instrukt does not deploy to receives nothing; instances that share a
/// home share one file.
pub(super) fn targets(user_home: &Path, instances: &[ConfiguredInstance]) -> Targets {
    let mut targets = Targets::default();
    for instance in instances {
        let Some(destination) = destination_of(&instance.runtime) else {
            continue;
        };
        // Only an absolute home or link folder is ever written to.
        let relative = instance.unknown_home.clone().or_else(|| {
            instance
                .home
                .iter()
                .chain(&instance.link_folders)
                .find(|path| path.is_relative())
                .cloned()
        });
        if let Some(path) = relative {
            targets
                .unknown
                .push((instance.instance.to_string(), destination.cli.name(), path));
            continue;
        }
        let home = match &instance.home {
            Some(home) => home.clone(),
            None => {
                let home = user_home.join(destination.default_home);
                targets.defaulted.push((
                    instance.instance.to_string(),
                    destination.cli.name(),
                    home.clone(),
                ));
                home
            }
        };
        let id = instance.instance.to_string();
        match targets
            .files
            .iter_mut()
            .find(|file| file.destination.cli == destination.cli && file.home == home)
        {
            Some(file) => file.instances.push(id),
            None => targets.files.push(TargetFile {
                destination,
                home: home.clone(),
                instances: vec![id],
            }),
        }
        for folder in &instance.link_folders {
            let link = TargetLink {
                link: folder.join(destination.file),
                target: home.join(destination.file),
            };
            if !targets.links.contains(&link) {
                targets.links.push(link);
            }
        }
    }
    targets
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
