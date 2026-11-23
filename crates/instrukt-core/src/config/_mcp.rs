//! MCP server declarations, reusable host-level OAuth clients (`mcp_clients`),
//! and built-in per-company OIDC issuer settings (`mcp-access`, "Declaration").

use std::collections::BTreeMap;

use serde_json::json;
use serde_yaml_ng::Value;

use super::_error::{ConfigError, FindingKind};
use super::_interpolate::{join, key_text};
use super::_schema::{KeyFinding, LayerKind};
use crate::ids::ProjectSlug;

/// How a company home obtains a speaker's login for an MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpeakerLogin {
    #[default]
    Stored,
    SignIn,
}

/// How a `speaker_login: sign-in` server obtains a speaker's Bearer token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignInMode {
    /// Interactive per-speaker browser OAuth authorization code flow.
    #[default]
    AuthorizationCode,
    /// Zero-click Enterprise-Managed Authorization / ID-JAG `jwt-bearer`
    /// signed by the company's built-in per-company OIDC issuer.
    Issuer,
}

impl SignInMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthorizationCode => "authorization-code",
            Self::Issuer => "issuer",
        }
    }
}

/// A pre-registered OAuth client declared directly in a company home under
/// `mcp.<name>.sign_in.client`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInClient {
    /// `${NAME}` reference in the company home's `secrets.env`.
    pub id: String,
    /// Optional `${NAME}` reference in the company home's `secrets.env`.
    pub secret: Option<String>,
}

/// Custom identity endpoint for a non-OpenID Provider authorization server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInIdentity {
    /// Absolute `https` address (without credentials or fragment).
    pub url: String,
    /// Dot-separated path into the JSON response holding the email address.
    pub email_field: String,
}

/// Sign-in settings under `mcp.<name>.sign_in`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpSignIn {
    pub mode: SignInMode,
    pub scopes: Vec<String>,
    /// Key name of a reusable host-level OAuth client declared under global
    /// `mcp_clients.<name>`. Mutually exclusive with `client`.
    pub client_ref: Option<String>,
    /// Company-local pre-registered OAuth client. Mutually exclusive with
    /// `client_ref`.
    pub client: Option<SignInClient>,
    pub accept_return_without_issuer: bool,
    pub identity: Option<SignInIdentity>,
}

/// One declared MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    /// The server's name.
    pub name: ProjectSlug,
    /// The server's network address.
    pub url: String,
    /// One line saying what the server is for.
    pub description: String,
    /// Company home only: how a speaker's login is obtained (`stored` or `sign-in`).
    pub speaker_login: SpeakerLogin,
    /// Company home only, with `speaker_login: stored`: the name under
    /// `members.<member>.credentials` whose value a speaker's call carries.
    pub credential: Option<String>,
    /// A `${NAME}` reference: the company's own login in a company home, the
    /// machine's own login in the global layer.
    pub login: Option<String>,
    /// Company home only, with `speaker_login: sign-in`: sign-in options.
    pub sign_in: Option<McpSignIn>,
}

impl McpServer {
    /// Whether its calls carry no login.
    pub fn is_open(&self) -> bool {
        self.speaker_login == SpeakerLogin::Stored
            && self.credential.is_none()
            && self.login.is_none()
    }
}

/// One reusable host-level OAuth client declared under global `mcp_clients.<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpClient {
    pub name: ProjectSlug,
    /// Required `${NAME}` reference into `~/.instrukt/secrets.env`.
    pub id: String,
    /// Optional `${NAME}` reference into `~/.instrukt/secrets.env`.
    pub secret: Option<String>,
    /// Optional one-line description.
    pub description: Option<String>,
}

/// A company home's built-in per-company OIDC issuer settings (`issuer:`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyIssuerConfig {
    pub enabled: bool,
    /// Optional `${NAME}` reference into the company home's `secrets.env`.
    pub signing_key: Option<String>,
}

/// Default secret reference name for a company's built-in issuer signing key.
pub const DEFAULT_ISSUER_SIGNING_KEY_NAME: &str = "INSTRUKT_ISSUER_SIGNING_KEY";

