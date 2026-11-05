//! Raw project declarations and project-only signing material. No environment fallback.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use super::ConfigError;
use crate::machine::MachineId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebhookProtocol {
    Native,
    Github,
}

impl WebhookProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Github => "github",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookSource {
    secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_secret: Option<String>,
}

pub type WebhookSources = BTreeMap<WebhookProtocol, WebhookSource>;

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
    let sources: WebhookSources = serde_yaml_ng::from_value(raw.clone()).map_err(|_| invalid("webhooks", "expected only native/github source maps with required secret and optional previous_secret"))?;
    if raw.as_mapping().is_some_and(|sources| {
        sources
            .values()
            .any(|source| source.get("previous_secret").is_some_and(Value::is_null))
    }) {
        return Err(invalid(
            "webhooks.previous_secret",
            "a declared previous_secret must be an exact ${NAME} reference",
        ));
    }
    for (protocol, source) in &sources {
        for (field, text) in [
            ("secret", Some(&source.secret)),
            ("previous_secret", source.previous_secret.as_ref()),
        ] {
            if text.is_some_and(|text| reference(text).is_none()) {
                return Err(invalid(
                    &format!("webhooks.{}.{field}", protocol.as_str()),
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
    pub fn resolve(&self, project: &Path) -> Result<SigningSecrets, ConfigError> {
        let error = |reference: &str| {
            invalid(
                "webhooks",
                &format!(
                    "project {} cannot resolve {reference} from its .env",
                    project.display()
                ),
            )
        };
        let dotenv = project.join(".env");
        let metadata = std::fs::symlink_metadata(&dotenv).map_err(|_| error(&self.secret))?;
        if !metadata.file_type().is_file() {
            return Err(error(&self.secret));
        }
        let bytes = std::fs::read(dotenv).map_err(|_| error(&self.secret))?;
        let text = std::str::from_utf8(&bytes).map_err(|_| error(&self.secret))?;
        // Disable dotenvy's process-environment substitution while retaining
        // its quoting and escape grammar. Signing values are literal material.
        let literal = literal_values(text);
        let values = dotenvy::from_read_iter(literal.as_bytes())
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|_| error(&self.secret))?;
        let read = |text: &str| -> Result<String, ConfigError> {
            let name = reference(text)
                .ok_or_else(|| invalid("webhooks", "expected an exact ${NAME} reference"))?;
            values
                .get(name)
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .ok_or_else(|| error(text))
        };
        Ok(SigningSecrets {
            current: read(&self.secret)?,
            previous: self.previous_secret.as_deref().map(read).transpose()?,
        })
    }
}

pub(super) fn editor_schema() -> serde_json::Value {
    use serde_json::json;
    let source = json!({
        "type": "object", "additionalProperties": false, "required": ["secret"],
        "properties": {
            "secret": {"type": "string", "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$"},
            "previous_secret": {"type": "string", "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$"}
        }
    });
    json!({"type": "object", "additionalProperties": false, "properties": {"native": source, "github": source}})
}

/// Quote only unescaped interpolation markers; no variable lookup can occur in
/// the dotenv parser. Strong quotes already make their contents literal.
fn literal_values(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut prior_space = true;
    for ch in text.chars() {
        if comment {
            out.push(ch);
            if ch == '\n' {
                comment = false;
                prior_space = true;
            }
            continue;
        }
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' && quote != Some('\'') {
            out.push(ch);
            escaped = true;
        } else if ch == '#' && quote.is_none() && prior_space {
            comment = true;
            out.push(ch);
        } else if matches!(ch, '\'' | '"') && (quote.is_none() || quote == Some(ch)) {
            quote = if quote.is_some() { None } else { Some(ch) };
            out.push(ch);
        } else {
            if ch == '$' && quote != Some('\'') {
                out.push('\\');
            }
            out.push(ch);
        }
        prior_space = ch.is_whitespace();
    }
    out
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
