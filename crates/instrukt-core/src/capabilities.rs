//! Resolve role grants against the objects retained by an assignment.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

macro_rules! actions {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum Action { $(#[serde(rename = $name)] $variant),+ }
        impl Action {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub fn name(self) -> &'static str { match self { $(Self::$variant => $name),+ } }
        }
    };
}
actions! {
    DocsRead => "docs-read", DocsWrite => "docs-write",
    TodoRead => "todo-read", TodoWrite => "todo-write",
    ArtifactRead => "artifact-read", ArtifactWrite => "artifact-write", Stamp => "stamp",
    TodoOrchestration => "todo-orchestration", AssignmentWrite => "assignment-write", Integration => "integration",
    SessionObserve => "session-observe", HistoryRead => "history-read", SessionRecover => "session-recover", StandingBind => "standing-bind", ConversationLink => "conversation-link",
    AgentsRead => "agents-read", AgentsWrite => "agents-write",
    ConfigRead => "config-read", ConfigWrite => "config-write", SecretsWrite => "secrets-write",
    RepositoryRead => "repository-read", RepositoryWrite => "repository-write", CodeCommit => "code-commit",
    Publication => "publication", CodeCheck => "code-check", EvalRead => "eval-read", EventsRead => "events-read",
    InstallationRead => "installation-read", InstallationWrite => "installation-write", RunnerExecute => "runner-execute",
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleClass {
    Focused,
    Standing,
    Agent,
}
impl RoleClass {
    pub fn name(self) -> &'static str {
        match self {
            Self::Focused => "focused",
            Self::Standing => "standing",
            Self::Agent => "agent",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Own,
    Any,
}
impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Self::Own => "own",
            Self::Any => "any",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub action: Action,
    pub scope: Scope,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Allowance {
    Bundle(String),
    Grant(Grant),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub role: String,
    pub class: RoleClass,
    pub grants: BTreeSet<Grant>,
}
impl Capabilities {
    pub fn describe(&self) -> String {
        format!(
            "role: {}; class: {}; grants: {}",
            self.role,
            self.class.name(),
            self.grants
                .iter()
                .map(|grant| format!("{}:{}", grant.action.name(), grant.scope.name()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    pub fn scope(&self, action: Action) -> Option<Scope> {
        [Scope::Any, Scope::Own].into_iter().find(|scope| {
            self.grants.contains(&Grant {
                action,
                scope: *scope,
            })
        })
    }
    pub fn admits(&self, action: Action, owned: bool) -> bool {
        self.class != RoleClass::Focused
            || matches!(self.scope(action), Some(Scope::Any))
            || (owned && self.scope(action) == Some(Scope::Own))
    }
    pub fn refusal(&self, action: Action, object: &str) -> String {
        format!(
            "role `{}` (focused) cannot perform {} on {object} with its {} scope; return the required action to the dispatcher in your own output or assigned artifact",
            self.role,
            action.name(),
            self.scope(action).map(Scope::name).unwrap_or("ungranted")
        )
    }
}
/// Facts owned by work or runner; no environment variable supplies these values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub role: crate::ids::RoleName,
    pub project: Option<crate::ids::ProjectSlug>,
    pub procedure: Option<crate::ids::SnippetId>,
    pub todo: Option<String>,
    pub workspace: Option<PathBuf>,
    pub key: String,
    pub platform: bool,
    pub dispatcher: Option<crate::ids::SessionRef>,
}

#[cfg(test)]
mod tests {
    use super::Assignment;
    fn assignment() -> serde_json::Value {
        serde_json::json!({"role":"builder","project":"demo","procedure":"sdlc/procedure/build","todo":null,"workspace":null,"key":"assignment","platform":false,"dispatcher":null})
    }
    #[test]
    fn assignment_deserialization_rejects_an_invalid_role() {
        let mut value = assignment();
        value["role"] = "two\nlines".into();
        assert!(serde_json::from_value::<Assignment>(value).is_err());
    }
    #[test]
    fn assignment_deserialization_rejects_an_invalid_project() {
        let mut value = assignment();
        value["project"] = "../outside".into();
        assert!(serde_json::from_value::<Assignment>(value).is_err());
    }
    #[test]
    fn assignment_deserialization_rejects_an_invalid_procedure() {
        let mut value = assignment();
        value["procedure"] = "not-a-snippet".into();
        assert!(serde_json::from_value::<Assignment>(value).is_err());
    }
}
