//! The engine against the real sdlc flavor (`docs/global/sdlc/workflow/`):
//! obligations the shipped `prepare` and `work` flows derive from facts.

use std::collections::BTreeSet;

use instrukt_core::ids::{RoleName, SnippetId};
use proptest::prelude::*;

use super::_derive::{Derivation, Obligation, Responsible, Subject};
use super::_facts::{BoundSession, FactName, FactType, FactValue, Facts};
use super::_flavor::Flavor;
use super::_render::{ConfigLookup, RenderFailure, RenderFinding};
use super::_testing::{facts, no_config, sdlc_dir, session_config, sha, subject, text};

fn sdlc() -> Flavor {
    Flavor::load(&sdlc_dir()).unwrap_or_else(|err| panic!("{err}"))
}

fn derive(flow: &str, facts: Facts) -> Derivation {
    derive_with(flow, facts, &no_config)
}

fn derive_with(flow: &str, facts: Facts, config: &dyn ConfigLookup) -> Derivation {
    sdlc()
        .evaluate(flow, &subject(facts), config)
        .unwrap_or_else(|err| panic!("{err}"))
}

/// The stopping state and its action.
fn at(derived: &Derivation) -> (&str, Option<&str>) {
    (
        derived.state.as_str(),
        derived.obligation.as_ref().map(|o| o.action.as_str()),
    )
}

fn obligation(derived: Derivation) -> Obligation {
    derived
        .obligation
        .unwrap_or_else(|| panic!("no obligation"))
}

fn effect_names(obligation: &Obligation) -> Vec<&str> {
    obligation.effects.iter().map(|e| e.name.as_str()).collect()
}

fn procedure(id: &str) -> Responsible {
    Responsible::Procedure(SnippetId::parse(id).unwrap())
}

fn orchestrator(role: &str) -> Responsible {
    Responsible::Orchestrator(RoleName::parse(role).unwrap())
}

fn yes() -> FactValue {
    FactValue::Bool(true)
}

/// Facts of a current approving preparation review `r` of `target`.
fn approved(r: &str, target: &str) -> Vec<(String, FactValue)> {
    vec![
        (format!("{r}.verdict"), text("approve")),
        (format!("{r}.target_digest"), text(target)),
        (format!("{r}.report_digest"), text(&format!("{r}-findings"))),
    ]
}

fn with(mut facts: Facts, entries: Vec<(String, FactValue)>) -> Facts {
    for (name, value) in entries {
        facts.insert(&name, value).unwrap();
    }
    facts
}

/// A feature with its PRD approved and architecture skipped.
fn feature_through_approval() -> Facts {
    let base = facts(&[
        ("kind", text("feature")),
        ("prd.digest", text("prd-1")),
        ("prd_review_findings.digest", text("prd_review-findings")),
        ("prd_approval.digest", text("prd-1")),
        ("architecture_skipped.reason", text("no new structure")),
    ]);
    with(base, approved("prd_review", "prd-1"))
}

/// A feature whose plan is reviewed and approved too.
fn prepared_feature() -> Facts {
    let base = feature_through_approval();
    let base = with(
        base,
        vec![
            ("implementation_plan.digest".into(), text("plan-1")),
            (
                "plan_review_findings.digest".into(),
                text("plan_review-findings"),
            ),
        ],
    );
    with(base, approved("plan_review", "plan-1"))
}

// ---- prepare ----

#[test]
fn prepare_routes_each_kind_to_its_first_derivation() {
    for (kind, expected) in [
        ("feature", ("prd_drafting", "draft-prd")),
        ("change", ("plan_drafting", "draft-plan")),
        ("bug", ("diagnose", "dispatch-diagnosis")),
        ("task", ("task_preparation", "prepare-task")),
    ] {
        let derived = derive("prepare", facts(&[("kind", text(kind))]));
        assert_eq!(at(&derived), (expected.0, Some(expected.1)), "{kind}");
    }
}

#[test]
fn prepare_without_a_kind_owes_nothing() {
    let derived = derive("prepare", Facts::new());
    assert_eq!(at(&derived), ("start", None));
}

