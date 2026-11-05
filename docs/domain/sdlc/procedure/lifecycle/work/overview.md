---
eval: true
description: 'Work stage overview. Build, review, fix, finalize cycle driven by derived workflow obligations.'
activation_trigger: 'Use when asked to become the work orchestrator for a todo or drive instrukt todo work through build, review, fix, and finalize handoff.'
visibility: 'public'
role: work-orchestrator
parameters:
  - name: slug
    required: true
dispatch:
  agent: [claude, codex, gemini, deepseek]
  thinking_mode: slow
---

# Work Overview — Procedure

## Required reads

- @~/.instrukt/docs/general/principle/session-lifecycle.md
- @~/.instrukt/docs/general/concept/agent-characteristics.md
- @~/.instrukt/docs/general/procedure/orchestration.md
- @~/.instrukt/docs/sdlc/procedure/lifecycle/overview.md
- @~/.instrukt/docs/sdlc/policy/preparation-artifact-quality.md

## Goal

Drive `instrukt todo work {slug}` through the build-review-fix-finalize cycle
until the todo is handed to integration or returned to preparation. The
orchestrator executes the selected action and owed effects and supervises
the workers' progress.

### The doc layer is frozen — prepare authors, work executes

Prepare is the only lifecycle phase that authors doc snippets, on `main`. From PREPARED onward the doc layer is frozen for the

slug: no work-phase session — builder, reviewer, fixer, or finalizer — changes
the doc layer or takes its state as work. Writing under `docs/**`, folding
pending markers, and confirming or verifying whether they are folded are the same
violation whatever verb the task uses. Reading doc snippets as input is untouched
and required: the work phase binds in-scope feature-spec scenarios and implements
against published contracts. The slug's doc reflection already exists as pending
markers authored at prepare; the integrator folds them at delivery.
A doc need discovered mid-build (a missing scenario, a wrong planned contract)
is a plan defect: surface it and route through the kickback path (Step 9) so
prepare re-derives on `main` — never edit `docs/**` in the worktree.

### Work ends in delivery or kickback

Prepare settles intent, scope, and every decision the human owns
(`general/policy/human-contract`) before the todo reaches PREPARED; work
executes that settlement and holds no human lane. Every work outcome is
delivery or a kickback to Prepare (Step 9). A finding, blocker, or question
whose answer would change intent or scope is a plan defect and returns the
todo to Prepare, which decides whether its re-derivation involves the human.
Technical recovery, builder–reviewer disagreement, and tooling obstruction are
resolved by the agents. A worker's or reviewer's statement that a human
decision is needed is a claim the orchestrator assesses under this rule, never
a routing instruction.


## Preconditions

- The slug has current preparation evidence and delivered prerequisites.
- Its checked `input` and approved `implementation-plan` are readable.
- A feature also has an approved `prd` and approved or skipped `architecture`.
- A bug also has a settled `diagnosis`.
- The execution claim, holds and workspace permit work admission.

## Steps

On entry, inspect artifact provenance for pending or substantive drift and follow
`analyze-drift` or `return-to-prepare-drift` before dispatch. A kickback invalidates
the affected review bindings for the return-to-prepare route; release and cleanup
preserve branch, worktree and commits. A fix changes completion evidence: after
changes requested, new todo changes invalidate prior `build.complete` until the
builder records its new completion stamp. This keeps review behind finished fixes.


### 1. Enter the workflow

Call `instrukt todo work {slug}`. Read the derived state, selected transition,
action, rendered template and owed effects. Execute the configured commands;
the evaluator neither persists a phase nor performs worktree or session work.
Re-derive from current evidence after material outcomes. There is no receipt
or instruction-text token to parse.

The orchestrator is scoped to this slug. A holder resolves only to its direct
children through recorded group membership. Read a child's own checked intent,
plan and relevant specs before executing its action; holder artifacts are not
executable work input.

Worktree preparation, first branch publication, build-start evidence, stale
checks and admission synchronization are explicit orchestrator
commands. Finalization synchronizes again before handoff; after the readiness
stamp, work derives confirmation and cleanup without merging or republishing. Record their outcomes. On a merge conflict, abort that merge and
route recovery to the recorded builder; a missing builder is a dispatch gap.
A failed fetch or skipped merge is not successful synchronization.

Select workers using the procedure's `dispatch` or global `agents.order`,
availability first. Use `med` for builders and fixers, `slow` for build
reviewers and `fast` for finalizers. Preserve a recorded session per todo and
role; observe it with `instrukt sessions alive`, continue through `send`, and
start through `run` only when a new session is required by the runner contract.

### 2. Build

Create or reattach the runtime worktree with its setup-only brief, synchronize
upstream, make the opening commit and run configured project preparation. Record
the base commit and approved plan digest with `instrukt todo stamp {slug} --build-started`,
then write the quality checklist. Only then dispatch
`sdlc/procedure/lifecycle/work/build` with the approved plan. The builder
executes tasks, commits per task and stamps
`instrukt todo stamp {slug} --build-complete`. That stamp clears the prior
build verdict and finalize readiness. Re-enter `instrukt todo work {slug}`;
commits or a completion reply alone do not advance the work.

When a revised plan meets an existing build, direct the builder to reconcile
against the approved plan delta: retain what serves it, undo what it invalidates
and re-record `--build-started` against the current plan while preserving the
original base before continuing. No stored lifecycle phase supplies this decision.

### 3. Review

Dispatch `review-build`, counting rounds from verdict stamps and recording the exact
commit under review. Reuse the recorded reviewer while its session remains
available; a successful upstream merge does not itself admit another review.
The reviewer executes the shared lanes, writes `review-findings` and stamps
its verdict bound to the report and target. Read both and verify substantive
review under `general/procedure/orchestration` before deriving the next action.

