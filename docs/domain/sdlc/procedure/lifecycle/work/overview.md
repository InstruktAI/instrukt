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

- [general/principle/session-lifecycle](../../../../../general/principle/session-lifecycle.md)
- [general/concept/agent-characteristics](../../../../../general/concept/agent-characteristics.md)
- [general/procedure/orchestration](../../../../../general/procedure/orchestration.md)
- [sdlc/policy/preparation-artifact-quality](../../../policy/preparation-artifact-quality.md)

## Goal

Drive `instrukt todo work {slug}` through the build-review-fix-finalize cycle
until the todo is handed to integration or returned to preparation. The
orchestrator routes workspace actions and effects to the admitted shared child,
records its coordination outcomes and supervises the workers' progress.

### The doc layer is frozen — prepare authors, work executes

Prepare authors doc snippets in `<slug>/prepare` for immediate integration after
review. Work starts after that integration and preparation cleanup. The doc layer
is frozen for the work phase: no work-phase session — builder, reviewer, fixer, or finalizer — changes
the doc layer or takes its state as work. Writing under `docs/**`, folding
pending markers, and confirming or verifying whether they are folded are the same
violation whatever verb the task uses. Reading doc snippets as input is untouched
and required: the work phase binds in-scope feature-spec scenarios and implements
against published contracts. The slug's doc reflection was authored during preparation and folded by the
integrator at preparation integration, before build.
A doc need discovered mid-build (a missing scenario, a wrong planned contract)
is a plan defect: surface it and route through the kickback path (Step 9) so
prepare re-derives in its preparation worktree — never edit `docs/**` in the work-phase worktree.

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

### Phase worktrees

Preparation and work use separate T3-managed worktrees. instrukt derives `<slug>/prepare` and `<slug>/work`; agents never choose a bare slug or a role branch. Preparation starts from canonical main with `t3 thread start <project> - --worktree origin/main --branch <slug>/prepare --title <key>`. Its orchestrator and native shared children all use that preparation worktree. Creation refuses while the local phase branch still exists; retry or wake uses the retained native assignment, never another tree.

Review binds the committed preparation-worktree head. An approving `instrukt todo stamp <slug> --verdict approve --phase preparation_head`, bound to `preparation-review-findings`, releases the independent reviewer and appends `workflow.preparation_reviewed` atomically. The integrator merges those reviewed docs as soon as possible, before build, folds their pending markers in its integration worktree and publishes through the ordinary gate. Once integrated the docs are current on main; they do not wait for build delivery. Other todos that depend on them wait for that integration.

After confirmed preparation integration, the integrator removes the clean preparation worktree and deletes `<slug>/prepare` locally and on `origin`. Failure or dirty/in-use state retains the cleanup obligation and its cause; retained thread records and review evidence stay. Work starts later from the new main with `--worktree origin/main --branch <slug>/work`; on another computer its retained published work starts with `--worktree origin/<slug>/work --branch <slug>/work`. The work orchestrator and setup, builder, reviewer, fixer, deferrer and finalizer share that one work worktree through native shared delegation. Only the integrator writes main. Ad hoc authoring uses its own one-off worktree, reviewed before integration.


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

The work orchestrator is already bound to the todo's one runtime-managed
worktree with the todo branch checked out. Runner pickup uses the selected
adapter; deliberate entry uses `runtime.start_thread` with that same placement.
Preparation uses `<slug>/prepare`, reviewed and integrated immediately before a separate `<slug>/work` starts from updated main. Waking the retained work
orchestrator preserves its workspace. A deleted orchestrator whose branch is
still checked out needs safe worktree removal first or the runtime's own
existing-worktree start where available.

Coordinate this assignment and route workspace actions to its admitted child.
Keep the shared tree clean between roles. Native children use shared isolation
through `runtime.dispatch`; each commits directly on the todo branch. One
outstanding workspace-role attempt is the workflow's single admission, including
an unresolved dispatch. Completion stamps release writing attempts atomically;
verdicts and dispositions release read-only attempts. Reuse only active,
unreleased attempts. A released session retains evidence, not workspace admission.

Select workers from procedure dispatch preferences or `agents.order`, availability
first, and report runtime project defaults when selection is unsupported. Use
`med` for builder/fixer, `slow` for independent review and `fast` for finalizer.
Inspect native task evidence and current bindings before follow-up. A stopped
worker follows the todo-attempt failure contract: clean shared tree and native
stop evidence precede failed release; dirty or conflicted work remains outstanding
for recovery in place. A successor uses that same tree's current head.

### Child attention and re-entry

Reserve each worker with the selected `instrukt todo attempt` action. Delegate
its rendered brief, role, attempt, semantic key, display title and captured shared head through the
configured runtime instruction; record `todo bind` with the native child id and
matching shared workspace evidence. Follow the native operation instructions in
`general/procedure/ai-to-ai-operations`. The runtime supplies parent linkage and
reports the child's result. Yield while valid work continues.

