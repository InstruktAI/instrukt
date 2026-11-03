//! Validation of one layer document as a whole: its structure, the
//! cross-key rules and its references, judged against the configuration it
//! takes part in.

use std::path::Path;

use serde_yaml_ng::Value;

use super::_effective::effective_value;
use super::_error::{ConfigError, Finding};
use super::_interpolate::{Interpolator, Mode, join, key_text};
use super::_layer::{Layer, read_usable, require_usable};
use super::_runtime;
use super::_schema::{self, CURRENT_SCHEMA_VERSION, LayerKind};
use crate::home::Home;

/// The findings of `layer` as stored at `path` with content `value`.
pub(crate) fn findings_for(
    home: &Home,
    layer: &Layer,
    path: &Path,
    value: &Value,
) -> Result<Vec<Finding>, ConfigError> {
    let kind = layer.kind();
    let mut found = _schema::check(value, kind);
    if let Some(v) = value.get("schema_version").and_then(Value::as_u64)
        && v != CURRENT_SCHEMA_VERSION
    {
        found.push((
            "schema_version".to_owned(),
            format!("this instrukt writes schema_version {CURRENT_SCHEMA_VERSION}"),
        ));
    }
    let effective = match kind {
        LayerKind::Global => effective_value(Some(value), None),
        LayerKind::Project => {
            let global = read_usable(home, &Layer::Global)?;
            effective_value(global.as_ref(), Some(value))
        }
    };
    cross_checks(value, kind, &effective, &mut found);
    reference_checks(value, kind, &effective, &mut found);
    Ok(found
        .into_iter()
        .map(|(key, message)| Finding {
            document: path.to_path_buf(),
            key,
            message,
        })
        .collect())
}

/// `instrukt config validate`: the findings of `layer`. A missing layer,
/// or one of another schema version, is refused rather than judged.
pub fn validate(home: &Home, layer: &Layer) -> Result<Vec<Finding>, ConfigError> {
    let path = layer.path(home);
    let Some(doc) = super::_layer::read(&path)? else {
        return Err(ConfigError::NoLayer {
            path,
            door: layer.init_door(),
        });
    };
    require_usable(layer, &path, &doc.value)?;
    findings_for(home, layer, &path, &doc.value)
}

fn cross_checks(
    value: &Value,
    kind: LayerKind,
    effective: &Value,
    out: &mut Vec<(String, String)>,
) {
    if kind == LayerKind::Project
        && let Some(required) = value.get("requires").and_then(Value::as_sequence)
    {
        for (i, module) in required.iter().enumerate() {
            let Some(name) = module.as_str() else {
                continue;
            };
            let on = effective
                .get("modules")
                .and_then(|m| m.get(name))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if _schema::MODULE_KEYS.contains(&name) && !on {
                out.push((
                    join("requires", &i.to_string()),
                    format!("requires module `{name}`, which is switched off"),
                ));
            }
        }
    }
    if kind == LayerKind::Global
        && let Some(name) = value
            .get("runner")
            .and_then(|r| r.get("runtime"))
            .and_then(Value::as_str)
    {
        let entries = _runtime::entries(effective.get("runtimes"));
        if !entries.iter().any(|entry| entry.name == name) {
            out.push((
                "runner.runtime".to_owned(),
                format!("names no runtime of the runtime mapping: `{name}`"),
            ));
        }
    }
}

fn reference_checks(
    value: &Value,
    kind: LayerKind,
    effective: &Value,
    out: &mut Vec<(String, String)>,
) {
    let entries = _runtime::entries(effective.get("runtimes"));
    let interpolator = Interpolator::new(effective, &entries, Mode::Check);
    let mut strings = Vec::new();
    collect_strings(&_schema::permitted(value, kind), "", &mut strings);
    for (path, text) in strings {
        if let Err(err) = interpolator.expand_text(&text, Some(&path)) {
            let message = match err {
                ConfigError::MissingKey { .. } | ConfigError::MissingRuntimeField { .. } => {
                    format!("unresolved reference: {err}")
                }
                other => other.to_string(),
            };
            out.push((path, message));
        }
    }
}

fn collect_strings(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::String(text) => out.push((path.to_owned(), text.clone())),
        Value::Mapping(map) => {
            for (key, child) in map {
                collect_strings(child, &join(path, &key_text(key)), out);
            }
        }
        Value::Sequence(seq) => {
            for (i, child) in seq.iter().enumerate() {
                collect_strings(child, &join(path, &i.to_string()), out);
            }
        }
        _ => {}
    }
}
