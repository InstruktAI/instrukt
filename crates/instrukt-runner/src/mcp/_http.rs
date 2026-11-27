//! Shared HTTP and Tokio runtime bridge for `instrukt-runner`'s MCP layer.
//!
//! Uses `instrukt_outside::http::build_client` and
//! `instrukt_outside::runtime::block_on` so every MCP/OAuth call shares one
//! client factory, one runtime bridge, and the `mcp_address_admitted` guard.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::header::ACCEPT;
use instrukt_outside::http::{ClientConfig, build_client};
use instrukt_outside::runtime::block_on;
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRequest,
};
use serde_json::Value;

use super::McpError;

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
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("unreadable HTTP body: {error}"))?
    {
        if chunk.len() > max_bytes.saturating_sub(body.len()) {
            return Err(format!("HTTP response body exceeds {max_bytes} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Performs a bounded GET `Accept: application/json` request to an admitted URL.
pub(crate) async fn get_json_text(
    client: &reqwest::Client,
    url: &str,
    bearer_token: Option<&str>,
) -> Result<(u16, String), String> {
    if !instrukt_core::config::mcp_address_admitted(url) {
        return Err("endpoint address must be https (or loopback http)".into());
    }
    let mut request = client.get(url).header(ACCEPT, "application/json");
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("HTTP request failed: {error}"))?;
    let status = response.status().as_u16();
    let bytes = read_bounded_body(response, MAX_OAUTH_BODY_BYTES).await?;
    let text = String::from_utf8(bytes)
        .map_err(|error| format!("HTTP response is not valid UTF-8: {error}"))?;
    Ok((status, text))
}

/// Protected resource metadata fields observed while `rmcp`'s
/// `AuthorizationManager` runs RFC 9728 discovery.
#[derive(Debug, Clone, Default)]
pub(crate) struct CapturedResourceMetadata {
    pub resource: Option<String>,
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Vec<String>,
}

/// `rmcp::transport::auth::OAuthHttpClient` backed by `instrukt-outside`'s
/// `reqwest::Client`, enforcing `mcp_address_admitted` and no-redirects on
/// every outbound OAuth HTTP request.
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
}

impl OAuthHttpClient for AdmittedOAuthHttpClient {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let url = request.request.uri().to_string();
            if !instrukt_core::config::mcp_address_admitted(&url) {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("OAuth address `{url}` is neither https nor loopback"),
                )) as OAuthHttpClientError);
            }
            let req = reqwest::Request::try_from(request.request)
                .map_err(|error| Box::new(error) as OAuthHttpClientError)?;
            let response = self
                .client
                .execute(req)
                .await
                .map_err(|error| Box::new(error) as OAuthHttpClientError)?;
            let status = response.status();
            let version = response.version();
            let headers = response.headers().clone();
            let body = read_bounded_body(response, MAX_OAUTH_BODY_BYTES)
                .await
                .map_err(|msg| Box::new(std::io::Error::other(msg)) as OAuthHttpClientError)?;

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
        })
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
