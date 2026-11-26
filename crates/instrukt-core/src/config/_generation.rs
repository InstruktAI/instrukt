//! Configuration's part of generating a company's home
//! (`project/instrukt/spec/feature/configuration/company-homes`,
//! Generation): the decision whether a company's home is created, checked
//! out by contract or refused, the repository's name, and the skeleton's
//! project layer and ignore lines. Adoption itself is repository's.

use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use super::_error::ConfigError;
use super::_github_org::GITHUB_ORG_KEY;
use super::_schema::PROJECT_SCHEMA_VERSION;
use crate::ids::ProjectSlug;
use crate::interfaces::AccountKind;

/// What adding a company does, as configuration decides it from the facts
/// gathered for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Generation {
    /// The company's repository does not exist: create it in
    /// `organisation` as `repository`, with the skeleton.
    Create {
        organisation: String,
        repository: String,
    },
    /// The company's repository exists: it is the company's by contract and
    /// is checked out as the home, nothing on GitHub changed.
    CheckOut {
        organisation: String,
        repository: String,
    },
}

/// Why a company is not added. Nothing is created.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenerationRefusal {
    #[error(
        "the company's home {} exists, so nothing was changed in it or on GitHub; a home is never adopted or overwritten. If an earlier `instrukt init --company` was cut short after moving it into place, run `instrukt sync` in it",
        home.display()
    )]
    HomeExists { home: PathBuf },
    #[error(
        "{GITHUB_ORG_KEY} is not declared; adding a company needs the GitHub organisation that owns this computer's company repositories, so declare it with `instrukt config patch --global`; no repository and no home was created"
    )]
    NoOrganisation,
    #[error(
        "{GITHUB_ORG_KEY} names `{organisation}`, {}; adding a company needs the GitHub organisation that owns this computer's company repositories, so give {GITHUB_ORG_KEY} an organisation's login; no repository and no home was created",
        described(*kind)
    )]
    NotAnOrganisation {
        organisation: String,
        kind: AccountKind,
    },
}

fn described(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Organisation => "a GitHub organisation",
        AccountKind::Personal => "a personal GitHub account",
        AccountKind::Absent => "no GitHub account",
    }
}

/// The name of the company `key`'s repository, which is also the project
/// its project layer names: `company-<key>`.
pub fn company_repository(key: &ProjectSlug) -> Result<ProjectSlug, crate::ids::IdError> {
    ProjectSlug::parse(&format!("company-{key}"))
}

/// Decides what adding the company whose repository is `repository`
/// ([`company_repository`]) does: refused when its home exists, when
/// `github.org` is not declared, or when the account it names (`org_kind`)
/// is no organisation; otherwise checked out by contract when the
/// repository is `reachable`, created when it is not.
pub fn decide(
    repository: &ProjectSlug,
    home: &Path,
    home_exists: bool,
    declared_org: Option<&str>,
    org_kind: Option<AccountKind>,
    reachable: bool,
) -> Result<Generation, GenerationRefusal> {
    if home_exists {
        return Err(GenerationRefusal::HomeExists {
            home: home.to_path_buf(),
        });
    }
    let Some(organisation) = declared_org.map(str::trim).filter(|org| !org.is_empty()) else {
        return Err(GenerationRefusal::NoOrganisation);
    };
    match org_kind {
        Some(AccountKind::Organisation) => {}
        kind => {
            return Err(GenerationRefusal::NotAnOrganisation {
                organisation: organisation.to_owned(),
                kind: kind.unwrap_or(AccountKind::Absent),
            });
        }
    }
    let organisation = organisation.to_owned();
    let repository = repository.to_string();
    Ok(if reachable {
        Generation::CheckOut {
            organisation,
            repository,
        }
    } else {
        Generation::Create {
            organisation,
            repository,
        }
    })
}

