//! One MCP session over Streamable HTTP via `rmcp`, opened for one operation
//! and ended cleanly afterwards.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use http::{HeaderName, HeaderValue, header::AUTHORIZATION};
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, ContentBlock,
    ErrorData, Implementation, ProtocolVersion,
};
use rmcp::service::{ClientInitializeError, RoleClient, RunningService, ServiceError};
use rmcp::transport::common::client_side_sse::NeverRetry;
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    StreamableHttpError,
};
use serde::Serialize;
use serde_json::Value;

use super::_http;
use super::{McpError, Target};

/// The protocol versions instrukt speaks, newest first: each one's Streamable
/// HTTP transport (2024-11-05's HTTP+SSE transport is not spoken). The first
/// is the one offered.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// The JSON-RPC methods the client sends, in a session's order.
pub const INITIALIZE: &str = "initialize";
pub const INITIALIZED: &str = "notifications/initialized";
pub const LIST_TOOLS: &str = "tools/list";
pub const CALL_TOOL: &str = "tools/call";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MOST_PAGES: usize = 100;
const MOST_REPLY_BYTES: usize = 10 * 1024 * 1024;

/// One tool of a server's catalog.
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// The server's whole tool catalog, every page of it.
pub(super) fn list_tools(target: &Target<'_>) -> Result<Vec<Tool>, McpError> {
    let (client, config, name, url) = prepare_transport(target)?;
    _http::run_async(target.name, async move {
        let running = open_session(client, config, &name, &url).await?;
        let outcome = running
            .peer()
            .list_all_tools_with_max_pages(MOST_PAGES)
            .await
            .map_err(|error| map_service_error(&name, &url, error))
            .and_then(|listed| {
                listed
                    .items
                    .into_iter()
                    .map(|tool| convert_tool(&name, tool))
                    .collect()
            });
        let _ = running.cancel().await;
        outcome
    })?
}

/// The result of calling `tool`; a result the server marks as an error
/// is a failure carrying its content.
pub(super) fn call(target: &Target<'_>, tool: &str, arguments: Value) -> Result<Value, McpError> {
    let (client, config, name, url) = prepare_transport(target)?;
    let tool_name = tool.to_owned();
    _http::run_async(target.name, async move {
        let running = open_session(client, config, &name, &url).await?;
        let mut params = CallToolRequestParams::new(tool_name);
        if let Some(map) = arguments.as_object() {
            params = params.with_arguments(map.clone());
        }
        let outcome = running
            .call_tool(params)
            .await
            .map_err(|error| map_service_error(&name, &url, error))
            .and_then(|result| {
                if result.is_error == Some(true) {
                    return Err(McpError::Server {
                        server: name.clone(),
                        cause: cause_of_tool_error(&result),
                    });
                }
                serde_json::to_value(&result).map_err(|error| McpError::Server {
                    server: name.clone(),
                    cause: format!("failed to serialize tool result: {error}"),
                })
            });
        let _ = running.cancel().await;
        outcome
    })?
}

fn prepare_transport(
    target: &Target<'_>,
) -> Result<
    (
        reqwest::Client,
        StreamableHttpClientTransportConfig,
        String,
        String,
    ),
    McpError,
> {
    if target.authorization.is_some() && !instrukt_core::config::mcp_address_admitted(target.url) {
        return Err(McpError::Server {
            server: target.name.to_owned(),
            cause: "its address is neither https nor this computer's loopback, so no login is sent to it".into(),
        });
    }

    let client = _http::client(target.name, CONNECT_TIMEOUT, REQUEST_TIMEOUT, false)?;
    let mut config = StreamableHttpClientTransportConfig::with_uri(target.url)
        .max_json_response_size(MOST_REPLY_BYTES)
        .max_sse_event_size(MOST_REPLY_BYTES)
        .reinit_on_expired_session(false);
    config.allow_stateless = true;
    config.retry_config = Arc::new(NeverRetry::default());

    if let Some(authorization) = target.authorization {
        if let Some(bearer) = authorization.strip_prefix("Bearer ") {
            config = config.auth_header(bearer);
        } else {
            let mut value =
                HeaderValue::from_str(authorization).map_err(|error| McpError::Server {
                    server: target.name.to_owned(),
                    cause: format!("invalid Authorization header value: {error}"),
                })?;
            value.set_sensitive(true);
            let mut headers = HashMap::<HeaderName, HeaderValue>::new();
            headers.insert(AUTHORIZATION, value);
            config = config.custom_headers(headers);
        }
    }

    Ok((
        client,
        config,
        target.name.to_owned(),
        target.url.to_owned(),
    ))
}

