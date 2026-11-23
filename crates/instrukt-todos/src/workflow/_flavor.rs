//! Loading and validating a flavor's `workflow/` directory
//! (`docs/project/spec/domain/workflow.md`, "Flavor-facing contract").
//!
//! A flavor declares only its single unified `orchestrator` role, the runtime
//! capabilities it `requires`, and the `roles` its procedures declare — in
//! `workflow/workflow.yaml`. All activity routing lives in prose procedures,
//! never in a YAML state machine.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use instrukt_core::config::WorkflowCapability;
use instrukt_core::ids::RoleName;
use serde_yaml_ng::Value;

use super::_finding::{Finding, Problem};

/// The single manifest file a workflow flavor declares.
pub const WORKFLOW_FILE: &str = "workflow.yaml";

/// A flavor could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum FlavorError {
    #[error("cannot read the flavor directory {dir}: {source}")]
    Unreadable {
        dir: PathBuf,
        source: std::io::Error,
    },
    #[error("the flavor at {dir} is invalid:\n{}", list(findings))]
    Invalid {
        dir: PathBuf,
        findings: Vec<Finding>,
    },
}

fn list(findings: &[Finding]) -> String {
    findings
        .iter()
        .map(|finding| format!("  {finding}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A loaded, valid workflow flavor declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flavor {
    orchestrator: RoleName,
    requires: Vec<WorkflowCapability>,
    roles: Vec<RoleName>,
}

impl Flavor {
    /// Loads and validates `workflow.yaml` in `dir`, a published domain's
    /// `workflow/` directory.
    pub fn load(dir: &Path) -> Result<Flavor, FlavorError> {
        let entries = std::fs::read_dir(dir).map_err(|source| FlavorError::Unreadable {
            dir: dir.to_owned(),
            source,
        })?;
        let mut findings = Vec::new();
        let mut extra_yaml = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| FlavorError::Unreadable {
                dir: dir.to_owned(),
                source,
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name != WORKFLOW_FILE && name.ends_with(".yaml") && entry.path().is_file() {
                extra_yaml.push(name.to_owned());
            }
        }
        extra_yaml.sort();
        for name in extra_yaml {
            findings.push(Finding {
                file: PathBuf::from(&name),
                location: String::new(),
                problem: Problem::UnexpectedFile(name),
            });
        }

        let parsed = read_yaml(dir, WORKFLOW_FILE, &mut findings)
            .and_then(|doc| parse_workflow(&doc, &mut findings));

        match parsed {
            Some(flavor) if findings.is_empty() => Ok(flavor),
            _ => Err(FlavorError::Invalid {
                dir: dir.to_owned(),
                findings,
            }),
        }
    }

    /// The flavor's single unified todo orchestrator role.
    pub fn orchestrator(&self) -> &RoleName {
        &self.orchestrator
    }

    /// Runtime capabilities this workflow requires.
    pub fn requires(&self) -> &[WorkflowCapability] {
        &self.requires
    }

    /// The roles declared by `workflow.yaml`, in declaration order.
    pub fn roles(&self) -> &[RoleName] {
        &self.roles
    }
}

fn read_yaml(dir: &Path, file: &str, findings: &mut Vec<Finding>) -> Option<Value> {
    let text = match std::fs::read_to_string(dir.join(file)) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            findings.push(Finding {
                file: PathBuf::from(file),
                location: String::new(),
                problem: Problem::MissingFile,
            });
            return None;
        }
        Err(err) => {
            findings.push(Finding {
                file: PathBuf::from(file),
                location: String::new(),
                problem: Problem::Unreadable(err.to_string()),
            });
            return None;
        }
    };
    match serde_yaml_ng::from_str(&text) {
        Ok(value) => Some(value),
        Err(err) => {
            let location = err
                .location()
                .map(|at| format!("{}:{}", at.line(), at.column()))
                .unwrap_or_default();
            findings.push(Finding {
                file: PathBuf::from(file),
                location,
                problem: Problem::Yaml(err.to_string()),
            });
            None
        }
    }
}

fn push(findings: &mut Vec<Finding>, location: &str, problem: Problem) {
    findings.push(Finding {
        file: PathBuf::from(WORKFLOW_FILE),
        location: location.to_owned(),
        problem,
    });
}

