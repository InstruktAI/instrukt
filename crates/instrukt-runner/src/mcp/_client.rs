//! One MCP session over the Streamable HTTP transport of MCP version
//! 2025-06-18, opened for one operation and ended after it: initialize, the
//! initialized notification, the operation, then one request to end the
//! session the server opened, on every path.
//!
//! Every reply is checked here, at the one transport boundary: a reply that is
//! not a well-formed JSON-RPC answer to the request it was sent, or that lacks
//! what the method requires, is a failure, never an empty result.

use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::{McpError, Target};

/// The protocol version instrukt speaks.
pub(super) const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long establishing a connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long one request, its answer included, may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// The most pages a catalog may take.
const MOST_PAGES: usize = 100;

/// One tool of a server's catalog.
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// A session with one server. Dropping it asks the server, once, to end the
/// session it opened.
pub(super) struct Session<'a> {
    target: &'a Target<'a>,
    agent: ureq::Agent,
    session: Option<String>,
    /// Set once initialization agreed on the version.
    initialized: bool,
    next: u64,
}

impl<'a> Session<'a> {
    /// Opens a session with the server. A login is sent only to an address
    /// that keeps it off the open network.
    pub(super) fn open(target: &'a Target<'a>) -> Result<Self, McpError> {
        if target.authorization.is_some()
            && !instrukt_core::config::mcp_address_admitted(target.url)
        {
            return Err(McpError::Server {
                server: target.name.to_owned(),
                cause: "its address is neither https nor this computer's loopback, so no login is sent to it".into(),
            });
        }
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
            initialized: false,
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
        let version = initialized.get("protocolVersion").and_then(Value::as_str);
        if version != Some(PROTOCOL_VERSION) {
            return Err(session.failure(format!(
                "it answers protocol version {}, and instrukt speaks {PROTOCOL_VERSION}",
                version.unwrap_or("none")
            )));
        }
        for required in ["capabilities", "serverInfo"] {
            if !initialized.get(required).is_some_and(Value::is_object) {
                return Err(session.failure(format!("its initialization lacks `{required}`")));
            }
        }
        session.initialized = true;
        session.notify("notifications/initialized")?;
        Ok(session)
    }

