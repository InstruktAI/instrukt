//! Runtime catalogue facts and configured ordered tier chains.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use super::{ConfigError, Configuration, Tier};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    pub model: String,
    pub effort: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, serde_json::Value>,
}

/// Resolved native inputs; configuration preferences never masquerade as selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSelection {
    pub family: String,
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub options: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCatalog {
    pub provider: String,
    pub effort_option: String,
    pub models: BTreeMap<String, BTreeMap<String, Vec<serde_json::Value>>>,
}

/// T3 has no catalogue CLI. These facts come from its published model manifest,
/// provider instance contract and provider option descriptors, checked 2026-11-07.
/// https://github.com/pingdotgg/t3code/tree/main/apps/server/src/provider
pub fn agent_catalog() -> Result<BTreeMap<String, AgentCatalog>, ConfigError> {
    serde_json::from_str(include_str!("t3-catalog.json")).map_err(|error| {
        ConfigError::InvalidValue {
            key: "runtime catalogue".into(),
            message: error.to_string(),
        }
    })
}

pub fn validate_model(family: &str, entry: &ModelEntry) -> Result<AgentSelection, ConfigError> {
    let invalid = |reason: String| ConfigError::InvalidValue {
        key: format!("agents.{family}"),
        message: reason,
    };
    let catalog = agent_catalog()?;
    let agent = catalog
        .get(family)
        .ok_or_else(|| invalid(format!("unknown agent family `{family}`")))?;
    let options = agent
        .models
        .get(&entry.model)
        .ok_or_else(|| invalid(format!("unknown model `{}` for {family}", entry.model)))?;
    let effort_values = options.get(&agent.effort_option).ok_or_else(|| {
        invalid(format!(
            "model `{}` has no supported effort option",
            entry.model
        ))
    })?;
    if !effort_values.contains(&serde_json::Value::String(entry.effort.clone())) {
        return Err(invalid(format!(
            "unsupported {} value `{}` for {}",
            agent.effort_option, entry.effort, entry.model
        )));
    }
    for (key, value) in &entry.options {
        if key == &agent.effort_option
            || !options
                .get(key)
                .is_some_and(|allowed| allowed.contains(value))
        {
            return Err(invalid(format!(
                "unsupported option `{key}` value `{value}` for {}",
                entry.model
            )));
        }
    }
    let mut native_options = entry.options.clone();
    native_options.insert(
        agent.effort_option.clone(),
        serde_json::Value::String(entry.effort.clone()),
    );
    Ok(AgentSelection {
        family: family.into(),
        provider: agent.provider.clone(),
        model: entry.model.clone(),
        effort: entry.effort.clone(),
        options: native_options,
    })
}

impl Configuration {
    pub fn model_chain(
        &self,
        family: &str,
        tier: Tier,
    ) -> Result<Vec<AgentSelection>, ConfigError> {
        let entries = self.get(
            Some(&format!("agents.{family}.{}", tier.as_str())),
            &super::Env::default(),
        )?;
        let entries: Vec<ModelEntry> =
            serde_yaml_ng::from_value(entries).map_err(|e| ConfigError::InvalidValue {
                key: format!("agents.{family}.{}", tier.as_str()),
                message: e.to_string(),
            })?;
        if entries.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: format!("agents.{family}.{}", tier.as_str()),
                message: "chain is empty".into(),
            });
        }
        entries
            .iter()
            .map(|entry| validate_model(family, entry))
            .collect()
    }
}

pub(super) fn check_chains(value: &Value) -> Vec<super::_schema::KeyFinding> {
    let mut findings = Vec::new();
    let Some(agents) = value.get("agents").and_then(Value::as_mapping) else {
        return findings;
    };
    for (family, settings) in agents {
        let Some(family) = family.as_str() else {
            continue;
        };
        if matches!(family, "order" | "tier") {
            continue;
        }
        let Some(settings) = settings.as_mapping() else {
            continue;
        };
        for (tier, chain) in settings {
            let Some(tier) = tier.as_str().and_then(Tier::parse) else {
                continue;
            };
            match serde_yaml_ng::from_value::<Vec<ModelEntry>>(chain.clone()) {
                Ok(entries) if !entries.is_empty() => {
                    for (i, entry) in entries.iter().enumerate() {
                        if let Err(error) = validate_model(family, entry) {
                            findings.push(super::_schema::KeyFinding::new(
                                &format!("agents.{family}.{}.{i}", tier.as_str()),
                                super::FindingKind::InvalidValue,
                                error.to_string(),
                            ));
                        }
                    }
                }
                result => findings.push(super::_schema::KeyFinding::new(
                    &format!("agents.{family}.{}", tier.as_str()),
                    super::FindingKind::InvalidValue,
                    match result {
                        Err(error) => error.to_string(),
                        _ => "expected a nonempty model chain".into(),
                    },
                )),
            }
        }
    }
    findings
}
