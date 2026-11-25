//! Which pull requests' heads are reviewed
//! (`pull-request-review`, Which heads are reviewed): a pure function of
//! one pull request as GitHub lists it and the configured identities.

use std::collections::BTreeSet;

use instrukt_core::ids::CommitSha;
use serde_json::Value;

use crate::interfaces::ReviewIdentities;

/// An eligible pull request at its current head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Eligible {
    pub number: u64,
    pub head: CommitSha,
    pub head_branch: String,
    pub base_branch: String,
}

/// Why a pull request's head is not reviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ineligible {
    Draft,
    /// A todo's work lifecycle reviews it.
    OwnedByTodo,
    /// Its head branch is in a fork.
    Fork,
    /// Its author is outside the project's trusted authors, a
    /// dependency-update bot among them.
    UntrustedAuthor,
    /// The reviewer identity's own bot opened it.
    ReviewerBot,
    /// GitHub's answer lacks what eligibility reads.
    Unreadable,
}

/// The associations GitHub reports for a trusted human author.
const TRUSTED_ASSOCIATIONS: [&str; 3] = ["OWNER", "MEMBER", "COLLABORATOR"];

/// Whether `pull` of `repository` is reviewed automatically.
pub(crate) fn eligibility(
    pull: &Value,
    repository: &str,
    identities: &ReviewIdentities,
    todo_pulls: &BTreeSet<u64>,
) -> Result<Eligible, Ineligible> {
    let number = pull["number"].as_u64().ok_or(Ineligible::Unreadable)?;
    if pull["draft"] == Value::Bool(true) {
        return Err(Ineligible::Draft);
    }
    if todo_pulls.contains(&number) {
        return Err(Ineligible::OwnedByTodo);
    }
    if pull["head"]["repo"]["full_name"].as_str() != Some(repository) {
        return Err(Ineligible::Fork);
    }
    let user = &pull["user"];
    if user["type"] == "Bot" {
        if identities.reviewer.matches(user) {
            return Err(Ineligible::ReviewerBot);
        }
        if !identities.trusted_bots.iter().any(|bot| bot.matches(user)) {
            return Err(Ineligible::UntrustedAuthor);
        }
    } else if !pull["author_association"]
        .as_str()
        .is_some_and(|association| TRUSTED_ASSOCIATIONS.contains(&association))
    {
        return Err(Ineligible::UntrustedAuthor);
    }
    let text = |value: &Value| {
        value
            .as_str()
            .map(str::to_owned)
            .ok_or(Ineligible::Unreadable)
    };
    Ok(Eligible {
        number,
        head: pull["head"]["sha"]
            .as_str()
            .and_then(|sha| CommitSha::parse(sha).ok())
            .ok_or(Ineligible::Unreadable)?,
        head_branch: text(&pull["head"]["ref"])?,
        base_branch: text(&pull["base"]["ref"])?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_core::github::Bot;
    use serde_json::json;

    fn bot(id: u64, login: &str) -> Bot {
        Bot {
            id,
            node_id: format!("BOT_{id}"),
            login: login.into(),
        }
    }

    fn identities() -> ReviewIdentities {
        ReviewIdentities {
            trusted_bots: vec![bot(2, "author[bot]")],
            reviewer: bot(3, "reviewer[bot]"),
        }
    }

    fn pull(edit: impl FnOnce(&mut Value)) -> Value {
        let mut pull = json!({
            "number": 7, "draft": false,
            "user": {"login": "dev", "id": 9, "node_id": "U_9", "type": "User"},
            "author_association": "MEMBER",
            "head": {"ref": "feature", "sha": "a".repeat(40), "repo": {"full_name": "acme/demo"}},
            "base": {"ref": "main"},
        });
        edit(&mut pull);
        pull
    }

    fn judge(pull: &Value) -> Result<Eligible, Ineligible> {
        eligibility(pull, "acme/demo", &identities(), &BTreeSet::from([8]))
    }

    #[test]
    fn a_ready_pull_request_of_a_trusted_author_from_the_repository_is_reviewed() {
        assert_eq!(judge(&pull(|_| {})).map(|e| e.number), Ok(7));
    }

    #[test]
    fn a_draft_is_not_reviewed() {
        assert_eq!(
            judge(&pull(|p| p["draft"] = json!(true))),
            Err(Ineligible::Draft)
        );
    }

    #[test]
    fn a_todos_pull_request_is_not_reviewed() {
        assert_eq!(
            judge(&pull(|p| p["number"] = json!(8))),
            Err(Ineligible::OwnedByTodo)
        );
    }

    #[test]
    fn a_fork_is_not_reviewed() {
        assert_eq!(
            judge(&pull(
                |p| p["head"]["repo"]["full_name"] = json!("stranger/demo")
            )),
            Err(Ineligible::Fork)
        );
    }

    #[test]
    fn an_author_outside_the_trusted_authors_is_not_reviewed() {
        assert_eq!(
            judge(&pull(|p| p["author_association"] = json!("CONTRIBUTOR"))),
            Err(Ineligible::UntrustedAuthor)
        );
    }

    #[test]
    fn a_dependency_update_bot_is_not_reviewed() {
        assert_eq!(
            judge(&pull(|p| {
                p["user"] =
                    json!({"login": "renovate[bot]", "id": 99, "node_id": "BOT_99", "type": "Bot"});
                p["author_association"] = json!("MEMBER");
            })),
            Err(Ineligible::UntrustedAuthor)
        );
    }

    #[test]
    fn the_reviewer_identitys_own_bot_is_not_reviewed() {
        assert_eq!(
            judge(&pull(
                |p| p["user"] =
                    json!({"login": "reviewer[bot]", "id": 3, "node_id": "BOT_3", "type": "Bot"})
            )),
            Err(Ineligible::ReviewerBot)
        );
    }

    #[test]
    fn a_configured_apps_bot_is_a_trusted_author() {
        assert!(
            judge(&pull(|p| {
                p["user"] =
                    json!({"login": "author[bot]", "id": 2, "node_id": "BOT_2", "type": "Bot"});
                p["author_association"] = json!("NONE");
            }))
            .is_ok()
        );
    }
}