fn parse_workflow(doc: &Value, findings: &mut Vec<Finding>) -> Option<Flavor> {
    let Some(map) = doc.as_mapping() else {
        push(findings, "", Problem::Shape("a YAML mapping"));
        return None;
    };

    for key in map.keys() {
        let Some(name) = key.as_str() else {
            push(findings, "", Problem::Shape("string keys"));
            continue;
        };
        if !matches!(name, "orchestrator" | "requires" | "roles") {
            push(findings, name, Problem::UnknownKey(name.to_owned()));
        }
    }

    let orchestrator = match map.get(Value::from("orchestrator")) {
        None => {
            push(
                findings,
                "orchestrator",
                Problem::MissingKey("orchestrator"),
            );
            None
        }
        Some(value) => match value.as_str() {
            Some(text) => match RoleName::parse(text) {
                Ok(role) => Some(role),
                Err(err) => {
                    push(
                        findings,
                        "orchestrator",
                        Problem::InvalidRole(err.to_string()),
                    );
                    None
                }
            },
            None => {
                push(
                    findings,
                    "orchestrator",
                    Problem::Shape("a role name string"),
                );
                None
            }
        },
    };

    let requires = match map.get(Value::from("requires")) {
        None => {
            push(findings, "requires", Problem::MissingKey("requires"));
            None
        }
        Some(value) => match serde_yaml_ng::from_value::<Vec<WorkflowCapability>>(value.clone()) {
            Ok(caps) => Some(caps),
            Err(err) => {
                push(
                    findings,
                    "requires",
                    Problem::InvalidCapability(err.to_string()),
                );
                None
            }
        },
    };

    let roles = match map.get(Value::from("roles")) {
        None => {
            push(findings, "roles", Problem::MissingKey("roles"));
            None
        }
        Some(value) => match value.as_sequence() {
            None => {
                push(
                    findings,
                    "roles",
                    Problem::Shape("a sequence of role names"),
                );
                None
            }
            Some(items) => {
                if items.is_empty() {
                    push(findings, "roles", Problem::NoRoles);
                    None
                } else {
                    let mut parsed = Vec::with_capacity(items.len());
                    let mut seen = BTreeSet::new();
                    for (i, item) in items.iter().enumerate() {
                        let at = format!("roles[{i}]");
                        let Some(text) = item.as_str() else {
                            push(findings, &at, Problem::Shape("a role name string"));
                            continue;
                        };
                        match RoleName::parse(text) {
                            Ok(role) => {
                                if !seen.insert(role.clone()) {
                                    push(findings, &at, Problem::DuplicateRole(role.to_string()));
                                } else {
                                    parsed.push(role);
                                }
                            }
                            Err(err) => push(findings, &at, Problem::InvalidRole(err.to_string())),
                        }
                    }
                    Some(parsed)
                }
            }
        },
    };

    if let (Some(orch), Some(roles)) = (&orchestrator, &roles)
        && !roles.contains(orch)
    {
        push(
            findings,
            "orchestrator",
            Problem::UndeclaredOrchestrator(orch.to_string()),
        );
    }

    match (orchestrator, requires, roles) {
        (Some(orchestrator), Some(requires), Some(roles)) => Some(Flavor {
            orchestrator,
            requires,
            roles,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::_testing::{flavor_dir, sdlc_dir};
    use super::*;

    #[test]
    fn loads_shipped_sdlc_workflow_manifest() {
        let flavor = Flavor::load(&sdlc_dir()).unwrap();
        assert_eq!(flavor.orchestrator().as_str(), "todo-orchestrator");
        assert!(
            flavor
                .roles()
                .iter()
                .any(|r| r.as_str() == "todo-orchestrator")
        );
        assert!(flavor.roles().iter().any(|r| r.as_str() == "builder"));
        assert!(
            flavor
                .roles()
                .iter()
                .any(|r| r.as_str() == "build-reviewer")
        );
        assert!(flavor.roles().iter().any(|r| r.as_str() == "integrator"));
    }

    #[test]
    fn rejects_legacy_state_machine_files_and_missing_manifest() {
        let dir = flavor_dir(&[(
            "prepare.yaml",
            "flow: prepare\norchestrator: prepare-orchestrator\n",
        )]);
        let Err(FlavorError::Invalid { findings, .. }) = Flavor::load(dir.path()) else {
            panic!("expected invalid flavor");
        };
        assert!(
            findings
                .iter()
                .any(|f| matches!(f.problem, Problem::UnexpectedFile(_)))
        );
        assert!(
            findings
                .iter()
                .any(|f| matches!(f.problem, Problem::MissingFile))
        );
    }

    #[test]
    fn rejects_undeclared_orchestrator_and_unknown_keys() {
        let dir = flavor_dir(&[(
            "workflow.yaml",
            "orchestrator: lead\nrequires: []\nroles: [builder]\nstates: [start]\n",
        )]);
        let Err(FlavorError::Invalid { findings, .. }) = Flavor::load(dir.path()) else {
            panic!("expected invalid flavor");
        };
        assert!(
            findings
                .iter()
                .any(|f| matches!(f.problem, Problem::UndeclaredOrchestrator(_)))
        );
        assert!(
            findings
                .iter()
                .any(|f| matches!(f.problem, Problem::UnknownKey(ref k) if k == "states"))
        );
    }
}
