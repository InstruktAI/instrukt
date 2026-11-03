//! Interpolation: `{other.key}`, `${VAR}`, `{runtime.<field>}` and
//! `{runtime.session_id}`, expanded when a value is read.
//!
//! The same walk checks references without an environment for validation:
//! in check mode environment variables and the caller's runtime are not
//! consulted, but key references must resolve and must not form a cycle.

use serde_yaml_ng::Value;

use super::_error::ConfigError;
use super::_runtime::{self, Env, OPTIONAL_FIELDS, REQUIRED_FIELDS, RuntimeEntry, SESSION_ID};

/// What the interpolator resolves `${VAR}` and `runtime.*` against.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mode<'a> {
    /// Expand against the caller's environment.
    Expand(&'a Env),
    /// Check key references only; environment and runtime references
    /// expand to nothing.
    Check,
}

/// Resolves paths and expands references over one configuration value.
pub(crate) struct Interpolator<'a> {
    root: &'a Value,
    runtimes: &'a [RuntimeEntry],
    mode: Mode<'a>,
}

/// Where a path led: a value of the configuration, or the derived session id.
enum Located<'a> {
    Value { path: String, value: &'a Value },
    Derived(String),
}

impl<'a> Interpolator<'a> {
    pub(crate) fn new(root: &'a Value, runtimes: &'a [RuntimeEntry], mode: Mode<'a>) -> Self {
        Self {
            root,
            runtimes,
            mode,
        }
    }

    /// The value at dotted `path` (the whole configuration when `None`),
    /// every string in it expanded.
    pub(crate) fn get(&self, path: Option<&str>) -> Result<Value, ConfigError> {
        let Some(path) = path else {
            return self.expand_value(self.root, "");
        };
        match self.locate(path)? {
            Located::Derived(text) => Ok(Value::String(text)),
            Located::Value { path, value } => self.expand_value(value, &path),
        }
    }

    /// `text` with every reference expanded; `origin` is the path the text
    /// is stored at, when it is stored in the configuration (for cycles).
    pub(crate) fn expand_text(
        &self,
        text: &str,
        origin: Option<&str>,
    ) -> Result<String, ConfigError> {
        let mut stack: Vec<String> = origin.map(str::to_owned).into_iter().collect();
        self.expand_str(text, &mut stack)
    }

