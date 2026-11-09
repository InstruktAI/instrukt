//! Public App configuration declarations and capability data; no authentication policy.
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
fn setup(cause: &str) -> AppError {
    AppError::Setup(cause.into())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bot {
    pub id: u64,
    pub node_id: String,
    pub login: String,
}
impl Bot {
    pub fn author(&self) -> String {
        format!(
            "{} <{}+{}@users.noreply.github.com>",
            self.login, self.id, self.login
        )
    }
    pub fn matches(&self, actor: &Value) -> bool {
        actor["id"].as_u64() == Some(self.id)
            && actor["node_id"].as_str() == Some(self.node_id.as_str())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub app_id: u64,
    pub slug: String,
    pub bot: Bot,
    pub private_key: String,
}
/// Validate public configuration without reading keys or making requests.
pub fn inventory(value: &serde_yaml_ng::Value) -> Result<BTreeMap<String, App>, AppError> {
    let apps: BTreeMap<String, App> = serde_yaml_ng::from_value(value.clone()).map_err(|_| {
        setup("github.apps requires app_id, slug, bot {id,node_id,login}, private_key ${NAME}")
    })?;
    let (mut ids, mut slugs, mut bots, mut nodes, mut logins) = (
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
    );
    for app in apps.values() {
        if app.app_id == 0
            || app.bot.id == 0
            || app.slug.is_empty()
            || !app
                .slug
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || app.bot.login != format!("{}[bot]", app.slug)
            || app.bot.node_id.is_empty()
            || !ids.insert(app.app_id)
            || !slugs.insert(app.slug.to_lowercase())
            || !bots.insert(app.bot.id)
            || !nodes.insert(&app.bot.node_id)
            || !logins.insert(app.bot.login.to_lowercase())
            || !crate::config::is_secret_reference(&app.private_key)
        {
            return Err(setup(
                "App/bot identities must be distinct and keys exact protected-file ${NAME} references",
            ));
        }
    }
    Ok(apps)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    Record,
    Author,
    Review,
    Board,
    Observe,
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    #[error(
        "GitHub App setup incomplete: {0}; repair github.apps and its repository installation with the configuration skill"
    )]
    Setup(String),
    #[error("GitHub App API unavailable; retry the pending operation")]
    Unreachable,
    #[error("GitHub App API deferred requests (rate limited)")]
    RateLimited { until: Option<Timestamp> },
    #[error(
        "GitHub App API refused {operation} ({status}); repair the selected App installation and permissions"
    )]
    Refused { operation: String, status: u16 },
}
/// Secret-bearing: intentionally neither Debug nor Serialize.
pub struct Credential {
    token: String,
    pub bot: Bot,
    pub expires: Timestamp,
}
impl Credential {
    pub fn new(token: String, bot: Bot, expires: Timestamp) -> Self {
        Self {
            token,
            bot,
            expires,
        }
    }
    pub fn token(&self) -> &str {
        &self.token
    }
}