async fn open_session(
    client: reqwest::Client,
    config: StreamableHttpClientTransportConfig,
    server: &str,
    url: &str,
) -> Result<RunningService<RoleClient, ClientConfig>, McpError> {
    let transport = StreamableHttpClientTransport::with_client(client, config);
    let client_config = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("instrukt", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2025_11_25);

    let running = client_config
        .serve(transport)
        .await
        .map_err(|error| map_init_error(server, url, error))?;

    let answered = running
        .peer_info()
        .map(|info| info.protocol_version.as_str().to_owned());
    if !answered
        .as_deref()
        .is_some_and(|version| PROTOCOL_VERSIONS.contains(&version))
    {
        let version = answered.unwrap_or_else(|| "none".to_owned());
        let _ = running.cancel().await;
        return Err(McpError::Server {
            server: server.to_owned(),
            cause: format!(
                "it answers protocol version {version}, and instrukt speaks {}",
                PROTOCOL_VERSIONS.join(", ")
            ),
        });
    }

    Ok(running)
}

fn convert_tool(server: &str, tool: rmcp::model::Tool) -> Result<Tool, McpError> {
    if tool.name.is_empty() {
        return Err(McpError::Server {
            server: server.to_owned(),
            cause: "a tool in its catalog has no name".into(),
        });
    }
    if tool.input_schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(McpError::Server {
            server: server.to_owned(),
            cause: format!("its tool `{}` has no input shape of type object", tool.name),
        });
    }
    let input_schema = tool.schema_as_json_value();
    Ok(Tool {
        name: tool.name.into_owned(),
        description: tool.description.map(Cow::into_owned),
        input_schema,
    })
}

fn cause_of_tool_error(result: &CallToolResult) -> String {
    let texts: Vec<&str> = result
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();
    if texts.is_empty() {
        serde_json::to_string(result).unwrap_or_else(|_| "tool reported an error".into())
    } else {
        texts.join("\n")
    }
}

fn format_rpc_error(error: &ErrorData) -> String {
    match &error.data {
        Some(Value::String(data)) => format!("{}: {data}", error.message),
        Some(Value::Null) | None => error.message.to_string(),
        Some(data) => format!("{}: {data}", error.message),
    }
}

fn map_init_error(server: &str, url: &str, error: ClientInitializeError) -> McpError {
    if error.is_authorization_required() {
        return McpError::Unauthorized {
            server: server.to_owned(),
            cause: error.to_string(),
        };
    }
    match error {
        ClientInitializeError::JsonRpcError(rpc) => McpError::Server {
            server: server.to_owned(),
            cause: format_rpc_error(&rpc),
        },
        ClientInitializeError::TransportError { error, .. } => {
            classify_transport_error(server, url, error.error.as_ref())
        }
        other => McpError::Server {
            server: server.to_owned(),
            cause: other.to_string(),
        },
    }
}

fn map_service_error(server: &str, url: &str, error: ServiceError) -> McpError {
    if error.is_authorization_required() {
        return McpError::Unauthorized {
            server: server.to_owned(),
            cause: error.to_string(),
        };
    }
    match error {
        ServiceError::McpError(rpc) => McpError::Server {
            server: server.to_owned(),
            cause: format_rpc_error(&rpc),
        },
        ServiceError::TransportSend(error) => {
            classify_transport_error(server, url, error.error.as_ref())
        }
        other => McpError::Server {
            server: server.to_owned(),
            cause: other.to_string(),
        },
    }
}

fn classify_transport_error(
    server: &str,
    url: &str,
    root: &(dyn std::error::Error + 'static),
) -> McpError {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(root);
    while let Some(err) = current {
        if err.is::<AuthRequiredError>() {
            return McpError::Unauthorized {
                server: server.to_owned(),
                cause: err.to_string(),
            };
        }
        if let Some(http_err) = err.downcast_ref::<StreamableHttpError<reqwest::Error>>() {
            return match http_err {
                StreamableHttpError::AuthRequired(auth) => McpError::Unauthorized {
                    server: server.to_owned(),
                    cause: auth.to_string(),
                },
                StreamableHttpError::UnexpectedServerResponse(msg)
                    if msg.starts_with("HTTP 401") =>
                {
                    McpError::Unauthorized {
                        server: server.to_owned(),
                        cause: msg.to_string(),
                    }
                }
                StreamableHttpError::Client(req_err)
                    if req_err.status() == Some(reqwest::StatusCode::UNAUTHORIZED) =>
                {
                    McpError::Unauthorized {
                        server: server.to_owned(),
                        cause: req_err.to_string(),
                    }
                }
                StreamableHttpError::Client(req_err)
                    if req_err.is_connect() || req_err.is_timeout() || req_err.is_request() =>
                {
                    McpError::Unreachable {
                        server: server.to_owned(),
                        url: url.to_owned(),
                        cause: req_err.to_string(),
                    }
                }
                StreamableHttpError::Io(io_err) => McpError::Unreachable {
                    server: server.to_owned(),
                    url: url.to_owned(),
                    cause: io_err.to_string(),
                },
                other => McpError::Server {
                    server: server.to_owned(),
                    cause: other.to_string(),
                },
            };
        }
        current = err.source();
    }
    McpError::Server {
        server: server.to_owned(),
        cause: root.to_string(),
    }
}
