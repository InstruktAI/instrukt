//! The workflow schema docs validation checks an authored flavor against
//! (core's `FlavorSchema`), answered by loading `workflow.yaml`.

use std::path::Path;

use instrukt_core::interfaces::{
    FlavorCheck, FlavorDeclarations, FlavorFinding, FlavorSchema, InterfaceError,
};

use super::_flavor::{Flavor, FlavorError};

/// Checks a `workflow/` directory by loading it as a flavor.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkflowSchema;

impl FlavorSchema for WorkflowSchema {
    fn check(&self, workflow_dir: &Path) -> Result<FlavorCheck, InterfaceError> {
        match Flavor::load(workflow_dir) {
            Ok(flavor) => Ok(FlavorCheck::Valid(declarations(&flavor))),
            Err(FlavorError::Invalid { findings, .. }) => Ok(FlavorCheck::Invalid(
                findings
                    .into_iter()
                    .map(|finding| FlavorFinding {
                        file: finding.file,
                        location: finding.location,
                        message: finding.problem.to_string(),
                    })
                    .collect(),
            )),
            Err(err @ FlavorError::Unreadable { .. }) => Err(InterfaceError {
                module: "todos",
                message: err.to_string(),
            }),
        }
    }
}

fn declarations(flavor: &Flavor) -> FlavorDeclarations {
    FlavorDeclarations {
        roles: flavor.roles().to_vec(),
        orchestrators: vec![("workflow".to_owned(), flavor.orchestrator().clone())],
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use instrukt_core::ids::RoleName;

    use super::super::_testing::{flavor_dir, sdlc_dir};
    use super::*;

    fn role(name: &str) -> RoleName {
        RoleName::parse(name).unwrap()
    }

    #[test]
    fn the_sdlc_flavor_declares_its_roles_and_unified_orchestrator() {
        let FlavorCheck::Valid(declared) = WorkflowSchema.check(&sdlc_dir()).unwrap() else {
            panic!("sdlc flavor is invalid");
        };
        let flavor = Flavor::load(&sdlc_dir()).unwrap();
        assert_eq!(declared.roles, flavor.roles());
        assert_eq!(
            declared.orchestrators,
            vec![("workflow".to_owned(), role("todo-orchestrator"))]
        );
    }

    #[test]
    fn an_invalid_flavor_reports_its_findings_by_file_and_location() {
        let dir = flavor_dir(&[(
            "workflow.yaml",
            "orchestrator: lead\nrequires: []\nroles: [builder]\n",
        )]);
        let FlavorCheck::Invalid(findings) = WorkflowSchema.check(dir.path()).unwrap() else {
            panic!("expected findings");
        };
        let places: Vec<(PathBuf, &str)> = findings
            .iter()
            .map(|f| (f.file.clone(), f.location.as_str()))
            .collect();
        assert_eq!(
            places,
            vec![(PathBuf::from("workflow.yaml"), "orchestrator")]
        );
    }

    #[test]
    fn an_unreadable_directory_is_an_interface_error() {
        let dir = flavor_dir(&[]);
        let err = WorkflowSchema
            .check(&dir.path().join("absent"))
            .unwrap_err();
        assert_eq!(err.module, "todos");
    }
}