/// The company skeleton's project layer: the project [`company_repository`]
/// names, declared a company's repository, with the module and review
/// switches every company's repository starts with.
pub fn skeleton_layer(key: &ProjectSlug) -> Result<Value, ConfigError> {
    let mut layer = Mapping::new();
    let mut set = |key: &str, value: Value| {
        layer.insert(Value::String(key.to_owned()), value);
    };
    set(
        "schema_version",
        Value::Number(PROJECT_SCHEMA_VERSION.into()),
    );
    let project = company_repository(key).map_err(|error| ConfigError::InvalidValue {
        key: "project".to_owned(),
        message: error.to_string(),
    })?;
    set("project", Value::String(project.to_string()));
    set("company_repository", Value::Bool(true));
    let mut modules = Mapping::new();
    modules.insert(Value::String("todos".to_owned()), Value::Bool(false));
    set("modules", Value::Mapping(modules));
    let mut review = Mapping::new();
    review.insert(Value::String("automatic".to_owned()), Value::Bool(true));
    set("review", Value::Mapping(review));
    Ok(Value::Mapping(layer))
}

/// Writes the company skeleton's project layer and ignore lines into the
/// working tree at `root`: `instrukt.yaml` as [`skeleton_layer`], through
/// the project layer's own writer, and the company's ignore lines in
/// `.gitignore`.
pub fn write_skeleton(root: &Path, key: &ProjectSlug) -> Result<(), ConfigError> {
    super::_company_document::ignore_local_material(root)?;
    super::_layer::write(
        &root.join(super::CONFIG_FILE_NAME),
        &[],
        &skeleton_layer(key)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AccountKind, Generation, GenerationRefusal, PROJECT_SCHEMA_VERSION, Path, PathBuf,
        ProjectSlug, company_repository, decide, skeleton_layer,
    };

    fn acme() -> ProjectSlug {
        ProjectSlug::parse("acme").unwrap()
    }

    fn decided(
        home_exists: bool,
        org: Option<&str>,
        kind: Option<AccountKind>,
        reachable: bool,
    ) -> Result<Generation, GenerationRefusal> {
        decide(
            &company_repository(&acme()).unwrap(),
            Path::new("/h/companies/acme"),
            home_exists,
            org,
            kind,
            reachable,
        )
    }

    #[test]
    fn an_unreachable_repository_is_created_and_a_reachable_one_checked_out_by_contract() {
        let org = Some(AccountKind::Organisation);
        assert_eq!(
            decided(false, Some("homes"), org, false),
            Ok(Generation::Create {
                organisation: "homes".to_owned(),
                repository: "company-acme".to_owned(),
            })
        );
        assert_eq!(
            decided(false, Some("homes"), org, true),
            Ok(Generation::CheckOut {
                organisation: "homes".to_owned(),
                repository: "company-acme".to_owned(),
            })
        );
    }

    #[test]
    fn an_existing_home_is_refused_before_anything_else() {
        assert_eq!(
            decided(true, None, None, true),
            Err(GenerationRefusal::HomeExists {
                home: PathBuf::from("/h/companies/acme"),
            })
        );
    }

    #[test]
    fn no_organisation_or_one_that_is_none_refuses_naming_the_prerequisite() {
        for (org, kind) in [(None, None), (Some("  "), None)] {
            assert_eq!(
                decided(false, org, kind, false),
                Err(GenerationRefusal::NoOrganisation)
            );
        }
        let personal = decided(false, Some("ana"), Some(AccountKind::Personal), false);
        assert_eq!(
            personal,
            Err(GenerationRefusal::NotAnOrganisation {
                organisation: "ana".to_owned(),
                kind: AccountKind::Personal,
            })
        );
        insta::assert_snapshot!("not_an_organisation", personal.unwrap_err().to_string());
    }

    #[test]
    fn the_skeletons_layer_is_a_company_repository_with_todos_off_and_review_on() {
        let layer = skeleton_layer(&acme()).unwrap();
        assert_eq!(
            serde_yaml_ng::to_string(&layer).unwrap(),
            format!(
                "schema_version: {PROJECT_SCHEMA_VERSION}\nproject: company-acme\ncompany_repository: true\nmodules:\n  todos: false\nreview:\n  automatic: true\n"
            )
        );
        assert!(matches!(
            super::super::classify_root(Some(serde_yaml_ng::to_string(&layer).unwrap().as_bytes())),
            super::super::RootDocument::ProjectLayer {
                company_repository: true,
                ..
            }
        ));
    }
}
