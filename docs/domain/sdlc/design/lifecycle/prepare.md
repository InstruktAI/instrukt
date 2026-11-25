---
description: 'Prepare phase design: todo-orchestrator routes composable preparation primitives (intaker, architect, planner) in <slug>/work and submits each derived artifact to an independently dispatched reviewer. PRD is the human approval gate.'
---

# Prepare — Design

## Required reads

- [sdlc/design/lifecycle/overview](overview.md)

## Purpose

Define how preparation produces the aligned artifacts that downstream
implementation consumes.

Prepare exists to turn a problem impulse into build-ready truth. One unified
`todo-orchestrator` routes preparation in prose from the factual ledger,
dispatching focused role workers (`intaker`, optional `architect`, `planner`)
in `<slug>/work` for the steps the todo needs. Each derived artifact is
verified by an independently dispatched reviewer session. The PRD is verified
by the human, who is the sole authority on intent.

## Inputs/Outputs

**Inputs**

- whatever the todo carries: a brain dump (`input`), existing artifacts, or
  any other starting state — all treated as input to the process
- the codebase (ground truth)
- the todo list (signal about project direction and adjacent work)
- applicable documentation and domain context
- whether a human is in the conversation, read from the conversation itself

**Outputs**

- `prd` (intake) — plus the feature spec snippet with Gherkin for Category A work
- architecture: structural domain-spec / design-doc snippet updates plus the `architecture` delivery-reasoning artifact (when not skipped)
- `implementation-plan`

## Invariants

- One unified `todo-orchestrator` drives the lifecycle in prose from the factual
  ledger (`instrukt todo show <slug> --json`), dispatching only the preparation
  primitives that earn their keep in `<slug>/work`. Bounded bugs and small
  localized changes may fast-track directly to build and independent review;
  non-trivial changes and features run the appropriate preparation steps.

- Each preparation primitive has its own focused role and procedure:
  Intake (`intaker` → `sdlc/procedure/lifecycle/prepare/intake`),
  Architecture (`architect` → `sdlc/procedure/lifecycle/prepare/architecture`),
  and Plan (`planner` → `sdlc/procedure/lifecycle/prepare/plan-draft`).

- Reviewers are independently dispatched. Each derived artifact
  (PRD, architecture, plan) is reviewed by a separate reviewer session
  (`prd-reviewer`, `architecture-reviewer`, `plan-reviewer`). `todo-orchestrator`
  reads the findings and verdict from the factual ledger before routing the next step.
- The PRD is the human approval gate (`approvals.prd`), enforced in code before
  implementation or finalization for features and any todo carrying a `prd`.
  Architecture and plan advance under reviewer judgment and prose orchestration.
- The architecture step is optional and runs only when structural boundaries or
  durable state change. Planning is optional for bounded bugs and small localized
  changes on the fast track.
- Each phase grounds in the prior phase's output. Human refinement at any
  layer invalidates downstream approvals and re-enters the affected phase.
- Automation supplies supporting facts for judgment; it does not decide
  scope, trade-offs, or design quality on its own.
- **Gherkin (`Given / When / Then`) belongs in the feature spec snippet, not in `prd`.**
  PRD success criteria are outcome-shaped in the product owner's language —
  observable indicators of success without pre-imposing the test author's
  grammar. For Category A work, intake authors the Gherkin behavioral
  contract directly into the feature spec snippet (derived from those
  outcome-shaped criteria); for Category B/C work there is no Gherkin
  artifact. Carrying Gherkin in the PRD pushes the PO into a downstream
  author's format.

## Primary flows

### Composable preparation primitives

`todo-orchestrator` routes preparation in prose from the factual ledger rather
than a fixed state machine:

| Primitive    | Procedure                                       | Role      | Authority gate                          |
| ------------ | ----------------------------------------------- | --------- | --------------------------------------- |
| Intake       | `sdlc/procedure/lifecycle/prepare/intake`       | `intaker` | `prd_approved` (`approvals.prd` gate)   |
| Architecture | `sdlc/procedure/lifecycle/prepare/architecture` | `architect` | `architecture_review` verdict `approve` |
| Plan         | `sdlc/procedure/lifecycle/prepare/plan-draft`   | `planner` | `plan_review` verdict `approve`         |

After each primitive completes, `todo-orchestrator` reads `instrukt todo show {slug} --json`
and routes the next primitive in prose — dispatch an independent reviewer, pause
for human approval, or advance to build in the same `<slug>/work` worktree.

### Intake

The `intaker` interviews the Product Owner — the human — and fills the PRD template
via elicitation. PRD success
criteria are outcome-shaped, in PO language. For Category A work the `intaker` also
authors the feature spec snippet's Gherkin acceptance scenarios in `<slug>/work`. When the
work warrants it, `architecture_skipped` may be recorded with reason. The `intaker`
writes the PRD and presents it for review and approval. The
human is the sole authority on PRD correctness because only the human owns
intent.

### Derivation primitives (architecture, plan)

For each derivation primitive, `todo-orchestrator` dispatches the owning role in
`<slug>/work`, then dispatches an independent reviewer session against the written
artifact. On `approve`, `todo-orchestrator` advances to the next primitive or build;
on `needs_work`, it routes findings back to the owning role for remediation and
re-review.

### No human in the conversation

With no human to interview, intake has no source for product intent beyond the
`input` artifact. A basis rich enough to write the PRD from proceeds to
authoring; a sparse or ambiguous one stops the prepare flow against `@input`
rather than producing a PRD nothing authorized. `prd_approved` is the human's
gate either way, so the chain pauses there until the human engages. Once the
PRD is approved, all other phases (architecture, plan) proceed under reviewer
authority.

### Refinement

When the human refines an artifact, the change classifies into one of four
scopes:

- **Intent-level** (new thinking, scope changes) → re-enter intake with the
  delta. Downstream approvals invalidated.
- **Architecture-level** (design-snippet change) → re-enter architecture
  phase. Plan approval invalidated.
- **Plan-level** (task adjustment) → re-enter plan phase.

### Drift

When the factual ledger reports that artifact-referenced paths, snippets, or
dependencies changed since an artifact was authored, `todo-orchestrator`
dispatches a lean `sdlc/procedure/lifecycle/prepare/drift-analyze` worker. It
stamps each pending artifact as `cosmetic` or `substantive`. Substantive drift
invalidates the affected review/approval and `todo-orchestrator` re-derives the
affected artifact before continuing.

## Failure modes

- PRD success criteria stay vague because intake elicitation was shallow.
- The BA front-loads Gherkin into the PRD instead of the feature spec
  snippet, pushing the PO into a test author's grammar.
- A procedure drifts into orchestrator-shaped action (dispatching workers
  for sub-phases) instead of doing the phase work itself.
- A PRD the human has never approved is auto-stamped `prd_approved`
  without the human's review.
- Reviewer findings cannot be addressed by the active procedure, and the
  chain grinds rather than surfacing the blocker.
- Drift is detected but the affected phase isn't re-derived, leaving
  downstream artifacts grounded in stale upstream truth.

## See Also

- [sdlc/procedure/lifecycle/overview](../../procedure/lifecycle/overview.md) —
  the unified prose-routed lifecycle playbook
- [sdlc/procedure/lifecycle/prepare/intake](../../procedure/lifecycle/prepare/intake.md)
- [sdlc/procedure/lifecycle/prepare/architecture](../../procedure/lifecycle/prepare/architecture.md)
- [sdlc/procedure/lifecycle/prepare/plan-draft](../../procedure/lifecycle/prepare/plan-draft.md)
