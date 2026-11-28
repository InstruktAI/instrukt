//! The company identity a company repository's committed project layer
//! declares (`project/instrukt/spec/domain/configuration`, The company
//! identity): the key its `project` carries after `company-`, and the name
//! people write the company by, `company_name`.

use serde_yaml_ng::Value;

use super::_error::FindingKind;
use super::_schema::KeyFinding;
use crate::ids::ProjectSlug;

/// The project-layer key that holds the name people write the company by.
pub const COMPANY_NAME_KEY: &str = "company_name";

/// The prefix generation gives a company's repository (`company-<key>`).
const REPOSITORY_PREFIX: &str = "company-";

/// The name people write a company by: one line of text that is not blank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyName(String);

impl CompanyName {
    /// `text` as a company's name, or why it is none.
    pub fn parse(text: &str) -> Result<Self, CompanyNameError> {
        if text.contains(['\n', '\r']) {
            Err(CompanyNameError::MoreThanOneLine)
        } else if text.trim().is_empty() {
            Err(CompanyNameError::Blank)
        } else {
            Ok(Self(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for CompanyName {
    type Err = CompanyNameError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl std::fmt::Display for CompanyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl serde::Serialize for CompanyName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for CompanyName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// Why a text is no company's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CompanyNameError {
    #[error("a company's name is the one line people write the company by; this one is blank")]
    Blank,
    #[error(
        "a company's name is the one line people write the company by; this one holds a line break"
    )]
    MoreThanOneLine,
}

/// The company a company repository belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyIdentity {
    /// The company's key: the repository's `project` after `company-`.
    pub key: ProjectSlug,
    /// The name people write the company by.
    pub name: CompanyName,
}

/// Why a project-layer document declares no company identity. Each names
/// the key and its repair, an `instrukt config patch` of the project layer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityFinding {
    #[error(
        "the repository has no project layer `instrukt.yaml` declaring its company identity; add one with `project: company-<key>` and `company_name` (`instrukt config patch`)"
    )]
    NoProjectLayer,
    #[error("the project layer `instrukt.yaml` cannot be read as YAML ({0})")]
    Unreadable(String),
    #[error(
        "`project` does not have the form `company-<key>` with a key of a company key's grammar, so the repository declares no company identity; set it with `instrukt config patch`"
    )]
    Key,
    #[error(
        "`company_name`, the name people write the company by, is missing, so the repository declares no company identity; set it with `instrukt config patch`"
    )]
    NameMissing,
    #[error("`company_name` is invalid: {0}; set it with `instrukt config patch`")]
    Name(CompanyNameError),
    #[error(
        "`company_name` is not text; set the name people write the company by with `instrukt config patch`"
    )]
    NameNotText,
}

/// The company identity the project-layer document `document` (its bytes,
/// `None` when the repository has none) declares, or the finding that it
/// declares none. The only source of the key and the name a company plugin
/// rendering takes.
pub fn company_identity(document: Option<&[u8]>) -> Result<CompanyIdentity, IdentityFinding> {
    let bytes = document.ok_or(IdentityFinding::NoProjectLayer)?;
    let value: Value = serde_yaml_ng::from_slice(bytes)
        .map_err(|error| IdentityFinding::Unreadable(error.to_string()))?;
    identity_of(&value)
}

/// The company identity of a parsed project-layer document.
pub(crate) fn identity_of(value: &Value) -> Result<CompanyIdentity, IdentityFinding> {
    let key = value
        .get("project")
        .and_then(Value::as_str)
        .and_then(|project| project.strip_prefix(REPOSITORY_PREFIX))
        .and_then(|key| ProjectSlug::parse(key).ok())
        .ok_or(IdentityFinding::Key)?;
    let name = match value.get(COMPANY_NAME_KEY) {
        None | Some(Value::Null) => return Err(IdentityFinding::NameMissing),
        Some(Value::String(text)) => CompanyName::parse(text).map_err(IdentityFinding::Name)?,
        Some(_) => return Err(IdentityFinding::NameNotText),
    };
    Ok(CompanyIdentity { key, name })
}

/// The project layer `value`'s findings about the company identity: in a
/// company repository, the absence of one, against `modules.events` so it
/// withholds the home's event responses alone; elsewhere, a `company_name`
/// with no company repository to name.
pub(super) fn findings(value: &Value) -> Vec<KeyFinding> {
    let company_repository = value.get("company_repository").and_then(Value::as_bool) == Some(true);
    if company_repository {
        return match identity_of(value) {
            Ok(_) => Vec::new(),
            Err(finding) => vec![KeyFinding::new(
                "modules.events",
                FindingKind::NoCompanyIdentity,
                format!(
                    "a company repository declares its company identity, and event responses are withheld until it does: {finding}"
                ),
            )],
        };
    }
    match value.get(COMPANY_NAME_KEY) {
        None | Some(Value::Null) => Vec::new(),
        Some(_) => vec![KeyFinding::new(
            COMPANY_NAME_KEY,
            FindingKind::InvalidValue,
            "is valid only beside `company_repository: true`; remove it or declare the repository a company's",
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompanyIdentity, CompanyName, CompanyNameError, IdentityFinding, company_identity, findings,
    };
    use crate::config::FindingKind;
    use crate::ids::ProjectSlug;

    fn value(text: &str) -> serde_yaml_ng::Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    #[test]
    fn a_company_layer_declares_its_key_and_name() {
        assert_eq!(
            company_identity(Some(
                b"schema_version: 6\nproject: company-acme\ncompany_repository: true\ncompany_name: Acme Inc\n"
            )),
            Ok(CompanyIdentity {
                key: ProjectSlug::parse("acme").unwrap(),
                name: CompanyName::parse("Acme Inc").unwrap(),
            })
        );
    }

    #[test]
    fn no_layer_a_project_without_the_prefix_or_no_name_declares_no_identity() {
        assert_eq!(company_identity(None), Err(IdentityFinding::NoProjectLayer));
        assert_eq!(
            company_identity(Some(b"project: acme\ncompany_name: Acme\n")),
            Err(IdentityFinding::Key)
        );
        assert_eq!(
            company_identity(Some(b"project: company-\ncompany_name: Acme\n")),
            Err(IdentityFinding::Key)
        );
        assert_eq!(
            company_identity(Some(b"project: company-acme\n")),
            Err(IdentityFinding::NameMissing)
        );
        assert_eq!(
            company_identity(Some(b"project: company-acme\ncompany_name: \"  \"\n")),
            Err(IdentityFinding::Name(CompanyNameError::Blank))
        );
    }

    #[test]
    fn a_company_repository_without_identity_is_a_finding_against_events() {
        let found = findings(&value(
            "schema_version: 6\nproject: company-acme\ncompany_repository: true\n",
        ));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, "modules.events");
        assert_eq!(found[0].kind, FindingKind::NoCompanyIdentity);
        assert!(
            findings(&value(
                "project: company-acme\ncompany_repository: true\ncompany_name: Acme\n"
            ))
            .is_empty()
        );
    }

    #[test]
    fn a_company_name_without_a_company_repository_is_a_finding_naming_it() {
        let found = findings(&value("project: demo\ncompany_name: Acme\n"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, "company_name");
        assert!(findings(&value("project: demo\n")).is_empty());
    }

    #[test]
    fn company_names_are_one_line_that_is_not_blank() {
        assert_eq!(CompanyName::parse("  "), Err(CompanyNameError::Blank));
        assert_eq!(
            CompanyName::parse("Acme\nInc"),
            Err(CompanyNameError::MoreThanOneLine)
        );
    }
}
