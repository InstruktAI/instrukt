//! Live RFC 9728 Protected Resource Metadata + RFC 8414 / OpenID Connect
//! Authorization Server discovery via `rmcp::transport::auth::AuthorizationManager`.
//!
//! Writes no configuration or state.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationMetadata, AuthorizationMetadataSource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use super::_http::{AdmittedOAuthHttpClient, CapturedResourceMetadata, run_async, string_list};
use super::McpError;

/// IETF Identity Assertion JWT Authorization Grant type advertised by EMA servers.
pub const JWT_BEARER_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

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

/// Full discovered authorization server metadata used internally by sign-in,
/// dynamic registration, and issuer token exchange flows.
#[derive(Debug, Clone)]
pub(crate) struct DiscoveredAuth {
    pub report: DiscoveryReport,
    pub metadata: AuthorizationMetadata,
    pub issuer: String,
}

impl DiscoveredAuth {
    pub(crate) fn authorization_endpoint(&self) -> Option<&str> {
        non_empty(&self.metadata.authorization_endpoint)
    }

    pub(crate) fn token_endpoint(&self) -> Option<&str> {
        non_empty(&self.metadata.token_endpoint)
    }

    pub(crate) fn registration_endpoint(&self) -> Option<&str> {
        self.metadata
            .registration_endpoint
            .as_deref()
            .and_then(non_empty)
    }

    pub(crate) fn revocation_endpoint(&self) -> Option<&str> {
        additional_str(&self.metadata, "revocation_endpoint")
    }

    pub(crate) fn userinfo_endpoint(&self) -> Option<&str> {
        additional_str(&self.metadata, "userinfo_endpoint")
    }

    pub(crate) fn jwks_uri(&self) -> Option<&str> {
        self.metadata.jwks_uri.as_deref().and_then(non_empty)
    }

    pub(crate) fn token_endpoint_auth_methods(&self) -> Vec<String> {
        string_list(
            self.metadata
                .additional_fields
                .get("token_endpoint_auth_methods_supported"),
        )
    }
}

/// Builds a configured `rmcp` `AuthorizationManager` backed by
/// `AdmittedOAuthHttpClient` and seeded with discovered `metadata`.
pub(crate) fn auth_manager(
    server_url: &str,
    auth: &DiscoveredAuth,
) -> Result<AuthorizationManager, McpError> {
    let parsed = validate_probe_url(server_url)?;
    let http = AdmittedOAuthHttpClient::new(server_url, CONNECT_TIMEOUT, REQUEST_TIMEOUT)?;
    run_async(server_url, async {
        let mut manager = AuthorizationManager::new_with_oauth_http_client(parsed, Arc::new(http))
            .await
            .map_err(|error| McpError::Server {
                server: server_url.to_owned(),
                cause: format!("failed to initialize OAuth manager: {error}"),
            })?;
        manager.set_metadata(auth.metadata.clone());
        Ok(manager)
    })?
}

/// Probe a remote MCP server's HTTPS (or loopback HTTP) URL live and report
/// its RFC 9728 / RFC 8414 / OIDC / EMA capabilities. Writes nothing.
pub fn discover(url: &str) -> Result<DiscoveryReport, McpError> {
    let (resolution, captured) = resolve_live(url)?;
    match resolution {
        Ok(resolved) if resolved.source.is_discovered() => {
            Ok(build_report(url, &captured, &resolved.metadata))
        }
        Ok(_) => Ok(empty_report(url, None, Vec::new(), Vec::new())),
        Err(AuthError::AuthorizationServersUnavailable {
            authorization_servers,
            ..
        }) => Ok(empty_report(
            url,
            captured.resource,
            authorization_servers,
            captured.scopes_supported,
        )),
        Err(_) => {
            let servers = if captured.resource.is_some() {
                captured.authorization_servers
            } else {
                Vec::new()
            };
            Ok(empty_report(
                url,
                captured.resource,
                servers,
                captured.scopes_supported,
            ))
        }
    }
}

