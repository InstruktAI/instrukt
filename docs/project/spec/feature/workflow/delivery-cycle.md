---
description: Unified delivery cycle enforced by three hard integrity invariants in a single todo worktree, with prose-routed build, review, fix, deferral, and finalize primitives.
---
# Delivery Cycle — Spec

## Required reads

- [project/instrukt/spec/feature/workflow/artifact-currency](artifact-currency.md)

## What it is

A repository todo runs end-to-end under the unified `todo-orchestrator` in one
`<slug>/work` worktree. Code enforces three hard integrity invariants and
exposes a facts-only ledger; prose procedures own when to diagnose, plan,
build, review, fix, process deferrals, and finalize.

## Canonical fields

### Three hard integrity invariants

1. **No interference (attempt lock):** at most one active `Workspace` or `Child`
   attempt may be open on `<slug>/work` at a time. Attempt reservation
   (`instrukt todo attempt`) requires a clean worktree on the recorded branch;
   completion stamps (`--role-complete`, `--build-complete`, review verdicts,
   `--deferrals-processed`) release the attempt atomically.
2. **Independent exact-SHA review gate:** `--verdict approve --phase build_review`
   requires an independent reviewer session distinct from the orchestrator and
   all writer roles (`builder`, `fixer`, `setup`, `finalizer`, `intaker`,
   `architect`, `planner`). `--finalize-ready` (callable by a retained completed
   `finalizer` or directly by the claim-holding `todo-orchestrator`) requires
   no open workspace attempt, a clean worktree matching local and published
   `origin/<slug>/work` HEAD, and a current independent `build_review` `approve`
   stamp bound to that **exact commit SHA** and current `review-findings`
   digest. `--finalize-ready` appends `workflow.finalized` atomically.
3. **Human-owned approval gates:** configured `approvals.prd` is enforced on
   `builder`, `fixer`, and `finalizer` reservations and on `--finalize-ready`
   whenever the todo is a `feature` or carries a `prd` artifact.

### Fast-track and reversible routing

Code imposes no mandatory `implementation-plan`, `setup`, `--build-started`,
`quality-checklist`, or separate `finalizer` gate on bounded bugs or localized
changes. The orchestrator may dispatch `builder` directly in `<slug>/work`,
follow with independent `build-reviewer`, push the exact reviewed HEAD, stamp
`--finalize-ready`, and unclaim to hand off to `integrate`. If unexpected blast
radius surfaces mid-build, the orchestrator routes to planning or doc updates in
prose without skip-flag proliferation.

### Single workspace & cleanup

Repository todos share `<slug>/work` from freshly fetched `origin/main` (or
`origin/<slug>/work` when resuming on another computer). After `integrate`
promotes the exact verified commit and records `instrukt todo deliver <slug> --commit <sha>`,
the clean worktree and local/origin `<slug>/work` branch are removed while
retaining thread and attempt records.

## Use cases

### UC-DC1: Deliver a change for integration

```gherkin
Given a developer has an active repository todo whose dependencies are delivered
When the todo is implemented, independently reviewed at its exact HEAD, and finalized
Then an integration candidate is handed off only with exact-SHA independent approval and matching published candidate evidence
And findings requiring changes return to implementation before that handoff
```

### UC-DC2: A fix reaches the next review and the delivered todo branch

```gherkin
Given a developer's change has review findings requiring fixes
When the orchestrator dispatches a shared fixer after reviewer release
Then it commits corrections directly on the todo branch in the same worktree
When it records build completion at the clean head
Then completion and attempt release commit together
And the next independent reviewer sees those corrections at the current shared head
And retained attempt evidence identifies the fixer and completed commit
```

### UC-DC3: Role-branch handoff (retired)

Retired: every role commits directly in the todo's single shared worktree.

### UC-DC4: Serial workspace attempts preserve clean handoff between roles

```gherkin
Given a developer's todo-orchestrator is bound to the todo's single worktree
When a shared workspace role completes its work and records completion at the clean todo head
Then that stamp releases the attempt atomically
And the next role (such as builder) starts in the same worktree on the same todo branch only after that release
```

### UC-DC5: Deferral disposition permits finalization without changing the code

```gherkin
Given a developer's approved change has deferred items awaiting disposition
When the shared deferrer records dispositions covering the current artifact and stamps processing
Then that attempt is released without changing the todo branch
And finalization can proceed in that same clean worktree at its current head
But missing or stale processing evidence keeps the deferrer outstanding and blocks readiness
```

### UC-DC6: Integration receives a currently reviewed exact-SHA candidate

```gherkin
Given a developer's candidate is published in the shared todo worktree
When the candidate head changes, independent review examines the new exact HEAD in the same worktree
And readiness waits for that review's independent exact-SHA approval and release
When readiness is recorded for that matching candidate
Then readiness and the finalized event commit together
```

### UC-DC7: Recover a failed workspace attempt without losing committed work

```gherkin
Given a developer's workspace child has committed work on the shared todo branch but cannot finish
When native evidence confirms it has stopped and the shared tree is clean
And the orchestrator records the failed attempt with its reason and workspace evidence
Then its release preserves the committed work and creates no completion stamp
When a successor is admitted
Then it continues in that same worktree from the current todo head
And completed implementation receives independent review before finalization
But dirty or conflicted work keeps the original attempt outstanding for recovery in place
```

### UC-DC8: Finish delivery without leftover phase resources

```gherkin
Given a developer's reviewed work branch has integrated and the todo is delivered
When cleanup removes the worktree and branch
Then the recorded worktree and merged work branch are removed locally and on origin
And retained native thread and role-attempt evidence remains readable
```

### UC-DC9: Fast-track a bounded change while enforcing the three integrity invariants

```gherkin
Given a developer has a bounded change or bug in its single todo worktree
When the unified todo-orchestrator dispatches a builder directly without an implementation plan, setup, or quality checklist
Then concurrent workspace attempts are refused by the interference lock
And finalization is refused until an independent reviewer approves the exact published commit SHA
When the independent reviewer approves that exact commit SHA
Then the claim-holding orchestrator can record finalize readiness directly for integration
```

## Known caveats

- Re-creating a deleted orchestrator thread while its todo branch remains checked
  out requires existing-worktree binding or safe worktree removal first.

## See Also

- [general/principle/trust-and-guardrails](../../../../general/principle/trust-and-guardrails.md)
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