Use the configured dispatch value with shared isolation. Verify the native child
inherits the recorded todo worktree, todo branch and captured current head before
binding. A mismatch remains an unresolved assignment gap. Read its native result,
artifacts, completion stamp and release before re-entering work. Resolve input
waits within the approved plan or route a plan defect through Step 9.

### 2. Build

Reserve a setup child with `instrukt todo attempt {slug} --role setup --brief -`,
delegate its setup-only brief through the configured shared dispatch value, and
record `todo bind`. That child synchronizes upstream, makes the opening commit,
runs configured preparation and first publication, records original-base and
approved-plan evidence with `--build-started`, and writes the quality checklist.
Its `--role-complete` stamp releases setup at the clean shared head. Confirm all
outcomes and release before builder admission.

Dispatch `sdlc/procedure/lifecycle/work/build` in that same worktree. The builder
commits per task and records `--build-complete`, which clears prior verdict and
readiness and releases the attempt atomically. Re-enter work from that evidence;
commits or a native completion reply alone establish no release.

For an updated approved plan, direct the builder to reconcile its delta and
re-record `--build-started` while preserving the original base before continuing.

### 3. Review

Dispatch a shared independent reviewer after the writing attempt is released.
Bind the current clean todo head as the target. The reviewer reads that head in
the same worktree, writes complete `review-findings` and stamps the digest-bound
verdict; that transaction releases its attempt. Verify substantive review under
`general/procedure/orchestration` before choosing the next action. Review rounds
count verdict stamps; a successful upstream merge alone admits no review.

### 4. Fix (if needed)

A current changes-requested verdict routes findings to a new shared builder-role
attempt as fixer after reviewer release. The fixer applies
`sdlc/procedure/lifecycle/fix-review-findings`, records dispositions, commits on
the todo branch and stamps `--build-complete`. Confirm its atomic release before
independent review resumes at that current shared head.

Route upstream conflicts, failed project checks, dirty workspace, missing stamps
and stale publication to the admitted worker. Candidate suite execution belongs
to integration. Review counts are informational; continuation follows
`sdlc/procedure/lifecycle/review` and preserves unresolved findings.

### 5. Finalize

After current approval and deferral processing, dispatch a shared finalizer.
It synchronizes upstream and publishes the todo branch, verifies clean worktree,
local todo branch and remote head equality, then records `--role-complete`.
That transaction releases its attempt. Changed content or review evidence then
receives independent review at the current head in the same worktree; its verdict
releases that reviewer. Changes requested require a shared fixer, completion and
another review. Undetermined currency returns to the evidence route.

When current approval, finalizer completion and matching candidate evidence hold
with no outstanding workspace role, send the retained finalizer the record-only
`--finalize-ready` follow-up. Retry only that write after interruption; further
workspace work needs a new attempt and completion. Readiness and
`workflow.finalized` append commit together for the exact project/todo/SHA.
Confirm the handoff, run `instrukt todo update <slug> --unclaim`, confirm release
and record remaining obligations. Integration belongs to the standing integrator.

After confirmed delivery, the integrator removes this todo's one worktree from
the project folder. A dirty/in-use tree or failed removal remains owed. The
runtime thread record and retained attempt evidence stay. A blocked finalizer
preserves shared work and follows the same in-place failure contract as setup.

### 6. Deferrals

If `deferrals` exists and is pending, execute the deferrals action before
finalizer dispatch. Decide each entry under
`sdlc/procedure/lifecycle/work/deferrals`, record dispositions and stamp
`--deferrals-processed` for the current artifact. That stamp releases the deferrer
attempt without a branch advance. Confirm release before dispatching a finalizer.

### 7. Bug fix

Bug work follows the same builder-role attempt rules. Its mandate is the checked `input`, settled
causal `diagnosis` and reviewed plan. Diagnosis prescribes no fix or verification
approach; the plan derives them. The builder implements that plan, binds the
prepared scenarios where permitted and stamps build completion as above.

### 8. A stamp the worker owed and did not make

The orchestrator never stamps an outcome on a worker's behalf.
Read the record before acting. A step requires its artifact and stamp, bound
to current evidence; neither elapsed time nor a successful reply establishes it.
Read the native child result, inspect current record evidence, and return
a missing obligation to its retained worker through a bounded follow-up when it
can continue. Preserve an unavailable attempt's failure evidence and route its
recovery under the owning contract. Keep the established outcome and acting
session in the record.

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
then record the handoff and any remaining obligations. Preserve conversations
needed for later rounds or human input. The runtime owns session ending and
cleanup. An unfinished human conversation keeps its context and gives up its
claim.

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

- [sdlc/procedure/lifecycle/work/build](build.md)
- [sdlc/procedure/lifecycle/work/review-build](review-build.md)
- [sdlc/procedure/lifecycle/fix-review-findings](../fix-review-findings.md)
- [sdlc/procedure/lifecycle/work/finalize](finalize.md)
- [sdlc/procedure/lifecycle/work/deferrals](deferrals.md)