    /// The server's whole tool catalog, every page of it.
    pub(super) fn list_tools(mut self) -> Result<Vec<Tool>, McpError> {
        let mut tools = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            if seen.len() >= MOST_PAGES {
                return Err(self.failure(format!("its catalog takes more than {MOST_PAGES} pages")));
            }
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let page = self.request("tools/list", params)?;
            let listed = page
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| self.failure("its catalog page lacks `tools`".into()))?;
            for tool in listed {
                tools.push(self.tool(tool)?);
            }
            cursor = match page.get("nextCursor") {
                None | Some(Value::Null) => None,
                Some(Value::String(next)) => Some(next.clone()),
                Some(_) => return Err(self.failure("its `nextCursor` is not text".into())),
            };
            let Some(next) = &cursor else {
                break;
            };
            if seen.contains(next) {
                return Err(self.failure("it repeats a catalog cursor".into()));
            }
            seen.push(next.clone());
        }
        Ok(tools)
    }

    fn tool(&self, tool: &Value) -> Result<Tool, McpError> {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| self.failure("a tool in its catalog has no name".into()))?;
        let input_schema = tool
            .get("inputSchema")
            .filter(|schema| schema.is_object())
            .cloned()
            .ok_or_else(|| self.failure(format!("its tool `{name}` has no input shape")))?;
        let description = match tool.get("description") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => {
                return Err(self.failure(format!(
                    "its tool `{name}` has a description that is not text"
                )));
            }
        };
        Ok(Tool {
            name: name.to_owned(),
            description,
            input_schema,
        })
    }

    /// The result of calling `tool`; a result the server marks as an error
    /// is a failure carrying its content.
    pub(super) fn call(mut self, tool: &str, arguments: Value) -> Result<Value, McpError> {
        let result = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        )?;
        if !result.get("content").is_some_and(Value::is_array) {
            return Err(self.failure("its tool result lacks `content`".into()));
        }
        match result.get("isError") {
            None | Some(Value::Bool(false)) => Ok(result),
            Some(Value::Bool(true)) => Err(self.failure(cause_of(&result))),
            Some(_) => Err(self.failure("its `isError` is not true or false".into())),
        }
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
        if self.initialized {
            request = request.header("MCP-Protocol-Version", PROTOCOL_VERSION);
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

    /// The result of one request, from the one well-formed answer to it.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next;
        self.next += 1;
        let mut response = self.post(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        if self.session.is_none()
            && let Some(session) = response
                .headers()
                .get("mcp-session-id")
                .and_then(|value| value.to_str().ok())
        {
            self.session = Some(session.to_owned());
        }
        let status = response.status().as_u16();
        let kind = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !(200..300).contains(&status) {
            let text = response.body_mut().read_to_string().unwrap_or_default();
            return Err(self.failure(format!("HTTP {status} to {method}: {}", text.trim())));
        }
        let answer = if kind.starts_with("text/event-stream") {
            // The answer completes the request as soon as its event arrives.
            let reader = std::io::BufReader::new(response.body_mut().as_reader());
            answer_in_stream(reader, id)
                .map_err(|error| self.failure(format!("unreadable answer to {method}: {error}")))?
        } else if kind.starts_with("application/json") {
            let text = response
                .body_mut()
                .read_to_string()
                .map_err(|error| self.failure(format!("unreadable answer to {method}: {error}")))?;
            serde_json::from_str::<Value>(&text).ok()
        } else {
            return Err(self.failure(format!(
                "its answer to {method} is neither JSON nor an event stream"
            )));
        }
        .ok_or_else(|| self.failure(format!("no answer to {method}")))?;
        let envelope = answer
            .as_object()
            .filter(|envelope| envelope.get("jsonrpc").and_then(Value::as_str) == Some("2.0"))
            .filter(|envelope| envelope.get("id").and_then(Value::as_u64) == Some(id))
            .ok_or_else(|| {
                self.failure(format!(
                    "its answer to {method} is not a JSON-RPC answer to that request"
                ))
            })?;
        match (envelope.get("result"), envelope.get("error")) {
            (Some(Value::Object(result)), None) => Ok(Value::Object(result.clone())),
            (None, Some(error)) => Err(self.failure(cause_of_error(error, method))),
            _ => Err(self.failure(format!(
                "its answer to {method} carries neither one result object nor one error"
            ))),
        }
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
}

impl Drop for Session<'_> {
    /// Asks the server, once, to end the session it opened; a server that does
    /// not end it is logged and changes no outcome.
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        let mut request = self
            .agent
            .delete(self.target.url)
            .header("Mcp-Session-Id", &session)
            .header("MCP-Protocol-Version", PROTOCOL_VERSION);
        if let Some(authorization) = self.target.authorization {
            request = request.header("Authorization", authorization);
        }
        match request.call() {
            Ok(response) if response.status().is_success() || response.status().as_u16() == 405 => {
            }
            Ok(response) => tracing::warn!(
                event = "mcp_session_not_ended",
                server = self.target.name,
                status = response.status().as_u16()
            ),
            Err(_) => tracing::warn!(
                event = "mcp_session_not_ended",
                server = self.target.name,
                status = "unreachable"
            ),
        }
    }
}

/// The JSON-RPC message answering `id` in an event stream, read until it
/// arrives; earlier events, notifications and requests among them, are passed.
fn answer_in_stream(reader: impl std::io::BufRead, id: u64) -> std::io::Result<Option<Value>> {
    let mut data: Vec<String> = Vec::new();
    let mut lines = reader.lines();
    loop {
        let line = lines.next().transpose()?;
        let ended = line.is_none();
        let line = line.unwrap_or_default();
        if line.is_empty() {
            if !data.is_empty() {
                if let Ok(message) = serde_json::from_str::<Value>(&data.join("\n"))
                    && message.get("id").and_then(Value::as_u64) == Some(id)
                    && (message.get("result").is_some() || message.get("error").is_some())
                {
                    return Ok(Some(message));
                }
                data.clear();
            }
            if ended {
                return Ok(None);
            }
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest).to_owned());
        }
    }
}

/// A JSON-RPC error's cause: its message, and its data when it has any.
fn cause_of_error(error: &Value, method: &str) -> String {
    let Some((Some(message), Some(_))) = error.as_object().map(|error: &Map<String, Value>| {
        (
            error.get("message").and_then(Value::as_str),
            error.get("code").and_then(Value::as_i64),
        )
    }) else {
        return format!("its error answer to {method} has no code and message");
    };
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
    use super::{Value, answer_in_stream};

    #[test]
    fn an_answer_completes_the_stream_read_at_its_own_event() {
        let stream = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"id\":2,\"result\":{}}\n\nnot a valid line that must never be read\n";
        let answer = answer_in_stream(std::io::Cursor::new(stream), 2).unwrap();
        assert_eq!(
            answer
                .as_ref()
                .and_then(|answer| answer.get("id"))
                .and_then(Value::as_u64),
            Some(2)
        );
        let none = answer_in_stream(
            std::io::Cursor::new("data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}\n\n"),
            2,
        )
        .unwrap();
        assert!(none.is_none());
    }
}