/// Whether a login may be sent to `url`: an `https` address, or an `http`
/// address on this computer's loopback, so a login never crosses the network
/// unprotected.
pub fn admitted_address(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "https" => parsed.host().is_some(),
        "http" => match parsed.host() {
            Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        },
        _ => false,
    }
}

/// Whether `url` is a valid `sign_in.identity.url`: an absolute `https`
/// address (or loopback `http`? No — spec states absolute `https` without
/// credentials or fragment) without credentials or fragment.
pub fn admitted_identity_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.fragment().is_none()
}

fn finding(path: &str, kind: FindingKind, message: impl Into<String>) -> KeyFinding {
    KeyFinding::new(path, kind, message)
}

/// The findings of a global `mcp_clients` map.
pub(super) fn check_clients(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(clients) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            "expected a mapping of OAuth client names to declarations",
        ));
        return;
    };
    for (name, declaration) in clients {
        let name = key_text(name);
        let at = join(path, &name);
        if ProjectSlug::parse(&name).is_err() {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                format!("mcp client `{name}`: a client name follows the grammar of a project slug"),
            ));
        }
        let Some(fields) = declaration.as_mapping() else {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                format!("mcp client `{name}`: expected a mapping"),
            ));
            continue;
        };
        if !fields.contains_key("id") {
            out.push(finding(
                &join(&at, "id"),
                FindingKind::Missing,
                format!(
                    "mcp client `{name}`: missing `id` (expected a ${{NAME}} reference into ~/.instrukt/secrets.env)"
                ),
            ));
        }
        for (field, value) in fields {
            let field = key_text(field);
            let field_at = join(&at, &field);
            match field.as_str() {
                "id" | "secret" => {
                    if !value.as_str().is_some_and(super::is_secret_reference) {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            format!(
                                "mcp client `{name}`: `{field}` expects an exact ${{NAME}} reference into ~/.instrukt/secrets.env; literal values are not stored"
                            ),
                        ));
                    }
                }
                "description" => match value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    Some(text) if text.contains(['\n', '\r']) => out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!("mcp client `{name}`: `description` is one line"),
                    )),
                    Some(_) => {}
                    None => out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!("mcp client `{name}`: `description` expects nonempty text"),
                    )),
                },
                _ => out.push(finding(
                    &field_at,
                    FindingKind::UnknownKey,
                    format!(
                        "mcp client `{name}`: unknown field; a host OAuth client takes id, secret and description"
                    ),
                )),
            }
        }
    }
}

/// The findings of a company home's `issuer` block.
pub(super) fn check_issuer(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(fields) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            "expected a mapping with `enabled` (boolean) and optional `signing_key` (${NAME})",
        ));
        return;
    };
    if !fields.contains_key("enabled") {
        out.push(finding(
            &join(path, "enabled"),
            FindingKind::Missing,
            "missing `enabled` (true or false)",
        ));
    }
    for (field, value) in fields {
        let field = key_text(field);
        let field_at = join(path, &field);
        match field.as_str() {
            "enabled" => {
                if !value.is_bool() {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        "expected true or false",
                    ));
                }
            }
            "signing_key" => {
                if !value.as_str().is_some_and(super::is_secret_reference) {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        "expected an exact ${NAME} reference into the company home's secrets.env; a literal key is not stored",
                    ));
                }
            }
            _ => out.push(finding(
                &field_at,
                FindingKind::UnknownKey,
                "unknown field; `issuer` takes enabled and signing_key",
            )),
        }
    }
}

