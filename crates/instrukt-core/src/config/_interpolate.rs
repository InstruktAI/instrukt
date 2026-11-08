use serde_yaml_ng::Value;

use super::_error::ConfigError;
use super::_runtime::{Env, Runtime};

/// What the interpolator resolves `${VAR}` and `runtime.*` against.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mode<'a> {
    /// Expand against the caller's environment.
    Expand(&'a Env),
    Secrets(&'a crate::home::Home),
    /// Check references without an environment; variables and the derived
    /// caller session expand to empty values.
    Check,
}

/// Resolves paths and expands references over one configuration value.
pub(crate) struct Interpolator<'a> {
    root: &'a Value,
    mode: Mode<'a>,
}

/// Where a path led: a value of the configuration, or the derived session id.
enum Located<'a> {
    Value { path: String, value: &'a Value },
    Derived(String),
}

impl<'a> Interpolator<'a> {
    pub(crate) fn new(root: &'a Value, mode: Mode<'a>) -> Self {
        Self { root, mode }
    }

    /// The value at dotted `path` (the whole configuration when `None`),
    /// every string expanded except webhook credential references, which
    /// remain stored references for their typed accessor.
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
        if path == "webhooks" || path.starts_with("webhooks.") {
            return Ok(value.clone());
        }
        match value {
            Value::String(text) => {
                let mut stack = vec![path.to_owned()];
                Ok(Value::String(self.expand_str(text, &mut stack)?))
            }
            Value::Mapping(map) => {
                let mut out = serde_yaml_ng::Mapping::new();
                for (key, child) in map {
                    let child_path = join(path, &key_text(key));
                    let child = self.expand_value(child, &child_path)?;
                    out.insert(key.clone(), child);
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
        scan(text, |token| match token {
            Token::Variable(name) => self.variable(name),
            Token::Reference(path) => self.reference(path, stack),
        })
    }

    fn variable(&self, name: &str) -> Result<String, ConfigError> {
        match self.mode {
            Mode::Check => Ok(String::new()),
            Mode::Secrets(home) => super::resolve_secret(home, &format!("${{{name}}}")),
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
        match self.locate(path)? {
            Located::Derived(text) => Ok(text),
            Located::Value { path, value } => self.follow(path, value, stack),
        }
    }

    /// Expands the value a reference led to, refusing a reference back to
    /// a value already being expanded.
    fn follow(
        &self,
        path: String,
        value: &Value,
        stack: &mut Vec<String>,
    ) -> Result<String, ConfigError> {
        if path.starts_with("webhooks.") {
            return value
                .as_str()
                .map(str::to_owned)
                .ok_or(ConfigError::NotScalar { path });
        }
        if stack.contains(&path) {
            let mut chain = stack.clone();
            chain.push(path);
            return Err(ConfigError::Cycle { chain });
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
        if fields != ["session_id"] {
            if fields.len() != 1 || !super::_runtime::ACTIONS.contains(&fields[0]) {
                return Err(missing(path, &fields.join("."), "runtime"));
            }
            let runtime = self
                .root
                .get("runner")
                .and_then(|r| r.get("runtime"))
                .and_then(Value::as_str)
                .ok_or(ConfigError::NoActiveRuntime)?;
            if Runtime::named(runtime).is_none() {
                return Err(ConfigError::InvalidValue {
                    key: "runner.runtime".into(),
                    message: format!("unknown compiled runtime adapter `{runtime}`"),
                });
            }
            return self.walk(&["runtimes", runtime, fields[0]], path);
        }
        if let Mode::Check = self.mode {
            return Ok(Located::Derived(String::new()));
        }
        let Mode::Expand(env) = self.mode else {
            return Err(ConfigError::NoActiveRuntime);
        };
        let selected = self
            .root
            .get("runner")
            .and_then(|r| r.get("runtime"))
            .and_then(Value::as_str)
            .and_then(Runtime::named);
        let session = selected
            .and_then(|r| r.caller(env))
            .ok_or(ConfigError::NoActiveRuntime)?;
        Ok(Located::Derived(session.id.to_string()))
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

/// One reference found in a text.
enum Token<'t> {
    /// `${NAME}`.
    Variable(&'t str),
    /// `{dotted.path}`.
    Reference(&'t str),
}

/// `text` with every reference replaced by what `expand` answers for it;
/// anything that is not a reference stays literally.
fn scan<'t>(
    text: &'t str,
    mut expand: impl FnMut(Token<'t>) -> Result<String, ConfigError>,
) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(['$', '{']) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some(inner) = tail.strip_prefix("${")
            && let Some(end) = inner.find('}')
            && is_variable_name(&inner[..end])
        {
            out.push_str(&expand(Token::Variable(&inner[..end]))?);
            rest = &inner[end + 1..];
            continue;
        }
        if let Some(inner) = tail.strip_prefix('{')
            && let Some(end) = inner.find('}')
            && is_reference_path(&inner[..end])
        {
            out.push_str(&expand(Token::Reference(&inner[..end]))?);
            rest = &inner[end + 1..];
            continue;
        }
        let width = tail.chars().next().map_or(1, char::len_utf8);
        out.push_str(&tail[..width]);
        rest = &tail[width..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `text` with every reference replaced by `placeholder`, and whether the
/// text consists of nothing but references (and whitespace).
pub(crate) fn mask_references(text: &str, placeholder: &str) -> (String, bool) {
    let mut found = false;
    let masked = scan(text, |_| {
        found = true;
        Ok(placeholder.to_owned())
    })
    .unwrap_or_else(|_| text.to_owned());
    let bare = scan(text, |_| Ok(String::new())).unwrap_or_else(|_| text.to_owned());
    (masked, found && bare.trim().is_empty())
}

fn missing(path: &str, segment: &str, parent: &str) -> ConfigError {
    ConfigError::MissingKey {
        path: path.to_owned(),
        segment: segment.to_owned(),
        parent: format!("`{parent}`"),
    }
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
    struct NativeCaller;
    impl crate::interfaces::CallerIdentityLookup for NativeCaller {
        fn lookup_caller(&self, env: &Env) -> Option<crate::interfaces::CallerIdentity> {
            Some(crate::interfaces::CallerIdentity {
                session: crate::ids::SessionRef::parse(&format!("t3:{}", env.get("TEST_SESSION")?))
                    .ok()?,
                agent: None,
            })
        }
        fn caller_variables(&self) -> &'static [&'static str] {
            &["TEST_SESSION"]
        }
    }
    #[test]
    fn references_expand_and_cycles_fail() {
        let root: Value =
            serde_yaml_ng::from_str("base: /srv\npath: '{base}/${USER}'\na: '{b}'\nb: '{a}'\n")
                .unwrap();
        let env = Env::from_pairs([("USER", "developer")]);
        let i = Interpolator::new(&root, Mode::Expand(&env));
        assert_eq!(
            i.get(Some("path")).unwrap(),
            Value::String("/srv/developer".into())
        );
        assert!(matches!(i.get(Some("a")), Err(ConfigError::Cycle { .. })));
    }
    #[test]
    fn caller_context_never_selects_a_runtime() {
        let env = Env::from_pairs([("TEST_SESSION", "thread:1")]).with_caller(&NativeCaller);
        let root: Value = serde_yaml_ng::from_str("runner: {runtime: t3}").unwrap();
        assert_eq!(
            Interpolator::new(&root, Mode::Expand(&env))
                .get(Some("runtime.session_id"))
                .unwrap(),
            Value::String("thread:1".into())
        );
        assert!(
            Interpolator::new(&Value::Null, Mode::Expand(&env))
                .get(Some("runtime.session_id"))
                .is_err()
        );
        assert!(
            Interpolator::new(&root, Mode::Expand(&env))
                .get(Some("runtime.send"))
                .is_err()
        );
    }
}