    fn expand_value(&self, value: &Value, path: &str) -> Result<Value, ConfigError> {
        match value {
            Value::String(text) => {
                let mut stack = vec![path.to_owned()];
                Ok(Value::String(self.expand_str(text, &mut stack)?))
            }
            Value::Mapping(map) => {
                let mut out = serde_yaml_ng::Mapping::new();
                for (key, child) in map {
                    let child_path = join(path, &key_text(key));
                    out.insert(key.clone(), self.expand_value(child, &child_path)?);
                }
                Ok(Value::Mapping(out))
            }
            Value::Sequence(seq) => seq
                .iter()
                .enumerate()
                .map(|(i, child)| self.expand_value(child, &join(path, &i.to_string())))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Sequence),
            other => Ok(other.clone()),
        }
    }

    fn expand_str(&self, text: &str, stack: &mut Vec<String>) -> Result<String, ConfigError> {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(['$', '{']) {
            out.push_str(&rest[..start]);
            let tail = &rest[start..];
            if let Some(inner) = tail.strip_prefix("${")
                && let Some(end) = inner.find('}')
                && is_variable_name(&inner[..end])
            {
                out.push_str(&self.variable(&inner[..end])?);
                rest = &inner[end + 1..];
                continue;
            }
            if let Some(inner) = tail.strip_prefix('{')
                && let Some(end) = inner.find('}')
                && is_reference_path(&inner[..end])
            {
                out.push_str(&self.reference(&inner[..end], stack)?);
                rest = &inner[end + 1..];
                continue;
            }
            // Not a reference: keep the character literally.
            let width = tail.chars().next().map_or(1, char::len_utf8);
            out.push_str(&tail[..width]);
            rest = &tail[width..];
        }
        out.push_str(rest);
        Ok(out)
    }

    fn variable(&self, name: &str) -> Result<String, ConfigError> {
        match self.mode {
            Mode::Check => Ok(String::new()),
            Mode::Expand(env) => {
                env.get(name)
                    .map(str::to_owned)
                    .ok_or_else(|| ConfigError::UnsetVariable {
                        name: name.to_owned(),
                    })
            }
        }
    }

    fn reference(&self, path: &str, stack: &mut Vec<String>) -> Result<String, ConfigError> {
        let (path, value) = match self.locate(path)? {
            Located::Derived(text) => return Ok(text),
            Located::Value { path, value } => (path, value),
        };
        if stack.contains(&path) {
            let mut chain = stack.clone();
            chain.push(path);
            return Err(ConfigError::Cycle {
                chain: chain.join(" -> "),
            });
        }
        match value {
            Value::String(text) => {
                stack.push(path);
                let expanded = self.expand_str(text, stack);
                stack.pop();
                expanded
            }
            Value::Bool(b) => Ok(b.to_string()),
            Value::Number(n) => Ok(n.to_string()),
            _ => Err(ConfigError::NotScalar { path }),
        }
    }

    /// Resolves a dotted path to its canonical location.
    fn locate(&self, path: &str) -> Result<Located<'a>, ConfigError> {
        let segments: Vec<&str> = path.split('.').collect();
        if segments.first() == Some(&"runtime") {
            return self.locate_runtime(path, &segments[1..]);
        }
        self.walk(&segments, path)
    }

    fn locate_runtime(&self, path: &str, fields: &[&str]) -> Result<Located<'a>, ConfigError> {
        let entry = match self.mode {
            Mode::Check => {
                return match fields {
                    [] => Err(ConfigError::NotScalar {
                        path: path.to_owned(),
                    }),
                    [field] if is_runtime_field(field) => Ok(Located::Derived(String::new())),
                    _ => Err(missing(
                        path,
                        fields.first().copied().unwrap_or(""),
                        "runtime",
                    )),
                };
            }
            Mode::Expand(env) => {
                _runtime::active(self.runtimes, env).ok_or(ConfigError::NoActiveRuntime)?
            }
        };
        let entry_path = format!("runtimes.{}", entry.index);
        if fields == [SESSION_ID] {
            let value = match self.mode {
                Mode::Expand(env) => env.get(&entry.session_id_env).unwrap_or_default(),
                Mode::Check => "",
            };
            return Ok(Located::Derived(value.to_owned()));
        }
        if let [field, ..] = fields
            && !self.entry_has(entry, field)
        {
            return Err(ConfigError::MissingRuntimeField {
                runtime: entry.name.clone(),
                field: (*field).to_owned(),
            });
        }
        let segments: Vec<&str> = entry_path
            .split('.')
            .chain(fields.iter().copied())
            .collect();
        self.walk(&segments, path)
    }

    fn entry_has(&self, entry: &RuntimeEntry, field: &str) -> bool {
        self.root
            .get("runtimes")
            .and_then(|r| r.get(entry.index))
            .and_then(|e| e.get(field))
            .is_some()
    }

    fn walk(&self, segments: &[&str], asked: &str) -> Result<Located<'a>, ConfigError> {
        let mut value = self.root;
        let mut walked = String::new();
        for segment in segments {
            let next = match value {
                Value::Mapping(map) => map.get(*segment),
                Value::Sequence(seq) => segment.parse::<usize>().ok().and_then(|i| seq.get(i)),
                _ => None,
            };
            let Some(next) = next else {
                let parent = if walked.is_empty() {
                    "the configuration".to_owned()
                } else {
                    format!("`{walked}`")
                };
                return Err(ConfigError::MissingKey {
                    path: asked.to_owned(),
                    segment: (*segment).to_owned(),
                    parent,
                });
            };
            walked = join(&walked, segment);
            value = next;
        }
        Ok(Located::Value {
            path: walked,
            value,
        })
    }
}

fn missing(path: &str, segment: &str, parent: &str) -> ConfigError {
    ConfigError::MissingKey {
        path: path.to_owned(),
        segment: segment.to_owned(),
        parent: format!("`{parent}`"),
    }
}

fn is_runtime_field(field: &str) -> bool {
    field == SESSION_ID || REQUIRED_FIELDS.contains(&field) || OPTIONAL_FIELDS.contains(&field)
}

pub(crate) fn join(path: &str, segment: &str) -> String {
    if path.is_empty() {
        segment.to_owned()
    } else {
        format!("{path}.{segment}")
    }
}