/// The findings of an `mcp` map declared in `layer`.
pub(super) fn check(value: &Value, path: &str, layer: LayerKind, out: &mut Vec<KeyFinding>) {
    let Some(servers) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            "expected a mapping of server names to declarations",
        ));
        return;
    };
    for (name, declaration) in servers {
        let name = key_text(name);
        let at = join(path, &name);
        if ProjectSlug::parse(&name).is_err() {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                format!("server `{name}`: a server name follows the grammar of a project slug"),
            ));
        }
        let Some(fields) = declaration.as_mapping() else {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                format!("server `{name}`: expected a mapping"),
            ));
            continue;
        };
        for required in ["url", "description"] {
            if !fields.contains_key(required) {
                out.push(finding(
                    &join(&at, required),
                    FindingKind::Missing,
                    format!("server `{name}`: missing `{required}`"),
                ));
            }
        }
        let has_credential = fields.contains_key("credential");
        let raw_speaker_login = fields.get("speaker_login").and_then(Value::as_str);
        let is_sign_in = raw_speaker_login == Some("sign-in");
        let is_explicit_stored = raw_speaker_login == Some("stored");

        if layer == LayerKind::Company && is_explicit_stored && !has_credential {
            out.push(finding(
                &join(&at, "credential"),
                FindingKind::Missing,
                format!("server `{name}`: `speaker_login: stored` requires `credential`"),
            ));
        }
        if layer == LayerKind::Company && is_sign_in && has_credential {
            out.push(finding(
                &join(&at, "credential"),
                FindingKind::InvalidValue,
                format!(
                    "server `{name}`: `credential` cannot be combined with `speaker_login: sign-in`"
                ),
            ));
        }

        for (field, value) in fields {
            let field = key_text(field);
            let field_at = join(&at, &field);
            let text = value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty());
            match field.as_str() {
                "url" if text.is_some_and(|url| !admitted_address(url)) => out.push(finding(
                    &field_at,
                    FindingKind::InvalidValue,
                    format!(
                        "server `{name}`: `url` is an https address, or an http address on this computer's loopback; a login would cross the network unprotected"
                    ),
                )),
                "description" if text.is_some_and(|text| text.contains(['\n', '\r'])) => {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!("server `{name}`: `description` is one line"),
                    ));
                }
                "url" | "description" if text.is_some() => {}
                "url" | "description" => out.push(finding(
                    &field_at,
                    FindingKind::InvalidValue,
                    format!("server `{name}`: `{field}` expects nonempty text"),
                )),
                "speaker_login" if layer != LayerKind::Company => out.push(finding(
                    &field_at,
                    FindingKind::NotPermittedInLayer,
                    format!(
                        "server `{name}`: `speaker_login` is declared in a company home only"
                    ),
                )),
                "speaker_login" => {
                    if !matches!(value.as_str(), Some("stored" | "sign-in")) {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            format!(
                                "server `{name}`: `speaker_login` expects `stored` or `sign-in`"
                            ),
                        ));
                    }
                }
                "credential" if layer != LayerKind::Company => out.push(finding(
                    &field_at,
                    FindingKind::NotPermittedInLayer,
                    format!(
                        "server `{name}`: `credential` names a member's own login and is declared in a company home only"
                    ),
                )),
                "credential" if text.is_some() => {}
                "credential" => out.push(finding(
                    &field_at,
                    FindingKind::InvalidValue,
                    format!(
                        "server `{name}`: `credential` expects the name under members.<member>.credentials"
                    ),
                )),
                "sign_in" if layer != LayerKind::Company => out.push(finding(
                    &field_at,
                    FindingKind::NotPermittedInLayer,
                    format!("server `{name}`: `sign_in` is declared in a company home only"),
                )),
                "sign_in" => {
                    if !is_sign_in {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            format!(
                                "server `{name}`: `sign_in` requires `speaker_login: sign-in`"
                            ),
                        ));
                    } else {
                        check_sign_in(&name, value, &field_at, out);
                    }
                }
                "login" => {
                    if !value.as_str().is_some_and(super::is_secret_reference) {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            format!(
                                "server `{name}`: `login` expects an exact ${{NAME}} reference into the secrets file; a literal login is not stored"
                            ),
                        ));
                    } else if layer == LayerKind::Company && !has_credential && !is_sign_in {
                        out.push(finding(
                            &field_at,
                            FindingKind::Missing,
                            format!(
                                "server `{name}`: a company's `login` requires `credential` or `speaker_login: sign-in`"
                            ),
                        ));
                    }
                }
                _ => out.push(finding(
                    &field_at,
                    FindingKind::UnknownKey,
                    format!(
                        "server `{name}`: unknown field; a server takes url, description, speaker_login, credential, sign_in and login"
                    ),
                )),
            }
        }
    }
}

