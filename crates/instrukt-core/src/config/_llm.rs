//! Global capability-free LLM endpoint configuration (`llm` in
//! `~/.instrukt/instrukt.yaml`)
//! (`project/instrukt/spec/feature/configuration/layered-configuration`,
//! `project/instrukt/spec/domain/screening`).

use std::fmt;
use std::time::Duration;

use serde_json::json;
use serde_yaml_ng::Value;

use super::_error::{ConfigError, Finding, FindingKind};
use super::_interpolate::{join, key_text};
use super::_layer::{Layer, inspected, read};
use super::_schema::{KeyFinding, LayerKind, non_blank_pattern, secret_reference};
use super::_secrets::{SecretSnapshot, SecretSource, SecretValue, reference};
use crate::home::Home;

/// The top-level global-layer key holding purpose-scoped LLM endpoint settings.
pub const LLM_KEY: &str = "llm";

/// The purpose key under `llm` for capability-free calls and `instrukt is-safe`.
pub const LLM_SCREENING_KEY: &str = "screening";

/// Default per-request HTTP timeout (seconds) when `llm.screening.timeout` is omitted.
pub const DEFAULT_LLM_TIMEOUT_SECS: u64 = 45;

/// Minimum allowed `llm.screening.timeout` (seconds).
pub const MIN_LLM_TIMEOUT_SECS: u64 = 1;

/// Maximum allowed `llm.screening.timeout` (seconds).
pub const MAX_LLM_TIMEOUT_SECS: u64 = 600;

const REQUIRED_FIELDS: [&str; 2] = ["url", "model"];
const OPTIONAL_FIELDS: [&str; 2] = ["api_key", "timeout"];
const REDACTED_SECRET_PLACEHOLDER: &str = "<protected-file reference>";

/// Validated, admitted OpenAI-compatible endpoint configuration with optional
/// resolved API key held solely for HTTP I/O. Never implements `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct LlmEndpointConfig {
    pub url: url::Url,
    pub model: String,
    pub api_key: Option<SecretValue>,
    pub timeout: Duration,
}

impl fmt::Debug for LlmEndpointConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmEndpointConfig")
            .field("url", &self.url.as_str())
            .field("model", &self.model)
            .field("api_key", &self.api_key)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Three-state admission result for the machine's global `llm.screening`
/// declaration.
#[derive(Debug, Clone)]
pub enum LlmAdmission {
    /// Global layer has no `llm.screening` declaration (or global layer does not exist).
    Unconfigured,
    /// Global `llm.screening` declaration validated and resolved its optional API key.
    Admitted(Box<LlmEndpointConfig>),
    /// Global `llm` / `llm.screening` declaration is present but invalid or missing its secret.
    Withheld(Vec<Finding>),
}

/// Whether `finding` withholds screening admission while leaving the rest of
/// the global layer admitted.
pub fn withholds_llm(finding: &Finding) -> bool {
    finding.key == LLM_KEY || finding.key.starts_with("llm.")
}

/// Inspects the global layer's `llm.screening` declaration directly without
/// loading `agents.yaml` or project layers.
pub fn inspect_global_screening(home: &Home) -> Result<LlmAdmission, ConfigError> {
    let path = home.global_config();
    let Some(doc) = read(&path)? else {
        return Ok(LlmAdmission::Unconfigured);
    };
    let global = inspected(&Layer::Global, &path, doc.value)?;
    let Some(raw) = global.get(LLM_KEY).filter(|value| !value.is_null()) else {
        return Ok(LlmAdmission::Unconfigured);
    };
    let mut key_findings = Vec::new();
    check(raw, LLM_KEY, &mut key_findings);
    let screening_raw = raw
        .as_mapping()
        .and_then(|map| map.get(LLM_SCREENING_KEY))
        .filter(|value| !value.is_null());
    if key_findings.is_empty() && screening_raw.is_none() {
        return Ok(LlmAdmission::Unconfigured);
    }
    let secrets =
        needs_secret_snapshot(raw).then(|| SecretSnapshot::read(home, &SecretSource::Machine));
    if let Some(secrets) = &secrets {
        check_secrets(raw, secrets, &mut key_findings);
    }
    if !key_findings.is_empty() {
        let findings = key_findings
            .into_iter()
            .map(|finding| Finding {
                document: path.clone(),
                key: finding.key,
                kind: finding.kind,
                message: finding.message,
            })
            .collect();
        return Ok(LlmAdmission::Withheld(findings));
    }
    let Some(endpoint_raw) = screening_raw else {
        return Ok(LlmAdmission::Unconfigured);
    };
    let config = parse_endpoint(endpoint_raw, secrets.as_ref())?;
    Ok(LlmAdmission::Admitted(Box::new(config)))
}

