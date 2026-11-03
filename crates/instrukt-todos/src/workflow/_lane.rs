//! Which lane a todo is owed, from its `work` and `prepare` derivations
//! (`work-dispatch`, "Todo pickup"; `preparation`, "Kind routing").
//!
//! The work flow decides: a todo is in preparation when the work flow's
//! obligation belongs to the prepare flow's orchestrator, or when the work
//! flow can only exit through a step of its own orchestrator while the
//! prepare flow still owes a procedure's work. Otherwise its preparation is
//! complete and the work flow is owed, which for a task is its run.

use instrukt_core::ids::{RoleName, SnippetId};
use instrukt_runner::interfaces::{Lane, TodoKind};

use super::_derive::{Derivation, Responsible};

/// The lane for a todo of `kind` whose flows derived `work` and `prepare`.
/// `prepare_orchestrator` is the prepare flow's orchestrator role;
/// `role_of` answers a procedure's declared `role`.
pub(crate) fn select_lane(
    kind: TodoKind,
    work: &Derivation,
    prepare: &Derivation,
    prepare_orchestrator: &RoleName,
    role_of: &dyn Fn(&SnippetId) -> Option<RoleName>,
) -> Lane {
    let preparation_owes_procedure = matches!(
        prepare.obligation.as_ref().map(|o| &o.responsible),
        Some(Responsible::Procedure(_))
    );
    let routes_to_preparation = match work.obligation.as_ref().map(|o| &o.responsible) {
        Some(Responsible::Procedure(procedure)) => {
            role_of(procedure).as_ref() == Some(prepare_orchestrator)
        }
        Some(Responsible::Orchestrator(_)) | None => preparation_owes_procedure,
    };
    if routes_to_preparation {
        Lane::Prepare
    } else if kind == TodoKind::Task {
        Lane::TaskRun
    } else {
        Lane::Work
    }
}

#[cfg(test)]
mod tests {
    use instrukt_core::config::Approver;

    use super::super::_derive::Subject;
    use super::super::_flavor::Flavor;
    use super::super::_provider::{
        Approvers, Observations, PrerequisiteStanding, ProviderInput, derive_facts,
    };
    use super::super::_testing::{EvidenceBuilder, digest, no_config, sdlc_dir, todo};
    use super::*;
    use crate::evidence::{
        ApprovalGate, ApprovedBy, PreparationReview, ReviewVerdict, StampFact, TodoEvidence,
        artifact_names,
    };

    const PREPARE_OVERVIEW: &str = "sdlc/procedure/lifecycle/prepare/overview";

    /// The docs index's role for the prepare overview, the only procedure
    /// these tests need a role for.
    fn indexed(procedure: &SnippetId) -> Option<RoleName> {
        (procedure.as_str() == PREPARE_OVERVIEW)
            .then(|| RoleName::parse("prepare-orchestrator").unwrap())
    }

    fn unindexed(_: &SnippetId) -> Option<RoleName> {
        None
    }

    /// The lane the sdlc flavor derives for `evidence` with delivered
    /// prerequisites and nothing observed in the repository.
    fn lane_of(evidence: &TodoEvidence, role_of: &dyn Fn(&SnippetId) -> Option<RoleName>) -> Lane {
        let flavor = Flavor::load(&sdlc_dir()).unwrap();
        let facts = derive_facts(&ProviderInput {
            evidence,
            prerequisites: &[PrerequisiteStanding::Delivered],
            approvers: Approvers {
                prd: Approver::Human,
                task: Approver::Human,
            },
            observed: &Observations::default(),
        })
        .unwrap();
        let subject = Subject {
            project: evidence.project.clone(),
            slug: evidence.slug.clone(),
            facts,
        };
        let work = flavor.evaluate("work", &subject, &no_config).unwrap();
        let prepare = flavor.evaluate("prepare", &subject, &no_config).unwrap();
        let role = flavor.orchestrator("prepare").unwrap();
        select_lane(evidence.kind, &work, &prepare, role, role_of)
    }

    fn plan_review(verdict: ReviewVerdict) -> StampFact {
        StampFact::PreparationVerdict {
            phase: PreparationReview::PlanReview,
            verdict,
            target: digest("plan-1"),
            report: digest("plan-findings-1"),
        }
    }

    /// A change whose plan review approved the current plan.
    fn prepared_change() -> EvidenceBuilder {
        todo(TodoKind::Change)
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-1")
            .artifact(artifact_names::PLAN_REVIEW_FINDINGS, "plan-findings-1")
            .stamp(plan_review(ReviewVerdict::Approve))
    }

    #[test]
    fn an_unprepared_change_is_owed_preparation() {
        let evidence = todo(TodoKind::Change)
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-1")
            .build();
        assert_eq!(lane_of(&evidence, &indexed), Lane::Prepare);
    }

    #[test]
    fn a_work_obligation_of_the_prepare_orchestrator_routes_to_preparation() {
        // The plan was reviewed and needs work: the work flow owes the
        // prepare overview (await-prepare), whose role is the prepare
        // orchestrator; the prepare flow owes the planner's revision.
        let evidence = todo(TodoKind::Change)
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-1")
            .artifact(artifact_names::PLAN_REVIEW_FINDINGS, "plan-findings-1")
            .stamp(plan_review(ReviewVerdict::NeedsWork))
            .build();
        assert_eq!(lane_of(&evidence, &indexed), Lane::Prepare);
        // Without the index the overview's role is unknown, and the work
        // flow keeps the todo.
        assert_eq!(lane_of(&evidence, &unindexed), Lane::Work);
    }

    #[test]
    fn a_prepared_change_is_owed_its_work() {
        assert_eq!(lane_of(&prepared_change().build(), &indexed), Lane::Work);
    }

    #[test]
    fn a_kickback_returns_prepared_work_to_preparation() {
        let evidence = prepared_change()
            .stamp(StampFact::Kickback {
                reason: "the plan misses the API".into(),
                input: digest("input-2"),
                invalidated_plan: Some(digest("plan-1")),
                invalidated_reviews: crate::evidence::plan_review_only(),
            })
            .build();
        assert_eq!(lane_of(&evidence, &indexed), Lane::Prepare);
    }

    #[test]
    fn an_unauthorized_task_is_owed_preparation() {
        let evidence = todo(TodoKind::Task)
            .artifact(artifact_names::TASK_PLAN, "task-1")
            .build();
        assert_eq!(lane_of(&evidence, &indexed), Lane::Prepare);
    }

    #[test]
    fn an_authorized_task_is_owed_its_run() {
        let evidence = todo(TodoKind::Task)
            .artifact(artifact_names::TASK_PLAN, "task-1")
            .stamp(StampFact::Approval {
                gate: ApprovalGate::Task,
                by: ApprovedBy::Human,
                target: digest("task-1"),
                carried_from: None,
            })
            .build();
        assert_eq!(lane_of(&evidence, &indexed), Lane::TaskRun);
    }
}