fn check_sign_in(server: &str, value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(fields) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            format!("server `{server}`: `sign_in` expects a mapping"),
        ));
        return;
    };
    if fields.contains_key("client") && fields.contains_key("client_ref") {
        out.push(finding(
            &join(path, "client_ref"),
            FindingKind::InvalidValue,
            format!(
                "server `{server}`: `sign_in.client_ref` and `sign_in.client` are mutually exclusive"
            ),
        ));
    }
    for (field, value) in fields {
        let field = key_text(field);
        let field_at = join(path, &field);
        match field.as_str() {
            "mode" => {
                if !matches!(value.as_str(), Some("authorization-code" | "issuer")) {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!(
                            "server `{server}`: `sign_in.mode` expects `authorization-code` or `issuer`"
                        ),
                    ));
                }
            }
            "scopes" => {
                let valid = value.as_sequence().is_some_and(|seq| {
                    seq.iter()
                        .all(|item| item.as_str().is_some_and(|s| !s.trim().is_empty()))
                });
                if !valid {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!("server `{server}`: `sign_in.scopes` expects a list of nonempty scope strings"),
                    ));
                }
            }
            "client_ref" => {
                let valid = value
                    .as_str()
                    .is_some_and(|s| ProjectSlug::parse(s.trim()).is_ok());
                if !valid {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!(
                            "server `{server}`: `sign_in.client_ref` expects a global mcp_clients.<name> key"
                        ),
                    ));
                }
            }
            "client" => check_sign_in_client(server, value, &field_at, out),
            "accept_return_without_issuer" => {
                if !value.is_bool() {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!(
                            "server `{server}`: `sign_in.accept_return_without_issuer` expects true or false"
                        ),
                    ));
                }
            }
            "identity" => check_sign_in_identity(server, value, &field_at, out),
            _ => out.push(finding(
                &field_at,
                FindingKind::UnknownKey,
                format!(
                    "server `{server}`: unknown `sign_in` field; `sign_in` takes mode, scopes, client_ref, client, accept_return_without_issuer and identity"
                ),
            )),
        }
    }
}

fn check_sign_in_client(server: &str, value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(fields) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            format!("server `{server}`: `sign_in.client` expects a mapping with `id` and optional `secret`"),
        ));
        return;
    };
    if !fields.contains_key("id") {
        out.push(finding(
            &join(path, "id"),
            FindingKind::Missing,
            format!("server `{server}`: `sign_in.client` requires `id`"),
        ));
    }
    for (field, value) in fields {
        let field = key_text(field);
        let field_at = join(path, &field);
        match field.as_str() {
            "id" | "secret" => {
                if !value.as_str().is_some_and(super::is_secret_reference) {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!(
                            "server `{server}`: `sign_in.client.{field}` expects an exact ${{NAME}} reference into the company home's secrets.env; a literal value is not stored"
                        ),
                    ));
                }
            }
            _ => out.push(finding(
                &field_at,
                FindingKind::UnknownKey,
                format!("server `{server}`: unknown `sign_in.client` field; takes id and secret"),
            )),
        }
    }
}

fn check_sign_in_identity(server: &str, value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(fields) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            format!("server `{server}`: `sign_in.identity` expects a mapping with `url` and `email_field`"),
        ));
        return;
    };
    for required in ["url", "email_field"] {
        if !fields.contains_key(required) {
            out.push(finding(
                &join(path, required),
                FindingKind::Missing,
                format!("server `{server}`: `sign_in.identity` requires `{required}`"),
            ));
        }
    }
    for (field, value) in fields {
        let field = key_text(field);
        let field_at = join(path, &field);
        match field.as_str() {
            "url" => {
                let valid = value
                    .as_str()
                    .map(str::trim)
                    .is_some_and(admitted_identity_url);
                if !valid {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!(
                            "server `{server}`: `sign_in.identity.url` expects an absolute https address without credentials or fragment"
                        ),
                    ));
                }
            }
            "email_field" => {
                let valid = value.as_str().map(str::trim).is_some_and(|field| {
                    !field.is_empty() && field.split('.').all(|seg| !seg.is_empty())
                });
                if !valid {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!(
                            "server `{server}`: `sign_in.identity.email_field` expects a nonempty dot-separated JSON field path"
                        ),
                    ));
                }
            }
            _ => out.push(finding(
                &field_at,
                FindingKind::UnknownKey,
                format!(
                    "server `{server}`: unknown `sign_in.identity` field; takes url and email_field"
                ),
            )),
        }
    }
}

