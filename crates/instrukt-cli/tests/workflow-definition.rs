//! A project's workflow as its flavor configures it
//! (`docs/project/spec/feature/workflow/workflow-definition.md`): the next
//! obligation follows the published definition, actions and instructions,
//! and changing them changes the obligation with the same engine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/runtime.rs"]
mod runtime;
#[path = "support/work.rs"]
mod support;
#[path = "support/workflow.rs"]
mod workflow;

use support::redact;
use workflow::Crew;

// spec: project/instrukt/spec/feature/workflow/workflow-definition UC-WD1
#[test]
fn the_next_obligation_follows_the_configured_order_actions_and_instructions() {
    let crew = Crew::new();
    let slug = "tidy-config";
    crew.project
        .create(slug, "change", "Sort the configuration keys.");
    let before = crew.next("prepare", slug);

    // The developer reconfigures the flavor: a change now starts with a
    // PRD, drafted by another command, with other instructions.
    let flavor = &crew.flavor.repo;
    let prepare = flavor
        .read("docs/global/crew/workflow/prepare.yaml")
        .replace(
            "  to: plan_drafting\n  when: {equals: [kind, change]}\n  action: draft-plan",
            "  to: prd_drafting\n  when: {equals: [kind, change]}\n  action: draft-prd",
        );
    let actions = flavor
        .read("docs/global/crew/workflow/actions.yaml")
        .replace(
            "draft-prd:\n  command: instrukt docs get crew/procedure/intake",
            "draft-prd:\n  command: instrukt docs get crew/procedure/prepare-lead --closure",
        );
    flavor
        .write("docs/global/crew/workflow/prepare.yaml", &prepare)
        .write("docs/global/crew/workflow/actions.yaml", &actions)
        .write(
            "docs/global/crew/workflow/templates/draft-prd.md",
            "Write down what {slug} must achieve before planning it.\n",
        );
    crew.flavor
        .republish(&crew.project.env, "start changes with a PRD");
    let after = crew.next("prepare", slug);

    insta::assert_snapshot!(
        "reconfigured-obligation",
        redact(&format!("{before}=====\n{after}"))
    );
}
