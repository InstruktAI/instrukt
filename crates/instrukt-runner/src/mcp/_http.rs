//! Shared HTTP and Tokio runtime bridge for `instrukt-runner`'s MCP layer.
//!
//! Uses `instrukt_outside::http::build_client` and
//! `instrukt_outside::runtime::block_on` so every MCP/OAuth call shares one
//! client factory, one runtime bridge, bounded body reads, and the
//! `mcp_address_admitted` guard.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::header::ACCEPT;
pub(crate) use instrukt_outside::http::redact_url;
use instrukt_outside::http::{ClientConfig, build_client};
use instrukt_outside::runtime::block_on;
use rmcp::transport::auth::{
    AuthError, OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRequest,
};
use serde_json::Value;

use super::McpError;

const RFC6749_CODES: &[&str] = &[
    "invalid_client",
    "invalid_grant",
    "invalid_request",
    "unauthorized_client",
    "unsupported_grant_type",
    "invalid_scope",
    "access_denied",
    "server_error",
    "temporarily_unavailable",
];

fn rfc6749_code(raw: &str) -> Option<&'static str> {
    RFC6749_CODES
        .iter()
        .copied()
        .find(|code| raw.contains(code))
}

/// Returns `Some` only when `raw` matches a closed RFC 6749 error code
/// verbatim so arbitrary provider response strings are never echoed.
pub(crate) fn exact_rfc6749_code(raw: &str) -> Option<&'static str> {
    RFC6749_CODES.iter().copied().find(|code| raw == *code)
}

/// Maps `rmcp::transport::auth::AuthError` to a fixed, credential-safe summary
/// before persisting to SQLite (`mcp_sign_ins.failure`) or returning to callers,
/// never echoing provider `error_description` bodies or reflected credentials.
pub(crate) fn sanitize_auth_error(error: &AuthError) -> String {
    match error {
        AuthError::AuthorizationRequired => "OAuth authorization required".into(),
        AuthError::AuthorizationFailed(_) => "OAuth authorization failed".into(),
        AuthError::TokenExchangeFailed(raw) => match rfc6749_code(raw) {
            Some(code) => format!("OAuth token exchange rejected ({code})"),
            None => "OAuth token exchange rejected by authorization server".into(),
        },
        AuthError::TokenRefreshFailed(raw) => match rfc6749_code(raw) {
            Some(code) => format!("OAuth token refresh failed ({code})"),
            None => "OAuth token refresh failed".into(),
        },
        AuthError::TokenRefreshRejected(raw) => match rfc6749_code(raw) {
            Some(code) => format!("OAuth refresh token rejected ({code})"),
            None => "OAuth refresh token was rejected".into(),
        },
        AuthError::CredentialStoreError(_) => "OAuth credential store error".into(),
        AuthError::HttpError(err) => match err.status() {
            Some(status) => format!("OAuth HTTP error ({})", status.as_u16()),
            None if err.is_timeout() => "OAuth HTTP request timed out".into(),
            None if err.is_connect() => "OAuth HTTP connection failed".into(),
            None => "OAuth HTTP request failed".into(),
        },
        AuthError::OAuthError(_) => "OAuth protocol error".into(),
        AuthError::MetadataError(_) => "OAuth metadata error".into(),
        AuthError::DiscoveryRequestFailed { .. } => {
            "OAuth metadata discovery request failed".into()
        }
        AuthError::ProtectedResourceMetadataInvalid { .. } => {
            "protected resource metadata is invalid".into()
        }
        AuthError::AuthorizationServersUnavailable { .. } => {
            "advertised authorization servers published no usable metadata".into()
        }
        AuthError::PkceUnsupported => "authorization server does not support PKCE S256".into(),
        AuthError::UrlError(_) => "invalid OAuth endpoint URL".into(),
        AuthError::NoAuthorizationSupport => "no OAuth authorization support detected".into(),
        AuthError::InvalidTokenType(_) => "unsupported OAuth token_type".into(),
        AuthError::TokenExpired => "OAuth token expired".into(),
        AuthError::InvalidScope(_) => "invalid OAuth scope".into(),
        AuthError::RegistrationFailed(_) => {
            "authorization server rejected dynamic client registration".into()
        }
        AuthError::InsufficientScope { .. } => "insufficient OAuth scope".into(),
        AuthError::AuthorizationServerMismatch { .. } => {
            "authorization server issuer mismatch".into()
        }
        AuthError::AuthorizationServerMissingIssuer { .. } => {
            "authorization server response missing required issuer".into()
        }
        AuthError::ClientCredentialsError(_) => "OAuth client credentials error".into(),
        AuthError::InternalError(_) => "internal OAuth client error".into(),
        _ => "OAuth operation failed".into(),
    }
}