/// Resolves `llm.screening` from an effective configuration's value using
/// `<selected INSTRUKT_HOME>/secrets.env`.
pub(super) fn from_effective(
    home: &Home,
    value: &Value,
) -> Result<Option<LlmEndpointConfig>, ConfigError> {
    let Some(raw) = value.get(LLM_KEY).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let mut findings = Vec::new();
    check(raw, LLM_KEY, &mut findings);
    let screening_raw = raw
        .as_mapping()
        .and_then(|map| map.get(LLM_SCREENING_KEY))
        .filter(|v| !v.is_null());
    if findings.is_empty() && screening_raw.is_none() {
        return Ok(None);
    }
    let secrets =
        needs_secret_snapshot(raw).then(|| SecretSnapshot::read(home, &SecretSource::Machine));
    if let Some(secrets) = &secrets {
        check_secrets(raw, secrets, &mut findings);
    }
    if let Some(first) = findings.into_iter().next() {
        return Err(ConfigError::InvalidValue {
            key: first.key,
            message: first.message,
        });
    }
    let Some(endpoint_raw) = screening_raw else {
        return Ok(None);
    };
    parse_endpoint(endpoint_raw, secrets.as_ref()).map(Some)
}

/// Redacts non-reference literal `api_key` values and embedded URL credentials,
/// queries, or fragments under `llm.*` before display or interpolation.
pub(super) fn redact(value: &mut Value) {
    let Some(llm) = value.get_mut(LLM_KEY).and_then(Value::as_mapping_mut) else {
        return;
    };
    for entry in llm.values_mut() {
        let Some(map) = entry.as_mapping_mut() else {
            continue;
        };
        if let Some(api_key) = map.get_mut("api_key")
            && !api_key.is_null()
            && !api_key.as_str().is_some_and(super::is_secret_reference)
        {
            *api_key = Value::String(REDACTED_SECRET_PLACEHOLDER.into());
        }
        if let Some(url_val) = map.get_mut("url")
            && let Some(raw) = url_val.as_str()
            && let Some(redacted) = redact_endpoint_url(raw)
        {
            *url_val = Value::String(redacted);
        }
    }
}

fn redact_endpoint_url(raw: &str) -> Option<String> {
    match url::Url::parse(raw.trim()) {
        Ok(mut parsed) => {
            let has_sensitive = !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some();
            if !has_sensitive {
                return None;
            }
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            Some(parsed.to_string())
        }
        Err(_) => {
            if raw.contains('@') || raw.contains('?') || raw.contains('#') {
                Some(REDACTED_SECRET_PLACEHOLDER.into())
            } else {
                None
            }
        }
    }
}

/// Validates the shape of `llm` at `path` (syntax-only for `${NAME}` secret
/// references; secret-file resolution happens at call entry).
pub(super) fn check(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(map) = value.as_mapping() else {
        out.push(KeyFinding::new(
            path,
            FindingKind::InvalidValue,
            "expected a mapping",
        ));
        return;
    };
    for (key, entry_value) in map {
        let purpose = key_text(key);
        let child = join(path, &purpose);
        if purpose != LLM_SCREENING_KEY {
            out.push(KeyFinding::new(
                &child,
                FindingKind::UnknownKey,
                "unknown key",
            ));
            continue;
        }
        check_endpoint(entry_value, &child, out);
    }
}

