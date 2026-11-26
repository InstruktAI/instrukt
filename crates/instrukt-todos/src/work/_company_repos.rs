//! Work's GitHub boundary for company repositories
//! (`project/instrukt/spec/feature/work/github-apps`, Company repositories):
//! what kind of account a login names, read from GitHub's public account
//! record without authentication, and the company repository's creation,
//! administrator and default branch, under the project identity's
//! installation on the owning organisation.

use instrukt_core::config::Configuration;
use instrukt_core::github::{AppError, Operation};
use instrukt_core::ids::{GitHubLogin, GitHubOrganisation, ProjectSlug};
use instrukt_core::interfaces::{
    AccountKind, CompanyRepositories, CompanyRepositoryError, Creation, GitHubAccounts,
    InterfaceError,
};
use jiff::Timestamp;
use serde_json::{Value, json};

use super::_apps::{Answer, Apps, Clock, GitHubAccess, Method, agent, api_base};

fn unanswered(message: impl Into<String>) -> InterfaceError {
    InterfaceError {
        module: "todos",
        message: message.into(),
    }
}

/// The account kind GitHub's public record of a login says: `type`
/// `Organization` or `User`; any other type is no answer this boundary
/// gives.
fn kind_of(login: &str, record: &Value) -> Result<AccountKind, InterfaceError> {
    match record["type"].as_str() {
        Some("Organization") => Ok(AccountKind::Organisation),
        Some("User") => Ok(AccountKind::Personal),
        Some(other) => Err(unanswered(format!(
            "`{login}` is a GitHub account of type {other}, neither an organisation nor a person"
        ))),
        None => Err(unanswered(format!(
            "GitHub's record of `{login}` names no account type"
        ))),
    }
}

impl GitHubAccounts for GitHubAccess {
    fn account_kind(&self, login: &str) -> Result<AccountKind, InterfaceError> {
        if login.is_empty()
            || !login
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Ok(AccountKind::Absent);
        }
        let config = Configuration::load_global(&self.home)
            .map_err(|error| unanswered(format!("cannot read the machine layer: {error}")))?;
        let api = api_base(&config).map_err(|error| unanswered(error.to_string()))?;
        let mut response = agent()
            .get(&format!("{api}/users/{login}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|error| unanswered(format!("GitHub is unreachable: {error}")))?;
        let status = response.status().as_u16();
        if status == 404 {
            return Ok(AccountKind::Absent);
        }
        let limits = super::_github::Limits::read(response.headers());
        if limits.refusal_throttles(status) {
            return Err(unanswered(
                "GitHub's rate limit for unauthenticated reads is reached",
            ));
        }
        if !(200..300).contains(&status) {
            return Err(unanswered(format!(
                "GitHub answered {status} for the account `{login}`"
            )));
        }
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| unanswered(format!("GitHub's answer cannot be read: {error}")))?;
        let record: Value = serde_json::from_str(&text)
            .map_err(|_| unanswered("GitHub's account record is not JSON"))?;
        kind_of(login, &record)
    }
}

/// Whether GitHub's 422 answer `body` to creating a repository says a
/// repository of that name already exists.
fn already_exists(body: &Value) -> bool {
    body["errors"].as_array().is_some_and(|errors| {
        errors.iter().any(|error| {
            error["field"] == "name"
                && error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("already exists"))
        })
    })
}

/// The failure of `what` GitHub answered with `answer`.
fn failed(what: &str, answer: &Answer) -> CompanyRepositoryError {
    let said = answer.body["message"]
        .as_str()
        .map(|message| format!(": {message}"))
        .unwrap_or_default();
    CompanyRepositoryError::Failed(format!("GitHub refused {what} ({}{said})", answer.status))
}

/// The changes this boundary makes to a company's repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mutation {
    Create,
    GrantAdmin,
    SetDefaultBranch,
}

impl Mutation {
    /// The mutation as its diagnostic record names it.
    fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::GrantAdmin => "grant_admin",
            Self::SetDefaultBranch => "set_default_branch",
        }
    }
}

/// The record of one company-repository mutation GitHub answered: which
/// operation, on which repository, with which status. Nothing of the
/// request, the credential or the administrator is recorded.
fn mutated(mutation: Mutation, org: &str, repo: &str, status: u16, succeeded: bool) {
    let operation = mutation.label();
    if succeeded {
        tracing::info!(
            event = "company_repository_mutated",
            operation,
            repository = %format_args!("{org}/{repo}"),
            status,
        );
    } else {
        tracing::warn!(
            event = "company_repository_mutation_refused",
            operation,
            repository = %format_args!("{org}/{repo}"),
            status,
        );
    }
}

fn unreached(what: &str, error: &AppError) -> CompanyRepositoryError {
    CompanyRepositoryError::Failed(format!("GitHub could not be asked {what}: {error}"))
}

