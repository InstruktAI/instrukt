//! A selection's effort and extra settings at the T3 boundary: translated to
//! the provider's native option ids, carried by a native launch's model
//! selection, and admitted against what T3 reports the selected model takes.
use instrukt_core::config::{AgentSelection, Effort, HarnessKind};
use serde_json::{Value, json};

use crate::sessions::SessionsError;

/// The selection with its effort as the native option of its harness kind,
/// beside its extra settings. Effort remains a closed semantic variant until
/// this native boundary.
pub(super) fn native(selection: &AgentSelection) -> AgentSelection {
    let mut native = selection.clone();
    if let Some(effort) = selection.effort {
        let key = match selection.kind {
            HarnessKind::Codex | HarnessKind::Grok | HarnessKind::Kiro | HarnessKind::Muse => {
                "reasoningEffort"
            }
            HarnessKind::Antigravity => "thought_level",
            HarnessKind::Pi => "thinking",
            HarnessKind::OpenCode => "variant",
            HarnessKind::Claude
            | HarnessKind::Cursor
            | HarnessKind::Fx
            | HarnessKind::AcpRegistry => "effort",
        };
        let value = match effort {
            Effort::None => "none",
            Effort::Minimal => "minimal",
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
            Effort::Ultra => "ultra",
            Effort::Ultrathink => "ultrathink",
            Effort::Off => "off",
            Effort::Auto => "auto",
        };
        native
            .options
            .insert(key.into(), Value::String(value.into()));
    }
    native
}

/// A native launch's `modelSelection` (T3 `ModelSelection`): the instance and
/// model, and the native options as T3's list of option `id` and `value`;
/// a selection without effort or settings sends no options.
pub(super) fn model_selection(selection: &AgentSelection) -> Value {
    let options = native(selection).options;
    let mut value = json!({"instanceId": selection.instance, "model": selection.model});
    if !options.is_empty() {
        value["options"] = options
            .into_iter()
            .map(|(id, value)| json!({"id": id, "value": value}))
            .collect();
    }
    value
}

/// Admits a new launch's native options against the option descriptors the
/// selected model has in `provider`, a `server.getConfig` provider entry. A
/// launch never drops an option it cannot apply: the start is refused before
/// creation, naming each rejected option.
pub(super) fn admit(provider: &Value, selection: &AgentSelection) -> Result<(), SessionsError> {
    let model = selection.model.as_str();
    let descriptors = provider["models"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| {
            entry["slug"] == model
                || entry["aliases"]
                    .as_array()
                    .is_some_and(|aliases| aliases.iter().any(|alias| *alias == model))
        })
        .and_then(|entry| entry.pointer("/capabilities/optionDescriptors"))
        .and_then(Value::as_array);
    let problems = rejected(&native(selection).options, descriptors);
    if problems.is_empty() {
        return Ok(());
    }
    Err(SessionsError::Refused(format!(
        "T3 model `{model}` of provider instance `{}` does not accept the options of profile `{}`: {}; change that profile's effort or options in agents.yaml",
        selection.instance,
        selection.profile,
        problems.join("; ")
    )))
}