#[test]
fn a_drafted_prd_owes_an_independent_review_rendered_for_the_todo() {
    let derived = derive(
        "prepare",
        facts(&[("kind", text("feature")), ("prd.digest", text("prd-1"))]),
    );
    assert_eq!(at(&derived), ("prd_review", Some("review-prd")));
    let obligation = obligation(derived);
    assert!(obligation.command.is_ok() && obligation.brief.is_ok());
    assert_eq!(
        obligation.responsible,
        procedure("sdlc/procedure/lifecycle/prepare/review-prd")
    );
}

#[test]
fn a_current_needs_work_verdict_returns_findings_to_the_prd_author() {
    let mut f = facts(&[
        ("kind", text("feature")),
        ("prd.digest", text("prd-1")),
        ("prd_review_findings.digest", text("prd_review-findings")),
    ]);
    f = with(f, approved("prd_review", "prd-1"));
    f.insert("prd_review.verdict", text("needs_work")).unwrap();
    assert_eq!(
        at(&derive("prepare", f)),
        ("prd_revision", Some("revise-prd"))
    );
}

#[test]
fn a_review_of_an_earlier_prd_is_not_current() {
    let mut f = feature_through_approval();
    f.insert("prd.digest", text("prd-2")).unwrap();
    assert_eq!(
        at(&derive("prepare", f)),
        ("prd_review", Some("review-prd"))
    );
}

#[test]
fn an_approved_prd_review_awaits_the_configured_approval() {
    let mut f = feature_through_approval();
    f.insert("prd_approval.digest", text("prd-0")).unwrap();
    assert_eq!(
        at(&derive("prepare", f)),
        ("prd_approval", Some("await-prd-approval"))
    );
}

#[test]
fn approval_with_skipped_architecture_moves_to_planning() {
    assert_eq!(
        at(&derive("prepare", feature_through_approval())),
        ("plan_drafting", Some("draft-plan"))
    );
}

#[test]
fn approval_without_a_skip_reason_owes_architecture() {
    let mut f = feature_through_approval();
    f.insert("architecture_skipped.reason", text("")).unwrap();
    assert_eq!(
        at(&derive("prepare", f)),
        ("architecture_drafting", Some("draft-architecture"))
    );
}

#[test]
fn substantive_prd_drift_owes_rederivation_before_review() {
    let mut f = prepared_feature();
    f.insert("drift.prd", text("substantive")).unwrap();
    assert_eq!(
        at(&derive("prepare", f)),
        ("prd_rederivation", Some("rederive-prd"))
    );
}

#[test]
fn a_seeded_plan_is_redrafted_rather_than_reviewed() {
    let mut f = prepared_feature();
    f.insert("implementation_plan.seeded", yes()).unwrap();
    assert_eq!(
        at(&derive("prepare", f)),
        ("plan_seeded", Some("redraft-plan-seeded"))
    );
}

#[test]
fn pending_drift_owes_analysis_before_release() {
    let mut f = prepared_feature();
    f.insert("drift.architecture", text("pending")).unwrap();
    assert_eq!(
        at(&derive("prepare", f)),
        ("drift_analysis", Some("analyze-drift"))
    );
}

#[test]
fn unreviewed_claim_commits_owe_review_with_the_shas_as_one_argument() {
    let mut f = prepared_feature();
    f.insert(
        "claim_commits.unreviewed",
        FactValue::ShaList(vec![sha('a'), sha('b')]),
    )
    .unwrap();
    let derived = derive("prepare", f);
    assert_eq!(
        at(&derived),
        ("commit_review_owed", Some("review-claim-commits"))
    );
    assert!(obligation(derived).command.is_ok());
}

#[test]
fn a_prepared_feature_is_released() {
    let derived = derive_with("prepare", prepared_feature(), &session_config);
    assert_eq!(
        at(&derived),
        ("prepared_release_pending", Some("release-prepared"))
    );
    let obligation = obligation(derived);
    assert!(obligation.command.is_ok() && obligation.brief.is_ok());
    assert_eq!(obligation.responsible, orchestrator("prepare-orchestrator"));
}

#[test]
fn the_release_brief_needs_the_runtime_session_from_configuration() {
    let derived = derive("prepare", prepared_feature());
    assert_eq!(
        obligation(derived).brief,
        Err(RenderFailure {
            findings: vec![RenderFinding::Unknown("runtime.session_id".into())]
        })
    );
}

