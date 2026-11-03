//! The supported agent CLIs: where each loads its user-scope instructions
//! and, for those that need one, the settings key that makes it load
//! project `AGENTS.md` files. Extracted from the predecessor's entrypoint
//! deployment; not configuration.

use serde_json::{Map, Value};

/// A settings key that makes an agent CLI load `AGENTS.md` files.
#[derive(Debug, Clone, Copy)]
pub(super) struct LoadingKey {
    /// The JSON settings file, relative to the user's home directory.
    pub settings: &'static str,
    /// The key's path of object members.
    pub key: &'static [&'static str],
    /// The value it takes.
    pub value: fn() -> Value,
}

/// One supported agent CLI.
#[derive(Debug, Clone, Copy)]
pub(super) struct AgentTarget {
    /// The agent's name; its addendum is `agents/<NAME>.addendum.md`.
    pub name: &'static str,
    /// Its user directory, relative to the user's home directory; a CLI
    /// whose directory does not exist is skipped.
    pub dir: &'static str,
    /// The instruction file it loads, inside `dir`.
    pub entrypoint: &'static str,
    /// The setting it needs to load project instruction files; `None` when
    /// it loads them already.
    pub loading: Option<LoadingKey>,
}

impl AgentTarget {
    /// `agents/<NAME>.addendum.md`.
    pub(super) fn addendum(&self) -> String {
        format!("agents/{}.addendum.md", self.name.to_uppercase())
    }
}

/// The supported agent CLIs.
pub(super) const TARGETS: [AgentTarget; 4] = [
    AgentTarget {
        name: "claude",
        dir: ".claude",
        entrypoint: "CLAUDE.md",
        loading: Some(LoadingKey {
            settings: ".claude/settings.json",
            key: &[
                "pluginConfigs",
                "agents-md@builtin",
                "options",
                "instructionFiles",
            ],
            value: || Value::String("claude-md-and-agents-md".to_owned()),
        }),
    },
    AgentTarget {
        name: "codex",
        dir: ".codex",
        entrypoint: "AGENTS.md",
        loading: None,
    },
    AgentTarget {
        name: "gemini",
        dir: ".gemini/antigravity-cli",
        entrypoint: "AGENTS.md",
        loading: Some(LoadingKey {
            settings: ".gemini/antigravity-cli/settings.json",
            key: &["context", "fileName"],
            value: || {
                Value::Array(vec![
                    Value::String("AGENTS.md".to_owned()),
                    Value::String("GEMINI.md".to_owned()),
                ])
            },
        }),
    },
    AgentTarget {
        name: "deepseek",
        dir: ".dsh",
        entrypoint: "AGENTS.md",
        loading: None,
    },
];

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
