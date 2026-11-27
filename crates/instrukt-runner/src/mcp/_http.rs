//! Shared HTTP and Tokio runtime bridge for `instrukt-runner`'s MCP layer.
//!
//! Uses `instrukt_outside::http::build_client` and
//! `instrukt_outside::runtime::block_on` so every MCP/OAuth call shares one
//! client factory, one runtime bridge, and the `mcp_address_admitted` guard.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use http::header::ACCEPT;
use instrukt_outside::http::{
    ClientConfig, Judgment, build_client, judge_reqwest_error, judge_response,
};
use instrukt_outside::runtime::block_on;
use instrukt_outside::{Attempted, Gate, Key, Report, Settings, Tried};
use jiff::Timestamp;
use rmcp::transport::auth::{
    AuthError, OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRequest,
};
use serde_json::Value;

use super::McpError;

/// Shared outside gate for MCP outbound requests in this process.
pub(crate) fn mcp_gate() -> &'static Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    GATE.get_or_init(|| {
        Gate::new(
            Arc::new(Timestamp::now),
            Arc::new(Instant::now),
            Report::log(),
        )
    })
}

fn rfc6749_code(raw: &str) -> Option<&'static str> {
    const CODES: &[&str] = &[
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
    CODES.iter().copied().find(|code| raw.contains(code))
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

/// Derives a stable `(origin, path)` gate scope from an admitted HTTP URL so
/// query parameters (including OAuth parameters) never leak into gate keys.
pub(crate) fn gate_scope(url: &str) -> (String, String) {
    match url::Url::parse(url) {
        Ok(parsed) => {
            let mut origin = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
            if let Some(port) = parsed.port() {
                origin.push_str(&format!(":{port}"));
            }
            (origin, parsed.path().to_owned())
        }
        Err(_) => (url.to_owned(), "/".to_owned()),
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

/// Typed failure from [`read_bounded_body`] so transport errors keep their
/// `reqwest::Error` classification for [`judge_reqwest_error`].
#[derive(Debug)]
pub(crate) enum BodyReadError {
    Transport(reqwest::Error),
    TooLarge(usize),
}

impl std::fmt::Display for BodyReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "unreadable HTTP body: {error}"),
            Self::TooLarge(max_bytes) => {
                write!(f, "HTTP response body exceeds {max_bytes} bytes")
            }
        }
    }
}

impl std::error::Error for BodyReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::TooLarge(_) => None,
        }
    }
}

/// Reads a `reqwest::Response` body up to `max_bytes`, failing before
/// buffering unbounded payloads while preserving transport error types.
pub(crate) async fn read_bounded_body(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, BodyReadError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(BodyReadError::Transport)? {
        if chunk.len() > max_bytes.saturating_sub(body.len()) {
            return Err(BodyReadError::TooLarge(max_bytes));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Typed failure from [`get_json_text`] / [`post_form_text`] keeping temporary
/// gate unavailability (carrying the gate's `Wait`) distinct from permanent
/// defects.
#[derive(Debug)]
pub(crate) enum GatedTextError {
    Unavailable(String),
    Defect(String),
}

impl std::fmt::Display for GatedTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(msg) | Self::Defect(msg) => f.write_str(msg),
        }
    }
}

fn judge_body_error(
    error: &BodyReadError,
    settings: &Settings<'_>,
) -> Option<instrukt_outside::Unavailable> {
    match error {
        BodyReadError::Transport(req_err) => match judge_reqwest_error(req_err, settings) {
            Judgment::Unavailable(unavailable) => Some(unavailable),
            // `reqwest::Response::chunk()` wraps mid-stream HTTP body/EOF
            // termination in outer `Decode` (`reqwest` 0.13.5); no JSON or
            // UTF-8 decoding runs here, so any remaining chunk error is a
            // transport interruption judged through the HTTP adapter.
            Judgment::Success | Judgment::Defect => match instrukt_outside::http::judge(
                &instrukt_outside::http::HttpAnswer::Transport(
                    instrukt_outside::http::TransportFailure::Unreachable,
                ),
                settings,
            ) {
                Judgment::Unavailable(unavailable) => Some(unavailable),
                Judgment::Success | Judgment::Defect => None,
            },
        },
        BodyReadError::TooLarge(_) => None,
    }
}