/// Cross-document checks for a company home's `mcp` declarations:
/// - `sign_in.client_ref` must name a declared global `mcp_clients.<name>` key.
/// - `sign_in.mode: issuer` requires `issuer.enabled: true` in the company home.
pub(super) fn check_company_cross_refs(
    company_doc: &Value,
    global_doc: Option<&Value>,
    out: &mut Vec<KeyFinding>,
) {
    let Some(servers) = company_doc.get("mcp").and_then(Value::as_mapping) else {
        return;
    };
    let global_clients = global_doc
        .and_then(|doc| doc.get("mcp_clients"))
        .and_then(Value::as_mapping);
    let issuer_enabled = company_doc
        .get("issuer")
        .and_then(|issuer| issuer.get("enabled"))
        .and_then(Value::as_bool)
        == Some(true);

    for (name, declaration) in servers {
        let name = key_text(name);
        let Some(sign_in) = declaration.get("sign_in").and_then(Value::as_mapping) else {
            continue;
        };
        if let Some(client_ref) = sign_in
            .get("client_ref")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let exists = global_clients
                .is_some_and(|clients| clients.contains_key(Value::String(client_ref.to_owned())));
            if !exists {
                out.push(finding(
                    &format!("mcp.{name}.sign_in.client_ref"),
                    FindingKind::UnresolvedReference,
                    format!(
                        "server `{name}`: `sign_in.client_ref` names `{client_ref}`, which is not declared under global `mcp_clients`"
                    ),
                ));
            }
        }
        if sign_in.get("mode").and_then(Value::as_str) == Some("issuer") && !issuer_enabled {
            out.push(finding(
                &format!("mcp.{name}.sign_in.mode"),
                FindingKind::InvalidValue,
                format!(
                    "server `{name}`: `sign_in.mode: issuer` requires `issuer.enabled: true` in this company home; initialize it with `instrukt mcp issuer init --company <name>`"
                ),
            ));
        }
    }
}

/// The servers an `mcp` value declared in `layer` names, by name.
pub(super) fn servers(
    raw: Option<&Value>,
    layer: LayerKind,
) -> Result<BTreeMap<ProjectSlug, McpServer>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let mut found = Vec::new();
    check(raw, "mcp", layer, &mut found);
    if let Some(first) = found.first() {
        return Err(ConfigError::InvalidValue {
            key: first.key.clone(),
            message: first.message.clone(),
        });
    }
    let mut servers = BTreeMap::new();
    for (name, declaration) in raw.as_mapping().into_iter().flatten() {
        let name = ProjectSlug::parse(&key_text(name)).map_err(|e| ConfigError::InvalidValue {
            key: "mcp".into(),
            message: e.to_string(),
        })?;
        let text = |field: &str| {
            declaration
                .get(field)
                .and_then(Value::as_str)
                .map(|text| text.trim().to_owned())
        };
        let speaker_login = match text("speaker_login").as_deref() {
            Some("sign-in") => SpeakerLogin::SignIn,
            _ => SpeakerLogin::Stored,
        };
        let sign_in = declaration
            .get("sign_in")
            .map(parse_sign_in)
            .or_else(|| (speaker_login == SpeakerLogin::SignIn).then(McpSignIn::default));
        servers.insert(
            name.clone(),
            McpServer {
                name,
                url: text("url").unwrap_or_default(),
                description: text("description").unwrap_or_default(),
                speaker_login,
                credential: text("credential"),
                login: text("login"),
                sign_in,
            },
        );
    }
    Ok(servers)
}

