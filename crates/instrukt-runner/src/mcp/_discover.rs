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

use super::_http::{
    AdmittedOAuthHttpClient, CapturedResourceMetadata, client as http_client, redact_url,
    run_async, sanitize_auth_error, string_list,
};
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
        Ok::<_, McpError>(manager)
    })?
}

struct LiveResolution {
    resolution: Result<rmcp::transport::auth::AuthorizationMetadataResolution, AuthError>,
    captured: CapturedResourceMetadata,
}

/// Probe a remote MCP server's HTTPS (or loopback HTTP) URL live and report
/// its RFC 9728 / RFC 8414 / OIDC / EMA capabilities. Writes nothing.
pub fn discover(url: &str) -> Result<DiscoveryReport, McpError> {
    let LiveResolution {
        resolution,
        captured,
    } = resolve_live(url)?;
    classify_discovery_resolution(url, resolution, captured)
}

fn classify_discovery_resolution(
    url: &str,
    resolution: Result<rmcp::transport::auth::AuthorizationMetadataResolution, AuthError>,
    captured: CapturedResourceMetadata,
) -> Result<DiscoveryReport, McpError> {
    let safe_url = redact_url(url);
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
        Err(error @ (AuthError::DiscoveryRequestFailed { .. } | AuthError::HttpError(_))) => {
            Err(McpError::Unreachable {
                server: safe_url.clone(),
                url: safe_url,
                cause: sanitize_auth_error(&error),
            })
        }
        Err(error) => Err(McpError::Server {
            server: safe_url,
            cause: format!(
                "OAuth metadata discovery failed: {}",
                sanitize_auth_error(&error)
            ),
        }),
    }
}

/// Discover authorization server metadata for sign-in / issuer token exchange,
/// enforcing RFC 9728 `resource` and RFC 8414 `issuer` matching.
pub(crate) fn discover_auth(url: &str) -> Result<DiscoveredAuth, McpError> {
    let LiveResolution {
        resolution,
        captured,
    } = resolve_live(url)?;
    let safe_url = redact_url(url);
    let resolved = resolution.map_err(|error| McpError::Server {
        server: safe_url.clone(),
        cause: format!(
            "OAuth metadata discovery failed: {}",
            sanitize_auth_error(&error)
        ),
    })?;
    if resolved.source != AuthorizationMetadataSource::ProtectedResourceMetadata {
        return Err(McpError::Server {
            server: safe_url,
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

/// Probes the MCP endpoint with an unauthenticated `POST initialize` so servers
/// advertising RFC 9728 `WWW-Authenticate` challenges only on POST requests are
/// discovered, while unreachable/5xx/429 responses surface as
/// `McpError::Unreachable` rather than looking like a non-OAuth server.
async fn probe_post_initialize_challenge(url: &str) -> Result<Option<String>, McpError> {
    let client = http_client(url, CONNECT_TIMEOUT, REQUEST_TIMEOUT, false)?;
    let init_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": super::_client::INITIALIZE,
        "params": {
            "protocolVersion": super::_client::PROTOCOL_VERSIONS[0],
            "capabilities": {},
            "clientInfo": {
                "name": "instrukt",
                "version": env!("CARGO_PKG_VERSION"),
            }
        }
    });
    let safe_url = redact_url(url);
    let response = client
        .post(url)
        .header(http::header::ACCEPT, "application/json, text/event-stream")
        .json(&init_body)
        .send()
        .await
        .map_err(|error| McpError::Unreachable {
            server: safe_url.clone(),
            url: safe_url.clone(),
            cause: error.without_url().to_string(),
        })?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        let mut fallback = None;
        for value in response.headers().get_all(http::header::WWW_AUTHENTICATE) {
            let Ok(text) = value.to_str() else {
                continue;
            };
            if text.contains("resource_metadata") {
                return Ok(Some(text.to_owned()));
            }
            if fallback.is_none() {
                fallback = Some(text.to_owned());
            }
        }
        return Ok(fallback);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return Err(McpError::Unreachable {
            server: safe_url.clone(),
            url: safe_url,
            cause: format!("HTTP {}", status.as_u16()),
        });
    }
    Ok(None)
}

fn resolve_live(url: &str) -> Result<LiveResolution, McpError> {
    let parsed = validate_probe_url(url)?;
    let prm_slot = Arc::new(Mutex::new(None));
    let http = AdmittedOAuthHttpClient::new(url, CONNECT_TIMEOUT, REQUEST_TIMEOUT)?
        .with_prm_observer(Arc::clone(&prm_slot));
    let resolution = run_async(url, async {
        let challenge = probe_post_initialize_challenge(url).await?;
        let manager = AuthorizationManager::new_with_oauth_http_client(parsed, Arc::new(http))
            .await
            .map_err(|error| McpError::Server {
                server: url.to_owned(),
                cause: format!(
                    "failed to initialize OAuth discovery: {}",
                    sanitize_auth_error(&error)
                ),
            })?;
        Ok::<_, McpError>(
            manager
                .resolve_metadata_from_challenge(challenge.as_deref())
                .await,
        )
    })??;
    let captured = prm_slot
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default();
    Ok(LiveResolution {
        resolution,
        captured,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_request_failure_returns_unreachable_instead_of_empty_report() {
        let endpoint = "https://user:secret@mcp.example.com/v1?api_key=leak";
        let metadata_url =
            Url::parse("https://auth.example.com/.well-known/oauth-authorization-server").unwrap();
        let error = AuthError::DiscoveryRequestFailed {
            url: metadata_url,
            source: Box::new(std::io::Error::other("HTTP 503")),
        };

        let result = classify_discovery_resolution(
            endpoint,
            Err(error),
            CapturedResourceMetadata::default(),
        );
        assert!(
            matches!(
                result,
                Err(McpError::Unreachable { ref url, .. })
                    if url == "https://mcp.example.com/v1"
            ),
            "expected redacted McpError::Unreachable, got {result:?}"
        );
    }

    #[test]
    fn advertised_servers_without_metadata_still_return_empty_capability_report() {
        let endpoint = "https://mcp.example.com/v1";
        let prm_url =
            Url::parse("https://mcp.example.com/.well-known/oauth-protected-resource").unwrap();
        let error = AuthError::AuthorizationServersUnavailable {
            resource_metadata_url: prm_url,
            authorization_servers: vec!["https://auth.example.com".into()],
        };
        let captured = CapturedResourceMetadata {
            resource: Some(endpoint.into()),
            authorization_servers: vec!["https://auth.example.com".into()],
            scopes_supported: vec!["read".into()],
        };

        let report = classify_discovery_resolution(endpoint, Err(error), captured).unwrap();
        assert_eq!(report.resource.as_deref(), Some(endpoint));
        assert_eq!(
            report.authorization_servers,
            vec!["https://auth.example.com"]
        );
        assert_eq!(report.scopes_supported, vec!["read"]);
        assert!(!report.pkce_s256_supported);
        assert!(!report.registration_supported);
    }

    #[test]
    fn metadata_validation_error_fails_instead_of_returning_empty_report() {
        let endpoint = "https://mcp.example.com/v1";
        let error = AuthError::AuthorizationServerMissingIssuer {
            expected_issuer: "https://auth.example.com".into(),
        };

        let result = classify_discovery_resolution(
            endpoint,
            Err(error),
            CapturedResourceMetadata::default(),
        );
        assert!(
            matches!(result, Err(McpError::Server { .. })),
            "expected McpError::Server, got {result:?}"
        );
    }
}
