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