fn parse_sign_in(raw: &Value) -> McpSignIn {
    let mode = match raw.get("mode").and_then(Value::as_str) {
        Some("issuer") => SignInMode::Issuer,
        _ => SignInMode::AuthorizationCode,
    };
    let scopes = raw
        .get("scopes")
        .and_then(Value::as_sequence)
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_owned()))
                .collect()
        })
        .unwrap_or_default();
    let client_ref = raw
        .get("client_ref")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_owned());
    let client = raw.get("client").map(|c| SignInClient {
        id: c
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned(),
        secret: c
            .get("secret")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_owned()),
    });
    let accept_return_without_issuer = raw
        .get("accept_return_without_issuer")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let identity = raw.get("identity").map(|id| SignInIdentity {
        url: id
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned(),
        email_field: id
            .get("email_field")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned(),
    });
    McpSignIn {
        mode,
        scopes,
        client_ref,
        client,
        accept_return_without_issuer,
        identity,
    }
}

/// The reusable host-level OAuth clients declared under global `mcp_clients`.
pub(super) fn clients(
    raw: Option<&Value>,
) -> Result<BTreeMap<ProjectSlug, McpClient>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let mut found = Vec::new();
    check_clients(raw, "mcp_clients", &mut found);
    if let Some(first) = found.first() {
        return Err(ConfigError::InvalidValue {
            key: first.key.clone(),
            message: first.message.clone(),
        });
    }
    let mut clients = BTreeMap::new();
    for (name, declaration) in raw.as_mapping().into_iter().flatten() {
        let name = ProjectSlug::parse(&key_text(name)).map_err(|e| ConfigError::InvalidValue {
            key: "mcp_clients".into(),
            message: e.to_string(),
        })?;
        let text = |field: &str| {
            declaration
                .get(field)
                .and_then(Value::as_str)
                .map(|text| text.trim().to_owned())
        };
        clients.insert(
            name.clone(),
            McpClient {
                name,
                id: text("id").unwrap_or_default(),
                secret: text("secret"),
                description: text("description"),
            },
        );
    }
    Ok(clients)
}

/// A company home's `issuer` block, when declared.
pub(super) fn company_issuer(
    raw: Option<&Value>,
) -> Result<Option<CompanyIssuerConfig>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut found = Vec::new();
    check_issuer(raw, "issuer", &mut found);
    if let Some(first) = found.first() {
        return Err(ConfigError::InvalidValue {
            key: first.key.clone(),
            message: first.message.clone(),
        });
    }
    let enabled = raw.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let signing_key = raw
        .get("signing_key")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_owned());
    Ok(Some(CompanyIssuerConfig {
        enabled,
        signing_key,
    }))
}

/// JSON Schema for `mcp` in `layer`.
pub(super) fn mcp_json_schema(layer: LayerKind) -> serde_json::Value {
    let secret_ref = json!({"type": "string", "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$"});
    let mut properties = serde_json::Map::new();
    properties.insert(
        "url".into(),
        json!({
            "type": "string",
            "description": "Remote Streamable HTTP MCP server URL (https, or loopback http)."
        }),
    );
    properties.insert(
        "description".into(),
        json!({
            "type": "string",
            "description": "One-line description of what this MCP server is for."
        }),
    );
    properties.insert(
        "login".into(),
        json!({
            "type": "string",
            "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$",
            "description": "Exact ${NAME} reference into secrets.env (set with `instrukt secrets set`)."
        }),
    );
    if layer == LayerKind::Company {
        properties.insert(
            "speaker_login".into(),
            json!({
                "enum": ["stored", "sign-in"],
                "default": "stored",
                "description": "How a speaker's login is obtained: `stored` (per-member credential) or `sign-in` (OAuth or built-in OIDC issuer)."
            }),
        );
        properties.insert(
            "credential".into(),
            json!({
                "type": "string",
                "description": "With `speaker_login: stored`, the credential key under `members.<member>.credentials`."
            }),
        );
        properties.insert(
            "sign_in".into(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "description": "Sign-in options when `speaker_login: sign-in`.",
                "properties": {
                    "mode": {
                        "enum": ["authorization-code", "issuer"],
                        "default": "authorization-code",
                        "description": "`authorization-code` (interactive browser sign-in) or `issuer` (zero-click built-in per-company OIDC issuer `jwt-bearer`)."
                    },
                    "scopes": {
                        "type": "array",
                        "items": {"type": "string", "minLength": 1},
                        "description": "OAuth scopes requested during sign-in."
                    },
                    "client_ref": {
                        "type": "string",
                        "description": "Key name of a reusable host-level OAuth client declared under global `mcp_clients.<name>`. Mutually exclusive with `client`."
                    },
                    "client": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["id"],
                        "description": "Company-local pre-registered OAuth client. Mutually exclusive with `client_ref`.",
                        "properties": {
                            "id": secret_ref,
                            "secret": secret_ref
                        }
                    },
                    "accept_return_without_issuer": {
                        "type": "boolean",
                        "default": false,
                        "description": "Accept an OAuth return without RFC 9207 `iss` parameter."
                    },
                    "identity": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["url", "email_field"],
                        "description": "Identity lookup request for non-OpenID Provider authorization servers.",
                        "properties": {
                            "url": {"type": "string", "description": "Absolute https URL without credentials or fragment."},
                            "email_field": {"type": "string", "description": "Dot-separated JSON field path to the email address."}
                        }
                    }
                }
            }),
        );
    }
    json!({
        "type": "object",
        "additionalProperties": {
            "type": "object",
            "additionalProperties": false,
            "required": ["url", "description"],
            "properties": properties
        }
    })
}

