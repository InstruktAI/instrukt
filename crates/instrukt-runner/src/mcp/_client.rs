//! One MCP session over the Streamable HTTP transport, opened for one
//! request and closed after it: initialize, the initialized notification,
//! the request, then a DELETE that ends the session.

use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use super::{McpError, Target};

/// The protocol version this client offers; the server's answer is the one
/// its later requests name.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long establishing a connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long one request, its answer included, may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// One tool of a server's catalog.
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

pub(super) struct Session<'a> {
    target: &'a Target<'a>,
    agent: ureq::Agent,
    session: Option<String>,
    /// The negotiated protocol version, once initialization answered.
    version: Option<String>,
    next: u64,
}

impl<'a> Session<'a> {
    /// Opens a session with the server.
    pub(super) fn open(target: &'a Target<'a>) -> Result<Self, McpError> {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        let mut session = Self {
            target,
            agent,
            session: None,
            version: None,
            next: 1,
        };
        let initialized = session.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "instrukt", "version": env!("CARGO_PKG_VERSION")},
            }),
        )?;
        session.version = Some(
            initialized
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION)
                .to_owned(),
        );
        session.notify("notifications/initialized")?;
        Ok(session)
    }

    /// The server's whole tool catalog, every page of it.
    pub(super) fn list_tools(self) -> Result<Vec<Tool>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut session = self;
        loop {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let page = session.request("tools/list", params)?;
            for tool in page
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                tools.push(Tool {
                    name: tool
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    input_schema: tool.get("inputSchema").cloned().unwrap_or(Value::Null),
                });
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|next| !next.is_empty())
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        session.close();
        Ok(tools)
    }

    /// The result of calling `tool`; a result the server marks as an error
    /// is a failure carrying its content.
    pub(super) fn call(mut self, tool: &str, arguments: Value) -> Result<Value, McpError> {
        let result = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        self.close();
        let result = result?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(self.failure(cause_of(&result)));
        }
        Ok(result)
    }

    fn failure(&self, cause: String) -> McpError {
        McpError::Server {
            server: self.target.name.to_owned(),
            cause,
        }
    }

    fn post(&self, body: &Value) -> Result<ureq::http::Response<ureq::Body>, McpError> {
        let mut request = self
            .agent
            .post(self.target.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(version) = &self.version {
            request = request.header("MCP-Protocol-Version", version);
        }
        if let Some(session) = &self.session {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(authorization) = self.target.authorization {
            request = request.header("Authorization", authorization);
        }
        request
            .send(body.to_string())
            .map_err(|error| McpError::Unreachable {
                server: self.target.name.to_owned(),
                url: self.target.url.to_owned(),
                cause: error.to_string(),
            })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next;
        self.next += 1;
        let mut response = self.post(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            self.session = Some(session.to_owned());
        }
        let status = response.status().as_u16();
        let streamed = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|kind| kind.starts_with("text/event-stream"));
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| self.failure(format!("unreadable answer to {method}: {error}")))?;
        if !(200..300).contains(&status) {
            return Err(self.failure(format!("HTTP {status} to {method}: {}", text.trim())));
        }
        let answer = if streamed {
            events(&text)
                .into_iter()
                .find(|message| message.get("id").and_then(Value::as_u64) == Some(id))
        } else {
            serde_json::from_str::<Value>(&text).ok()
        }
        .ok_or_else(|| self.failure(format!("no answer to {method}")))?;
        if let Some(error) = answer.get("error") {
            return Err(self.failure(cause_of_error(error)));
        }
        Ok(answer.get("result").cloned().unwrap_or(Value::Null))
    }

    fn notify(&mut self, method: &str) -> Result<(), McpError> {
        let mut response = self.post(&json!({"jsonrpc": "2.0", "method": method}))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let text = response.body_mut().read_to_string().unwrap_or_default();
            return Err(self.failure(format!("HTTP {status} to {method}: {}", text.trim())));
        }
        Ok(())
    }

    /// Ends the server's session; a server that keeps none, or refuses the
    /// request, needs nothing more.
    fn close(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        let mut request = self
            .agent
            .delete(self.target.url)
            .header("Mcp-Session-Id", &session)
            .header(
                "MCP-Protocol-Version",
                self.version.as_deref().unwrap_or(PROTOCOL_VERSION),
            );
        if let Some(authorization) = self.target.authorization {
            request = request.header("Authorization", authorization);
        }
        drop(request.call());
    }
}

/// The JSON-RPC messages of a text/event-stream body.
fn events(text: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut data: Vec<&str> = Vec::new();
    for line in text.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if !data.is_empty() {
                if let Ok(message) = serde_json::from_str(&data.join("\n")) {
                    messages.push(message);
                }
                data.clear();
            }
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    messages
}

/// A JSON-RPC error's cause: its message, and its data when it has any.
fn cause_of_error(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("an error without a message");
    match error.get("data") {
        Some(Value::String(data)) => format!("{message}: {data}"),
        Some(Value::Null) | None => message.to_owned(),
        Some(data) => format!("{message}: {data}"),
    }
}

/// The cause a tool result marked as an error carries: its text contents.
fn cause_of(result: &Value) -> String {
    let texts: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect();
    if texts.is_empty() {
        result.to_string()
    } else {
        texts.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_stream_yields_each_message_of_its_data_lines() {
        let messages = events(
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\nid: 7\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"id\":2,\"result\":{}}\n",
        );
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].get("id").and_then(Value::as_u64), Some(2));
    }
}