#[test]
fn a_bug_moves_from_diagnosis_to_planning_once_settled() {
    let diagnosed = facts(&[("kind", text("bug")), ("diagnosis.digest", text("dx-1"))]);
    assert_eq!(
        at(&derive("prepare", diagnosed.clone())),
        ("diagnosis_examination", Some("examine-diagnosis"))
    );
    let settled = diagnosed
        .with("diagnosis_settled.digest", text("dx-1"))
        .unwrap();
    assert_eq!(
        at(&derive("prepare", settled)),
        ("plan_drafting", Some("draft-plan"))
    );
}

#[test]
fn an_authorized_task_is_released() {
    let task = facts(&[
        ("kind", text("task")),
        ("task_plan.digest", text("t-1")),
        ("task_approved.target_digest", text("t-1")),
        ("task_approved.by", text("peer")),
    ]);
    assert_eq!(
        at(&derive("prepare", task)),
        ("prepared_release_pending", Some("release-prepared"))
    );
}

// ---- work ----

/// A prepared change with delivered dependencies.
fn prepared_change() -> Facts {
    let base = facts(&[
        ("kind", text("change")),
        ("deps.delivered", yes()),
        ("implementation_plan.digest", text("plan-1")),
        ("plan_review_findings.digest", text("plan_review-findings")),
    ]);
    with(base, approved("plan_review", "plan-1"))
}

/// A prepared change whose build has started in its worktree.
fn building_change() -> Facts {
    with(
        prepared_change(),
        vec![
            ("worktree.path".into(), text("/wt/add-login")),
            ("build.start_base_sha".into(), text("base-1")),
            ("build.plan_digest".into(), text("plan-1")),
            ("quality_checklist.digest".into(), text("qc-1")),
        ],
    )
}

#[test]
fn a_delivered_todo_owes_worktree_removal_while_the_worktree_exists() {
    let delivered = facts(&[
        ("delivery.commit", FactValue::Sha(sha('d'))),
        ("worktree.exists", yes()),
        ("worktree.path", text("/wt/add-login")),
    ]);
    let derived = derive("work", delivered);
    assert_eq!(
        at(&derived),
        ("delivered_cleanup", Some("cleanup-delivered"))
    );
    let obligation = obligation(derived);
    assert_eq!(effect_names(&obligation), ["worktree.remove"]);
    assert!(obligation.effects[0].command.is_ok());
    assert_eq!(obligation.responsible, orchestrator("work-orchestrator"));

    let removed = facts(&[("delivery.commit", FactValue::Sha(sha('d')))]);
    assert!(effect_names(&obligation_of("work", removed)).is_empty());
}

fn obligation_of(flow: &str, facts: Facts) -> Obligation {
    obligation(derive(flow, facts))
}

#[test]
fn undelivered_dependencies_hold_work() {
    let mut f = prepared_change();
    f.insert("deps.delivered", FactValue::Bool(false)).unwrap();
    assert_eq!(
        at(&derive("work", f)),
        ("dependencies_open", Some("await-dependencies"))
    );
}

#[test]
fn unprepared_work_waits_for_preparation() {
    let f = facts(&[("kind", text("feature")), ("deps.delivered", yes())]);
    assert_eq!(
        at(&derive("work", f)),
        ("not_prepared", Some("await-prepare"))
    );
}

#[test]
fn substantive_drift_returns_work_to_preparation() {
    let mut f = building_change();
    f.insert("drift.implementation_plan", text("substantive"))
        .unwrap();
    assert_eq!(
        at(&derive("work", f)),
        ("returned_to_prepare", Some("return-to-prepare-drift"))
    );
}

#[test]
fn pending_drift_holds_only_initial_build_admission() {
    let mut before = prepared_change();
    before.insert("drift.prd", text("pending")).unwrap();
    assert_eq!(
        at(&derive("work", before)),
        ("drift_analysis", Some("analyze-drift"))
    );
    let mut after = building_change();
    after.insert("drift.prd", text("pending")).unwrap();
    assert_eq!(
        at(&derive("work", after)),
        ("building", Some("dispatch-build"))
    );
}

