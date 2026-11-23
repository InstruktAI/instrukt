---
description: Workflow bounded context — facts-only ledger, three hard integrity invariants, flavor manifest, and integration gate.
---

# Workflow — Spec

## What it is

Workflow separates **code-owned facts and integrity invariants** from
**intelligence-owned activity routing** (`general/principle/intelligence-boundaries`,
`general/principle/trust-and-guardrails`).

Code owns a facts-only ledger (`instrukt todo show <slug> [--json]` and
`instrukt todo work <slug>`), three hard integrity invariants (no interference,
independent exact-SHA review gate at `integrate`, human-owned approval gates),
and runner dispatch assignment. Intelligence owns all activity routing in clear
prose (`sdlc/procedure/lifecycle/overview`).

## Canonical fields

| Key | Contract |
| --- | --- |
| `orchestrator` | Single unified orchestrator role name in `workflow/workflow.yaml` (e.g. `todo-orchestrator`), listed in `roles` and declared by one procedure. |
| `requires` | Typed list of runtime capabilities (`worktree_create`, `worktree_attach`, `shared_delegation`, `link`, `notify`, `agent_selection`). |
| `roles` | Non-empty list of unique role names declared by the domain's non-subscribed procedures. |

## Ubiquitous language

- **Factual ledger:** read-only structured observation of a todo's artifacts, digests, stamps, claims, bindings, workspace/git state, open findings, drift, prerequisites, and gate status.
- **Hard integrity invariant:** a code-enforced check where restoring state cannot repair the harm (interference lock, independent exact-SHA review gate at `integrate`, human-owned `approvals.prd`/`approvals.task` gates).
- **Unified orchestrator:** the single `todo-orchestrator` role routing composable primitives in prose inside one `<slug>/work` worktree (or with no worktree for tasks).
- **Integration candidate:** an exact published commit SHA on `<slug>/work` carrying an independent reviewer's `build_review` `approve` stamp bound to that exact SHA and current `review-findings` report, handed to `integrate` via `--finalize-ready`.

<!-- planned:event-driven-pr-review-gate -->
- **Review coverage:** whether every commit the active candidate adds to the canonical branch is an independently approved head, reproduced mechanically from approved or canonical content, a reproduced revert of a failing delivery, or exempted by its author. The active candidate is the integration branch's head while the canonical branch does not contain it (`project/instrukt/spec/feature/workflow/integration-gate`).
<!-- /planned:event-driven-pr-review-gate -->

## Ownership boundary

- **Owns:** flavor manifest schema (`workflow/workflow.yaml`), pure factual ledger derivation (`TodoLedger`), repository observations (worktree cleanliness, heads, grounding drift), exact-SHA independent review readiness check, and runner `WorkflowAssignments` dispatch selection.
- **Does not own:** activity micro-routing (owned by prose procedures), work-record storage/reflection (owned by `work`), or session/process execution (owned by `runner`).

<!-- planned:event-driven-pr-review-gate -->
Workflow also derives the active candidate's review coverage on request, from
the repository, the independent approvals todo records bind to exact commits,
work's answer on each other commit's current review evidence and the runner's
red observation of the canonical head. It is a derived fact and
workflow stores it nowhere. Its rule belongs to
`project/instrukt/spec/feature/workflow/integration-gate`; the approval it
earns is work's record and the review it otherwise owes is the runner's
request.
<!-- /planned:event-driven-pr-review-gate -->

The current assignment workflow derives for a todo also names the session holding
its execution claim and states whether the todo waits on a person. Work decides
that wait, by the rule it owns for its `Needs you` marker
(`project/instrukt/spec/feature/work/github-record`); workflow answers only
whether an approval gate currently waits for a human, as it does for that
marker, and relays work's decision and the claim's session from the evidence of
the same derivation. The runner consumes both for execution observation
(`project/instrukt/spec/feature/runner/session-continuity`).

## Event subjects and origins

- **Name:** `workflow.finalized` (occurrence).
- **Origin:** appended atomically with `instrukt todo stamp <slug> --finalize-ready` after all three integrity invariants pass, waking the standing `integrator` with project, todo slug, and exact commit SHA.

## Context map position

- **Work → workflow:** work supplies `WorkEvidence` and invokes workflow's gate checks on attempt reservation, completion stamps, and `--finalize-ready`.
- **Runner → workflow:** runner calls `WorkflowAssignments::assignment` on each tick, tracks progress via the factual progress fingerprint, and routes `workflow.finalized` and `runner.ci_completed` to the standing `integrator`.
- **Docs → workflow:** docs validation checks authored `workflow/workflow.yaml` and procedure role declarations through `FlavorSchema` and `FlavorRoles`.

<!-- planned:event-driven-pr-review-gate -->
- **Workflow → runner (review coverage):** workflow supplies the active candidate's review coverage; the runner supplies the canonical head's CI result as it observed it.
- **Work → workflow (review coverage):** work supplies which review evidence is current for a commit; the review status a commit carries on GitHub is no input to coverage.
<!-- /planned:event-driven-pr-review-gate -->

## Governing designs and decisions

- `general/principle/intelligence-boundaries` — code owns facts and integrity invariants; prose owns judgment and routing.
- `general/principle/trust-and-guardrails` — guardrails exist only where state restoration cannot repair harm (interference, contract violation, human-owned gates).
- `general/policy/human-contract` — human-owned approvals (`approvals.prd`, `approvals.task`) remain enforced where configured.

## Use cases

Feature-level behavioral scenarios live in:

- [project/instrukt/spec/feature/workflow/workflow-definition](../feature/workflow/workflow-definition.md)
- [project/instrukt/spec/feature/workflow/preparation](../feature/workflow/preparation.md)
- [project/instrukt/spec/feature/workflow/delivery-cycle](../feature/workflow/delivery-cycle.md)
- [project/instrukt/spec/feature/workflow/integration-gate](../feature/workflow/integration-gate.md)
- [project/instrukt/spec/feature/workflow/artifact-currency](../feature/workflow/artifact-currency.md)
