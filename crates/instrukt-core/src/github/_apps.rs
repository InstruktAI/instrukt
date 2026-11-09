//! Verified Apps issue bounded, memory-only credentials from protected keys.
use crate::{
    config::{Agents, Configuration},
    home::Home,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

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
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub app_id: u64,
    pub slug: String,
    pub bot: Bot,
    pub private_key: String,
}
#[derive(Debug, Clone, Copy)]
pub enum Operation {
    Record,
    Author,
    Review,
    Board,
    Observe,
}
impl Operation {
    fn permissions(self) -> Value {
        match self {
            Self::Record => json!({"issues":"write","contents":"read","pull_requests":"read"}),
            Self::Author => json!({"contents":"write","pull_requests":"write"}),
            Self::Review => json!({"contents":"read","pull_requests":"write"}),
            Self::Board => json!({"organization_projects":"write"}),
            Self::Observe => json!({"checks":"read","statuses":"read","contents":"read"}),
        }
    }
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
fn setup(cause: &str) -> AppError {
    AppError::Setup(cause.into())
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

/// Secret-bearing: intentionally neither Debug nor Serialize.
pub struct Credential {
    token: String,
    pub bot: Bot,
    pub expires: Timestamp,
}
impl Credential {
    pub fn token(&self) -> &str {
        &self.token
    }
}

pub struct Apps {
    home: Home,
    api: String,
    apps: BTreeMap<String, App>,
    project: String,
    roles: BTreeMap<String, String>,
    agent: ureq::Agent,
}
impl Apps {
    pub fn load(home: &Home) -> Result<Self, AppError> {
        let config = Configuration::load_global(home)
            .map_err(|_| setup("cannot read machine-global configuration"))?;
        let apps = config
            .github_apps()
            .map_err(|_| setup("cannot read github.apps"))?;
        let project = config
            .github_project_app()
            .map_err(|_| setup("cannot read github.project_app"))?
            .ok_or_else(|| setup("github.project_app is missing"))?
            .to_owned();
        let agents = Agents::load(home).map_err(|_| setup("cannot read agents.yaml roles"))?;
        let roles: BTreeMap<_, _> = agents
            .roles
            .into_iter()
            .filter_map(|(r, s)| s.github_app.map(|a| (r, a)))
            .collect();
        if !apps.contains_key(&project) || roles.values().any(|r| !apps.contains_key(r)) {
            return Err(setup(
                "github.project_app or roles.<role>.github_app refers to an unknown profile",
            ));
        }
        let api = super::api_url(home).map_err(|_| setup("cannot read github.api_url"))?;
        let url = url::Url::parse(&api).map_err(|_| setup("invalid github.api_url"))?;
        if url.scheme() != "https"
            && !(url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
        {
            return Err(setup(
                "github.api_url requires HTTPS (HTTP only on loopback)",
            ));
        }
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .timeout_connect(Some(Duration::from_secs(10)))
            .max_redirects(0)
            .user_agent("instrukt")
            .build()
            .into();
        Ok(Self {
            home: home.clone(),
            api: api.trim_end_matches('/').into(),
            apps,
            project,
            roles,
            agent,
        })
    }
    pub fn issue(
        &self,
        repository: &str,
        role: Option<&str>,
        op: Operation,
        now: Timestamp,
    ) -> Result<Credential, AppError> {
        let name = role
            .and_then(|r| self.roles.get(r))
            .unwrap_or(&self.project);
        let app = &self.apps[name];
        let key = crate::config::resolve_secret(&self.home, &app.private_key).map_err(|_| {
            setup(
                "selected App key unavailable in owner-only secrets.env; use instrukt secrets set",
            )
        })?;
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(key.as_bytes())
            .map_err(|_| setup("selected App key is not usable RSA PEM"))?;
        let jwt = jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),&json!({"iat":now.as_second()-60,"exp":now.as_second()+540,"iss":app.app_id.to_string()}),&key).map_err(|_|setup("cannot sign selected App JWT"))?;
        let identity = self.request("GET", "/app", &jwt, None, now)?;
        if identity["id"].as_u64() != Some(app.app_id)
            || identity["slug"].as_str() != Some(&app.slug)
        {
            return Err(setup(
                "selected key's App identity does not match configuration",
            ));
        }
        self.verify_bot(app, now)?;
        let installation = self.request(
            "GET",
            &format!("/repos/{repository}/installation"),
            &jwt,
            None,
            now,
        )?;
        let id = installation["id"]
            .as_u64()
            .filter(|_| installation["app_id"].as_u64() == Some(app.app_id))
            .ok_or_else(|| setup("repository installation does not match selected App"))?;
        let permissions = op.permissions();
        check_permissions(&installation["permissions"], &permissions)?;
        let (_, repo) = repository
            .split_once('/')
            .ok_or_else(|| setup("invalid repository"))?;
        let response = self.request(
            "POST",
            &format!("/app/installations/{id}/access_tokens"),
            &jwt,
            Some(&json!({"repositories":[repo],"permissions":permissions})),
            now,
        )?;
        check_permissions(&response["permissions"], &permissions)?;
        let expires: Timestamp = response["expires_at"]
            .as_str()
            .and_then(|s| s.parse::<Timestamp>().ok())
            .filter(|t| t.as_second() > now.as_second() + 30)
            .ok_or_else(|| setup("issued credential has no usable expiry"))?;
        let token = response["token"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| setup("issuance returned no installation credential"))?;
        let repos = self.request(
            "GET",
            "/installation/repositories?per_page=100",
            token,
            None,
            now,
        )?;
        if !repos["repositories"].as_array().is_some_and(|rs| {
            rs.iter()
                .any(|r| r["full_name"].as_str() == Some(repository))
        }) {
            return Err(setup("issued credential cannot reach repository"));
        }
        let viewer = self.request(
            "POST",
            "/graphql",
            token,
            Some(&json!({"query":"query { viewer { id login databaseId } }"})),
            now,
        )?;
        let viewer = &viewer["data"]["viewer"];
        if viewer["databaseId"].as_u64() != Some(app.bot.id)
            || viewer["id"].as_str() != Some(&app.bot.node_id)
            || viewer["login"].as_str() != Some(&app.bot.login)
        {
            return Err(setup(
                "issued credential's actor does not match the selected App bot",
            ));
        }
        Ok(Credential {
            token: token.into(),
            bot: app.bot.clone(),
            expires,
        })
    }
    fn verify_bot(&self, app: &App, now: Timestamp) -> Result<(), AppError> {
        let public = self.request("GET", &format!("/apps/{}", app.slug), "", None, now)?;
        let bot = self.request("GET", &format!("/users/{}", app.bot.login), "", None, now)?;
        if public["id"].as_u64() != Some(app.app_id)
            || public["slug"].as_str() != Some(&app.slug)
            || !app.bot.matches(&bot)
            || bot["login"].as_str() != Some(&app.bot.login)
            || bot["type"] != "Bot"
        {
            return Err(setup(
                "configured App/bot stable identity does not match GitHub",
            ));
        }
        Ok(())
    }
    pub fn verified_bots(&self, now: Timestamp) -> Result<Vec<Bot>, AppError> {
        self.apps
            .values()
            .map(|app| {
                self.verify_bot(app, now)?;
                Ok(app.bot.clone())
            })
            .collect()
    }
    fn request(
        &self,
        method: &str,
        endpoint: &str,
        bearer: &str,
        body: Option<&Value>,
        now: Timestamp,
    ) -> Result<Value, AppError> {
        let url = match (endpoint, self.api.strip_suffix("/api/v3")) {
            ("/graphql", Some(host)) => format!("{host}/api/graphql"),
            _ => format!("{}{endpoint}", self.api),
        };
        let mut response = if method == "POST" {
            headers(self.agent.post(&url), bearer)
                .header("Content-Type", "application/json")
                .send(body.map(Value::to_string).unwrap_or_default())
        } else {
            headers(self.agent.get(&url), bearer).call()
        }
        .map_err(|_| AppError::Unreachable)?;
        let status = response.status().as_u16();
        let h = response.headers();
        if status == 429
            || (status == 403
                && (h.get("x-ratelimit-remaining").is_some_and(|h| h == "0")
                    || h.contains_key("retry-after")))
        {
            let until = h
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .and_then(|v| Timestamp::from_second(v).ok());
            let until = until.or_else(|| {
                h.get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<i64>().ok())
                    .filter(|seconds| *seconds >= 0)
                    .and_then(|seconds| now.as_second().checked_add(seconds))
                    .and_then(|second| Timestamp::from_second(second).ok())
            });
            return Err(AppError::RateLimited { until });
        }
        if !(200..300).contains(&status) {
            return Err(AppError::Refused {
                operation: endpoint.split('?').next().unwrap_or(endpoint).into(),
                status,
            });
        }
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|_| AppError::Unreachable)?;
        serde_json::from_str(&text).map_err(|_| setup("unreadable App API response"))
    }
}
fn check_permissions(actual: &Value, required: &Value) -> Result<(), AppError> {
    for (name, wanted) in required.as_object().into_iter().flatten() {
        if actual[name] != *wanted && !(wanted == "read" && actual[name] == "write") {
            return Err(setup(&format!(
                "installation credential lacks required {name} permission"
            )));
        }
    }
    Ok(())
}

fn headers<B>(request: ureq::RequestBuilder<B>, bearer: &str) -> ureq::RequestBuilder<B> {
    let request = request
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if bearer.is_empty() {
        request
    } else {
        request.header("Authorization", &format!("Bearer {bearer}"))
    }
}