fn check_endpoint(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(map) = value.as_mapping() else {
        out.push(KeyFinding::new(
            path,
            FindingKind::InvalidValue,
            "expected a mapping",
        ));
        return;
    };
    for required in REQUIRED_FIELDS {
        if !map.get(required).is_some_and(|v| !v.is_null()) {
            out.push(KeyFinding::new(
                &join(path, required),
                FindingKind::Missing,
                "missing",
            ));
        }
    }
    for (key, field_value) in map {
        let name = key_text(key);
        let child = join(path, &name);
        if !REQUIRED_FIELDS.contains(&name.as_str()) && !OPTIONAL_FIELDS.contains(&name.as_str()) {
            out.push(KeyFinding::new(
                &child,
                FindingKind::UnknownKey,
                "unknown key",
            ));
            continue;
        }
        if field_value.is_null() {
            continue;
        }
        match name.as_str() {
            "url" => {
                if !field_value
                    .as_str()
                    .is_some_and(super::_webhooks::ingress_url_admitted)
                {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "expected an absolute https URL (or loopback http URL) without embedded credentials, query or fragment",
                    ));
                }
            }
            "model" => {
                if !field_value.as_str().is_some_and(|s| !s.trim().is_empty()) {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "expected a non-empty model identifier",
                    ));
                }
            }
            "api_key" => {
                if !field_value.as_str().is_some_and(|s| reference(s).is_some()) {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "credentials must be an exact `${NAME}` secret reference into `~/.instrukt/secrets.env`; store the value with `instrukt secrets set <NAME>`",
                    ));
                }
            }
            "timeout"
                if field_value.as_u64().is_none_or(|seconds| {
                    !(MIN_LLM_TIMEOUT_SECS..=MAX_LLM_TIMEOUT_SECS).contains(&seconds)
                }) =>
            {
                out.push(KeyFinding::new(
                    &child,
                    FindingKind::InvalidValue,
                    format!(
                        "expected an integer timeout from {MIN_LLM_TIMEOUT_SECS} to {MAX_LLM_TIMEOUT_SECS} seconds"
                    ),
                ));
            }
            _ => {}
        }
    }
}

fn needs_secret_snapshot(raw: &Value) -> bool {
    raw.as_mapping()
        .and_then(|map| map.get(LLM_SCREENING_KEY))
        .and_then(Value::as_mapping)
        .and_then(|ep| ep.get("api_key"))
        .and_then(Value::as_str)
        .is_some_and(|s| reference(s).is_some())
}