#[test]
fn a_prepared_change_without_a_worktree_owes_workspace_setup() {
    assert_eq!(
        at(&derive("work", prepared_change())),
        ("workspace_request", Some("setup-workspace"))
    );
}

#[test]
fn build_admission_owes_refresh_and_upstream_sync_until_the_base_is_recorded() {
    let mut f = prepared_change();
    f.insert("worktree.path", text("/wt/add-login")).unwrap();
    let derived = derive("work", f.clone());
    assert_eq!(
        at(&derived),
        ("build_admission_pending", Some("await-build-start"))
    );
    assert_eq!(
        effect_names(&obligation(derived)),
        ["record.refresh", "git.fetch-main", "git.merge-main"]
    );

    f.insert("build.start_base_sha", text("base-1")).unwrap();
    assert_eq!(effect_names(&obligation_of("work", f)), ["record.refresh"]);
}

#[test]
fn a_started_build_is_dispatched_then_reviewed_when_complete() {
    let derived = derive("work", building_change());
    assert_eq!(at(&derived), ("building", Some("dispatch-build")));
    let dispatch = obligation(derived);
    assert!(dispatch.command.is_ok() && dispatch.brief.is_ok());
    assert_eq!(
        dispatch.responsible,
        procedure("sdlc/procedure/lifecycle/work/build")
    );
    let mut complete = building_change();
    complete.insert("build.complete", yes()).unwrap();
    assert_eq!(
        at(&derive("work", complete)),
        ("review", Some("review-build"))
    );
}

#[test]
fn a_changed_plan_owes_build_reconciliation() {
    let mut f = building_change();
    f.insert("implementation_plan.digest", text("plan-2"))
        .unwrap();
    f.insert("plan_review.target_digest", text("plan-2"))
        .unwrap();
    assert_eq!(
        at(&derive("work", f)),
        ("build_reconcile", Some("reconcile-build"))
    );
}

fn reviewed(verdict: &str, binding: &str) -> Facts {
    with(
        building_change(),
        vec![
            ("build.complete".into(), yes()),
            ("build_review.verdict".into(), text(verdict)),
            ("build_review.binding".into(), text(binding)),
        ],
    )
}

#[test]
fn requested_changes_return_findings_to_the_builder() {
    assert_eq!(
        at(&derive(
            "work",
            reviewed("changes_requested", "exact_target")
        )),
        ("fixing", Some("relay-findings"))
    );
}

#[test]
fn an_undetermined_review_binding_is_reported_not_routed() {
    assert_eq!(
        at(&derive("work", reviewed("approve", "undetermined"))),
        ("review_undetermined", Some("review-evidence-undetermined"))
    );
}

#[test]
fn approved_work_owes_deferrals_then_finalization() {
    let approved = reviewed("approve", "clean_upstream_merges");
    assert_eq!(
        at(&derive("work", approved.clone())),
        ("approved", Some("dispatch-finalize"))
    );
    let deferring = approved.with("deferrals.pending", yes()).unwrap();
    assert_eq!(
        at(&derive("work", deferring)),
        ("deferrals_owed", Some("process-deferrals"))
    );
}

#[test]
fn matching_finalization_evidence_hands_off_to_integration() {
    let head = FactValue::Sha(sha('f'));
    let ready = with(
        reviewed("approve", "exact_target"),
        vec![
            ("finalize.status".into(), text("ready")),
            ("finalize.sha".into(), head.clone()),
            ("worktree.clean".into(), yes()),
            ("worktree.head".into(), head.clone()),
            ("branch.head".into(), head.clone()),
            ("remote.head".into(), head),
        ],
    );
    let derived = derive_with("work", ready.clone(), &session_config);
    assert_eq!(
        at(&derived),
        ("handoff_pending", Some("hand-off-integration"))
    );
    assert!(obligation(derived).brief.is_ok());

    let mismatched = ready.with("remote.head", FactValue::Sha(sha('e'))).unwrap();
    assert_eq!(
        at(&derive("work", mismatched)),
        ("finalize_mismatch", Some("restamp-finalize"))
    );
}

