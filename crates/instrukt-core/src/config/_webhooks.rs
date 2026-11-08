//! Raw project declarations and project-only signing material. No environment fallback.

use crate::home::Home;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use super::ConfigError;
use crate::machine::MachineId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebhookProtocol {
    Native,
    Github,
    #[serde(rename = "hmac-sha256")]
    HmacSha256,
}

impl WebhookProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Github => "github",
            Self::HmacSha256 => "hmac-sha256",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookSource {
    secret: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    state: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject_from: Option<String>,
    #[serde(default = "native", skip_serializing_if = "is_native")]
    verify: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_secret: Option<String>,
}

pub type WebhookSources = BTreeMap<String, WebhookSource>;

fn native() -> String {
    "native".into()
}

fn is_native(value: &str) -> bool {
    value == "native"
}

/// One event-domain segment, shared by declarations and HTTP routing.
pub fn webhook_domain(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// An event name consists of one or more nonempty domain-shaped segments.
pub fn webhook_name(text: &str) -> bool {
    text.split('.').all(webhook_domain)
}

/// Expanded values never implement Debug or Serialize.
pub struct SigningSecrets {
    pub current: String,
    pub previous: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ingress {
    pub computer: MachineId,
    pub port: u16,
}

fn invalid(key: &str, message: &str) -> ConfigError {
    ConfigError::InvalidValue {
        key: key.into(),
        message: message.into(),
    }
}

fn reference(text: &str) -> Option<&str> {
    let name = text.strip_prefix("${")?.strip_suffix('}')?;
    let mut bytes = name.bytes();
    if !bytes
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        || !bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return None;
    }
    Some(name)
}

pub(super) fn parse(raw: Option<&Value>) -> Result<WebhookSources, ConfigError> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    // Do not include serde's error: a malformed declaration may contain literal material.
    let sources: WebhookSources = serde_yaml_ng::from_value(raw.clone()).map_err(|_| invalid("webhooks", "expected domain-keyed source maps with required secret and optional previous_secret, verify, header, name_from or name"))?;
    if raw.as_mapping().is_some_and(|sources| {
        sources.values().any(|source| {
            [
                "previous_secret",
                "header",
                "name",
                "name_from",
                "subject_from",
            ]
            .iter()
            .any(|field| source.get(*field).is_some_and(Value::is_null))
        })
    }) {
        return Err(invalid(
            "webhooks",
            "declared optional source fields cannot be null",
        ));
    }
    for (domain, source) in &sources {
        let key = format!("webhooks.{domain}");
        if !webhook_domain(domain) {
            return Err(invalid(
                "webhooks",
                "source domains must match [a-z][a-z0-9_]*",
            ));
        }
        if source
            .subject_from
            .as_deref()
            .is_some_and(|field| field.is_empty() || field.split('.').any(str::is_empty))
            || source.state.iter().any(|name| !webhook_name(name))
            || (!source.state.is_empty()
                && source
                    .subject_from
                    .as_deref()
                    .is_none_or(|field| field.is_empty() || field.split('.').any(str::is_empty)))
        {
            return Err(invalid(
                &key,
                "state names require a nonempty dotted subject_from payload field",
            ));
        }
        let protocol = source.protocol()?;
        if source.name.is_some() && source.name_from.is_some()
            || protocol == WebhookProtocol::Github
                && (source.name.is_some() || source.name_from.is_some())
        {
            return Err(invalid(
                &key,
                "choose exactly one name source: native body, name_from, name or verify: github",
            ));
        }
        if source
            .name
            .as_deref()
            .is_some_and(|name| !webhook_name(name))
            || source.name_from.as_deref().is_some_and(str::is_empty)
        {
            return Err(invalid(
                &key,
                "name must match the event-name grammar; name_from must be a nonempty top-level field",
            ));
        }
        let header_valid = source.header.as_deref().is_some_and(|header| {
            !header.is_empty()
                && header
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        });
        if protocol == WebhookProtocol::HmacSha256 && !header_valid
            || protocol != WebhookProtocol::HmacSha256 && source.header.is_some()
        {
            return Err(invalid(
                &key,
                "hmac-sha256 requires a valid HTTP header name; native and github use their built-in headers",
            ));
        }
        for (field, text) in [
            ("secret", Some(&source.secret)),
            ("previous_secret", source.previous_secret.as_ref()),
        ] {
            if text.is_some_and(|text| reference(text).is_none()) {
                return Err(invalid(
                    &format!("webhooks.{domain}.{field}"),
                    "expected an exact ${NAME} reference; literal signing material is forbidden",
                ));
            }
        }
    }
    Ok(sources)
}

