//! Configuration's part of generating a company's home
//! (`project/instrukt/spec/feature/configuration/company-homes`,
//! Generation): the decision whether a company's home is created, checked
//! out by contract or refused, the repository's name, and the skeleton's
//! project layer and ignore lines. Adoption itself is repository's.

use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use super::_error::ConfigError;
use super::_github_org::GITHUB_ORG_KEY;
use super::_identity::{COMPANY_NAME_KEY, CompanyName};
use super::_schema::PROJECT_SCHEMA_VERSION;
use crate::ids::{GitHubLogin, GitHubOrganisation, ProjectSlug};
use crate::interfaces::AccountKind;

/// What adding a company does, as configuration decides it from the facts
/// gathered for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Generation {
    /// The company's repository does not exist: create it in
    /// `organisation` as `repository`, with the skeleton, for the company
    /// named `name` with `admin` as its GitHub administrator.
    Create {
        organisation: GitHubOrganisation,
        repository: ProjectSlug,
        name: CompanyName,
        admin: GitHubLogin,
    },
    /// The company's repository exists: it is the company's by contract and
    /// is checked out as the home, nothing on GitHub changed.
    CheckOut {
        organisation: GitHubOrganisation,
        repository: ProjectSlug,
    },
}

/// A flag of `instrukt init --company` that creating a company's repository
/// needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationFlag {
    Name,
    Admin,
}

impl std::fmt::Display for CreationFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Name => "--name",
            Self::Admin => "--admin",
        })
    }
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
    #[error(
        "creating the company's repository needs `{flag}`, and its repository does not exist yet; give it and run the command again; nothing was created"
    )]
    FlagMissing { flag: CreationFlag },
    #[error(
        "`--admin {admin}` is {}; the company's GitHub administrator is a person's own GitHub account; nothing was created",
        described(*kind)
    )]
    AdminNotPersonal {
        admin: GitHubLogin,
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

/// Refuses adding a company whose home `home` exists: a home is never
/// adopted or overwritten. The first question of adding a company, asked
/// before anything else is read.
pub fn admit_home(home: &Path, home_exists: bool) -> Result<(), GenerationRefusal> {
    if home_exists {
        return Err(GenerationRefusal::HomeExists {
            home: home.to_path_buf(),
        });
    }
    Ok(())
}

/// What adding a company asks for: the company's repository
/// ([`company_repository`]), its home and whether that exists, the declared
/// `github.org`, and the name and administrator the caller gave.
#[derive(Debug, Clone, Copy)]
pub struct GenerationRequest<'a> {
    pub repository: &'a ProjectSlug,
    pub home: &'a Path,
    pub home_exists: bool,
    pub declared_org: Option<&'a str>,
    pub name: Option<&'a CompanyName>,
    pub admin: Option<&'a GitHubLogin>,
}

/// The facts outside configuration that the generation decision asks for,
/// each only when the decision needs it.
pub trait GenerationFacts {
    type Error: From<GenerationRefusal>;
    /// The kind of GitHub account `login` names.
    fn account_kind(&self, login: &str) -> Result<AccountKind, Self::Error>;
    /// Whether `repository` of `organisation` can be reached.
    fn reachable(
        &self,
        organisation: &GitHubOrganisation,
        repository: &ProjectSlug,
    ) -> Result<bool, Self::Error>;
}