### 4. Fix (if needed)

A current changes-requested verdict sends the findings to the recorded builder
with `instrukt sessions send`. The builder applies
`sdlc/procedure/lifecycle/fix-review-findings`, records dispositions, commits
fixes and stamps completion. Reuse the reviewer for the next admitted round.

Recoverable process failures belong to the worker: upstream conflicts, failed
project checks, dirty worktrees, missing stamps and stale branch publication.
The orchestrator classifies and routes recovery, without editing code or tests.
A technical or architectural blocker does not itself require the human.
Candidate suite execution belongs to integration.

Review counts are informational. At the configured threshold, continuation
follows `sdlc/procedure/lifecycle/review`; the count refuses nothing and does
not transfer ownership or weaken an unresolved finding.

### 5. Finalize

Current approval permits finalization only after deferrals are settled.
Dispatch `finalize` with the approval evidence. The finalizer merges latest
upstream main, resolves conflicts, publishes the branch and stamps readiness.
Re-enter `instrukt todo work {slug}`. Compare the clean worktree head, local
branch head, remote branch head and stamped commit. A mismatch requires
reconciliation and a new stamp before handoff.

The `--finalize-ready` stamp records the handoff and appends `workflow.finalized`
idempotently by project/todo/SHA, making the exact finalized commit available
to the standing integrator. Run
`instrukt todo update <slug> --unclaim` and confirm lane release
and execute the configured terminal cleanup obligations. The work orchestrator
ends this assignment here; integration remains the standing integrator's work.
A blocked finalizer returns to recovery, not a wider integration mandate.

### 6. Deferrals

If `deferrals` exists and is pending, execute the deferrals action before
finalizer dispatch. Decide each entry under
`sdlc/procedure/lifecycle/work/deferrals`, record dispositions and stamp processing.

### 7. Bug fix

Bug work uses the same builder. Its mandate is the checked `input`, settled
causal `diagnosis` and reviewed plan. Diagnosis prescribes no fix or verification
approach; the plan derives them. The builder implements that plan, binds the
prepared scenarios where permitted and stamps build completion as above.

### 8. A stamp the worker owed and did not make

The orchestrator never stamps an outcome on a worker's behalf.
Read the record before acting. A step requires its artifact and stamp, bound
to current evidence; neither elapsed time nor a successful reply establishes it.
The runner sends once to the same session when a run ended without required
evidence or the session is gone; continued absence is an anomaly. Do not duplicate
work or overwrite a worker mid-write. Recovery follows the owning contract,
with the established outcome and acting session retained in the record.

### 9. Kickback to prepare (orchestrator-owned)

A build can stall on a plan defect that no fix resolves — the implementation plan
cannot be executed as written and needs re-derivation, not a fix round. The builder
surfaces the reason to its orchestrator and preserves its work.

The orchestrator owns the technical kickback decision under
`general/policy/human-contract`; the worker does not invoke it. Read the
builder's evidence, establish why re-preparation can resolve the plan defect
within authorized intent, and record that direction before acting. A build
that differs from the plan only in file placement, helper names, or
decomposition shows no plan defect and is no basis for kickback
(`sdlc/policy/preparation-artifact-quality`); a review that asks for kickback on
that ground alone is answered through the review loop, its finding and verdict
intact (`sdlc/procedure/lifecycle/review`). A needed
change to product intent or scope is itself a kickback ground: Prepare
re-derives and alone decides whether that involves the human. A change to the
governing process contract follows `sdlc/policy/process-change-authority`. An
unresolved technical question returns to evidence and its agent owner, never to a human merely because a
fix round failed. Kickback supplies no approval of the revised plan; normal
independent preparation review remains required.

Run `instrukt todo stamp {slug} --kickback --reason '<why the plan cannot be built>'`
through the configured kickback command. The record carries the reason in
`input`, invalidates the implementation plan and resets finalization. Execute
`instrukt todo update <slug> --unclaim` and the cleanup effects and confirm release. Preserve the worktree,
branches and build commits. The stamp alone performs no session side effects.

The runner owns the next preparation dispatch. Its planner reads the recorded
kickback; independent review and applicable approval remain owed. A new input
version requires its intent check. When the todo is prepared again, the runner
owns work dispatch under the project's pickup policy; reconcile the existing
build with the re-approved plan under Step 2.

Confirm `claim.held` is false after `instrukt todo update <slug> --unclaim`,
then end each assignment child from `sessions.bound` with
`instrukt sessions end <id>` before ending the completed lane.

## Outputs

- Code implementing the plan, committed and independently reviewed.
- Findings resolved and deferrals decided.
- A matching finalized branch and recorded integration handoff, or a grounded
  kickback with retained working resources and confirmed lane release.

## Recovery

- Inspect and route worker obstruction under `general/procedure/orchestration`.
- A failed command leaves its effect owed; re-derive from current evidence
  rather than treating an ignored failure as success.
- When review continuation stops, record the grounded cause and next owner
  under the shared convergence contract. Findings retain their severity.
- Re-entry derives from the record and current repository evidence. Runtime
  status, turn output and a remembered phase cannot substitute for that evidence.

## See also

- ~/.instrukt/docs/sdlc/procedure/lifecycle/work/build.md
- ~/.instrukt/docs/sdlc/procedure/lifecycle/work/review-build.md
- ~/.instrukt/docs/sdlc/procedure/lifecycle/fix-review-findings.md
- ~/.instrukt/docs/sdlc/procedure/lifecycle/work/finalize.md
- ~/.instrukt/docs/sdlc/procedure/lifecycle/work/deferrals.md