/// Maximum response body size read for OAuth metadata, token, JWKS, and
/// identity endpoints.
pub(crate) const MAX_OAUTH_BODY_BYTES: usize = 1024 * 1024;

/// Builds a `reqwest::Client` through `instrukt-outside`'s shared factory.
pub(crate) fn client(
    server: &str,
    connect_timeout: Duration,
    request_timeout: Duration,
    follow_redirects: bool,
) -> Result<reqwest::Client, McpError> {
    build_client(&ClientConfig {
        connect_timeout,
        request_timeout,
        follow_redirects,
        ..ClientConfig::default()
    })
    .map_err(|error| McpError::Server {
        server: server.to_owned(),
        cause: format!("failed to build HTTP client: {error}"),
    })
}

/// Runs an async future on the workspace-shared Tokio runtime bridge.
pub(crate) fn run_async<F, T>(server: &str, future: F) -> Result<T, McpError>
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    block_on(future).map_err(|error| McpError::Server {
        server: server.to_owned(),
        cause: error.to_string(),
    })
}

/// Reads a `reqwest::Response` body up to `max_bytes`, failing before
/// buffering unbounded payloads.
pub(crate) async fn read_bounded_body(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    instrukt_outside::http::read_bounded_body(response, max_bytes)
        .await
        .map_err(|error| error.to_string())
}

async fn send_bounded_text(
    url: &str,
    build: impl FnOnce() -> reqwest::RequestBuilder,
) -> Result<(u16, String), String> {
    if !instrukt_core::config::mcp_address_admitted(url) {
        return Err("endpoint address must be https (or loopback http)".into());
    }
    let response = build()
        .send()
        .await
        .map_err(|error| format!("HTTP request failed: {}", error.without_url()))?;
    let status = response.status().as_u16();
    let bytes = read_bounded_body(response, MAX_OAUTH_BODY_BYTES).await?;
    let text = String::from_utf8(bytes)
        .map_err(|error| format!("HTTP response is not valid UTF-8: {error}"))?;
    Ok((status, text))
}

/// Performs a bounded GET `Accept: application/json` request to an admitted URL.
pub(crate) async fn get_json_text(
    client: &reqwest::Client,
    url: &str,
    bearer_token: Option<&str>,
) -> Result<(u16, String), String> {
    send_bounded_text(url, || {
        let mut request = client.get(url).header(ACCEPT, "application/json");
        if let Some(token) = bearer_token {
            request = request.bearer_auth(token);
        }
        request
    })
    .await
}

/// Performs a bounded POST `application/x-www-form-urlencoded` request with
/// `Accept: application/json` to an admitted URL.
pub(crate) async fn post_form_text(
    client: &reqwest::Client,
    url: &str,
    form: &[(&str, &str)],
) -> Result<(u16, String), String> {
    send_bounded_text(url, || {
        client
            .post(url)
            .header(ACCEPT, "application/json")
            .form(form)
    })
    .await
}

/// Protected resource metadata fields observed while `rmcp`'s
/// `AuthorizationManager` runs RFC 9728 discovery.
#[derive(Debug, Clone, Default)]
pub(crate) struct CapturedResourceMetadata {
    pub resource: Option<String>,
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Vec<String>,
}

/// `rmcp::transport::auth::OAuthHttpClient` enforcing `mcp_address_admitted`,
/// no-redirects, and bounded bodies on outbound OAuth HTTP requests.
pub(crate) struct AdmittedOAuthHttpClient {
    client: reqwest::Client,
    captured_prm: Option<Arc<Mutex<Option<CapturedResourceMetadata>>>>,
}