#[test]
fn a_task_routes_to_the_task_runner() {
    assert_eq!(
        at(&derive("work", facts(&[("kind", text("task"))]))),
        ("task_runner_route", Some("await-task-runner"))
    );
}

// ---- responsibility ----

/// The sdlc actions whose commands name no procedure.
const ORCHESTRATOR_STEPS: [&str; 10] = [
    "release-prepared",
    "return-to-prepare-kickback",
    "return-to-prepare-drift",
    "await-dependencies",
    "await-task-runner",
    "await-build-start",
    "setup-workspace",
    "cleanup-delivered",
    "review-evidence-undetermined",
    "hand-off-integration",
];

#[test]
fn every_sdlc_action_belongs_to_its_procedure_or_its_flows_orchestrator() {
    let flavor = sdlc();
    let steps: BTreeSet<&str> = ORCHESTRATOR_STEPS.into_iter().collect();
    let mut seen_steps = BTreeSet::new();
    for (flow, definition) in &flavor.flows {
        for transition in &definition.transitions {
            let (id, action) = &transition.action;
            let responsible = Responsible::of(action, definition);
            if steps.contains(id.as_str()) {
                seen_steps.insert(id.as_str());
                assert_eq!(
                    responsible,
                    Responsible::Orchestrator(definition.orchestrator.clone()),
                    "{flow}: {id}"
                );
            } else {
                let named = flavor
                    .action_procedures()
                    .find(|(named, _)| *named == id)
                    .map(|(_, procedure)| Responsible::Procedure(procedure.clone()));
                assert_eq!(Some(responsible), named, "{flow}: {id}");
            }
        }
    }
    // Every procedure-less action is selectable by some flow.
    assert_eq!(seen_steps, steps);
    assert_eq!(
        flavor.actions.len() - flavor.action_procedures().count(),
        ORCHESTRATOR_STEPS.len()
    );
}

// ---- invariants over arbitrary facts ----

/// A value for `name` chosen by `pick`, from small pools so equal digests
/// and SHAs occur often.
fn value_for(name: FactName, pick: u8) -> FactValue {
    let n = usize::from(pick);
    match name.fact_type() {
        FactType::Bool => FactValue::Bool(n % 2 == 0),
        FactType::Int => FactValue::Int(i64::from(pick % 4)),
        FactType::Text => text(["d1", "d2", ""][n % 3]),
        FactType::Sha => FactValue::Sha(sha(['a', 'b'][n % 2])),
        FactType::Enum(members) => text(members[n % members.len()]),
        FactType::ShaList => FactValue::ShaList(vec![sha('a'); n % 3]),
        FactType::SessionList => FactValue::Sessions(vec![
            BoundSession {
                runtime: "rt".into(),
                session_id: "s".into(),
            };
            n % 2
        ]),
    }
}

fn arbitrary_facts() -> impl Strategy<Value = Vec<(FactName, FactValue)>> {
    let names: Vec<FactName> = FactName::all().collect();
    prop::collection::vec(prop::option::of(any::<u8>()), names.len()).prop_map(move |picks| {
        names
            .iter()
            .zip(picks)
            .filter_map(|(name, pick)| pick.map(|p| (*name, value_for(*name, p))))
            .collect()
    })
}

fn build(entries: impl Iterator<Item = (FactName, FactValue)>) -> Subject {
    let mut facts = Facts::new();
    for (name, value) in entries {
        facts.insert(name.as_str(), value).unwrap();
    }
    subject(facts)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The same definition and facts yield the same result, whatever order
    /// the provider supplied the facts in; the shipped flows never revisit
    /// a state.
    #[test]
    fn sdlc_evaluation_is_deterministic_and_order_independent(entries in arbitrary_facts()) {
        let flavor = sdlc();
        let forward = build(entries.clone().into_iter());
        let backward = build(entries.into_iter().rev());
        for flow in ["prepare", "work"] {
            let first = flavor.evaluate(flow, &forward, &session_config);
            prop_assert!(first.is_ok(), "{flow}: {first:?}");
            prop_assert_eq!(&first, &flavor.evaluate(flow, &forward, &session_config));
            prop_assert_eq!(&first, &flavor.evaluate(flow, &backward, &session_config));
        }
    }
}