/// Decides what adding a company does: refused when its home exists
/// ([`admit_home`]), when `github.org` is not declared, or when the account
/// it names is no organisation; checked out by contract when the repository
/// can be reached; otherwise created, which needs a name and an
/// administrator that is a person's own account.
pub fn decide<F: GenerationFacts>(
    request: &GenerationRequest<'_>,
    facts: &F,
) -> Result<Generation, F::Error> {
    admit_home(request.home, request.home_exists)?;
    let Some(declared) = request
        .declared_org
        .map(str::trim)
        .filter(|org| !org.is_empty())
    else {
        return Err(GenerationRefusal::NoOrganisation.into());
    };
    let not_an_organisation = |kind| GenerationRefusal::NotAnOrganisation {
        organisation: declared.to_owned(),
        kind,
    };
    let kind = facts.account_kind(declared)?;
    if kind != AccountKind::Organisation {
        return Err(not_an_organisation(kind).into());
    }
    let organisation = GitHubOrganisation::parse(declared)
        .map_err(|_| not_an_organisation(AccountKind::Absent))?;
    let repository = request.repository.clone();
    if facts.reachable(&organisation, &repository)? {
        return Ok(Generation::CheckOut {
            organisation,
            repository,
        });
    }
    let missing = |flag| GenerationRefusal::FlagMissing { flag };
    let name = request.name.ok_or(missing(CreationFlag::Name))?.clone();
    let admin = request.admin.ok_or(missing(CreationFlag::Admin))?.clone();
    let kind = facts.account_kind(admin.as_str())?;
    if kind != AccountKind::Personal {
        return Err(GenerationRefusal::AdminNotPersonal { admin, kind }.into());
    }
    Ok(Generation::Create {
        organisation,
        repository,
        name,
        admin,
    })
}

