---
description: The configured work cycle builds, reviews, revises and finalizes a prepared todo, with explicit command ownership for every mechanical effect.
delivered_by: [instrukt-first-build]
---
# Delivery Cycle — Spec

## Required reads

@project/instrukt/spec/feature/workflow/artifact-currency

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

Before builder admission, the record carries the original base commit, approved
plan digest and a separately written quality-checklist artifact. If the approved plan changes during an existing build, the
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
engine. The orchestrator invokes it directly or through the effect executor;
the runtime adapter provides session capabilities rather than workflow policy.

| Inventory item | Responsible command or capability | Outcome and boundary |
| --- | --- | --- |
| Claim adoption, holds, role and title | Work-record entry and runner commands | Enforce one live driver and lane ownership. A denied operation remains denied; it is not a workflow phase. |
| Entry existence, delivery, dependency and stash-debt checks | Entry commands and provider | Admit eligible work or expose the concrete refusal before dispatch. Manual preparation retains its separate dependency rule. |
| Drift detection and stale checks | Orchestrator evidence-refresh command and fact provider | Obtain current repository and artifact evidence; publish facts for derivation. Agent analysis supplies classifications. |
| Setup and role worktree creation | Runtime through the sessions facade | Establish the todo branch from the canonical or existing remote todo head; create a separate T3 worktree per role attempt at its captured head and return its path. Setup receives only its setup brief. |
| Merge of upstream canonical branch | Orchestrator merge command before initial build-start evidence | Expose skipped merges, fetch failures and conflicts. On conflict, abort the merge and route recovery to the live builder's own procedure. An absent builder is an explicit dispatch gap. |
| Opening commit and project preparation | Orchestrator after upstream synchronization | Make the opening commit, then run the project’s `prepare` action when declared, before build-start evidence. |
| First-entry branch publication | Orchestrator branch-publication command | Publish with lease protection; report the outcome rather than assuming publication. |
| Build-start evidence | Record-writing command at builder admission | Capture base, plan and checklist without recording a workflow phase. |
| Build dispatch and plan reconciliation | Orchestrator action using configured session capabilities | Reuse an active, unreleased builder attempt or start a new attempt at the current todo head; present the current plan or reconciliation obligation. |
| Build completion and role handoff | Completion-stamp command and orchestrator ref advance | Record the role head, advance the todo branch by fast-forward only, retain released attempt evidence, invalidate the previous build verdict and reset finalization. |
| Review dispatch and rounds | Orchestrator dispatch and verdict stamps | Count rounds from verdict stamps; no admission write. Re-evaluation or session reuse alone counts no round. |
| Verdict currency | Provider over target, findings and repository evidence | Supply current, stale or undetermined binding facts without executing a hidden transition. |
| Findings relay and fixes | Orchestrator dispatch | Deliver findings to an active builder attempt, or start a new fixer attempt at the current todo head after the preceding attempt is released. |
| Deferrals | Orchestrator deferrals action | Obtain a disposition per deferred item and stamp current processing, releasing the deferrer attempt without a branch advance before finalizer dispatch. |
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

Builder admission requires the quality-checklist artifact to exist. Setup
orders worktree creation, upstream synchronization, opening commit, project
preparation when declared, first branch publication, setup completion and confirmed
fast-forward handoff, build-start record, quality-checklist presence, then builder dispatch. Plan reconciliation
re-records `--build-started` against the current plan before resuming the build.
Cleanup observes each retained attempt path with `worktree.exists` and
`done_when: {not: {exists: worktree.exists}}`.

A checklist-only admission gap does not repeat synchronization after the
build-start stamp. First branch publication follows the opening commit and
project preparation. Once finalize readiness is recorded, confirmation and
handoff owe no merge or branch-publication effect.

### Sequential role attempts and branch handoff

The todo branch is the durable handoff between roles. On T3, setup, builder,
reviewer, fixer, deferrer and finalizer each use their own role worktree under
`project/instrukt/spec/feature/runner/session-continuity`. Only one workspace-role attempt of
that todo is admitted at a time; the claim-holding orchestrator
coordinates their serialized work and mechanical handoffs. A pending start or branch handoff prevents the next role
from starting. Reuse is limited to follow-ups within an active attempt.

A code-writing role completes by committing its work and recording the required
completion stamp bound to its branch head. The orchestrator checks that head
and a clean worktree, and advances the todo branch to it only when the current
todo head is an ancestor. The ref update compares the expected old head;
a concurrent ref change requires re-observation. This moves only the todo
branch, never the canonical branch. No merge, rebase or force-update substitutes
for the fast-forward. A read-only review records its verdict at the captured
todo head and releases its attempt in the verdict transaction without changing
either branch. Deferral processing records dispositions covering the current
deferrals artifact and releases the deferrer attempt in the
`--deferrals-processed` transaction, also without a branch advance. Both
read-only releases require a clean worktree at the captured todo head.

If advancement is not a fast-forward, preserve both heads and the role worktree,
record a plain finding naming the branches and commits, and automatically hold
the todo with that reason. Later roles and integration handoff remain blocked.
The orchestrator reconciles the finding through the workflow's revision route;
release of the hold follows the existing work-record command contract.

