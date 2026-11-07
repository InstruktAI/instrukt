//! Validated event type selection, shared by docs and runner consumers.

use serde::{Deserialize, Serialize};

/// An exact name, a whole-segment prefix ending in `.*`, or `*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TypePattern(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "malformed event type pattern `{0}`: use an exact name, a whole-segment prefix followed by `.*`, or `*`"
)]
pub struct TypePatternError(String);

/// A dotted name contains nonempty segments, without whitespace or wildcards.
pub fn is_type_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|segment| {
            !segment.is_empty() && !segment.contains('*') && !segment.contains(char::is_whitespace)
        })
}

impl TypePattern {
    pub fn parse(text: &str) -> Result<Self, TypePatternError> {
        if text == "*" || is_type_name(text.strip_suffix(".*").unwrap_or(text)) {
            Ok(Self(text.to_owned()))
        } else {
            Err(TypePatternError(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn matches(&self, name: &str) -> bool {
        if !is_type_name(name) {
            return false;
        }
        if self.0 == "*" {
            return true;
        }
        match self.0.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix) && name.len() > prefix.len(),
            None => self.0 == name,
        }
    }
}

impl TryFrom<String> for TypePattern {
    type Error = TypePatternError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<TypePattern> for String {
    fn from(value: TypePattern) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::TypePattern;

    #[test]
    fn a_prefix_selects_descendants() {
        assert!(
            TypePattern::parse("runner.*")
                .unwrap()
                .matches("runner.ci.completed")
        );
    }

    #[test]
    fn a_prefix_does_not_select_itself() {
        assert!(!TypePattern::parse("runner.*").unwrap().matches("runner"));
    }

    #[test]
    fn a_prefix_does_not_select_a_partial_segment() {
        assert!(
            !TypePattern::parse("run.*")
                .unwrap()
                .matches("runner.completed")
        );
    }

    #[test]
    fn embedded_wildcards_are_refused() {
        assert!(TypePattern::parse("runner.ci*").is_err());
    }

    #[test]
    fn empty_segments_are_refused() {
        assert!(TypePattern::parse("runner..*").is_err());
    }

    #[test]
    fn whitespace_is_refused() {
        assert!(TypePattern::parse("runner. completed").is_err());
    }

    #[test]
    fn an_exact_pattern_selects_only_its_name() {
        assert!(
            !TypePattern::parse("runner.ci")
                .unwrap()
                .matches("runner.ci.completed")
        );
    }

    #[test]
    fn the_wildcard_selects_external_types() {
        assert!(TypePattern::parse("*").unwrap().matches("erp.invoice_paid"));
    }
}

/// Whether undelivered facts may be replaced by a newer state of one subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryKind {
    Occurrence,
    State,
}
