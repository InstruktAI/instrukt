//! Live RFC 9728 Protected Resource Metadata + RFC 8414 / OpenID Connect
//! Authorization Server discovery (`mcp-access`, "Discovery" &
//! "Live OAuth discovery and ingress reachability check").
//!
//! Writes no configuration or state.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use super::McpError;

/// IETF Identity Assertion JWT Authorization Grant type advertised by EMA servers.
pub const JWT_BEARER_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_METADATA_BYTES: u64 = 1024 * 1024;

/// Public report emitted by `instrukt mcp discover <url> [--json]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub url: String,
    pub resource: Option<String>,
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Vec<String>,
    pub registration_supported: bool,
    pub pkce_s256_supported: bool,
    pub iss_parameter_supported: bool,
    pub openid_provider: bool,
    pub ema_jwt_bearer_supported: bool,
}

/// Full discovered authorization server metadata used internally by sign-in
/// and issuer token exchange flows.
#[derive(Debug, Clone)]
pub(crate) struct DiscoveredAuth {
    pub report: DiscoveryReport,
    pub issuer: String,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub registration_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
    pub userinfo_endpoint: Option<String>,
    pub jwks_uri: Option<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into()
}

/// Probe a remote MCP server's HTTPS (or loopback HTTP) URL live and report
/// its RFC 9728 / RFC 8414 / OIDC / EMA capabilities. Writes nothing.
pub fn discover(url: &str) -> Result<DiscoveryReport, McpError> {
    let parsed = validate_probe_url(url)?;
    let agent = http_agent();
    let prm_hint = probe_www_authenticate(&agent, parsed.as_str());
    let prm = fetch_protected_resource_metadata(&agent, &parsed, prm_hint.as_deref());

    let (resource, authorization_servers, mut scopes_supported) = match prm {
        Some(doc) => {
            let resource = doc
                .get("resource")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let servers = string_list(doc.get("authorization_servers"));
            let scopes = string_list(doc.get("scopes_supported"));
            (resource, servers, scopes)
        }
        None => {
            let origin = origin_string(&parsed);
            (None, vec![origin], Vec::new())
        }
    };

    let selected_issuer = authorization_servers.first().cloned();
    let (oauth_doc, oidc_doc) = match selected_issuer.as_deref() {
        Some(issuer) => fetch_auth_server_metadata(&agent, issuer),
        None => (None, None),
    };

    if scopes_supported.is_empty() {
        scopes_supported = oauth_doc
            .as_ref()
            .or(oidc_doc.as_ref())
            .map(|doc| string_list(doc.get("scopes_supported")))
            .unwrap_or_default();
    }

    let primary = oauth_doc.as_ref().or(oidc_doc.as_ref());
    let registration_supported = primary
        .and_then(|d| d.get("registration_endpoint"))
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    let pkce_s256_supported = primary
        .map(|d| string_list(d.get("code_challenge_methods_supported")))
        .is_some_and(|methods| methods.iter().any(|m| m == "S256"));
    let iss_parameter_supported = primary
        .and_then(|d| d.get("authorization_response_iss_parameter_supported"))
        .and_then(Value::as_bool)
        == Some(true);
    let openid_provider = oidc_doc.as_ref().or(oauth_doc.as_ref()).is_some_and(|d| {
        d.get("userinfo_endpoint")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
            && d.get("jwks_uri")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
    });
    let ema_jwt_bearer_supported = primary
        .map(|d| string_list(d.get("grant_types_supported")))
        .is_some_and(|grants| grants.iter().any(|g| g == JWT_BEARER_GRANT_TYPE));

    let effective_auth_servers = if oauth_doc.is_some() || oidc_doc.is_some() || resource.is_some()
    {
        authorization_servers
    } else {
        Vec::new()
    };

    Ok(DiscoveryReport {
        url: url.to_owned(),
        resource,
        authorization_servers: effective_auth_servers,
        scopes_supported,
        registration_supported,
        pkce_s256_supported,
        iss_parameter_supported,
        openid_provider,
        ema_jwt_bearer_supported,
    })
}