async fn send_gated_text(
    url: &str,
    build: impl FnOnce() -> reqwest::RequestBuilder,
) -> Result<(u16, String), GatedTextError> {
    if !instrukt_core::config::mcp_address_admitted(url) {
        return Err(GatedTextError::Defect(
            "endpoint address must be https (or loopback http)".into(),
        ));
    }
    let (origin, path) = gate_scope(url);
    let key = Key::new("mcp-oauth", [origin.as_str()]).item(path.as_str());
    let gate = mcp_gate();
    let settings = Settings::default();
    let attempted = gate
        .attempt_async(&key, &settings, || async {
            let response = match build().send().await {
                Ok(response) => response,
                Err(error) => {
                    let message = format!("HTTP request failed: {error}");
                    return match judge_reqwest_error(&error, &settings) {
                        Judgment::Unavailable(unavailable) => {
                            Tried::Unavailable(unavailable, Err(message))
                        }
                        Judgment::Success | Judgment::Defect => Tried::Defect(Err(message)),
                    };
                }
            };
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let now = gate.now();
            // Judge status and headers (including Retry-After) before reading
            // the body so an interrupted or oversized body never discards a
            // received temporary wait.
            let status_judgment = judge_response(status, &headers, now, None, &settings);
            let body_text = match read_bounded_body(response, MAX_OAUTH_BODY_BYTES).await {
                Ok(bytes) => String::from_utf8(bytes)
                    .map_err(|error| (None, format!("HTTP response is not valid UTF-8: {error}"))),
                Err(error) => Err((judge_body_error(&error, &settings), error.to_string())),
            };
            match (status_judgment, body_text) {
                (Judgment::Unavailable(unavailable), Ok(text)) => {
                    Tried::Unavailable(unavailable, Ok((status, text)))
                }
                (Judgment::Unavailable(unavailable), Err((_, message))) => {
                    Tried::Unavailable(unavailable, Err(message))
                }
                (Judgment::Success | Judgment::Defect, Ok(text)) => Tried::Done((status, text)),
                (Judgment::Success | Judgment::Defect, Err((Some(unavailable), message))) => {
                    Tried::Unavailable(unavailable, Err(message))
                }
                (Judgment::Success | Judgment::Defect, Err((None, message))) => {
                    Tried::Defect(Err(message))
                }
            }
        })
        .await;
    match attempted {
        Attempted::Done(answer) | Attempted::Defect(Ok(answer)) => Ok(answer),
        Attempted::Wait {
            wait,
            error: Some(Ok((status, body_text))),
        } => {
            let detail = serde_json::from_str::<Value>(&body_text)
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
                .map_or_else(
                    || format!("HTTP {status}"),
                    |code| format!("HTTP {status}: {code}"),
                );
            Err(GatedTextError::Unavailable(format!("{wait} ({detail})")))
        }
        Attempted::Wait {
            wait,
            error: Some(Err(message)),
        } => Err(GatedTextError::Unavailable(format!("{wait} ({message})"))),
        Attempted::Wait { wait, error: None } => Err(GatedTextError::Unavailable(wait.to_string())),
        Attempted::Defect(Err(message)) => Err(GatedTextError::Defect(message)),
    }
}

/// Performs a bounded GET `Accept: application/json` request to an admitted URL
/// through the shared MCP outside gate.
pub(crate) async fn get_json_text(
    client: &reqwest::Client,
    url: &str,
    bearer_token: Option<&str>,
) -> Result<(u16, String), GatedTextError> {
    send_gated_text(url, || {
        let mut request = client.get(url).header(ACCEPT, "application/json");
        if let Some(token) = bearer_token {
            request = request.bearer_auth(token);
        }
        request
    })
    .await
}