/// Checks at call entry that an optional `llm.screening.api_key` `${NAME}`
/// reference resolves to a non-blank value in `<selected INSTRUKT_HOME>/secrets.env`.
pub(super) fn check_secrets(value: &Value, secrets: &SecretSnapshot, out: &mut Vec<KeyFinding>) {
    let Some(ref_text) = value
        .as_mapping()
        .and_then(|map| map.get(LLM_SCREENING_KEY))
        .and_then(Value::as_mapping)
        .and_then(|ep| ep.get("api_key"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let Some(var_name) = reference(ref_text) else {
        return;
    };
    if let Err(error) = secrets.resolve(ref_text) {
        out.push(KeyFinding::new(
            &join(&join(LLM_KEY, LLM_SCREENING_KEY), "api_key"),
            FindingKind::Missing,
            format!("{error}; store it with `instrukt secrets set {var_name}`"),
        ));
    }
}

/// JSON Schema describing `llm` and `llm.screening` in the global layer.
pub(super) fn json_schema() -> serde_json::Value {
    let non_blank = non_blank_pattern();
    json!({
        "type": "object",
        "description": "Direct capability-free LLM endpoints keyed by internal purpose (global layer only).",
        "additionalProperties": false,
        "properties": {
            "screening": {
                "type": "object",
                "description": "OpenAI-compatible chat-completions endpoint used by capability-free calls and `instrukt is-safe`.",
                "required": REQUIRED_FIELDS,
                "additionalProperties": false,
                "properties": {
                    "url": {
                        "type": "string",
                        "format": "uri",
                        "description": "Base URL of an OpenAI-compatible chat-completions service (`https://...`, or loopback `http://...`) with no embedded credentials, query, or fragment."
                    },
                    "model": {
                        "type": "string",
                        "pattern": non_blank,
                        "description": "Non-empty model identifier served by the endpoint. Bring a plain model with no built-in search or browsing; instrukt performs no runtime check on which model is configured."
                    },
                    "api_key": secret_reference(
                        "Optional API key for the screening endpoint (omit for local servers that need no key)",
                        LayerKind::Global,
                    ),
                    "timeout": {
                        "type": "integer",
                        "minimum": MIN_LLM_TIMEOUT_SECS,
                        "maximum": MAX_LLM_TIMEOUT_SECS,
                        "default": DEFAULT_LLM_TIMEOUT_SECS,
                        "description": "Per-request HTTP timeout in whole seconds."
                    }
                }
            }
        }
    })
}

fn parse_endpoint(
    raw: &Value,
    secrets: Option<&SecretSnapshot>,
) -> Result<LlmEndpointConfig, ConfigError> {
    let base = join(LLM_KEY, LLM_SCREENING_KEY);
    let map = raw.as_mapping().ok_or_else(|| ConfigError::InvalidValue {
        key: base.clone(),
        message: "expected a mapping".into(),
    })?;
    let str_field = |name: &str| -> Result<&str, ConfigError> {
        map.get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| ConfigError::InvalidValue {
                key: join(&base, name),
                message: "missing or invalid string".into(),
            })
    };
    let url_raw = str_field("url")?.trim();
    let url = url::Url::parse(url_raw).map_err(|err| ConfigError::InvalidValue {
        key: join(&base, "url"),
        message: err.to_string(),
    })?;
    let model = str_field("model")?.trim().to_owned();
    let api_key = match map.get("api_key").filter(|v| !v.is_null()) {
        Some(v) => {
            let ref_text = v.as_str().ok_or_else(|| ConfigError::InvalidValue {
                key: join(&base, "api_key"),
                message: "expected exact ${NAME} secret reference".into(),
            })?;
            let secrets = secrets.ok_or_else(|| ConfigError::InvalidValue {
                key: join(&base, "api_key"),
                message: "missing secrets snapshot".into(),
            })?;
            Some(SecretValue::new(secrets.resolve(ref_text)?))
        }
        None => None,
    };
    let timeout_secs = match map.get("timeout").filter(|v| !v.is_null()) {
        Some(v) => v
            .as_u64()
            .filter(|seconds| (MIN_LLM_TIMEOUT_SECS..=MAX_LLM_TIMEOUT_SECS).contains(seconds))
            .ok_or_else(|| ConfigError::InvalidValue {
                key: join(&base, "timeout"),
                message: format!(
                    "expected an integer timeout from {MIN_LLM_TIMEOUT_SECS} to {MAX_LLM_TIMEOUT_SECS} seconds"
                ),
            })?,
        None => DEFAULT_LLM_TIMEOUT_SECS,
    };
    Ok(LlmEndpointConfig {
        url,
        model,
        api_key,
        timeout: Duration::from_secs(timeout_secs),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LayerKind;

    #[test]
    fn check_accepts_https_url() {
        let doc: Value = serde_yaml_ng::from_str(
            "screening:\n  url: 'https://api.example.com/v1'\n  model: plain-model\n  api_key: null\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn check_accepts_loopback_http_url() {
        let doc: Value = serde_yaml_ng::from_str(
            "screening:\n  url: 'http://127.0.0.1:1234/v1'\n  model: plain-model\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn check_rejects_non_loopback_http_url() {
        let doc: Value = serde_yaml_ng::from_str(
            "screening:\n  url: 'http://remote.example.com/v1'\n  model: plain-model\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(findings.iter().any(|f| f.key == "llm.screening.url"));
    }

    #[test]
    fn check_rejects_url_with_embedded_credentials_without_disclosing_password() {
        let doc: Value = serde_yaml_ng::from_str(
            "screening:\n  url: 'https://user:pass-leak@api.example.com/v1'\n  model: plain-model\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(findings.iter().any(|f| f.key == "llm.screening.url"));
        assert!(findings.iter().all(|f| !f.message.contains("pass-leak")));
    }

    #[test]
    fn check_accepts_timeout_within_bounds() {
        let doc: Value = serde_yaml_ng::from_str(&format!(
            "screening:\n  url: http://127.0.0.1:1234/v1\n  model: plain\n  timeout: {DEFAULT_LLM_TIMEOUT_SECS}\n"
        ))
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn check_rejects_out_of_range_timeout() {
        let doc: Value = serde_yaml_ng::from_str(
            "screening:\n  url: http://127.0.0.1:1234/v1\n  model: plain\n  timeout: 0\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(findings.iter().any(|f| f.key == "llm.screening.timeout"));
    }

    #[test]
    fn check_rejects_non_integer_timeout() {
        let doc: Value = serde_yaml_ng::from_str(
            "screening:\n  url: http://127.0.0.1:1234/v1\n  model: plain\n  timeout: '45'\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(findings.iter().any(|f| f.key == "llm.screening.timeout"));
    }

    #[test]
    fn check_rejects_blank_model() {
        let doc: Value = serde_yaml_ng::from_str(
            "screening:\n  url: http://127.0.0.1:1234/v1\n  model: '   '\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(findings.iter().any(|f| f.key == "llm.screening.model"));
    }

    #[test]
    fn check_rejects_literal_api_key_without_disclosing_value() {
        let doc: Value = serde_yaml_ng::from_str(
            "screening:\n  url: http://127.0.0.1:1234/v1\n  model: plain\n  api_key: sk-literal-secret\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(findings.iter().any(|f| f.key == "llm.screening.api_key"));
        assert!(
            findings
                .iter()
                .all(|f| !f.message.contains("sk-literal-secret"))
        );
    }

    #[test]
    fn check_rejects_unknown_purpose() {
        let doc: Value =
            serde_yaml_ng::from_str("other_purpose:\n  url: https://example.com\n").unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "llm.other_purpose" && f.kind == FindingKind::UnknownKey)
        );
    }

    #[test]
    fn check_rejects_unknown_screening_subkey() {
        let doc: Value = serde_yaml_ng::from_str(
            "screening:\n  url: http://127.0.0.1:1234/v1\n  model: plain\n  extra: 1\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&doc, LLM_KEY, &mut findings);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "llm.screening.extra" && f.kind == FindingKind::UnknownKey)
        );
    }

    #[test]
    fn llm_is_refused_in_project_layer() {
        let project_doc: Value = serde_yaml_ng::from_str(&format!(
            "schema_version: {}\nllm:\n  screening:\n    url: http://127.0.0.1:1234/v1\n    model: m\n",
            crate::config::PROJECT_SCHEMA_VERSION
        ))
        .unwrap();
        let project_findings = crate::config::_schema::check(&project_doc, LayerKind::Project);
        assert!(
            project_findings
                .iter()
                .any(|f| f.key == "llm" && f.kind == FindingKind::NotPermittedInLayer)
        );
    }

    #[test]
    fn llm_is_refused_in_company_layer() {
        let company_doc: Value = serde_yaml_ng::from_str(&format!(
            "schema_version: {}\ncompany: acme\nllm:\n  screening:\n    url: http://127.0.0.1:1234/v1\n    model: m\n",
            crate::config::COMPANY_SCHEMA_VERSION
        ))
        .unwrap();
        let company_findings = crate::config::_schema::check(&company_doc, LayerKind::Company);
        assert!(
            company_findings
                .iter()
                .any(|f| f.key == "llm" && f.kind == FindingKind::NotPermittedInLayer)
        );
    }

    #[test]
    fn redact_replaces_literal_api_key() {
        let mut doc: Value = serde_yaml_ng::from_str(
            "llm:\n  screening:\n    url: http://127.0.0.1:1234/v1\n    model: plain-model\n    api_key: literal-key-leaked\n",
        )
        .unwrap();
        redact(&mut doc);
        assert_eq!(
            doc["llm"]["screening"]["api_key"].as_str(),
            Some(REDACTED_SECRET_PLACEHOLDER)
        );
    }

    #[test]
    fn redact_strips_embedded_url_credentials() {
        let mut doc: Value = serde_yaml_ng::from_str(
            "llm:\n  screening:\n    url: https://user:url-secret@api.example.com:8443/v1?token=q-secret#frag\n    model: plain-model\n",
        )
        .unwrap();
        redact(&mut doc);
        assert_eq!(
            doc["llm"]["screening"]["url"].as_str(),
            Some("https://api.example.com:8443/v1")
        );
    }

    #[test]
    fn redact_preserves_secret_reference() {
        let mut doc: Value = serde_yaml_ng::from_str(
            "llm:\n  screening:\n    url: http://127.0.0.1:1234/v1\n    model: plain-model\n    api_key: ${SCREENING_KEY}\n",
        )
        .unwrap();
        redact(&mut doc);
        assert_eq!(
            doc["llm"]["screening"]["api_key"].as_str(),
            Some("${SCREENING_KEY}")
        );
    }
}
