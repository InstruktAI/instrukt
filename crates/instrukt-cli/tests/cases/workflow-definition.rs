//! A project's workflow as its flavor configures it
//! (`docs/project/spec/feature/workflow/workflow-definition.md`): the flavor's
//! `workflow.yaml` declares the unified orchestrator, capabilities, and roles,
//! while code serves a facts-only ledger (`instrukt todo work` / `todo show`)
//! without any YAML state machine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::fixtures::work as support;
use crate::fixtures::workflow;
use support::redact;
use workflow::Crew;

// spec: project/instrukt/spec/feature/workflow/workflow-definition UC-WD1
#[test]
fn the_next_obligation_follows_the_configured_order_actions_and_instructions() {
    let crew = Crew::new();
    let slug = "tidy-config";
    crew.project
        .create(slug, "change", "Sort the configuration keys.");
    let before = crew.next("work", slug);

    // Reconfiguring the flavor's prose orchestrator role updates dispatch and
    // deliberate placement with zero engine changes.
    let flavor = &crew.flavor.repo;
    let workflow_yaml = flavor
        .read("docs/domain/crew/workflow/workflow.yaml")
        .replace("orchestrator: todo-lead", "orchestrator: delivery-lead")
        .replace("- todo-lead", "- delivery-lead");
    flavor
        .write("docs/domain/crew/workflow/workflow.yaml", &workflow_yaml)
        .write(
            "docs/domain/crew/procedure/todo-lead.md",
            "---\ndescription: The delivery-lead procedure.\nrole: delivery-lead\nactivation_trigger: When delivery is led.\nparameters:\n  - name: slug\n    required: true\n---\n\n# delivery-lead — Procedure\n\n## Goal\n\nRoute in prose.\n\n## Preconditions\n\nNone.\n\n## Steps\n\n1. Inspect ledger.\n\n## Outputs\n\nDone.\n\n## Recovery\n\nRetry.\n",
        );
    crew.flavor
        .republish(&crew.project.env, "rename unified orchestrator role");

    let shown = crew.todo(["show", slug, "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(parsed["ledger"]["slug"], slug);
    assert_eq!(
        parsed["ledger"]["gates"]["prd_approval"]["status"],
        "not_required"
    );

    insta::assert_snapshot!("reconfigured-obligation", redact(&before));
}
