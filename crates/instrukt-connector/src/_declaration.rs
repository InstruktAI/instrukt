//! A kind's declaration as data, and the values one company declared.
use std::collections::BTreeMap;

/// The fields a kind's `connectors.<kind>` entry takes, as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declaration {
    /// The kind's key under `connectors`.
    pub key: &'static str,
    /// The most characters one posted part of an answer holds for this kind.
    pub message_chars: usize,
    pub fields: &'static [Field],
}

/// One field of a declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub rule: Rule,
}

/// What a field's value must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// An exact `${NAME}` reference into the home's secrets file.
    SecretReference { required: bool },
    /// A positive whole number, `default` when omitted; `meaning` says what
    /// it is, as its finding and editor schema state it.
    PositiveCount { default: u64, meaning: &'static str },
}

/// The values one company declared for a kind, by field name; references
/// stay unresolved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    Count(u64),
}

impl Declared {
    /// Declares `name` as the text `value`.
    pub fn insert_text(&mut self, name: &str, value: &str) {
        self.values
            .insert(name.to_owned(), Value::Text(value.to_owned()));
    }

    /// Declares `name` as the count `value`.
    pub fn insert_count(&mut self, name: &str, value: u64) {
        self.values.insert(name.to_owned(), Value::Count(value));
    }

    /// The text declared for `name`, if it was declared as text.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.values.get(name)? {
            Value::Text(value) => Some(value),
            Value::Count(_) => None,
        }
    }

    /// The count declared for `name`, if it was declared as a count.
    pub fn count(&self, name: &str) -> Option<u64> {
        match self.values.get(name)? {
            Value::Count(value) => Some(*value),
            Value::Text(_) => None,
        }
    }
}