/// Discover authorization server metadata for sign-in / issuer token exchange,
/// enforcing RFC 9728 `resource` and RFC 8414 `issuer` matching.
pub(crate) fn discover_auth(url: &str) -> Result<DiscoveredAuth, McpError> {
    let parsed = validate_probe_url(url)?;
    let agent = http_agent();
    let prm_hint = probe_www_authenticate(&agent, parsed.as_str());
    let prm = fetch_protected_resource_metadata(&agent, &parsed, prm_hint.as_deref()).ok_or_else(
        || McpError::Server {
            server: url.to_owned(),
            cause: "server published no readable RFC 9728 protected resource metadata".into(),
        },
    )?;

    let resource = prm
        .get("resource")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if let Some(ref declared) = resource
        && declared.trim_end_matches('/') != url.trim_end_matches('/')
    {
        return Err(McpError::Server {
            server: url.to_owned(),
            cause: format!(
                "protected resource metadata `resource` (`{declared}`) does not match server URL `{url}`"
            ),
        });
    }

    let authorization_servers = string_list(prm.get("authorization_servers"));
    let issuer = authorization_servers
        .first()
        .cloned()
        .ok_or_else(|| McpError::Server {
            server: url.to_owned(),
            cause: "protected resource metadata lists no authorization_servers".into(),
        })?;
    let (oauth_doc, oidc_doc) = fetch_auth_server_metadata(&agent, &issuer);
    let primary = oauth_doc
        .as_ref()
        .or(oidc_doc.as_ref())
        .ok_or_else(|| McpError::Server {
            server: url.to_owned(),
            cause: format!("authorization server `{issuer}` published no valid discovery metadata"),
        })?;

    let mut scopes_supported = string_list(prm.get("scopes_supported"));
    if scopes_supported.is_empty() {
        scopes_supported = string_list(primary.get("scopes_supported"));
    }

    let get_str = |key: &str| {
        primary
            .get(key)
            .or_else(|| oidc_doc.as_ref().and_then(|d| d.get(key)))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let registration_endpoint = get_str("registration_endpoint");
    let userinfo_endpoint = oidc_doc
        .as_ref()
        .or(oauth_doc.as_ref())
        .and_then(|d| d.get("userinfo_endpoint"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let jwks_uri = oidc_doc
        .as_ref()
        .or(oauth_doc.as_ref())
        .and_then(|d| d.get("jwks_uri"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let pkce_s256_supported = string_list(primary.get("code_challenge_methods_supported"))
        .iter()
        .any(|m| m == "S256");
    let iss_parameter_supported = primary
        .get("authorization_response_iss_parameter_supported")
        .and_then(Value::as_bool)
        == Some(true);
    let openid_provider = userinfo_endpoint.is_some() && jwks_uri.is_some();
    let ema_jwt_bearer_supported = string_list(primary.get("grant_types_supported"))
        .iter()
        .any(|g| g == JWT_BEARER_GRANT_TYPE);

    let report = DiscoveryReport {
        url: url.to_owned(),
        resource,
        authorization_servers,
        scopes_supported,
        registration_supported: registration_endpoint.is_some(),
        pkce_s256_supported,
        iss_parameter_supported,
        openid_provider,
        ema_jwt_bearer_supported,
    };

    Ok(DiscoveredAuth {
        report,
        issuer,
        authorization_endpoint: get_str("authorization_endpoint"),
        token_endpoint: get_str("token_endpoint"),
        registration_endpoint,
        revocation_endpoint: get_str("revocation_endpoint"),
        userinfo_endpoint,
        jwks_uri,
        token_endpoint_auth_methods_supported: string_list(
            primary.get("token_endpoint_auth_methods_supported"),
        ),
    })
}

fn validate_probe_url(url: &str) -> Result<Url, McpError> {
    if !instrukt_core::config::mcp_address_admitted(url) {
        return Err(McpError::Server {
            server: url.to_owned(),
            cause: "address must be https (or loopback http)".into(),
        });
    }
    Url::parse(url).map_err(|error| McpError::Server {
        server: url.to_owned(),
        cause: format!("invalid URL: {error}"),
    })
}

fn origin_string(url: &Url) -> String {
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

/// Probe the MCP endpoint without credentials to read any `WWW-Authenticate`
/// `resource_metadata="..."` parameter per RFC 9728.
fn probe_www_authenticate(agent: &ureq::Agent, url: &str) -> Option<String> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": super::PROTOCOL_VERSIONS[0],
            "capabilities": {},
            "clientInfo": {"name": "instrukt", "version": env!("CARGO_PKG_VERSION")},
        }
    })
    .to_string();
    let response = agent
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .send(body)
        .ok()?;
    let header = response
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())?;
    parse_resource_metadata_param(header)
}

/// Extract `resource_metadata="..."` from a `WWW-Authenticate` header value.
pub(crate) fn parse_resource_metadata_param(header: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let key = "resource_metadata=";
    let pos = lower.find(key)?;
    let rest = header[pos + key.len()..].trim_start();
    if let Some(after_quote) = rest.strip_prefix('"') {
        let end = after_quote.find('"')?;
        let val = after_quote[..end].trim();
        (!val.is_empty()).then(|| val.to_owned())
    } else {
        let end = rest
            .find(|c: char| c == ',' || c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let val = rest[..end].trim();
        (!val.is_empty()).then(|| val.to_owned())
    }
}

fn fetch_protected_resource_metadata(
    agent: &ureq::Agent,
    server_url: &Url,
    hint: Option<&str>,
) -> Option<Value> {
    let mut candidates = Vec::new();
    if let Some(hint) = hint
        && instrukt_core::config::mcp_address_admitted(hint)
    {
        candidates.push(hint.to_owned());
    }
    let origin = origin_string(server_url);
    let path = server_url.path().trim_matches('/');
    if !path.is_empty() {
        candidates.push(format!(
            "{origin}/.well-known/oauth-protected-resource/{path}"
        ));
    }
    candidates.push(format!("{origin}/.well-known/oauth-protected-resource"));

    for candidate in candidates {
        if let Some(doc) = fetch_json(agent, &candidate)
            && doc.is_object()
        {
            return Some(doc);
        }
    }
    None
}

fn fetch_auth_server_metadata(agent: &ureq::Agent, issuer: &str) -> (Option<Value>, Option<Value>) {
    let Ok(parsed) = validate_probe_url(issuer) else {
        return (None, None);
    };
    let origin = origin_string(&parsed);
    let path = parsed.path().trim_matches('/');

    let oauth_candidates = if path.is_empty() {
        vec![format!("{origin}/.well-known/oauth-authorization-server")]
    } else {
        vec![format!(
            "{origin}/.well-known/oauth-authorization-server/{path}"
        )]
    };
    let oidc_candidates = if path.is_empty() {
        vec![format!("{origin}/.well-known/openid-configuration")]
    } else {
        vec![
            format!("{origin}/.well-known/openid-configuration/{path}"),
            format!("{origin}/{path}/.well-known/openid-configuration"),
        ]
    };

    let fetch_matching = |urls: Vec<String>| -> Option<Value> {
        for url in urls {
            let Some(doc) = fetch_json(agent, &url) else {
                continue;
            };
            let matches_issuer = doc
                .get("issuer")
                .and_then(Value::as_str)
                .is_some_and(|found| found.trim_end_matches('/') == issuer.trim_end_matches('/'));
            if matches_issuer {
                return Some(doc);
            }
        }
        None
    };

    let oauth = fetch_matching(oauth_candidates);
    let oidc = fetch_matching(oidc_candidates);
    (oauth, oidc)
}

fn fetch_json(agent: &ureq::Agent, url: &str) -> Option<Value> {
    if !instrukt_core::config::mcp_address_admitted(url) {
        return None;
    }
    let mut response = agent
        .get(url)
        .header("Accept", "application/json")
        .call()
        .ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_METADATA_BYTES)
        .read_to_string()
        .ok()?;
    serde_json::from_str(&text).ok()
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::parse_resource_metadata_param;

    #[test]
    fn resource_metadata_parameter_is_extracted_from_www_authenticate() {
        assert_eq!(
            parse_resource_metadata_param(
                r#"Bearer realm="mcp", resource_metadata="https://api.example/.well-known/oauth-protected-resource""#
            ),
            Some("https://api.example/.well-known/oauth-protected-resource".into())
        );
        assert_eq!(parse_resource_metadata_param("Bearer realm=\"mcp\""), None);
    }
}