/// The options T3 cannot apply, by the rules T3 0.1.8 checks a delegated
/// task's options with (`OrchestratorMcpService.invalidOptionSelections`): a
/// model reported without descriptors is not checked. T3 carries an option
/// value only as text or a boolean.
fn rejected(
    options: &std::collections::BTreeMap<String, Value>,
    descriptors: Option<&Vec<Value>>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (id, value) in options {
        if !value.is_string() && !value.is_boolean() {
            problems.push(format!(
                "option {id} is {value}, but T3 takes only text or a boolean"
            ));
            continue;
        }
        let Some(descriptors) = descriptors else {
            continue;
        };
        let Some(descriptor) = descriptors.iter().find(|d| d["id"] == id.as_str()) else {
            let known: Vec<&str> = descriptors
                .iter()
                .filter_map(|d| d["id"].as_str())
                .collect();
            let known = if known.is_empty() {
                "none".into()
            } else {
                known.join(", ")
            };
            problems.push(format!("unknown option {id}; supported options: {known}"));
            continue;
        };
        let choices: Vec<&str> = descriptor["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|choice| choice["id"].as_str())
            .collect();
        match descriptor["type"].as_str() {
            Some("boolean") if !value.is_boolean() => {
                problems.push(format!("option {id} expects a boolean value"));
            }
            Some("select") if !choices.iter().any(|choice| *value == *choice) => {
                problems.push(format!(
                    "option {id} must be one of: {}",
                    choices.join(", ")
                ));
            }
            _ => {}
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::{admit, model_selection};
    use crate::sessions::SessionsError;
    use instrukt_core::config::{AgentSelection, Effort, HarnessKind};
    use instrukt_core::ids::{ProfileName, ProviderInstanceId};
    use serde_json::json;

    fn opus(effort: Option<Effort>, settings: serde_json::Value) -> AgentSelection {
        AgentSelection {
            profile: ProfileName::parse("claude").unwrap(),
            kind: HarnessKind::Claude,
            instance: ProviderInstanceId::parse("claudeAgent").unwrap(),
            model: "claude-opus-5-5".into(),
            effort,
            options: serde_json::from_value(settings).unwrap(),
        }
    }
    /// A `server.getConfig` provider entry of T3 `v0.1.8` (`ServerProvider`)
    /// whose Opus model reports Claude's effort and fast-mode descriptors.
    fn provider() -> serde_json::Value {
        let efforts: Vec<_> = ["low", "medium", "high", "xhigh", "max", "ultrathink"]
            .map(|id| json!({"id":id,"label":id}))
            .into();
        json!({"instanceId":"claudeAgent","driver":"claudeAgent","models":[
            {"slug":"claude-opus-5-5","name":"Claude Opus 5.5","isCustom":false,"capabilities":{"optionDescriptors":[
                {"id":"effort","label":"Reasoning","type":"select","options":efforts},
                {"id":"fastMode","label":"Fast mode","type":"boolean"}]}},
            {"slug":"claude-custom","name":"Custom","isCustom":true,"capabilities":null}]})
    }

    // Kills: a launch sending only instance and model, or options in another shape.
    #[test]
    fn a_launch_carries_the_translated_effort_and_settings() {
        assert_eq!(
            model_selection(&opus(Some(Effort::High), json!({"fastMode": true}))),
            json!({"instanceId":"claudeAgent","model":"claude-opus-5-5","options":[
                {"id":"effort","value":"high"},{"id":"fastMode","value":true}]})
        );
        assert_eq!(
            model_selection(&opus(None, json!({}))),
            json!({"instanceId":"claudeAgent","model":"claude-opus-5-5"})
        );
    }

    // Kills: admitting an effort or setting the selected model does not take.
    #[test]
    fn a_launch_is_refused_for_options_the_model_does_not_take() {
        let refused = admit(
            &provider(),
            &opus(
                Some(Effort::Ultra),
                json!({"fastMode": "yes", "contextWindow": "1m"}),
            ),
        );
        let Err(SessionsError::Refused(reason)) = refused else {
            panic!("expected a refusal, got {refused:?}");
        };
        assert_eq!(
            reason,
            "T3 model `claude-opus-5-5` of provider instance `claudeAgent` does not accept the options of profile `claude`: unknown option contextWindow; supported options: effort, fastMode; option effort must be one of: low, medium, high, xhigh, max, ultrathink; option fastMode expects a boolean value; change that profile's effort or options in agents.yaml"
        );
    }

    // Kills: refusing options the model takes, or a model T3 reports no descriptors for.
    #[test]
    fn a_launch_is_admitted_with_options_the_model_takes_or_cannot_check() {
        assert!(
            admit(
                &provider(),
                &opus(Some(Effort::High), json!({"fastMode": true}))
            )
            .is_ok()
        );
        let mut custom = opus(Some(Effort::Ultra), json!({}));
        custom.model = "claude-custom".into();
        assert!(admit(&provider(), &custom).is_ok());
    }
}
