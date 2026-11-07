---
description: The configured work cycle builds, reviews, revises and finalizes a prepared todo, with explicit command ownership for every mechanical effect.
delivered_by: [instrukt-first-build]
---
# Delivery Cycle — Spec

## Required reads

- [project/instrukt/spec/feature/workflow/artifact-currency](artifact-currency.md)

## What it is

A developer's prepared change moves through implementation, independent
review, revision, deferral disposition and finalization to an integration
handoff. The flow returns the obligation; configured commands perform work
and record evidence. The evaluator owns no worktree, session or stored phase.

## Canonical fields

### Work admission and progress

Work requires an existing, undelivered todo, delivered dependencies and current
preparation evidence. Entry also honors the work-record claim, hold and
workspace rules. Tasks route to task execution rather than this cycle.

Before normal builder admission, the record carries the original base commit,
approved plan digest and a separately written quality-checklist artifact.
Setup synchronization precedes those records; implementation requires them. If the approved plan changes during an existing build, the
orchestrator directs the builder to reconcile that build with the current
plan. The next call derives the obligation from the updated evidence.

The builder's completion stamp is build-completion evidence. Its write clears
the previous build-review verdict and finalize readiness. It does not run a
test or lint suite. Review follows completion; the integration contract owns
the candidate verification gate.

A current changes-requested review returns findings to the builder. The
builder records dispositions, commits fixes and stamps completion before
another review. Current approval permits deferral processing and finalization.
Any work-branch commit after completion clears `build.complete` until a new
completion stamp. A still-current independent approval may nevertheless
continue to deferrals and finalization, including after only clean upstream
merges; that approval’s current binding supplies this route, not a revived
completion fact. Fix commits after changes-requested never use that route.
Stale and undetermined evidence follow
`project/instrukt/spec/feature/workflow/artifact-currency`.

### Command and effect ownership

Every mechanical operation below is a configured command outside the pure
engine. The orchestrator invokes record commands and routes workspace effects to the
admitted child; the runtime adapter supplies capabilities rather than workflow policy.

| Inventory item | Responsible command or capability | Outcome and boundary |
| --- | --- | --- |
| Claim adoption, holds, role and title | Work-record entry and runner commands | Enforce one live driver and lane ownership. A denied operation remains denied; it is not a workflow phase. |
| Entry existence, delivery, dependency and stash-debt checks | Entry commands and provider | Admit eligible work or expose the concrete refusal before dispatch. Manual preparation retains its separate dependency rule. |
| Drift detection and stale checks | Orchestrator evidence-refresh command and fact provider | Obtain current repository and artifact evidence; publish facts for derivation. Agent analysis supplies classifications. |
| Todo worktree placement | Runner adapter on pickup, configured `runtime.start_thread` for deliberate entry | Derive `<slug>/work` and create it through the runtime from main after preparation integration, or from `origin/<slug>/work` on another computer; refuse an existing local branch. |
| Setup | Native shared setup child | Synchronize upstream, make the opening commit, run configured project preparation and publish the todo branch, then stamp role completion. |
| Build-start evidence | Bound setup child before completion | Record original base, approved plan digest and separate quality-checklist artifact before implementation. |
| Build and reconciliation | Native shared builder/fixer child | Reuse only an active attempt; new attempts capture the current shared head and implement the approved plan. |
| Build completion | `--build-complete` | Bind the clean todo head, clear the prior verdict/readiness and release that attempt atomically. |
| Setup/finalizer completion | `--role-complete` | Bind the clean todo head and release that attempt atomically without establishing build completion or readiness. |
| Synchronization failure | Outstanding child and its parent | Preserve the shared work and resolve within that attempt. Confirmed native stop plus a clean tree permits record-only failed release; a successor uses the same tree's current head. |
| Review dispatch and rounds | Orchestrator dispatch and verdict stamps | Count rounds from verdict stamps; no admission write. Re-evaluation or session reuse alone counts no round. |
| Verdict currency | Provider over target, findings and repository evidence | Supply current, stale or undetermined binding facts without executing a hidden transition. |
| Findings relay and fixes | Orchestrator dispatch | Deliver findings to an active builder attempt, or start a new fixer attempt at the current todo head after the preceding attempt is released. |
| Deferrals | Orchestrator deferrals action | Obtain a disposition per deferred item and stamp current processing, releasing the deferrer attempt in the same transaction before finalizer dispatch. |
| Review-approved notification | Orchestrator's finalization action | Convey current approval directly to the finalizer; there is no separate review-approved event. |
| Finalizer dispatch and readiness check | Orchestrator action and provider | Dispatch finalization, then compare the clean worktree, branch, remote and readiness stamp. |
| Kickback | Work-orchestrator record command and lane-release commands | Append the reason to input, invalidate the plan, reset finalization, release the lane and retain worktree, branch and commits. |
| Substantive-drift return | Work-orchestrator commands | Release the lane for preparation while preserving the working resources; re-derivation belongs to the affected layer. |
| Finalize-ready handoff | Finalize-ready stamp (including its event-log append) and lane-release commands | Make the candidate available to the standing integrator, confirm claim release, record the handoff and leave the work lane. |
| Preparation completion | Preparation orchestrator release commands | Confirm claim release and record the preparation handoff; the runner owns later dispatch. |

