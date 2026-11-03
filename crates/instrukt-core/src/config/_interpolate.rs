//! Interpolation: `{other.key}`, `${VAR}`, `{runtime.<field>}` and
//! `{runtime.session_id}`, expanded when a value is read.
//!
//! The stored runtime mapping is the exception: its entries are templates
//! the runner fills per session, so reading `runtimes` (or any path under
//! it) answers them as stored. They expand only through the active-runtime
//! view, `runtime.<field>`.
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
    /// Check references without an environment: variables expand to
    /// nothing, and `runtime.*` resolves against the runtime-mapping entry
    /// at this index when given (validation checks each candidate active
    /// runtime in turn), to nothing otherwise.
    Check(Option<usize>),
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
    /// every string in it expanded — except the stored runtime mapping,
    /// which is answered unexpanded wherever it appears.
    pub(crate) fn get(&self, path: Option<&str>) -> Result<Value, ConfigError> {
        let Some(path) = path else {
            return self.expand_value(self.root, "");
        };
        if is_stored_mapping(path) {
            return match self.walk(&path.split('.').collect::<Vec<_>>(), path)? {
                Located::Value { value, .. } => Ok(value.clone()),
                Located::Derived(text) => Ok(Value::String(text)),
            };
        }
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
                    let child = if is_stored_mapping(&child_path) {
                        child.clone()
                    } else {
                        self.expand_value(child, &child_path)?
                    };
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
            Mode::Check(_) => Ok(String::new()),
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
        if let Mode::Check(active) = self.mode
            && let Some(field) = path.strip_prefix("runtime.")
        {
            return self.check_runtime_reference(path, field, active, stack);
        }
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

    /// In validation, a `{runtime.<field>}` reference follows the candidate
    /// active entry's field, so a cycle through it is found; a field the
    /// entry lacks, or no candidate, expands to nothing.
    fn check_runtime_reference(
        &self,
        path: &str,
        field: &str,
        active: Option<usize>,
        stack: &mut Vec<String>,
    ) -> Result<String, ConfigError> {
        if field == SESSION_ID {
            return Ok(String::new());
        }
        if !is_runtime_field(field) {
            return Err(missing(path, field, "runtime"));
        }
        let value = active.and_then(|index| {
            self.root
                .get("runtimes")
                .and_then(|r| r.get(index))
                .and_then(|entry| entry.get(field))
        });
        match (active, value) {
            (Some(index), Some(value)) => {
                self.follow(format!("runtimes.{index}.{field}"), value, stack)?;
                Ok(String::new())
            }
            _ => Ok(String::new()),
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
            Mode::Check(_) => {
                return match fields {
                    [] => Err(ConfigError::NotScalar {
                        path: path.to_owned(),
                    }),
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
                Mode::Check(_) => "",
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

/// Whether `path` addresses the stored runtime mapping or a part of it.
fn is_stored_mapping(path: &str) -> bool {
    path == "runtimes" || path.starts_with("runtimes.")
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

    const BASE: &str = "/srv";
    const USER: &str = "dev";
    const HOST_SESSION: &str = "h-1";
    const CLI_SESSION: &str = "c-7";

    fn doc() -> Value {
        serde_yaml_ng::from_str(&format!(
            "base: {BASE}\n\
             path: '{{base}}/app'\n\
             deep: '{{path}}/bin ${{USER_NAME}}'\n\
             json: '{{\"k\": 1}} {{not a ref}}'\n\
             n: 3\n\
             loop_a: '{{loop_b}}'\n\
             loop_b: '{{loop_a}}'\n\
             self: 'x{{self}}'\n\
             tree: {{a: 1}}\n\
             runtimes:\n\
             \x20 - {{name: host, session_id_env: HOST_ID, conversation: 'host {{runtime.session_id}}'}}\n\
             \x20 - {{name: cli, session_id_env: CLI_ID, conversation: 'cli {{runtime.session_id}}'}}\n"
        ))
        .unwrap()
    }

    fn get(doc: &Value, env: &Env, path: &str) -> Result<Value, ConfigError> {
        let runtimes = _runtime::entries(doc.get("runtimes"));
        Interpolator::new(doc, &runtimes, Mode::Expand(env)).get(Some(path))
    }

    fn text(doc: &Value, env: &Env, path: &str) -> String {
        get(doc, env, path).unwrap().as_str().unwrap().to_owned()
    }

    fn check(doc: &Value, text: &str, origin: &str) -> Result<String, ConfigError> {
        check_as(doc, None, text, origin)
    }

    fn check_as(
        doc: &Value,
        active: Option<usize>,
        text: &str,
        origin: &str,
    ) -> Result<String, ConfigError> {
        let runtimes = _runtime::entries(doc.get("runtimes"));
        Interpolator::new(doc, &runtimes, Mode::Check(active)).expand_text(text, Some(origin))
    }

    #[test]
    fn references_expand_recursively_and_from_the_environment() {
        let env = Env::from_pairs([("USER_NAME", USER)]);
        let doc = doc();
        assert_eq!(text(&doc, &env, "deep"), format!("{BASE}/app/bin {USER}"));
        let literal = doc.get("json").and_then(Value::as_str).unwrap();
        assert_eq!(text(&doc, &env, "json"), literal, "non-references stay");
    }

    #[test]
    fn an_unset_variable_is_refused() {
        let err = get(&doc(), &Env::default(), "deep").unwrap_err();
        assert!(matches!(err, ConfigError::UnsetVariable { name } if name == "USER_NAME"));
    }

    #[test]
    fn cycles_and_self_references_are_refused_with_their_chain() {
        let doc = doc();
        let err = get(&doc, &Env::default(), "loop_a").unwrap_err();
        assert!(
            matches!(err, ConfigError::Cycle { chain } if chain == ["loop_a", "loop_b", "loop_a"])
        );
        let err = get(&doc, &Env::default(), "self").unwrap_err();
        assert!(matches!(err, ConfigError::Cycle { chain } if chain == ["self", "self"]));
    }

    #[test]
    fn the_active_runtime_supplies_its_fields_and_session_id() {
        let doc = doc();
        let cli = Env::from_pairs([("CLI_ID", CLI_SESSION)]);
        assert_eq!(
            text(&doc, &cli, "runtime.conversation"),
            format!("cli {CLI_SESSION}")
        );
        assert_eq!(text(&doc, &cli, "runtime.session_id"), CLI_SESSION);
        let both = Env::from_pairs([("CLI_ID", CLI_SESSION), ("HOST_ID", HOST_SESSION)]);
        assert_eq!(
            text(&doc, &both, "runtime.conversation"),
            format!("host {HOST_SESSION}")
        );
        let err = get(&doc, &Env::default(), "runtime.conversation").unwrap_err();
        assert!(matches!(err, ConfigError::NoActiveRuntime));
        let err = get(&doc, &both, "runtime.start").unwrap_err();
        assert!(matches!(err, ConfigError::MissingRuntimeField { .. }));
    }

    #[test]
    fn a_missing_path_is_refused_and_containers_are_not_references() {
        let doc = doc();
        let err = get(&doc, &Env::default(), "tree.b").unwrap_err();
        assert!(matches!(err, ConfigError::MissingKey { segment, .. } if segment == "b"));
        assert!(matches!(
            check(&doc, "{tree}", "x"),
            Err(ConfigError::NotScalar { .. })
        ));
        assert!(check(&doc, "{n} ${ANY} {runtime.session_id}", "x").is_ok());
        assert!(matches!(
            check(&doc, "{runtime.bogus}", "x"),
            Err(ConfigError::MissingKey { .. })
        ));
        assert!(matches!(
            check(&doc, "{nope}", "x"),
            Err(ConfigError::MissingKey { .. })
        ));
    }

    #[test]
    fn validation_follows_runtime_references_into_the_candidate_runtime() {
        let doc: Value = serde_yaml_ng::from_str(
            "runtimes:\n\
             \x20 - {name: a, session_id_env: A, conversation: '{runtime.conversation}', start: '{runtime.send}', send: s}\n\
             \x20 - {name: b, session_id_env: B, send: '{runtime.start}', start: s}\n",
        )
        .unwrap();
        let err = check_as(
            &doc,
            Some(0),
            "{runtime.conversation}",
            "runtimes.0.conversation",
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Cycle { .. }));
        // Entry 0's start reads the active runtime's send; whichever runtime
        // is active, no reference leads back to where it started.
        for active in [Some(0), Some(1), None] {
            assert!(check_as(&doc, active, "{runtime.send}", "runtimes.0.start").is_ok());
        }
    }

    #[test]
    fn the_stored_runtime_mapping_reads_unexpanded_and_the_rest_expands() {
        let doc = doc();
        let stored = doc.get("runtimes").unwrap().clone();
        let none = Env::default();
        assert_eq!(get(&doc, &none, "runtimes").unwrap(), stored);
        assert_eq!(
            get(&doc, &none, "runtimes.1.conversation").unwrap(),
            stored[1]["conversation"]
        );
        let mut small = doc.clone();
        for key in ["loop_a", "loop_b", "self"] {
            small.as_mapping_mut().unwrap().remove(key);
        }
        let env = Env::from_pairs([("USER_NAME", USER)]);
        let runtimes = _runtime::entries(small.get("runtimes"));
        let whole = Interpolator::new(&small, &runtimes, Mode::Expand(&env))
            .get(None)
            .unwrap();
        assert_eq!(whole["runtimes"], stored);
        assert_eq!(whole["deep"], text(&small, &env, "deep").as_str());
        let cli = Env::from_pairs([("CLI_ID", CLI_SESSION)]);
        assert_ne!(
            get(&doc, &cli, "runtime.conversation").unwrap(),
            stored[1]["conversation"],
            "the active-runtime view expands"
        );
    }

    #[test]
    fn masking_marks_text_made_only_of_references() {
        assert_eq!(
            mask_references("{a} <${B}>", "r"),
            ("r <r>".to_owned(), false)
        );
        assert_eq!(mask_references(" {a} ", "r"), (" r ".to_owned(), true));
        assert_eq!(mask_references("plain", "r"), ("plain".to_owned(), false));
    }
}
