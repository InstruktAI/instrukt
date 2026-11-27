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
independent exact-SHA handoff gate, human-owned approval gates), and runner
dispatch assignment. The handoff gate admits an exact candidate commit to
`integrate` only with a current independent build-review approval and a
current verification disposition that admits handoff, both bound to that
commit. Intelligence owns all activity routing in clear prose
(`sdlc/procedure/lifecycle/overview`).

## Canonical fields

| Key | Contract |
| --- | --- |
| `orchestrator` | Single unified orchestrator role name in `workflow/workflow.yaml` (e.g. `todo-orchestrator`), listed in `roles` and declared by one procedure. |
| `requires` | Typed list of runtime capabilities (`worktree_create`, `worktree_attach`, `shared_delegation`, `link`, `notify`, `agent_selection`). |
| `roles` | Non-empty list of unique role names declared by the domain's non-subscribed procedures. |

## Ubiquitous language

- **Factual ledger:** read-only structured observation of a todo's artifacts, digests, stamps, claims, bindings, workspace/git state, open findings, verification dispositions, drift, prerequisites, and gate status.
- **Hard integrity invariant:** a code-enforced check where restoring state cannot repair the harm (interference lock, independent exact-SHA handoff gate, human-owned `approvals.prd`/`approvals.task` gates).
- **Unified orchestrator:** the single `todo-orchestrator` role routing composable primitives in prose inside one `<slug>/work` worktree (or with no worktree for tasks).
- **Integration candidate:** an exact published commit SHA on `<slug>/work` carrying an independent reviewer's `build_review` `approve` stamp bound to that exact SHA and current `review-findings` report, and a current verification disposition for that SHA that admits handoff, handed to `integrate` via `--finalize-ready`.

- **Review coverage:** whether every commit the active candidate adds to the canonical branch is an independently approved head, reproduced mechanically from approved or canonical content, a reproduced revert of a failing delivery, a reproduced rendering of the repository's agent material, or exempted by its author. The active candidate is the integration branch's head while the canonical branch does not contain it (`project/instrukt/spec/feature/workflow/integration-gate`).

- **Worktree standing:** whether a todo's worktree on a computer is delivered, idle or under way, derived from the todo's record and the repository.

## Ownership boundary

- **Owns:** flavor manifest schema (`workflow/workflow.yaml`), pure factual ledger derivation (`TodoLedger`), repository observations (worktree cleanliness, heads, grounding drift), the exact-SHA handoff check (independent review approval and admitting verification disposition), which session may record which verification disposition and which may follow another for a commit (`project/instrukt/spec/feature/workflow/delivery-cycle`), and runner `WorkflowAssignments` dispatch selection.
- **Does not own:** activity micro-routing (owned by prose procedures), work-record storage/reflection (owned by `work`), or session/process execution (owned by `runner`).

Workflow also derives the active candidate's review coverage on request, from
the repository, the independent approvals todo records bind to exact commits,
work's answer on each other commit's current review evidence and the runner's
red observation of the canonical head. It is a derived fact and
workflow stores it nowhere. Its rule belongs to
`project/instrukt/spec/feature/workflow/integration-gate`; the approval it
earns is work's record and the review it otherwise owes is the runner's
request.

A rendering of the repository's agent material is reproduced by asking
publication's one rendering operation to render the parent commit's tree and
comparing the result with the commit's tree. What that material holds, and
how it is rendered, are publication's
(`project/instrukt/spec/domain/publication`); workflow compares trees and
judges no content. Which repositories' rendering commits coverage recognises
is stated by the integration gate's rule
(`project/instrukt/spec/feature/workflow/integration-gate`, Review coverage of
a candidate).

The current assignment workflow derives for a todo also names the session holding
its execution claim and states whether the todo waits on a person. Work decides
that wait, by the rule it owns for its `Needs you` marker
(`project/instrukt/spec/feature/work/github-record`); workflow answers only
whether an approval gate currently waits for a human, as it does for that
marker, and relays work's decision and the claim's session from the evidence of
the same derivation. The runner consumes both for execution observation
(`project/instrukt/spec/feature/runner/session-continuity`).

