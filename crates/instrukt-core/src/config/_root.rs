//! What a repository root's `instrukt.yaml` is, decided from its bytes
//! alone (`project/instrukt/spec/feature/publication/agent-entrypoints`,
//! "What makes a repository a company home").
//!
//! The caller reads the bytes from the view its operation works on — the
//! index a commit is made from, or the working tree — so the decision is
//! the view's and never the physical root's.

use serde_yaml_ng::Value;

use crate::ids::ProjectSlug;

/// What a repository root holds as `instrukt.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootDocument {
    /// No root `instrukt.yaml`.
    Absent,
    /// A project layer, with the slug it declares; `None` when it declares
    /// none.
    ProjectLayer(Option<ProjectSlug>),
    /// A company's settings document, naming the company.
    CompanySettings(ProjectSlug),
    /// A document that cannot be read: no YAML mapping, or a name that is
    /// not a valid name. Why, as a sentence.
    Unreadable(String),
}

/// Classifies the root document whose bytes are `bytes` (`None` when the
/// root holds none): a mapping declaring `company` is a company's settings
/// document, any other mapping the project layer.
pub fn classify_root(bytes: Option<&[u8]>) -> RootDocument {
    let Some(bytes) = bytes else {
        return RootDocument::Absent;
    };
    let value: Value = match serde_yaml_ng::from_slice(bytes) {
        Ok(value) => value,
        Err(error) => return RootDocument::Unreadable(format!("it is not YAML: {error}")),
    };
    let Value::Mapping(map) = value else {
        return RootDocument::Unreadable("it is not a YAML mapping".to_owned());
    };
    let name = |key: &str| -> Result<Option<ProjectSlug>, String> {
        match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => ProjectSlug::parse(text)
                .map(Some)
                .map_err(|error| format!("its `{key}` is not a valid name: {error}")),
            Some(_) => Err(format!("its `{key}` is not a name")),
        }
    };
    match name("company") {
        Ok(Some(company)) => RootDocument::CompanySettings(company),
        Ok(None) => match name("project") {
            Ok(project) => RootDocument::ProjectLayer(project),
            Err(reason) => RootDocument::Unreadable(reason),
        },
        Err(reason) => RootDocument::Unreadable(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_declaring_company_is_a_companys_settings() {
        assert_eq!(
            classify_root(Some(b"schema_version: 6\ncompany: acme\n")),
            RootDocument::CompanySettings(ProjectSlug::parse("acme").unwrap())
        );
    }

    #[test]
    fn any_other_mapping_is_the_project_layer() {
        assert_eq!(
            classify_root(Some(b"schema_version: 6\nproject: demo\n")),
            RootDocument::ProjectLayer(Some(ProjectSlug::parse("demo").unwrap()))
        );
    }

    #[test]
    fn no_document_is_absent() {
        assert_eq!(classify_root(None), RootDocument::Absent);
    }

    #[test]
    fn bytes_that_are_no_mapping_are_unreadable() {
        assert!(matches!(
            classify_root(Some(b"[: no")),
            RootDocument::Unreadable(_)
        ));
    }
}