impl AdmittedOAuthHttpClient {
    pub(crate) fn new(
        server: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, McpError> {
        Ok(Self {
            client: client(server, connect_timeout, request_timeout, false)?,
            captured_prm: None,
        })
    }

    pub(crate) fn with_prm_observer(
        mut self,
        slot: Arc<Mutex<Option<CapturedResourceMetadata>>>,
    ) -> Self {
        self.captured_prm = Some(slot);
        self
    }

    pub(crate) async fn execute_http(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, OAuthHttpClientError> {
        let url = request.uri().to_string();
        if !instrukt_core::config::mcp_address_admitted(&url) {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "OAuth address `{}` is neither https nor loopback",
                    redact_url(&url)
                ),
            )) as OAuthHttpClientError);
        }
        let req = reqwest::Request::try_from(request)
            .map_err(|error| Box::new(error.without_url()) as OAuthHttpClientError)?;
        let response = self
            .client
            .execute(req)
            .await
            .map_err(|error| Box::new(error.without_url()) as OAuthHttpClientError)?;
        let status = response.status();
        let version = response.version();
        let headers = response.headers().clone();
        let body = read_bounded_body(response, MAX_OAUTH_BODY_BYTES)
            .await
            .map_err(|message| Box::new(std::io::Error::other(message)) as OAuthHttpClientError)?;

        if status == http::StatusCode::OK
            && let Some(slot) = &self.captured_prm
            && let Ok(doc) = serde_json::from_slice::<Value>(&body)
            && doc.is_object()
            && (doc.get("authorization_servers").is_some()
                || doc.get("authorization_server").is_some()
                || doc.get("resource").is_some())
            && doc.get("token_endpoint").is_none()
        {
            let resource = doc
                .get("resource")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let mut authorization_servers = string_list(doc.get("authorization_servers"));
            if authorization_servers.is_empty()
                && let Some(single) = doc
                    .get("authorization_server")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            {
                authorization_servers.push(single.to_owned());
            }
            let scopes_supported = string_list(doc.get("scopes_supported"));
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(CapturedResourceMetadata {
                    resource,
                    authorization_servers,
                    scopes_supported,
                });
            }
        }

        let mut builder = http::Response::builder().status(status).version(version);
        for (name, value) in &headers {
            builder = builder.header(name, value);
        }
        builder
            .body(body)
            .map_err(|error| Box::new(error) as OAuthHttpClientError)
    }
}

impl OAuthHttpClient for AdmittedOAuthHttpClient {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move { self.execute_http(request.request).await })
    }
}

pub(crate) fn string_list(value: Option<&Value>) -> Vec<String> {
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
    use super::*;

    #[test]
    fn sanitize_auth_error_never_echoes_provider_payloads() {
        let err = AuthError::TokenExchangeFailed("secret-token-leaked-in-body".into());
        let msg = sanitize_auth_error(&err);
        assert!(!msg.contains("secret-token-leaked-in-body"), "{msg}");
        assert!(msg.contains("token exchange"), "{msg}");
    }

    #[test]
    fn exact_rfc6749_code_accepts_only_standard_codes() {
        assert_eq!(exact_rfc6749_code("invalid_grant"), Some("invalid_grant"));
        assert_eq!(
            exact_rfc6749_code("temporarily_unavailable"),
            Some("temporarily_unavailable")
        );
        assert_eq!(exact_rfc6749_code("invalid_grant: Bearer secret-123"), None);
        assert_eq!(exact_rfc6749_code("custom_provider_error"), None);
    }

    #[test]
    fn redact_url_strips_userinfo_query_and_fragment() {
        assert_eq!(
            redact_url("https://user:secret@auth.example.com:8443/oauth/token?api_key=leak#frag"),
            "https://auth.example.com:8443/oauth/token"
        );
        assert_eq!(
            redact_url("http://127.0.0.1:9000/register?token=secret"),
            "http://127.0.0.1:9000/register"
        );
        assert_eq!(redact_url("not a url with secret"), "<invalid-url>");
    }
}
