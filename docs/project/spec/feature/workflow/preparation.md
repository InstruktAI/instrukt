---
description: Composable preparation primitives (intake, diagnosis, architecture, planning, task preparation) routed in prose by the unified todo-orchestrator with enforced human-owned approval gates.
---
# Preparation — Spec

## Required reads

- [project/instrukt/spec/feature/workflow/artifact-currency](artifact-currency.md)

## What it is

Preparation primitives (intent check, diagnosis, PRD intake & review,
architecture & review, implementation planning & review, task-plan authoring)
are composable steps invoked by the single unified `todo-orchestrator` in prose
when a todo's kind, scope, and blast radius warrant them — inside the todo's
one `<slug>/work` worktree (or with no worktree for `kind: task`).

Bounded bugs and small localized changes may fast-track directly to
implementation and independent review without mandatory planning ceremony.
Features and todos carrying a `prd` artifact enforce configured `approvals.prd`
in code before implementation or finalization; tasks enforce configured
`approvals.task` in code before `task-runner` execution.

## Canonical fields

### Approval gates (Invariant 3)

- `approvals.prd` (default `human`, or `peer`): enforced in code on `builder`,
  `fixer`, and `finalizer` attempt reservations and on `--finalize-ready` for
  `kind: feature` and any todo with a `prd` artifact.
- `approvals.task` (default `human`, or `peer`): enforced in code before the
  runner dispatches `Lane::TaskRun` (`task-runner`).
- While unclaimed and waiting for a human approval (`waiting_for_human_approval`),
  the runner does not re-dispatch `todo-orchestrator`; applying an approval
  decision wakes the live orchestrator when present or makes the todo eligible
  for next pickup.

### Carry and attribution

An earlier approval carries to a revised artifact via `--carry-from <stamp-id>`
only under a standing independent `approve` review on the current artifact and
report digests, retaining original approver attribution.

### Single workspace

Repository todos use one `<slug>/work` worktree and branch from freshly fetched
`origin/main` (or `origin/<slug>/work` when continuing on another computer).
There is no separate `<slug>/prepare` worktree or pre-build preparation
integration handoff. `kind: task` creates no git worktree.

## Use cases

### UC-PR1: Prepare an intended feature for implementation

```gherkin
Given a developer has recorded an intended feature
When the unified todo-orchestrator runs in the todo's single worktree
Then it delegates intake, planning, and independent reviewers as native shared children
And code refuses builder or finalizer admission until configured PRD approval binds the current PRD
And findings return to the responsible author for revision
```

### UC-PR2: Resume preparation after giving an approval decision

```gherkin
Given a developer's todo is waiting at its configured approval gate
When an admissible approval or request-changes decision is applied
Then its live claim-holding orchestrator receives that decision as soon as possible
And inspects the updated factual ledger to continue
But when no live orchestrator exists
Then the decision remains recorded for the next pickup or deliberate start
And no orchestrator is started solely to deliver the decision
```

### UC-PR3: A derivation that changes no repository file proceeds to its review

```gherkin
Given a developer's todo is in progress
When the author of a derivation records it without changing any repository file
And records role completion at the unchanged head
Then the derivation's independent review can start
```

### UC-PR4: Gate feature implementation on configured PRD approval

```gherkin
Given a developer has a feature todo with a reviewed PRD
When no eligible PRD approval is recorded
Then reserving a builder, fixer, or finalizer attempt is refused
When an eligible PRD approval is stamped for the current PRD
Then builder reservation succeeds immediately in the same worktree
```

## See Also

- [general/policy/human-contract](../../../../general/policy/human-contract.md)
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
