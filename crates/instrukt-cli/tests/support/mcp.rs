//! An MCP server stand-in at the network boundary: the Streamable HTTP
//! transport on a loopback address, answering `initialize`, the initialized
//! notification, a two-page `tools/list` as an event stream, `tools/call` and
//! the closing DELETE. It records each request's JSON-RPC method and the
//! `Authorization` and `MCP-Protocol-Version` headers it carried. It answers
//! initialization with the protocol version it is started with.
//!
//! It knows which principal each login it accepts belongs to. Its tools:
//! `whoami` answers with its own name, the principal it authenticated and the
//! login the call carried, so a test sees whose login reached which server
//! and what of it reaches the agent; `fail` reports a failure naming that
//! login; `reject` answers HTTP 400 with a raw body that spells the login
//! with a Unicode escape; `nested` reports a failure whose message is JSON
//! text holding the login escaped; `malformed` answers a result whose text
//! block has no text.
#![allow(clippy::unwrap_used, reason = "test fixture")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

/// How a server departs from what MCP requires of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// Its initialization names no version of itself.
    ServerInfo,
    /// Its catalog gives a tool an input shape that is not an object.
    InputShape,
}

/// `login` with its first character written as a JSON Unicode escape.
fn escaped(login: &str) -> String {
    let mut chars = login.chars();
    chars
        .next()
        .map(|first| format!("\\u{:04x}{}", u32::from(first), chars.as_str()))
        .unwrap_or_default()
}

/// One request the stand-in received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    /// The JSON-RPC method; `None` for the DELETE that ends the session.
    pub method: Option<String>,
    /// The `Authorization` header it carried.
    pub authorization: Option<String>,
    /// The `MCP-Protocol-Version` header it carried.
    pub protocol_version: Option<String>,
}

pub struct FakeMcp {
    /// The server's address, as a declaration names it.
    pub url: String,
    received: Arc<Mutex<Vec<Received>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

const SESSION: &str = "stand-in-session";

impl FakeMcp {
    /// A server called `name` that accepts `principals`: each login it knows,
    /// with the principal it belongs to.
    pub fn start(name: &str, principals: &[(&str, &str)]) -> Self {
        Self::serve(name, principals, None, "2025-06-18")
    }

    /// A server like [`FakeMcp::start`] that departs from MCP as `flaw` says.
    pub fn flawed(name: &str, principals: &[(&str, &str)], flaw: Option<Flaw>) -> Self {
        Self::serve(name, principals, flaw, "2025-06-18")
    }

    /// A server like [`FakeMcp::start`] that answers initialization with
    /// protocol version `version`.
    pub fn answering(name: &str, principals: &[(&str, &str)], version: &'static str) -> Self {
        Self::serve(name, principals, None, version)
    }