/// JSON Schema for global `mcp_clients`.
pub(super) fn mcp_clients_json_schema() -> serde_json::Value {
    let secret_ref = json!({"type": "string", "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$"});
    json!({
        "type": "object",
        "description": "Reusable host-level OAuth clients shared across companies via `sign_in.client_ref`.",
        "additionalProperties": {
            "type": "object",
            "additionalProperties": false,
            "required": ["id"],
            "properties": {
                "id": {
                    "type": "string",
                    "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$",
                    "description": "Required ${NAME} reference into ~/.instrukt/secrets.env (set with `instrukt secrets set`)."
                },
                "secret": secret_ref,
                "description": {
                    "type": "string",
                    "description": "Optional one-line description of this shared host OAuth client."
                }
            }
        }
    })
}

/// JSON Schema for company `issuer`.
pub(super) fn issuer_json_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["enabled"],
        "description": "Built-in lightweight per-company OIDC issuer for `sign_in.mode: issuer`.",
        "properties": {
            "enabled": {
                "type": "boolean",
                "description": "Whether this company's built-in OIDC issuer is enabled."
            },
            "signing_key": {
                "type": "string",
                "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$",
                "description": "Optional ${NAME} reference into the company home's secrets.env holding the ES256 private signing key (set with `instrukt mcp issuer init --company <name>`)."
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn judged(text: &str, layer: LayerKind) -> Vec<(String, FindingKind)> {
        let mut out = Vec::new();
        check(
            &serde_yaml_ng::from_str(text).unwrap(),
            "mcp",
            layer,
            &mut out,
        );
        out.into_iter().map(|f| (f.key, f.kind)).collect()
    }

    #[test]
    fn a_complete_declaration_has_no_findings_in_either_place() {
        let company = "erp: {url: 'https://erp.example/mcp', description: ERP, credential: erp, login: '${ERP_LOGIN}'}\nwiki: {url: 'https://wiki.example/mcp', description: Wiki}\n";
        assert_eq!(judged(company, LayerKind::Company), []);
        let machine = "search: {url: 'https://search.example/mcp', description: Search, login: '${SEARCH_LOGIN}'}\n";
        assert_eq!(judged(machine, LayerKind::Global), []);
    }

    #[test]
    fn misdeclared_servers_are_findings_naming_the_server_and_the_field() {
        use FindingKind::{InvalidValue, Missing, NotPermittedInLayer, UnknownKey};
        assert_eq!(
            judged(
                "a: {url: 'https://a.example', description: A, login: plain, extra: 1}\nb: {description: B, login: '${B}'}\n",
                LayerKind::Company
            ),
            [
                ("mcp.a.login".to_owned(), InvalidValue),
                ("mcp.a.extra".to_owned(), UnknownKey),
                ("mcp.b.url".to_owned(), Missing),
                ("mcp.b.login".to_owned(), Missing),
            ]
        );
        assert_eq!(
            judged(
                "c: {url: 'https://c.example', description: C, credential: c}\n",
                LayerKind::Global
            ),
            [("mcp.c.credential".to_owned(), NotPermittedInLayer)]
        );
    }

    #[test]
    fn a_login_goes_only_to_https_or_this_computers_loopback() {
        for admitted in [
            "https://erp.example/mcp",
            "http://127.0.0.1:8080/mcp",
            "http://localhost/mcp",
            "http://[::1]:9000/mcp",
        ] {
            assert!(admitted_address(admitted));
        }
        for refused in [
            "http://erp.example/mcp",
            "http://10.0.0.5/mcp",
            "ftp://erp.example/mcp",
            "erp.example/mcp",
        ] {
            assert!(!admitted_address(refused));
        }
        assert_eq!(
            judged(
                "a: {url: 'http://erp.example/mcp', description: A}\nb: {url: 'https://b.example', description: \"two\\nlines\"}\n",
                LayerKind::Company
            ),
            [
                ("mcp.a.url".to_owned(), FindingKind::InvalidValue),
                ("mcp.b.description".to_owned(), FindingKind::InvalidValue),
            ]
        );
    }

    #[test]
    fn declared_servers_are_read_with_their_logins_as_references() {
        let servers = servers(
            Some(&serde_yaml_ng::from_str(
                "erp: {url: 'https://erp.example/mcp', description: ERP, credential: erp, login: '${ERP_LOGIN}'}\nwiki: {url: 'https://wiki.example/mcp', description: Wiki}\n",
            )
            .unwrap()),
            LayerKind::Company,
        )
        .unwrap();
        let erp = &servers[&ProjectSlug::parse("erp").unwrap()];
        assert_eq!(erp.credential.as_deref(), Some("erp"));
        assert!(
            erp.login
                .as_deref()
                .is_some_and(crate::config::is_secret_reference)
        );
        assert!(servers[&ProjectSlug::parse("wiki").unwrap()].is_open());
    }

    #[test]
    fn sign_in_client_ref_and_issuer_mode_validate_cleanly() {
        let company_yaml = "drive:\n  url: 'https://drive.example/mcp'\n  description: Drive\n  speaker_login: sign-in\n  sign_in:\n    client_ref: google\n    scopes: ['drive.readonly']\nema:\n  url: 'https://ema.example/mcp'\n  description: EMA\n  speaker_login: sign-in\n  sign_in:\n    mode: issuer\n";
        assert_eq!(judged(company_yaml, LayerKind::Company), []);

        let conflict = "drive:\n  url: 'https://drive.example/mcp'\n  description: Drive\n  speaker_login: sign-in\n  sign_in:\n    client_ref: google\n    client: {id: '${ID}'}\n";
        assert_eq!(
            judged(conflict, LayerKind::Company),
            [(
                "mcp.drive.sign_in.client_ref".to_owned(),
                FindingKind::InvalidValue
            )]
        );
    }

    #[test]
    fn mcp_clients_and_cross_refs_reject_missing_global_client_and_disabled_issuer() {
        let mut out = Vec::new();
        let clients_val: Value = serde_yaml_ng::from_str(
            "google: {id: '${GOOGLE_ID}', secret: '${GOOGLE_SECRET}', description: Google}\n",
        )
        .unwrap();
        check_clients(&clients_val, "mcp_clients", &mut out);
        assert!(out.is_empty());

        let company_doc: Value = serde_yaml_ng::from_str(
            "mcp:\n  drive: {url: 'https://d.example/mcp', description: D, speaker_login: sign-in, sign_in: {client_ref: missing}}\n  ema: {url: 'https://e.example/mcp', description: E, speaker_login: sign-in, sign_in: {mode: issuer}}\n",
        )
        .unwrap();
        let global_doc: Value =
            serde_yaml_ng::from_str("mcp_clients: {google: {id: '${GOOGLE_ID}'}}\n").unwrap();
        check_company_cross_refs(&company_doc, Some(&global_doc), &mut out);
        let pairs: Vec<_> = out.into_iter().map(|f| (f.key, f.kind)).collect();
        assert_eq!(
            pairs,
            [
                (
                    "mcp.drive.sign_in.client_ref".to_owned(),
                    FindingKind::UnresolvedReference
                ),
                ("mcp.ema.sign_in.mode".to_owned(), FindingKind::InvalidValue),
            ]
        );
    }
}