pub(crate) fn key_text(key: &Value) -> String {
    match key {
        Value::String(s) => s.clone(),
        other => serde_yaml_ng::to_string(other)
            .map(|s| s.trim_end().to_owned())
            .unwrap_or_default(),
    }
}

fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_reference_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Value {
        serde_yaml_ng::from_str(
            "base: /srv\n\
             path: '{base}/app'\n\
             deep: '{path}/bin ${USER_NAME}'\n\
             json: '{\"k\": 1} {not a ref}'\n\
             n: 3\n\
             loop_a: '{loop_b}'\n\
             loop_b: '{loop_a}'\n\
             self: 'x{self}'\n\
             tree: {a: 1}\n\
             runtimes:\n\
             \x20 - {name: host, session_id_env: HOST_ID, conversation: 'host show {runtime.session_id} --json'}\n\
             \x20 - {name: cli, session_id_env: CLI_ID, conversation: 'cli history {runtime.session_id}'}\n",
        )
        .unwrap()
    }

    fn get(doc: &Value, env: &Env, path: &str) -> Result<Value, ConfigError> {
        let runtimes = _runtime::entries(doc.get("runtimes"));
        Interpolator::new(doc, &runtimes, Mode::Expand(env)).get(Some(path))
    }

    fn text(doc: &Value, env: &Env, path: &str) -> String {
        get(doc, env, path).unwrap().as_str().unwrap().to_owned()
    }

    #[test]
    fn references_expand_recursively_and_from_the_environment() {
        let env = Env::from_pairs([("USER_NAME", "dev")]);
        let doc = doc();
        assert_eq!(text(&doc, &env, "deep"), "/srv/app/bin dev");
        assert_eq!(text(&doc, &env, "json"), "{\"k\": 1} {not a ref}");
    }

    #[test]
    fn an_unset_variable_is_named() {
        let err = get(&doc(), &Env::default(), "deep").unwrap_err();
        assert!(matches!(err, ConfigError::UnsetVariable { name } if name == "USER_NAME"));
    }

    #[test]
    fn cycles_and_self_references_are_named() {
        let doc = doc();
        let err = get(&doc, &Env::default(), "loop_a").unwrap_err();
        assert_eq!(
            err.to_string(),
            "reference cycle: loop_a -> loop_b -> loop_a"
        );
        let err = get(&doc, &Env::default(), "self").unwrap_err();
        assert_eq!(err.to_string(), "reference cycle: self -> self");
    }

    #[test]
    fn the_active_runtime_supplies_its_fields_and_session_id() {
        let doc = doc();
        let env = Env::from_pairs([("CLI_ID", "c-7"), ("USER_NAME", "dev")]);
        assert_eq!(text(&doc, &env, "runtime.conversation"), "cli history c-7");
        assert_eq!(text(&doc, &env, "runtime.session_id"), "c-7");
        assert_eq!(text(&doc, &env, "runtime.name"), "cli");
        let both = Env::from_pairs([("CLI_ID", "c-7"), ("HOST_ID", "h-1")]);
        assert_eq!(
            text(&doc, &both, "runtime.conversation"),
            "host show h-1 --json"
        );
        let err = get(&doc, &Env::default(), "runtime.conversation").unwrap_err();
        assert!(matches!(err, ConfigError::NoActiveRuntime));
        let err = get(&doc, &both, "runtime.start").unwrap_err();
        assert!(matches!(err, ConfigError::MissingRuntimeField { .. }));
    }

    #[test]
    fn a_missing_path_names_its_segment_and_containers_are_not_references() {
        let doc = doc();
        let err = get(&doc, &Env::default(), "tree.b").unwrap_err();
        assert_eq!(err.to_string(), "`tree.b` does not exist: no `b` in `tree`");
        let runtimes = _runtime::entries(doc.get("runtimes"));
        let check = Interpolator::new(&doc, &runtimes, Mode::Check);
        assert!(matches!(
            check.expand_text("{tree}", None),
            Err(ConfigError::NotScalar { .. })
        ));
        assert_eq!(
            check
                .expand_text("{n} ${ANY} {runtime.session_id}", None)
                .unwrap(),
            "3  "
        );
        assert!(check.expand_text("{runtime.bogus}", None).is_err());
        assert!(check.expand_text("{nope}", None).is_err());
    }
}