    fn serve(
        name: &str,
        principals: &[(&str, &str)],
        flaw: Option<Flaw>,
        version: &'static str,
    ) -> Self {
        let name = name.to_owned();
        let principals: std::collections::BTreeMap<String, String> = principals
            .iter()
            .map(|(login, principal)| ((*login).to_owned(), (*principal).to_owned()))
            .collect();
        let http = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", http.server_addr().to_ip().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let received = Arc::clone(&received);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let principal = |login: &Option<String>| match login {
                    None => "nobody".to_owned(),
                    Some(login) => principals
                        .get(login)
                        .cloned()
                        .unwrap_or_else(|| "an unknown principal".to_owned()),
                };
                while !stop.load(Ordering::Acquire) {
                    let Ok(Some(mut request)) = http.recv_timeout(Duration::from_millis(50)) else {
                        continue;
                    };
                    let carried = |field: &'static str| {
                        request
                            .headers()
                            .iter()
                            .find(|h| h.field.equiv(field))
                            .map(|h| h.value.as_str().to_owned())
                    };
                    let authorization = carried("Authorization");
                    let protocol_version = carried("MCP-Protocol-Version");
                    if *request.method() == tiny_http::Method::Get {
                        request.respond(tiny_http::Response::empty(405)).unwrap();
                        continue;
                    }
                    if *request.method() == tiny_http::Method::Delete {
                        received.lock().unwrap().push(Received {
                            method: None,
                            authorization,
                            protocol_version,
                        });
                        request.respond(tiny_http::Response::empty(200)).unwrap();
                        continue;
                    }
                    let mut body = String::new();
                    request.as_reader().read_to_string(&mut body).unwrap();
                    let message: Value = serde_json::from_str(&body).unwrap();
                    let method = message["method"].as_str().unwrap_or_default().to_owned();
                    received.lock().unwrap().push(Received {
                        method: Some(method.clone()),
                        authorization: authorization.clone(),
                        protocol_version,
                    });
                    let id = message["id"].clone();
                    let header = |name: &str, value: &str| {
                        tiny_http::Header::from_bytes(name, value).unwrap()
                    };
                    let json_answer = |result: Value| {
                        tiny_http::Response::from_string(
                            json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string(),
                        )
                        .with_header(header("Content-Type", "application/json"))
                    };
                    let who = principal(&authorization);
                    let login = authorization.unwrap_or_else(|| "no login".to_owned());
                    let response = match method.as_str() {
                        "initialize" => json_answer(json!({
                            "protocolVersion": version,
                            "capabilities": {"tools": {}},
                            "serverInfo": if flaw == Some(Flaw::ServerInfo) {
                                json!({"name": "stand-in"})
                            } else {
                                json!({"name": "stand-in", "version": "1"})
                            },
                        }))
                        .with_header(header("Mcp-Session-Id", SESSION)),
                        "notifications/initialized" => {
                            tiny_http::Response::from_string("").with_status_code(202)
                        }
                        "tools/list" => {
                            let page = if flaw == Some(Flaw::InputShape) {
                                json!({"tools": [{"name": "odd", "inputSchema": {"type": "string"}}]})
                            } else if message["params"]["cursor"] == "second" {
                                json!({"tools": [tool("fail", "Report a failure")]})
                            } else {
                                json!({
                                    "tools": [tool("whoami", "Say whose login reached the server")],
                                    "nextCursor": "second",
                                })
                            };
                            // As an event stream, with a notification ahead of the answer.
                            let stream = format!(
                                "event: message\ndata: {}\n\nevent: message\ndata: {}\n\n",
                                json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}}),
                                json!({"jsonrpc": "2.0", "id": id, "result": page}),
                            );
                            tiny_http::Response::from_string(stream)
                                .with_header(header("Content-Type", "text/event-stream"))
                        }
                        "tools/call" if message["params"]["name"] == "fail" => {
                            tiny_http::Response::from_string(
                                json!({"jsonrpc": "2.0", "id": id, "error": {
                                    "code": -32000,
                                    "message": format!("the login {login} may not do this"),
                                }})
                                .to_string(),
                            )
                            .with_header(header("Content-Type", "application/json"))
                        }
                        "tools/call" if message["params"]["name"] == "reject" => {
                            tiny_http::Response::from_string(format!(
                                "{{\"error\":\"refused\",\"login\":\"{}\"}}",
                                escaped(&login)
                            ))
                            .with_status_code(400)
                        }
                        "tools/call" if message["params"]["name"] == "nested" => {
                            let detail = format!("{{\"login\":\"{}\"}}", escaped(&login));
                            tiny_http::Response::from_string(
                                json!({"jsonrpc": "2.0", "id": id, "error": {
                                    "code": -32000,
                                    "message": "the request was refused",
                                    "data": detail,
                                }})
                                .to_string(),
                            )
                            .with_header(header("Content-Type", "application/json"))
                        }
                        "tools/call" if message["params"]["name"] == "malformed" => {
                            json_answer(json!({"content": [{"type": "text"}]}))
                        }
                        "tools/call" => json_answer(json!({
                            "content": [{"type": "text", "text": format!("{name} answered {who}, carrying {login}")}],
                            "isError": false,
                        })),
                        _ => tiny_http::Response::from_string("").with_status_code(400),
                    };
                    request.respond(response).unwrap();
                }
            })
        };
        Self {
            url,
            received,
            stop,
            thread: Some(thread),
        }
    }

    /// The requests received so far, in order.
    pub fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

fn tool(name: &str, description: &str) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": {}},
    })
}

impl Drop for FakeMcp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

/// Configuration for [`FakeOAuthMcp`]: an OAuth 2.1 / OIDC / EMA authorization
/// server paired with a Streamable HTTP MCP resource server on loopback.
#[derive(Debug, Clone)]
pub struct OAuthMcpOptions {
    pub name: String,
    pub registration_supported: bool,
    pub pkce_s256_supported: bool,
    pub iss_parameter_supported: bool,
    pub openid_provider: bool,
    pub ema_jwt_bearer_supported: bool,
    pub expected_client: Option<(String, String)>,
}

/// One verified EMA `jwt-bearer` assertion observed by [`FakeOAuthMcp`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedJwtBearer {
    pub iss: String,
    pub email: String,
    pub jti: String,
    pub resource: String,
}