pub(super) fn ingress(raw: Option<&Value>) -> Result<Ingress, ConfigError> {
    let ingress: Ingress =
        serde_yaml_ng::from_value(raw.cloned().unwrap_or_default()).map_err(|_| {
            invalid(
                "ingress",
                "requires stable computer ID and TCP port (1..65535)",
            )
        })?;
    if ingress.port == 0 {
        return Err(invalid("ingress.port", "must be nonzero"));
    }
    Ok(ingress)
}

impl WebhookSource {
    pub fn protocol(&self) -> Result<WebhookProtocol, ConfigError> {
        match self.verify.as_str() {
            "native" => Ok(WebhookProtocol::Native),
            "github" => Ok(WebhookProtocol::Github),
            "hmac-sha256" => Ok(WebhookProtocol::HmacSha256),
            value
                if !value.is_empty()
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') =>
            {
                Err(invalid(
                    "webhooks.verify",
                    &format!("unknown verification scheme {value}"),
                ))
            }
            _ => Err(invalid(
                "webhooks.verify",
                "unknown verification scheme (expected native, github or hmac-sha256)",
            )),
        }
    }

    pub fn signature_header(&self) -> &str {
        match self.verify.as_str() {
            "github" => "X-Hub-Signature-256",
            "hmac-sha256" => self.header.as_deref().unwrap_or_default(),
            _ => "X-Hook-Signature",
        }
    }

    pub fn name_from(&self) -> Option<&str> {
        self.name_from.as_deref()
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn resolve(&self, home: &Home) -> Result<SigningSecrets, ConfigError> {
        Ok(SigningSecrets {
            current: super::resolve_secret(home, &self.secret)?,
            previous: self
                .previous_secret
                .as_deref()
                .map(|r| super::resolve_secret(home, r))
                .transpose()?,
        })
    }
}

pub(super) fn editor_schema() -> serde_json::Value {
    use serde_json::json;
    let reference = json!({"type": "string", "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$"});
    let source = json!({
        "type": "object", "additionalProperties": false, "required": ["secret"],
        "properties": {
            "secret": reference, "previous_secret": reference,
            "verify": {"enum": ["native", "github", "hmac-sha256"], "default": "native"},
            "header": {"type": "string", "pattern": "^[!#$%&'*+.^_`|~0-9A-Za-z-]+$"},
            "state": {"type":"array","items":{"type":"string","pattern":"^[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)*$"}},
            "subject_from": {"type":"string","minLength":1},
            "name_from": {"type": "string", "minLength": 1},
            "name": {"type": "string", "pattern": "^[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)*$"}
        },
        "allOf": [
            {"if":{"required":["state"],"properties":{"state":{"minItems":1}}},"then":{"required":["subject_from"]}},
            {"not": {"required": ["name", "name_from"]}},
            {"if": {"required": ["verify"], "properties": {"verify": {"const": "github"}}},
             "then": {"not": {"anyOf": [{"required": ["name"]}, {"required": ["name_from"]}]}}},
            {"if": {"required": ["verify"], "properties": {"verify": {"const": "hmac-sha256"}}},
             "then": {"required": ["header"]}, "else": {"not": {"required": ["header"]}}}
        ]
    });
    json!({"type": "object", "propertyNames": {"pattern": "^[a-z][a-z0-9_]*$"}, "additionalProperties": source})
}

pub(super) fn redact(value: &mut Value) {
    let Some(sources) = value.get_mut("webhooks").and_then(Value::as_mapping_mut) else {
        return;
    };
    for source in sources.values_mut() {
        if let Some(fields) = source.as_mapping_mut() {
            for field in ["secret", "previous_secret"] {
                if let Some(value) = fields.get_mut(field)
                    && !value.as_str().is_some_and(|text| reference(text).is_some())
                {
                    *value = Value::String("[redacted]".into());
                }
            }
        }
    }
}

impl WebhookSource {
    pub fn delivery_kind(&self, name: &str) -> crate::event_type::DeliveryKind {
        if self.state.iter().any(|n| n == name) {
            crate::event_type::DeliveryKind::State
        } else {
            crate::event_type::DeliveryKind::Occurrence
        }
    }
    pub fn subject_from(&self) -> Option<&str> {
        self.subject_from.as_deref()
    }
}