Command failures remain explicit evidence for the orchestrator. A silent skip
is not a completed merge; an ignored push result is not a published branch;
a refused release is not a released claim. Recovery instructions come from
the role's procedure and configured template.

### Shared worktree and serialized attempts

Preparation and work use separate T3-managed worktrees. instrukt derives `<slug>/prepare` and `<slug>/work`; agents never choose a bare slug or a role branch. Preparation starts from canonical main with `t3 thread start <project> - --worktree origin/main --branch <slug>/prepare --title <key>`. Its orchestrator and native shared children all use that preparation worktree. Creation refuses while the local phase branch still exists; retry or wake uses the retained native assignment, never another tree. Deliberate entry first returns placement and the configured native start instruction; after native startup, `todo prepare|work <slug> --placement-evidence <file|->` records supplied native session, key, recorded worktree, branch, head and source. The caller must be that native session in the matching registered tree. Git cwd alone never binds an orchestrator. Runner-created bindings instead use its verified adapter result.

Review binds the committed preparation-worktree head. An approving `instrukt todo stamp <slug> --verdict approve --phase preparation_head`, bound to `preparation-review-findings`, releases the independent reviewer and appends `workflow.preparation_reviewed` atomically. The integrator merges those reviewed docs as soon as possible, before build, folds their pending markers in its integration worktree and publishes through the ordinary gate. Once integrated the docs are current on main; they do not wait for build delivery. Other todos that depend on them wait for that integration.

After confirmed preparation integration, the integrator removes the clean preparation worktree and deletes `<slug>/prepare` locally and on `origin`. Failure or dirty/in-use state retains the cleanup obligation and its cause; retained thread records and review evidence stay. Work starts later from the new main with `--worktree origin/main --branch <slug>/work`; on another computer its retained published work starts with `--worktree origin/<slug>/work --branch <slug>/work`. The work orchestrator and setup, builder, reviewer, fixer, deferrer and finalizer share that one work worktree through native shared delegation. Only the integrator writes main. Ad hoc authoring uses its own one-off worktree, reviewed before integration.

The work orchestrator coordinates only; setup, builder, reviewer, fixer,
deferrer and finalizer are native shared children through `runtime.dispatch`.
They all use that recorded worktree and todo branch. One outstanding workspace
attempt is the workflow's single admission. The next role waits for the required
stamp and confirmed release and a clean shared tree. The reviewer examines its
current captured head in that same tree. A runtime result alone releases nothing.

Setup orders upstream synchronization, opening commit, configured project
preparation, first publication, build-start evidence and the separately written
quality checklist, then `--role-complete`. The original base is captured before
synchronization. Builder admission requires all of that evidence. A checklist-only
gap does not repeat completed setup or synchronization. Plan reconciliation
updates the approved-plan binding before implementation resumes.

Completion stamps and release commit together. Review verdicts and deferral
processing likewise release atomically. Every commit is already on the todo
branch; role transition changes records, not Git refs. Released bindings retain
native child id, attempt, starting head, completed head and shared placement.

A synchronization conflict remains with the outstanding child. If it cannot
continue, the parent retains its native failure and workspace evidence. Failed
release under the todo-attempt contract requires a stopped child and clean tree;
dirty or conflicted work stays outstanding for recovery in place. A successor
starts at the current shared head after release, completes missing setup before
implementation, and preserves the original build base. Failure itself establishes
neither setup completion nor a successful build.

### Finalization and integration handoff

Deferrals settle before finalizer admission. The finalizer synchronizes and
publishes the todo branch, then checks its clean shared head equals local and
remote todo heads and records `--role-complete`. The stamp releases that attempt.
A changed candidate or review report receives independent review at the current
head in the same worktree after release; its verdict releases the reviewer.
Changes requested route through a shared fixer attempt and completion before
review. An undetermined binding returns to evidence.

With current approval, no outstanding workspace attempt and matching candidate
identities, the retained finalizer records `--finalize-ready` as a record-only
follow-up. This stamp and `workflow.finalized` append commit together,
idempotently by project/todo/SHA. Further workspace changes require a new
finalizer attempt and fresh completion. A mismatch returns reconciliation and
restamping, not an integration handoff under old evidence.

The work orchestrator confirms the event and claim release and records its
integration handoff. The standing integrator owns candidate verification under
`project/instrukt/spec/feature/workflow/integration-gate`. After confirmed
delivery it removes the todo's one worktree from the project folder, observing
`worktree.exists` until absent. Dirty/in-use trees or failed removal remain owed.
The runtime thread record and retained attempt evidence stay. Kickback and
substantive-drift return retain the todo branch, worktree and commits.