#[derive(Debug, Clone)]
struct IssuedCode {
    email: String,
    redirect_uri: String,
    code_challenge: String,
    nonce: Option<String>,
    client_id: String,
}

struct OAuthState {
    codes: std::collections::BTreeMap<String, IssuedCode>,
    tokens: std::collections::BTreeMap<String, String>,
    refresh_tokens: std::collections::BTreeMap<String, String>,
    revoked_tokens: std::collections::BTreeSet<String>,
    revoked_refresh_tokens: Vec<String>,
    revocations: Vec<(String, String)>,
    jwt_bearers: Vec<ObservedJwtBearer>,
    auth_code_exchanges: usize,
    refresh_exchanges: usize,
    userinfo_calls: usize,
    total_token_requests: usize,
    refresh_delay_ms: u64,
    expires_in: i64,
    omit_refresh_token: bool,
    userinfo_status: Option<u16>,
    userinfo_email_override: Option<String>,
    refresh_error: Option<(u16, String)>,
    next_token_id: usize,
}

/// Real loopback HTTP server implementing RFC 9728 PRM, RFC 8414 / OIDC
/// metadata, JWKS, authorization-code + PKCE S256 + OIDC `id_token`,
/// EMA `jwt-bearer` verification against the real ingress JWKS, and
/// Bearer-protected Streamable HTTP MCP.
pub struct FakeOAuthMcp {
    pub base_url: String,
    pub mcp_url: String,
    state: Arc<Mutex<OAuthState>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeOAuthMcp {
    pub fn start(opts: OAuthMcpOptions) -> Self {
        use base64::Engine as _;
        use ring::rand::SystemRandom;
        use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
        use sha2::{Digest, Sha256};

        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let keypair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap();
        let pub_bytes = keypair.public_key().as_ref();
        let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&pub_bytes[1..33]);
        let y = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&pub_bytes[33..65]);
        let kid = "fake-as-kid-1".to_owned();
        let as_jwks = json!({
            "keys": [{
                "kty": "EC",
                "crv": "P-256",
                "use": "sig",
                "alg": "ES256",
                "kid": kid,
                "x": x,
                "y": y,
            }]
        })
        .to_string();
        let as_private_pem = pem::encode(&pem::Pem::new("PRIVATE KEY", pkcs8.as_ref()));

        let http = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = http.server_addr().to_ip().unwrap();
        let base_url = format!("http://{addr}");
        let mcp_url = format!("{base_url}/mcp");

