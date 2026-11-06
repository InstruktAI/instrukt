//! One provider view, selected by typed source provenance, never by type alone.
use serde::Serialize;
use serde_json::{Map, Value};

use crate::events::{EventEnvelope, EventTime};

#[derive(Debug, Clone, Serialize)]
pub struct GithubView {
    pub source: &'static str,
    pub r#type: String,
    pub timestamp: EventTime,
    pub properties: Map<String, Value>,
    pub payload: Map<String, Value>,
}

pub fn github_view(event: &EventEnvelope) -> Option<GithubView> {
    if event.subject.kind() != "inbound_source"
        || event
            .extensions
            .get("inbound_verify")
            .and_then(Value::as_str)
            != Some("github")
    {
        return None;
    }
    let (domain, _) = event.subject.identity().split_once(':')?;
    let name = event.event_type.strip_prefix(&format!("{domain}."))?;
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .map_or(Value::Null, |s| Value::String(s.into()))
    };
    let mut properties = Map::from_iter([
        (
            "repo".into(),
            text(
                event
                    .data
                    .get("repository")
                    .and_then(|v| v.get("full_name")),
            ),
        ),
        (
            "sender".into(),
            text(event.data.get("sender").and_then(|v| v.get("login"))),
        ),
        ("action".into(), text(event.data.get("action"))),
        ("ref".into(), text(event.data.get("ref"))),
    ]);
    if name == "ping" {
        if let Some(zen) = event.data.get("zen").filter(|v| v.is_string()) {
            properties.insert("zen".into(), zen.clone());
        }
        if let Some(hook) = event
            .data
            .get("hook_id")
            .filter(|v| v.is_string() || v.is_i64() || v.is_u64())
        {
            properties.insert("hook_id".into(), hook.clone());
        }
    }
    Some(GithubView {
        source: "github",
        r#type: name.into(),
        timestamp: event.time.clone(),
        properties,
        payload: event.data.clone(),
    })
}