/// Discover authorization server metadata for sign-in / issuer token exchange,
/// enforcing RFC 9728 `resource` and RFC 8414 `issuer` matching.
pub(crate) fn discover_auth(url: &str) -> Result<DiscoveredAuth, McpError> {
    let (resolution, captured) = resolve_live(url)?;
    let resolved = resolution.map_err(|error| McpError::Server {
        server: url.to_owned(),
        cause: format!("OAuth metadata discovery failed: {error}"),
    })?;
    if resolved.source != AuthorizationMetadataSource::ProtectedResourceMetadata {
        return Err(McpError::Server {
            server: url.to_owned(),
            cause: "server published no readable RFC 9728 protected resource metadata".into(),
        });
    }
    if let Some(ref declared) = captured.resource
        && declared.trim_end_matches('/') != url.trim_end_matches('/')
    {
        return Err(McpError::Server {
            server: url.to_owned(),
            cause: format!(
                "protected resource metadata `resource` (`{declared}`) does not match server URL `{url}`"
            ),
        });
    }
    let metadata = resolved.metadata;
    let issuer = metadata
        .issuer
        .clone()
        .or_else(|| captured.authorization_servers.first().cloned())
        .ok_or_else(|| McpError::Server {
            server: url.to_owned(),
            cause: "authorization server discovery metadata omits `issuer`".into(),
        })?;
    let report = build_report(url, &captured, &metadata);
    Ok(DiscoveredAuth {
        report,
        metadata,
        issuer,
    })
}

fn resolve_live(
    url: &str,
) -> Result<
    (
        Result<rmcp::transport::auth::AuthorizationMetadataResolution, AuthError>,
        CapturedResourceMetadata,
    ),
    McpError,
> {
    let parsed = validate_probe_url(url)?;
    let prm_slot = Arc::new(Mutex::new(None));
    let http = AdmittedOAuthHttpClient::new(url, CONNECT_TIMEOUT, REQUEST_TIMEOUT)?
        .with_prm_observer(Arc::clone(&prm_slot));
    let resolution = run_async(url, async {
        let manager = AuthorizationManager::new_with_oauth_http_client(parsed, Arc::new(http))
            .await
            .map_err(|error| McpError::Server {
                server: url.to_owned(),
                cause: format!("failed to initialize OAuth discovery: {error}"),
            })?;
        Ok::<_, McpError>(manager.resolve_metadata().await)
    })??;
    let captured = prm_slot
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default();
    Ok((resolution, captured))
}

fn build_report(
    url: &str,
    captured: &CapturedResourceMetadata,
    metadata: &AuthorizationMetadata,
) -> DiscoveryReport {
    let authorization_servers = if !captured.authorization_servers.is_empty() {
        captured.authorization_servers.clone()
    } else {
        metadata.issuer.clone().into_iter().collect()
    };
    let scopes_supported = if !captured.scopes_supported.is_empty() {
        captured.scopes_supported.clone()
    } else {
        metadata.scopes_supported.clone().unwrap_or_default()
    };
    let registration_supported = metadata
        .registration_endpoint
        .as_deref()
        .and_then(non_empty)
        .is_some();
    let pkce_s256_supported = metadata
        .code_challenge_methods_supported
        .as_ref()
        .is_some_and(|methods| methods.iter().any(|m| m == "S256"));
    let iss_parameter_supported = metadata
        .additional_fields
        .get("authorization_response_iss_parameter_supported")
        .and_then(Value::as_bool)
        == Some(true);
    let openid_provider = additional_str(metadata, "userinfo_endpoint").is_some()
        && metadata.jwks_uri.as_deref().and_then(non_empty).is_some();
    let ema_jwt_bearer_supported =
        string_list(metadata.additional_fields.get("grant_types_supported"))
            .iter()
            .any(|g| g == JWT_BEARER_GRANT_TYPE);

    DiscoveryReport {
        url: url.to_owned(),
        resource: captured.resource.clone(),
        authorization_servers,
        scopes_supported,
        registration_supported,
        pkce_s256_supported,
        iss_parameter_supported,
        openid_provider,
        ema_jwt_bearer_supported,
    }
}

fn empty_report(
    url: &str,
    resource: Option<String>,
    authorization_servers: Vec<String>,
    scopes_supported: Vec<String>,
) -> DiscoveryReport {
    DiscoveryReport {
        url: url.to_owned(),
        resource,
        authorization_servers,
        scopes_supported,
        registration_supported: false,
        pkce_s256_supported: false,
        iss_parameter_supported: false,
        openid_provider: false,
        ema_jwt_bearer_supported: false,
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn additional_str<'a>(metadata: &'a AuthorizationMetadata, key: &str) -> Option<&'a str> {
    metadata
        .additional_fields
        .get(key)
        .and_then(Value::as_str)
        .and_then(non_empty)
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