        let state = Arc::new(Mutex::new(OAuthState {
            codes: std::collections::BTreeMap::new(),
            tokens: std::collections::BTreeMap::new(),
            refresh_tokens: std::collections::BTreeMap::new(),
            revoked_tokens: std::collections::BTreeSet::new(),
            revoked_refresh_tokens: Vec::new(),
            revocations: Vec::new(),
            jwt_bearers: Vec::new(),
            auth_code_exchanges: 0,
            refresh_exchanges: 0,
            userinfo_calls: 0,
            total_token_requests: 0,
            refresh_delay_ms: 0,
            expires_in: 3600,
            omit_refresh_token: false,
            userinfo_status: None,
            userinfo_email_override: None,
            refresh_error: None,
            next_token_id: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let state = Arc::clone(&state);
            let stop = Arc::clone(&stop);
            let base = base_url.clone();
            let mcp = mcp_url.clone();
            std::thread::spawn(move || {
                let hdr =
                    |name: &str, value: &str| tiny_http::Header::from_bytes(name, value).unwrap();
                while !stop.load(Ordering::Acquire) {
                    let Ok(Some(mut request)) = http.recv_timeout(Duration::from_millis(50)) else {
                        continue;
                    };
                    let url_str = request.url().to_owned();
                    let path = url_str.split('?').next().unwrap_or(&url_str);
                    let method = request.method().clone();
                    let auth_header = request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.as_str().to_owned());

                    if method == tiny_http::Method::Get
                        && path == "/.well-known/oauth-protected-resource"
                    {
                        let body = json!({
                            "resource": mcp,
                            "authorization_servers": [base],
                            "scopes_supported": ["mcp:read", "mcp:write"],
                        })
                        .to_string();
                        request
                            .respond(
                                tiny_http::Response::from_string(body)
                                    .with_header(hdr("Content-Type", "application/json")),
                            )
                            .unwrap();
                        continue;
                    }

                    if method == tiny_http::Method::Get
                        && (path == "/.well-known/oauth-authorization-server"
                            || path == "/.well-known/openid-configuration")
                    {
                        let mut doc = json!({
                            "issuer": base,
                            "authorization_endpoint": format!("{base}/authorize"),
                            "token_endpoint": format!("{base}/token"),
                            "revocation_endpoint": format!("{base}/revoke"),
                            "scopes_supported": ["openid", "email", "mcp:read"],
                            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
                        });
                        if opts.registration_supported {
                            doc["registration_endpoint"] = json!(format!("{base}/register"));
                        }
                        if opts.pkce_s256_supported {
                            doc["code_challenge_methods_supported"] = json!(["S256"]);
                        }
                        if opts.iss_parameter_supported {
                            doc["authorization_response_iss_parameter_supported"] = json!(true);
                        }
                        if opts.openid_provider {
                            doc["userinfo_endpoint"] = json!(format!("{base}/userinfo"));
                            doc["jwks_uri"] = json!(format!("{base}/jwks.json"));
                        }
                        let mut grants = vec!["authorization_code", "refresh_token"];
                        if opts.ema_jwt_bearer_supported {
                            grants.push("urn:ietf:params:oauth:grant-type:jwt-bearer");
                        }
                        doc["grant_types_supported"] = json!(grants);
                        request
                            .respond(
                                tiny_http::Response::from_string(doc.to_string())
                                    .with_header(hdr("Content-Type", "application/json")),
                            )
                            .unwrap();
                        continue;
                    }

                    if method == tiny_http::Method::Get && path == "/jwks.json" {
                        request
                            .respond(
                                tiny_http::Response::from_string(as_jwks.clone())
                                    .with_header(hdr("Content-Type", "application/json")),
                            )
                            .unwrap();
                        continue;
                    }

                    if method == tiny_http::Method::Get && path == "/userinfo" {
                        let token = auth_header
                            .as_deref()
                            .and_then(|v| v.strip_prefix("Bearer "))
                            .unwrap_or_default();
                        let (forced_status, email_override, email) = {
                            let mut s = state.lock().unwrap();
                            s.userinfo_calls += 1;
                            let active_email = if s.revoked_tokens.contains(token) {
                                None
                            } else {
                                s.tokens.get(token).cloned()
                            };
                            (
                                s.userinfo_status,
                                s.userinfo_email_override.clone(),
                                active_email,
                            )
                        };
                        if let Some(code) = forced_status {
                            request.respond(tiny_http::Response::empty(code)).unwrap();
                            continue;
                        }
                        if let Some(email) = email {
                            let reported_email = email_override.unwrap_or_else(|| email.clone());
                            let body = json!({
                                "sub": format!("sub:{email}"),
                                "email": reported_email,
                                "email_verified": true,
                            })
                            .to_string();
                            request
                                .respond(
                                    tiny_http::Response::from_string(body)
                                        .with_header(hdr("Content-Type", "application/json")),
                                )
                                .unwrap();
                        } else {
                            request.respond(tiny_http::Response::empty(401)).unwrap();
                        }
                        continue;
                    }

                    if method == tiny_http::Method::Post && path == "/revoke" {
                        let mut body = String::new();
                        request.as_reader().read_to_string(&mut body).unwrap();
                        let params: std::collections::BTreeMap<String, String> =
                            form_urlencoded::parse(body.as_bytes())
                                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                                .collect();
                        if let Some(tok) = params.get("token") {
                            let hint = params.get("token_type_hint").cloned().unwrap_or_default();
                            let mut s = state.lock().unwrap();
                            s.refresh_tokens.remove(tok);
                            s.revoked_refresh_tokens.push(tok.clone());
                            s.revocations.push((tok.clone(), hint));
                        }
                        request.respond(tiny_http::Response::empty(200)).unwrap();
                        continue;
                    }

                    if method == tiny_http::Method::Post && path == "/token" {
                        state.lock().unwrap().total_token_requests += 1;
                        let mut body = String::new();
                        request.as_reader().read_to_string(&mut body).unwrap();
                        let params: std::collections::BTreeMap<String, String> =
                            form_urlencoded::parse(body.as_bytes())
                                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                                .collect();
                        let grant = params.get("grant_type").map(String::as_str).unwrap_or("");
                        if grant == "refresh_token" {
                            if let Some((exp_id, exp_sec)) = &opts.expected_client {
                                let expected_basic = format!(
                                    "Basic {}",
                                    base64::engine::general_purpose::STANDARD
                                        .encode(format!("{exp_id}:{exp_sec}"))
                                );
                                if auth_header.as_deref() != Some(expected_basic.as_str()) {
                                    request
                                        .respond(
                                            tiny_http::Response::from_string(
                                                r#"{"error":"invalid_client"}"#,
                                            )
                                            .with_status_code(401),
                                        )
                                        .unwrap();
                                    continue;
                                }
                            }
                            let (delay_ms, forced_err, expires_in) = {
                                let s = state.lock().unwrap();
                                (s.refresh_delay_ms, s.refresh_error.clone(), s.expires_in)
                            };
                            if delay_ms > 0 {
                                std::thread::sleep(Duration::from_millis(delay_ms));
                            }
                            if let Some((status_code, err_code)) = forced_err {
                                let resp = json!({"error": err_code}).to_string();
                                request
                                    .respond(
                                        tiny_http::Response::from_string(resp)
                                            .with_status_code(status_code)
                                            .with_header(hdr("Content-Type", "application/json")),
                                    )
                                    .unwrap();
                                continue;
                            }
                            let refresh_tok =
                                params.get("refresh_token").cloned().unwrap_or_default();
                            let refreshed = {
                                let mut s = state.lock().unwrap();
                                s.refresh_tokens.remove(&refresh_tok).map(|email| {
                                    s.next_token_id += 1;
                                    s.refresh_exchanges += 1;
                                    let access =
                                        format!("oauth-token-{}-{}", s.next_token_id, email);
                                    let next_refresh =
                                        format!("refresh-token-{}-{}", s.next_token_id, email);
                                    s.tokens.insert(access.clone(), email.clone());
                                    s.refresh_tokens.insert(next_refresh.clone(), email);
                                    (access, next_refresh)
                                })
                            };
                            let Some((access_token, next_refresh)) = refreshed else {
                                request
                                    .respond(
                                        tiny_http::Response::from_string(
                                            r#"{"error":"invalid_grant"}"#,
                                        )
                                        .with_status_code(400),
                                    )
                                    .unwrap();
                                continue;
                            };
                            let resp = json!({
                                "token_type": "Bearer",
                                "access_token": access_token,
                                "refresh_token": next_refresh,
                                "expires_in": expires_in,
                                "scope": "openid email mcp:read",
                            })
                            .to_string();
                            request
                                .respond(
                                    tiny_http::Response::from_string(resp)
                                        .with_header(hdr("Content-Type", "application/json")),
                                )
                                .unwrap();
                            continue;
                        }
                        if grant == "authorization_code" {
                            if let Some((exp_id, exp_sec)) = &opts.expected_client {
                                let expected_basic = format!(
                                    "Basic {}",
                                    base64::engine::general_purpose::STANDARD
                                        .encode(format!("{exp_id}:{exp_sec}"))
                                );
                                if auth_header.as_deref() != Some(expected_basic.as_str()) {
                                    request
                                        .respond(
                                            tiny_http::Response::from_string(
                                                r#"{"error":"invalid_client"}"#,
                                            )
                                            .with_status_code(401),
                                        )
                                        .unwrap();
                                    continue;
                                }
                            }
                            let code = params.get("code").cloned().unwrap_or_default();
                            let verifier = params.get("code_verifier").cloned().unwrap_or_default();
                            let redirect = params.get("redirect_uri").cloned().unwrap_or_default();
                            let issued = state.lock().unwrap().codes.remove(&code);
                            let Some(issued) = issued else {
                                request
                                    .respond(
                                        tiny_http::Response::from_string(
                                            r#"{"error":"invalid_grant"}"#,
                                        )
                                        .with_status_code(400),
                                    )
                                    .unwrap();
                                continue;
                            };
                            let computed_challenge =
                                base64::engine::general_purpose::URL_SAFE_NO_PAD
                                    .encode(Sha256::digest(verifier.as_bytes()));
                            if computed_challenge != issued.code_challenge
                                || redirect != issued.redirect_uri
                            {
                                request
                                    .respond(
                                        tiny_http::Response::from_string(
                                            r#"{"error":"invalid_grant"}"#,
                                        )
                                        .with_status_code(400),
                                    )
                                    .unwrap();
                                continue;
                            }
                            let (access_token, refresh_token, expires_in) = {
                                let mut s = state.lock().unwrap();
                                s.next_token_id += 1;
                                s.auth_code_exchanges += 1;
                                let tok =
                                    format!("oauth-token-{}-{}", s.next_token_id, issued.email);
                                s.tokens.insert(tok.clone(), issued.email.clone());
                                let rtok = if s.omit_refresh_token {
                                    None
                                } else {
                                    let rt = format!(
                                        "refresh-token-{}-{}",
                                        s.next_token_id, issued.email
                                    );
                                    s.refresh_tokens.insert(rt.clone(), issued.email.clone());
                                    Some(rt)
                                };
                                (tok, rtok, s.expires_in)
                            };
                            let now_s = jiff::Timestamp::now().as_second();
                            let mut id_claims = json!({
                                "iss": base,
                                "sub": format!("sub:{}", issued.email),
                                "aud": issued.client_id,
                                "email": issued.email,
                                "email_verified": true,
                                "iat": now_s,
                                "exp": now_s + 300,
                            });
                            if let Some(n) = issued.nonce {
                                id_claims["nonce"] = json!(n);
                            }
                            let mut h = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
                            h.kid = Some(kid.clone());
                            let enc =
                                jsonwebtoken::EncodingKey::from_ec_pem(as_private_pem.as_bytes())
                                    .unwrap();
                            let id_token = jsonwebtoken::encode(&h, &id_claims, &enc).unwrap();
                            let mut resp_obj = json!({
                                "token_type": "Bearer",
                                "access_token": access_token,
                                "id_token": id_token,
                                "expires_in": expires_in,
                                "scope": "openid email mcp:read",
                            });
                            if let Some(rt) = refresh_token {
                                resp_obj["refresh_token"] = json!(rt);
                            }
                            let resp = resp_obj.to_string();
                            request
                                .respond(
                                    tiny_http::Response::from_string(resp)
                                        .with_header(hdr("Content-Type", "application/json")),
                                )
                                .unwrap();
                            continue;
                        }

                        if grant == "urn:ietf:params:oauth:grant-type:jwt-bearer" {
                            let assertion = params.get("assertion").cloned().unwrap_or_default();
                            let resource = params.get("resource").cloned().unwrap_or_default();
                            match verify_ema_assertion(&assertion, &base, &mcp) {
                                Ok(obs) => {
                                    let access_token = {
                                        let mut s = state.lock().unwrap();
                                        s.next_token_id += 1;
                                        let tok =
                                            format!("ema-token-{}-{}", s.next_token_id, obs.email);
                                        s.tokens.insert(tok.clone(), obs.email.clone());
                                        s.jwt_bearers.push(ObservedJwtBearer { resource, ..obs });
                                        tok
                                    };
                                    let resp = json!({
                                        "token_type": "Bearer",
                                        "access_token": access_token,
                                        "expires_in": 3600,
                                        "scope": params.get("scope").cloned().unwrap_or_default(),
                                    })
                                    .to_string();
                                    request
                                        .respond(
                                            tiny_http::Response::from_string(resp).with_header(
                                                hdr("Content-Type", "application/json"),
                                            ),
                                        )
                                        .unwrap();
                                }
                                Err(err) => {
                                    let resp = json!({"error": format!("invalid_grant: {err}")})
                                        .to_string();
                                    request
                                        .respond(
                                            tiny_http::Response::from_string(resp)
                                                .with_status_code(400)
                                                .with_header(hdr(
                                                    "Content-Type",
                                                    "application/json",
                                                )),
                                        )
                                        .unwrap();
                                }
                            }
                            continue;
                        }

                        request
                            .respond(
                                tiny_http::Response::from_string(
                                    r#"{"error":"unsupported_grant_type"}"#,
                                )
                                .with_status_code(400),
                            )
                            .unwrap();
                        continue;
                    }

                    if path == "/mcp" {
                        if method == tiny_http::Method::Get {
                            request.respond(tiny_http::Response::empty(405)).unwrap();
                            continue;
                        }
                        if method == tiny_http::Method::Delete {
                            request.respond(tiny_http::Response::empty(200)).unwrap();
                            continue;
                        }
                        let token = auth_header
                            .as_deref()
                            .and_then(|v| v.strip_prefix("Bearer "));
                        let email = token.and_then(|t| {
                            let s = state.lock().unwrap();
                            if s.revoked_tokens.contains(t) {
                                None
                            } else {
                                s.tokens.get(t).cloned()
                            }
                        });
                        let Some(email) = email else {
                            let mut discard = Vec::new();
                            let _ = request.as_reader().read_to_end(&mut discard);
                            let www = format!(
                                "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource\""
                            );
                            request
                                .respond(
                                    tiny_http::Response::empty(401)
                                        .with_header(hdr("WWW-Authenticate", &www)),
                                )
                                .unwrap();
                            continue;
                        };
                        let mut body = String::new();
                        request.as_reader().read_to_string(&mut body).unwrap();
                        let message: Value = serde_json::from_str(&body).unwrap();
                        let rpc_method = message["method"].as_str().unwrap_or_default();
                        let id = message["id"].clone();
                        let resp = match rpc_method {
                            "initialize" => tiny_http::Response::from_string(
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "protocolVersion": "2025-06-18",
                                        "capabilities": {"tools": {}},
                                        "serverInfo": {"name": opts.name, "version": "1"}
                                    }
                                })
                                .to_string(),
                            )
                            .with_header(hdr("Content-Type", "application/json"))
                            .with_header(hdr("Mcp-Session-Id", SESSION)),
                            "notifications/initialized" => {
                                tiny_http::Response::from_string("").with_status_code(202)
                            }
                            "tools/call" => tiny_http::Response::from_string(
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [{"type": "text", "text": format!("{} verified {email}", opts.name)}],
                                        "isError": false
                                    }
                                })
                                .to_string(),
                            )
                            .with_header(hdr("Content-Type", "application/json")),
                            _ => tiny_http::Response::from_string("").with_status_code(400),
                        };
                        request.respond(resp).unwrap();
                        continue;
                    }

                    request.respond(tiny_http::Response::empty(404)).unwrap();
                }
            })
        };

        Self {
            base_url,
            mcp_url,
            state,
            stop,
            thread: Some(thread),
        }
    }

    /// Simulate the user approving the OAuth `auth_link` in their browser:
    /// mints an authorization code bound to the PKCE challenge & redirect URI
    /// and delivers the real browser redirect to `redirect_uri` (the ingress receiver).
    pub fn approve_browser_link(&self, auth_link: &str, email: &str) -> u16 {
        let parsed = url::Url::parse(auth_link).unwrap();
        let params: std::collections::BTreeMap<String, String> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let state = params["state"].clone();
        let redirect_uri = params["redirect_uri"].clone();
        let code_challenge = params["code_challenge"].clone();
        let client_id = params["client_id"].clone();
        let nonce = params.get("nonce").cloned();
        let code = format!("authcode-{}", &state[..12.min(state.len())]);
        self.state.lock().unwrap().codes.insert(
            code.clone(),
            IssuedCode {
                email: email.to_owned(),
                redirect_uri: redirect_uri.clone(),
                code_challenge,
                nonce,
                client_id,
            },
        );
        let mut callback = url::Url::parse(&redirect_uri).unwrap();
        {
            let mut q = callback.query_pairs_mut();
            q.append_pair("code", &code);
            q.append_pair("state", &state);
            q.append_pair("iss", &self.base_url);
        }
        let client = instrukt_outside::http::build_client(&instrukt_outside::http::ClientConfig {
            follow_redirects: false,
            ..Default::default()
        })
        .unwrap();
        let url = callback.as_str().to_owned();
        instrukt_outside::runtime::block_on(async move {
            client.get(url).send().await.unwrap().status().as_u16()
        })
        .unwrap()
    }

    /// Revoke all currently issued Bearer tokens so the next MCP call gets HTTP 401.
    pub fn expire_all_tokens(&self) {
        let mut s = self.state.lock().unwrap();
        let keys: Vec<String> = s.tokens.keys().cloned().collect();
        for key in keys {
            s.revoked_tokens.insert(key);
        }
    }

    pub fn observed_jwt_bearers(&self) -> Vec<ObservedJwtBearer> {
        self.state.lock().unwrap().jwt_bearers.clone()
    }

    pub fn auth_code_exchanges(&self) -> usize {
        self.state.lock().unwrap().auth_code_exchanges
    }

    pub fn refresh_exchanges(&self) -> usize {
        self.state.lock().unwrap().refresh_exchanges
    }

    pub fn userinfo_calls(&self) -> usize {
        self.state.lock().unwrap().userinfo_calls
    }

    pub fn set_refresh_delay_ms(&self, ms: u64) {
        self.state.lock().unwrap().refresh_delay_ms = ms;
    }

    pub fn set_expires_in(&self, secs: i64) {
        self.state.lock().unwrap().expires_in = secs;
    }

    pub fn set_omit_refresh_token(&self, omit: bool) {
        self.state.lock().unwrap().omit_refresh_token = omit;
    }

    pub fn set_userinfo_status(&self, status: Option<u16>) {
        self.state.lock().unwrap().userinfo_status = status;
    }

    pub fn set_userinfo_email_override(&self, email: Option<String>) {
        self.state.lock().unwrap().userinfo_email_override = email;
    }

    pub fn set_refresh_error(&self, err: Option<(u16, String)>) {
        self.state.lock().unwrap().refresh_error = err;
    }

    pub fn revocations(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().revocations.clone()
    }

    pub fn total_token_requests(&self) -> usize {
        self.state.lock().unwrap().total_token_requests
    }

    pub fn revoked_refresh_tokens(&self) -> Vec<String> {
        self.state.lock().unwrap().revoked_refresh_tokens.clone()
    }
}