impl GitHubAccess {
    /// Runs `action` with the project identity's credential for
    /// administering repositories in `org`; a credential that cannot be
    /// issued is the organisation not being ready, unless GitHub could not
    /// be asked.
    fn administering<T>(
        &self,
        org: &str,
        now: Timestamp,
        action: impl FnOnce(&Apps, &str, Clock) -> Result<T, CompanyRepositoryError>,
    ) -> Result<T, CompanyRepositoryError> {
        let clock = Clock::starting(now);
        let mut app = None;
        let outcome = self.with_apps(|apps| {
            app = Some(apps.selected(None).slug.clone());
            let credential = apps.issue(org, None, Operation::CompanyRepository, clock)?;
            Ok(action(apps, credential.token(), clock))
        });
        match outcome {
            Ok(result) => result,
            Err(error @ (AppError::Unreachable | AppError::RateLimited { .. })) => {
                Err(unreached("for the project identity's credential", &error))
            }
            Err(error) => Err(CompanyRepositoryError::NotReady {
                app,
                organisation: org.to_owned(),
                cause: error.to_string(),
            }),
        }
    }
}

impl CompanyRepositories for GitHubAccess {
    fn ready(
        &self,
        org: &GitHubOrganisation,
        now: Timestamp,
    ) -> Result<(), CompanyRepositoryError> {
        self.administering(org.as_str(), now, |_, _, _| Ok(()))
    }

    fn create(
        &self,
        org: &GitHubOrganisation,
        repo: &ProjectSlug,
        now: Timestamp,
    ) -> Result<Creation, CompanyRepositoryError> {
        let (org, repo) = (org.as_str(), repo.as_str());
        self.administering(org, now, |apps, token, clock| {
            let what = format!("creating {org}/{repo}");
            let answer = apps
                .send(
                    Method::Post,
                    &format!("/orgs/{org}/repos"),
                    token,
                    Some(&json!({"name": repo, "private": true})),
                    clock,
                )
                .map_err(|error| unreached(&what, &error))?;
            let outcome = match answer.status {
                201 => Ok(Creation::Created),
                422 if already_exists(&answer.body) => Ok(Creation::AlreadyExists),
                _ => Err(failed(&what, &answer)),
            };
            mutated(Mutation::Create, org, repo, answer.status, outcome.is_ok());
            outcome
        })
    }

    fn grant_admin(
        &self,
        org: &GitHubOrganisation,
        repo: &ProjectSlug,
        login: &GitHubLogin,
        now: Timestamp,
    ) -> Result<(), CompanyRepositoryError> {
        let (org, repo, login) = (org.as_str(), repo.as_str(), login.as_str());
        self.administering(org, now, |apps, token, clock| {
            let what = format!("making {login} an administrator of {org}/{repo}");
            let answer = apps
                .send(
                    Method::Put,
                    &format!("/repos/{org}/{repo}/collaborators/{login}"),
                    token,
                    Some(&json!({"permission": "admin"})),
                    clock,
                )
                .map_err(|error| unreached(&what, &error))?;
            // 201: GitHub invited them; 204: a member, added at once.
            let outcome = match answer.status {
                201 | 204 => Ok(()),
                _ => Err(failed(&what, &answer)),
            };
            mutated(
                Mutation::GrantAdmin,
                org,
                repo,
                answer.status,
                outcome.is_ok(),
            );
            outcome
        })
    }

    fn set_default_branch(
        &self,
        org: &GitHubOrganisation,
        repo: &ProjectSlug,
        branch: &str,
        now: Timestamp,
    ) -> Result<(), CompanyRepositoryError> {
        let (org, repo) = (org.as_str(), repo.as_str());
        self.administering(org, now, |apps, token, clock| {
            let what = format!("setting the default branch of {org}/{repo} to {branch}");
            let answer = apps
                .send(
                    Method::Patch,
                    &format!("/repos/{org}/{repo}"),
                    token,
                    Some(&json!({"default_branch": branch})),
                    clock,
                )
                .map_err(|error| unreached(&what, &error))?;
            let outcome = match answer.status {
                200 => Ok(()),
                _ => Err(failed(&what, &answer)),
            };
            mutated(
                Mutation::SetDefaultBranch,
                org,
                repo,
                answer.status,
                outcome.is_ok(),
            );
            outcome
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountKind, already_exists, json, kind_of};

    #[test]
    fn only_a_name_that_already_exists_is_an_existing_repository() {
        assert!(already_exists(&json!({
            "message": "Repository creation failed.",
            "errors": [{"resource": "Repository", "code": "custom", "field": "name",
                        "message": "name already exists on this account"}]
        })));
        assert!(!already_exists(&json!({
            "message": "Repository creation failed.",
            "errors": [{"resource": "Repository", "code": "custom", "field": "name",
                        "message": "name is too long"}]
        })));
        assert!(!already_exists(&json!({"message": "Validation Failed"})));
    }

    #[test]
    fn an_accounts_type_decides_its_kind() {
        assert_eq!(
            kind_of("homes", &json!({"type": "Organization"})).unwrap(),
            AccountKind::Organisation
        );
        assert_eq!(
            kind_of("ana", &json!({"type": "User"})).unwrap(),
            AccountKind::Personal
        );
        assert!(kind_of("robot", &json!({"type": "Bot"})).is_err());
        assert!(kind_of("odd", &json!({})).is_err());
    }
}