Git refs and SQLite have no shared transaction. Before advancing, persist the
attempt's stamped source head and expected todo head as pending handoff evidence.
After the ref update, record its resulting todo head and mark the attempt
released in one work-record transaction. Recovery observes the refs: an exact
match with the recorded source head completes that recording idempotently;
an unchanged expected head permits retry; any other head requires a fresh
fast-forward check or the same finding and hold. A stamp alone never releases
an unfinished handoff. Released bindings retain branch, base, head and path.

### Finalization and integration handoff

Deferrals are settled before finalizer dispatch. Readiness requires a clean
finalizer worktree and equality of its head, the advanced local todo branch,
the remote todo branch and the commit named by the finalize-ready stamp. A
finalizer first records a `role_complete` stamp; its handoff advances the todo
branch and confirms attempt release before the finalize-ready stamp and
`workflow.finalized` commit together. The finalizer records readiness against
that retained binding; this record-only write admits no further workspace work.
Further workspace changes require a new finalizer attempt at the current todo
head and its own completion and confirmed handoff.
A readiness mismatch returns
an explicit reconciliation and restamping obligation; it does not hand off a
candidate under an earlier stamp.

The sole workflow event is `workflow.finalized`, addressed to the standing
integrator for the todo and finalized commit. Re-reading unchanged readiness
does not create a new logical handoff. Integration is governed by
`project/instrukt/spec/feature/workflow/integration-gate`.

### Settled command bindings

- Runtime setup uses setup-only `instrukt sessions run --worktree <name> --todo <slug> --brief -`;
  its durable start key and setup-attempt binding expose `worktree.path`. After
  upstream synchronization, the opening commit and project preparation, release
  setup through the same fast-forward handoff before builder admission.
- `instrukt todo stamp <slug> --build-started` captures the original base and
  approved plan digest; write the quality checklist separately before admission.
- Review rounds count verdict stamps; dispatch requires no admission write.
- Setup and finalizer completion use `instrukt todo stamp <slug> --role-complete`;
  builder/fixer completion uses `--build-complete`. Role-attempt evidence and
  admissibility are owned by `project/instrukt/spec/feature/work/stamps`.
- `instrukt todo stamp <slug> --finalize-ready` follows the finalizer branch
  handoff and records readiness with `workflow.finalized` in one transaction,
  idempotently by project/todo/SHA.
- Release with `instrukt todo update <slug> --unclaim`; confirm `claim.held`
  is false and record the handoff. The runtime owns session ending and
  cleanup; preserve conversations still needed for review or human input.
- The integrator runs `instrukt docs fold <slug>` before candidate verification,
  then `instrukt todo deliver <slug> --commit <sha>` after confirmed promotion.
  Delivered worktree cleanup covers every retained role-attempt path with
  `git worktree remove <path>` and `done_when` the path is absent, through the
  runtime's supported cleanup path. Returned preparation work retains its
  working resources.

### Use cases

#### UC-DC1: Deliver a prepared change for integration

```gherkin
Given a developer has a prepared change whose dependencies are delivered
When the developer has the change implemented through the project's workflow
Then an integration candidate is handed off only after current independent approval, deferral disposition and matching finalization evidence
And findings requiring changes return to implementation before that handoff
```

#### UC-DC2: A fix reaches the next review and the delivered todo branch

```gherkin
Given a developer's change has review findings requiring fixes
When the workflow dispatches a fixer on T3
Then the fixer works in its own worktree at the todo branch's current head
When the fixer commits the corrections and stamps completion
Then the todo branch advances by fast-forward to the fixer's stamped head
And the next independent review sees those corrections in its own worktree
And the released fixer branch and worktree remain recorded as evidence
```

#### UC-DC3: Divergent work is preserved for reconciliation

```gherkin
Given a developer's completed role branch diverges from the current todo branch
When the workflow attempts to hand that role's work to the next role
Then the todo branch is unchanged and the todo is held
And the finding identifies both branches and commits for reconciliation
And the role's work is preserved without dispatching the next role
```

#### UC-DC4: The builder receives the completed workspace setup

```gherkin
Given a developer's prepared change needs its workspace initialized on T3
When the workflow completes the todo-bound setup attempt
Then its committed setup reaches the todo branch by confirmed fast-forward handoff
And the builder starts in a new worktree at that advanced head only after setup is released
```

#### UC-DC5: Deferral disposition permits finalization without changing the code

```gherkin
Given a developer's approved change has deferred items awaiting disposition
When the deferrer records dispositions covering the current deferrals artifact and stamps processing
Then that attempt is released without changing the todo branch
And a finalizer can start in its own worktree at the current todo head
But missing or stale processing evidence keeps the deferrer outstanding and finalization pending
```

#### UC-DC6: Integration receives only a candidate whose finalizer handoff is confirmed

```gherkin
Given a developer's approved change has been synchronized and published by a finalizer
When the finalizer records role completion
Then integration readiness remains pending until the todo branch has reached that stamped head by confirmed fast-forward handoff
When the finalizer records readiness against matching clean worktree, local todo branch and remote branch evidence
Then the readiness stamp and finalized event are recorded together for that candidate
```

## Known caveats

- T3's host CLI creates worktrees but has no attach-existing-worktree option.
  The shipped sdlc delivery workflow uses sequential per-role worktrees and
  needs creation only. An upstream attach option would let roles share one
  worktree again; it is to be requested from T3, alongside pingdotgg/t3code#31621.

## See Also

- sdlc/principle/design-fundamentals
- general/principle/trust-and-guardrails
- general/principle/intelligence-boundaries