fn verify_ema_assertion(
    assertion: &str,
    expected_aud: &str,
    expected_resource: &str,
) -> Result<ObservedJwtBearer, String> {
    use base64::Engine as _;

    let header = jsonwebtoken::decode_header(assertion).map_err(|e| e.to_string())?;
    let parts: Vec<&str> = assertion.split('.').collect();
    if parts.len() != 3 {
        return Err("assertion is not a JWT".into());
    }
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| e.to_string())?;
    let unverified: Value = serde_json::from_slice(&payload_bytes).map_err(|e| e.to_string())?;
    let iss = unverified["iss"]
        .as_str()
        .ok_or_else(|| "missing iss".to_owned())?
        .to_owned();

    // Fetch real discovery + JWKS from the company's ingress endpoint!
    let client = instrukt_outside::http::build_client(&instrukt_outside::http::ClientConfig {
        connect_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(5),
        follow_redirects: false,
        ..Default::default()
    })
    .map_err(|e| format!("http client: {e}"))?;
    let disc_url = format!(
        "{}/.well-known/openid-configuration",
        iss.trim_end_matches('/')
    );
    let expected_iss = iss.clone();
    let (disc_text, jwks_text) = instrukt_outside::runtime::block_on(async move {
        let disc_resp = client
            .get(&disc_url)
            .send()
            .await
            .map_err(|e| format!("disc GET {disc_url}: {e}"))?;
        if disc_resp.status().as_u16() != 200 {
            return Err(format!(
                "discovery returned {}",
                disc_resp.status().as_u16()
            ));
        }
        let disc_text = disc_resp
            .text()
            .await
            .map_err(|e| format!("disc body: {e}"))?;
        let disc: Value =
            serde_json::from_str(&disc_text).map_err(|e| format!("disc json: {e}"))?;
        if disc["issuer"].as_str() != Some(expected_iss.as_str()) {
            return Err("discovery issuer mismatch".into());
        }
        let jwks_uri = disc["jwks_uri"].as_str().ok_or("missing jwks_uri")?;
        let jwks_resp = client
            .get(jwks_uri)
            .send()
            .await
            .map_err(|e| format!("jwks GET {jwks_uri}: {e}"))?;
        if jwks_resp.status().as_u16() != 200 {
            return Err(format!("jwks returned {}", jwks_resp.status().as_u16()));
        }
        let jwks_text = jwks_resp
            .text()
            .await
            .map_err(|e| format!("jwks body: {e}"))?;
        Ok::<_, String>((disc_text, jwks_text))
    })
    .map_err(|e| format!("runtime: {e}"))??;
    let _ = disc_text;
    let jwks: jsonwebtoken::jwk::JwkSet =
        serde_json::from_str(&jwks_text).map_err(|e| format!("jwks json: {e}"))?;
    let jwk = header
        .kid
        .as_deref()
        .and_then(|kid| jwks.find(kid))
        .or_else(|| jwks.keys.first())
        .ok_or("no matching JWK")?;
    let key = jsonwebtoken::DecodingKey::from_jwk(jwk).map_err(|e| e.to_string())?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
    validation.set_issuer(&[iss.as_str()]);
    validation.set_audience(&[expected_aud]);
    let verified = jsonwebtoken::decode::<Value>(assertion, &key, &validation)
        .map_err(|e| format!("JWT signature/claims failed: {e}"))?
        .claims;
    let iat = verified["iat"].as_i64().ok_or("missing iat")?;
    let exp = verified["exp"].as_i64().ok_or("missing exp")?;
    if exp - iat > 300 || exp <= iat {
        return Err("ID-JAG lifetime exceeds 5 minutes".into());
    }
    if verified["resource"].as_str() != Some(expected_resource) {
        return Err("resource mismatch".into());
    }
    let email = verified["email"]
        .as_str()
        .ok_or("missing email")?
        .to_owned();
    if verified["sub"].as_str() != Some(email.as_str()) {
        return Err("sub/email mismatch".into());
    }
    let jti = verified["jti"].as_str().ok_or("missing jti")?.to_owned();
    Ok(ObservedJwtBearer {
        iss,
        email,
        jti,
        resource: expected_resource.to_owned(),
    })
}

impl Drop for FakeOAuthMcp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}
