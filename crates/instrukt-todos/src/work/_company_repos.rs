//! Work's GitHub boundary for company repositories
//! (`project/instrukt/spec/feature/work/github-apps`, Company repositories):
//! what kind of account a login names, read from GitHub's public account
//! record without authentication.

use instrukt_core::config::Configuration;
use instrukt_core::interfaces::{AccountKind, GitHubAccounts, InterfaceError};
use serde_json::Value;

use super::_apps::{GitHubAccess, agent, api_base};

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