### Settled command bindings

- Reserve with `instrukt todo attempt <slug> --role <role> … --brief -`,
  delegate the returned assignment through `runtime.dispatch` with shared
  isolation, then `todo bind` the native child id and shared placement.
- `--role-complete` releases setup/finalizer; `--build-complete` releases
  builder/fixer; review verdicts and `--deferrals-processed` release their roles.
  Stamp and release are one transaction under the stamps contract.
- `--build-started` retains the original base and current approved plan;
  the quality checklist is a separate required artifact.
- `--finalize-ready` uses the retained successful finalizer binding after
  current approval and records the event with readiness.
- Release the lane with `instrukt todo update <slug> --unclaim`; confirm
  `claim.held` is false and record the integration handoff.
- The integrator folds pending docs before candidate verification, records
  `instrukt todo deliver <slug> --commit <sha>` after promotion, and executes
  `worktree.remove` for the one todo worktree. Failed removals remain owed.

### Fresh canonical base

Before creating either `<slug>/prepare` or `<slug>/work`, fetch origin and use
`t3 thread start <project> - --worktree origin/main --branch <slug>/<phase> --title <key>`.
Only the integrator writes and pushes gate-green exact commits to main; local
main can lag and supplies no new-phase base. A fetch failure leaves startup
unperformed, with its cause visible. A retained published `<slug>/work` on
another computer is continuation of the existing phase, using freshly fetched
`origin/<slug>/work` with the same local branch name; it is never reset to main.
For repositories whose canonical branch has another name, use that remote ref.

## Use cases

### UC-DC1: Deliver a prepared change for integration

```gherkin
Given a developer has a prepared change whose dependencies are delivered
When the developer has the change implemented through the project's workflow
Then an integration candidate is handed off only after current independent approval, deferral disposition and matching finalization evidence
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

### UC-DC4: The builder receives the completed workspace setup

```gherkin
Given a developer's work orchestrator is bound to the todo's one worktree
When its shared setup child completes synchronization, opening commit, project preparation, publication and build-admission evidence
And records role completion at the clean todo head
Then that stamp releases setup atomically
And the builder starts in the same worktree on the same todo branch only after that release
```

### UC-DC5: Deferral disposition permits finalization without changing the code

```gherkin
Given a developer's approved change has deferred items awaiting disposition
When the shared deferrer records dispositions covering the current artifact and stamps processing
Then that attempt is released without changing the todo branch
And a shared finalizer can start in that same clean worktree at its current head
But missing or stale processing evidence keeps the deferrer outstanding and finalization pending
```

### UC-DC6: Integration receives a currently reviewed final candidate

```gherkin
Given a developer's finalizer synchronized and published a changed candidate in the shared todo worktree
When it records role completion against matching clean worktree, todo branch and remote heads
Then completion releases that attempt atomically
And an independent reviewer examines the current head in the same worktree
And readiness waits for that review's current approval and release
When the retained finalizer records readiness for that matching candidate
Then readiness and the finalized event commit together
```

### UC-DC7: Recover failed setup without losing its work

```gherkin
Given a developer's setup child has committed work on the shared todo branch but cannot finish synchronization
When native evidence confirms it has stopped and the shared tree is clean
And the orchestrator records the failed attempt with its reason and workspace evidence
Then its release preserves the committed work and creates no completion stamp
When a successor is admitted
Then it continues in that same worktree from the current todo head
And completes missing setup and build-admission evidence before implementation
And completed implementation receives independent review before finalization
But dirty or conflicted work keeps the original attempt outstanding for recovery in place
```

### UC-DC8: Finish delivery without leftover phase resources

```gherkin
Given a developer's reviewed work branch has integrated and the todo is delivered
When the integrator performs the returned cleanup obligations
Then the recorded worktree and merged work branch are removed locally and on origin
And retained native thread and role-attempt evidence remains readable
But a failed or incomplete deletion remains owed until its absence is observed
```

## Known caveats

- Re-creating a deleted orchestrator thread while its todo branch remains checked
  out requires existing-worktree binding. T3's CLI cannot bind that tree; remove
  it safely first, or use the runtime's own existing-worktree start where available
  (T3 MCP). Waking a settled retained orchestrator uses its existing binding.

## See Also

- [sdlc/principle/design-fundamentals](../../../../domain/sdlc/principle/design-fundamentals.md)
- [general/principle/trust-and-guardrails](../../../../general/principle/trust-and-guardrails.md)
- [general/principle/intelligence-boundaries](../../../../general/principle/intelligence-boundaries.md)
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
- [project/instrukt/spec/feature/work/todos](../work/todos.md)
- [project/instrukt/spec/feature/work/stamps](../work/stamps.md)