/// Performs a single-shot bounded POST `application/x-www-form-urlencoded`
/// request with `Accept: application/json` to an admitted URL through the
/// shared MCP outside gate.
pub(crate) async fn post_form_text(
    client: &reqwest::Client,
    url: &str,
    form: &[(&str, &str)],
) -> Result<(u16, String), GatedTextError> {
    send_gated_text(url, || {
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

/// `rmcp::transport::auth::OAuthHttpClient` backed by `instrukt-outside`'s
/// `reqwest::Client` and shared `mcp_gate()`, enforcing `mcp_address_admitted`,
/// no-redirects, bounded bodies, and gate wait/outage tracking on every
/// outbound OAuth HTTP request.
pub(crate) struct AdmittedOAuthHttpClient {
    client: reqwest::Client,
    captured_prm: Option<Arc<Mutex<Option<CapturedResourceMetadata>>>>,
    last_unavailable: Option<Arc<Mutex<Option<String>>>>,
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
            last_unavailable: None,
        })
    }

    pub(crate) fn with_prm_observer(
        mut self,
        slot: Arc<Mutex<Option<CapturedResourceMetadata>>>,
    ) -> Self {
        self.captured_prm = Some(slot);
        self
    }

    pub(crate) fn with_unavailable_observer(mut self, slot: Arc<Mutex<Option<String>>>) -> Self {
        self.last_unavailable = Some(slot);
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
                format!("OAuth address `{url}` is neither https nor loopback"),
            )) as OAuthHttpClientError);
        }
        let (origin, path) = gate_scope(&url);
        let key = Key::new("mcp-oauth", [origin.as_str()]).item(path.as_str());
        let gate = mcp_gate();
        let settings = Settings::default();
        let req = reqwest::Request::try_from(request)
            .map_err(|error| Box::new(error) as OAuthHttpClientError)?;
        let attempted = gate
            .attempt_async(&key, &settings, || async {
                let response = match self.client.execute(req).await {
                    Ok(response) => response,
                    Err(error) => {
                        let judgment = judge_reqwest_error(&error, &settings);
                        let boxed = Box::new(error) as OAuthHttpClientError;
                        return match judgment {
                            Judgment::Unavailable(unavailable) => {
                                Tried::Unavailable(unavailable, Err(boxed))
                            }
                            Judgment::Success | Judgment::Defect => Tried::Defect(Err(boxed)),
                        };
                    }
                };
                let status = response.status();
                let version = response.version();
                let headers = response.headers().clone();
                let now = gate.now();
                // Judge status and headers (including Retry-After) before reading
                // the body so an interrupted or oversized body never discards a
                // received temporary wait.
                let status_judgment =
                    judge_response(status.as_u16(), &headers, now, None, &settings);
                let body = match read_bounded_body(response, MAX_OAUTH_BODY_BYTES).await {
                    Ok(body) => body,
                    Err(error) => {
                        let body_unavailable = judge_body_error(&error, &settings);
                        let boxed = Box::new(error) as OAuthHttpClientError;
                        return match (status_judgment, body_unavailable) {
                            (Judgment::Unavailable(unavailable), _) | (_, Some(unavailable)) => {
                                Tried::Unavailable(unavailable, Err(boxed))
                            }
                            (Judgment::Success | Judgment::Defect, None) => {
                                Tried::Defect(Err(boxed))
                            }
                        };
                    }
                };

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
                let built = match builder.body(body) {
                    Ok(built) => built,
                    Err(error) => {
                        let boxed = Box::new(error) as OAuthHttpClientError;
                        return match status_judgment {
                            Judgment::Unavailable(unavailable) => {
                                Tried::Unavailable(unavailable, Err(boxed))
                            }
                            Judgment::Success | Judgment::Defect => Tried::Defect(Err(boxed)),
                        };
                    }
                };
                match status_judgment {
                    Judgment::Unavailable(unavailable) => {
                        Tried::Unavailable(unavailable, Ok(built))
                    }
                    Judgment::Success | Judgment::Defect => Tried::Done(built),
                }
            })
            .await;
        if let Attempted::Wait { ref wait, .. } = attempted
            && let Some(slot) = &self.last_unavailable
            && let Ok(mut guard) = slot.lock()
        {
            *guard = Some(wait.to_string());
        }
        match attempted {
            Attempted::Done(response)
            | Attempted::Defect(Ok(response))
            | Attempted::Wait {
                error: Some(Ok(response)),
                ..
            } => Ok(response),
            Attempted::Wait {
                wait,
                error: Some(Err(error)),
            } => Err(Box::new(std::io::Error::other(format!("{wait} ({error})")))
                as OAuthHttpClientError),
            Attempted::Defect(Err(error)) => Err(error),
            Attempted::Wait { wait, error: None } => Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                wait.to_string(),
            )) as OAuthHttpClientError),
        }
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
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn sanitize_auth_error_never_echoes_provider_payloads() {
        let err = AuthError::TokenExchangeFailed("secret-token-leaked-in-body".into());
        let msg = sanitize_auth_error(&err);
        assert!(!msg.contains("secret-token-leaked-in-body"), "{msg}");
        assert!(msg.contains("token exchange"), "{msg}");
    }

    #[test]
    fn interrupted_body_is_judged_unavailable_while_oversized_body_is_defect()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            judge_body_error(&BodyReadError::TooLarge(1024), &Settings::default()),
            None
        );

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req = [0u8; 1024];
                let _ = std::io::Read::read(&mut stream, &mut req);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort");
                let _ = stream.flush();
            }
        });
        let http = client(
            "test",
            Duration::from_secs(2),
            Duration::from_secs(2),
            false,
        )?;
        let url = format!("http://{addr}/truncated");
        let outcome = run_async("test", async move {
            let response = http
                .get(&url)
                .send()
                .await
                .map_err(BodyReadError::Transport)?;
            read_bounded_body(response, MAX_OAUTH_BODY_BYTES).await
        })?;
        let _ = server.join();
        let Err(err) = outcome else {
            return Err("truncated body must fail".into());
        };
        let unavailable = judge_body_error(&err, &Settings::default());
        assert!(
            matches!(
                unavailable,
                Some(instrukt_outside::Unavailable {
                    reason: instrukt_outside::Reason::Unreachable,
                    ..
                })
            ),
            "expected Unreachable for premature body close, got: {unavailable:?} (err: {err})"
        );
        Ok(())
    }
}