Workflow also derives the standing of each todo worktree git lists for a
project on a computer: delivered, idle or under way. It is derived on request
and stored nowhere, and the runner consumes it for worktree reclamation
(`project/instrukt/spec/feature/runner/worktree-reclamation`). A worktree or
branch a live todo records is that todo's, whatever earlier todo carried the
same name. That a todo is delivered, parked or held is work's fact, relayed
from the evidence of the same derivation; parked and held are idle. Workflow
derives the two other idle readings from that record:

- **A candidate waits for integration** while a finalized hand-off stands for
  the todo's current head.
- **Work has gone back to preparation** while the todo has build evidence and
  the latest of its preparation evidence is later in the record's order than
  the latest of that build evidence. Preparation evidence is a version of the
  todo's intent, requirements, diagnosis, architecture or plan, or a
  preparation review verdict. Build evidence is a build start or completion,
  a build review verdict, a verification disposition, processed deferrals or
  a finalized hand-off.

An active todo in none of these states is under way. A kickback is neither
kind of evidence and its reason is never read
(`general/principle/intelligence-boundaries`): a candidate sent back for a
failing check stays under way until preparation evidence is written. A
structural holder has no lifecycle and no standing.

For a delivered todo workflow also answers what it still owes: its worktree,
its local work branch and its work branch on origin, each for as long as git
lists it for the project on that computer. It finds them from what git lists
there, never by reading through delivered work.

## Event subjects and origins

- **Name:** `workflow.finalized` (occurrence).
- **Origin:** appended atomically with `instrukt todo stamp <slug> --finalize-ready` after all three integrity invariants pass, waking the standing `integrator` with project, todo slug, and exact commit SHA.

## Context map position

- **Work → workflow:** work supplies `WorkEvidence` and invokes workflow's gate checks on attempt reservation, completion stamps, and `--finalize-ready`.
- **Runner → workflow:** runner calls `WorkflowAssignments::assignment` on each tick, tracks progress via the factual progress fingerprint, and routes `workflow.finalized` and `runner.ci_completed` to the standing `integrator`.
- **Docs → workflow:** docs validation checks authored `workflow/workflow.yaml` and procedure role declarations through `FlavorSchema` and `FlavorRoles`.
- **Runner → workflow (knowledge proposals):** runner routes `runner.proposal_approved` to a company home's standing `integrator`, naming the exact approved head to compose.
- **Publication → workflow (review coverage):** publication supplies its one operation rendering a tree's agent material, from which coverage reproduces a rendering commit.

- **Workflow → runner (review coverage):** workflow supplies the active candidate's review coverage; the runner supplies the canonical head's CI result as it observed it.
- **Work → workflow (review coverage):** work supplies which review evidence is current for a commit and whether a candidate's derived approval is one GitHub does not show yet; the review status a commit carries on GitHub is no input to coverage.

- **Workflow → runner (worktree standing):** workflow supplies each todo worktree's standing on the computer, and what a delivered todo still owes of its worktree and work branch; the runner reclaims by it and derives none of it.

## Governing designs and decisions

- `general/principle/intelligence-boundaries` — code owns facts and integrity invariants; prose owns judgment and routing.
- `general/principle/trust-and-guardrails` — guardrails exist only where state restoration cannot repair harm (interference, contract violation).
- `general/principle/verification-independence` — a change is verified only against a standard that originated outside the session that produced it.
- `general/policy/human-contract` — human-owned approvals (`approvals.prd`, `approvals.task`) remain enforced where configured.

## Use cases

Feature-level behavioral scenarios live in:

- [project/instrukt/spec/feature/workflow/workflow-definition](../feature/workflow/workflow-definition.md)
- [project/instrukt/spec/feature/workflow/preparation](../feature/workflow/preparation.md)
- [project/instrukt/spec/feature/workflow/delivery-cycle](../feature/workflow/delivery-cycle.md)
- [project/instrukt/spec/feature/workflow/integration-gate](../feature/workflow/integration-gate.md)
- [project/instrukt/spec/feature/workflow/artifact-currency](../feature/workflow/artifact-currency.md)