/// The company skeleton's project layer: the project [`company_repository`]
/// names, declared a company's repository with the company identity of
/// `key` and `name`, and the module and review switches every company's
/// repository starts with.
pub fn skeleton_layer(key: &ProjectSlug, name: &CompanyName) -> Result<Value, ConfigError> {
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
    set(COMPANY_NAME_KEY, Value::String(name.as_str().to_owned()));
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
pub fn write_skeleton(
    root: &Path,
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<(), ConfigError> {
    super::_company_document::ignore_local_material(root)?;
    super::_layer::write(
        &root.join(super::CONFIG_FILE_NAME),
        &[],
        &skeleton_layer(key, name)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AccountKind, CompanyName, CreationFlag, Generation, GenerationFacts, GenerationRefusal,
        GenerationRequest, GitHubLogin, GitHubOrganisation, Path, PathBuf, ProjectSlug,
        company_repository, decide, skeleton_layer,
    };

    fn acme() -> ProjectSlug {
        ProjectSlug::parse("acme").unwrap()
    }

    /// GitHub as the decision finds it: the declared owner's kind, the
    /// administrator's kind, and whether the repository is reached.
    struct Found {
        owner: AccountKind,
        admin: AccountKind,
        reachable: bool,
    }

    const ADMIN: &str = "acme-admin";

    impl GenerationFacts for Found {
        type Error = GenerationRefusal;

        fn account_kind(&self, login: &str) -> Result<AccountKind, GenerationRefusal> {
            Ok(if login == ADMIN {
                self.admin
            } else {
                self.owner
            })
        }

        fn reachable(
            &self,
            _organisation: &GitHubOrganisation,
            _repository: &ProjectSlug,
        ) -> Result<bool, GenerationRefusal> {
            Ok(self.reachable)
        }
    }

    /// GitHub holding the organisation `homes` and the personal account of
    /// the administrator, without the company's repository.
    const FRESH: Found = Found {
        owner: AccountKind::Organisation,
        admin: AccountKind::Personal,
        reachable: false,
    };

    /// What adding Acme decides with `found`, its home existing or not, the
    /// owner declared as `org`, and the flags given or not.
    fn decided(
        found: &Found,
        home_exists: bool,
        org: Option<&str>,
        name: Option<&CompanyName>,
        admin: Option<&GitHubLogin>,
    ) -> Result<Generation, GenerationRefusal> {
        let repository = company_repository(&acme()).unwrap();
        let request = GenerationRequest {
            repository: &repository,
            home: Path::new("/h/companies/acme"),
            home_exists,
            declared_org: org,
            name,
            admin,
        };
        decide(&request, found)
    }

    fn flags() -> (CompanyName, GitHubLogin) {
        (
            CompanyName::parse("Acme").unwrap(),
            GitHubLogin::parse(ADMIN).unwrap(),
        )
    }

    #[test]
    fn an_unreachable_repository_is_created_for_the_company_and_administrator_given() {
        let (name, admin) = flags();
        assert_eq!(
            decided(&FRESH, false, Some("homes"), Some(&name), Some(&admin)),
            Ok(Generation::Create {
                organisation: GitHubOrganisation::parse("homes").unwrap(),
                repository: company_repository(&acme()).unwrap(),
                name,
                admin,
            })
        );
    }

    #[test]
    fn a_reachable_repository_is_checked_out_by_contract_whatever_the_flags_name() {
        let (name, admin) = flags();
        // The administrator is no person's account: a check-out never asks.
        let found = Found {
            admin: AccountKind::Organisation,
            reachable: true,
            ..FRESH
        };
        assert_eq!(
            decided(&found, false, Some("homes"), Some(&name), Some(&admin)),
            Ok(Generation::CheckOut {
                organisation: GitHubOrganisation::parse("homes").unwrap(),
                repository: company_repository(&acme()).unwrap(),
            })
        );
    }

    #[test]
    fn an_existing_home_is_refused_before_anything_else() {
        assert_eq!(
            decided(&FRESH, true, None, None, None),
            Err(GenerationRefusal::HomeExists {
                home: PathBuf::from("/h/companies/acme"),
            })
        );
    }

    #[test]
    fn no_organisation_refuses_naming_the_prerequisite() {
        assert_eq!(
            decided(&FRESH, false, None, None, None),
            Err(GenerationRefusal::NoOrganisation)
        );
    }

    #[test]
    fn an_owner_that_is_no_organisation_refuses_naming_it() {
        let owner = "ana";
        let found = Found {
            owner: AccountKind::Personal,
            ..FRESH
        };
        let refused = decided(&found, false, Some(owner), None, None);
        assert_eq!(
            refused,
            Err(GenerationRefusal::NotAnOrganisation {
                organisation: owner.to_owned(),
                kind: AccountKind::Personal,
            })
        );
        insta::assert_snapshot!("not_an_organisation", refused.unwrap_err().to_string());
    }

    #[test]
    fn a_creation_without_a_name_is_refused() {
        let (_, admin) = flags();
        assert_eq!(
            decided(&FRESH, false, Some("homes"), None, Some(&admin)),
            Err(GenerationRefusal::FlagMissing {
                flag: CreationFlag::Name
            })
        );
    }

    #[test]
    fn a_creation_without_an_administrator_is_refused() {
        let (name, _) = flags();
        assert_eq!(
            decided(&FRESH, false, Some("homes"), Some(&name), None),
            Err(GenerationRefusal::FlagMissing {
                flag: CreationFlag::Admin
            })
        );
    }

    #[test]
    fn an_administrator_that_is_no_persons_own_account_is_refused() {
        let (name, admin) = flags();
        let found = Found {
            admin: AccountKind::Organisation,
            ..FRESH
        };
        assert_eq!(
            decided(&found, false, Some("homes"), Some(&name), Some(&admin)),
            Err(GenerationRefusal::AdminNotPersonal {
                admin,
                kind: AccountKind::Organisation,
            })
        );
    }

    #[test]
    fn the_skeletons_layer_is_a_company_repository_with_todos_off_and_review_on() {
        let name = CompanyName::parse("Acme Inc").unwrap();
        let layer = skeleton_layer(&acme(), &name).unwrap();
        let bytes = serde_yaml_ng::to_string(&layer).unwrap();
        assert!(matches!(
            super::super::classify_root(Some(bytes.as_bytes())),
            super::super::RootDocument::ProjectLayer {
                company_repository: true,
                ..
            }
        ));
        assert_eq!(
            super::super::company_identity(Some(bytes.as_bytes())),
            Ok(super::super::CompanyIdentity { key: acme(), name })
        );
    }
}
