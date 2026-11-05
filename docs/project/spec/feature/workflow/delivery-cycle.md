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
An explicit failed-attempt recovery may begin synchronization before those
records; implementation still requires them. If the approved plan changes during an existing build, the
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
| Merge of upstream canonical branch | Orchestrator synchronization command in the outstanding attempt | Expose skipped merges, fetch failures and conflicts. That setup, builder or finalizer attempt resolves its own synchronization conflict when it can; otherwise failed release precedes recovery-builder admission from its recorded role head. |
| Opening commit and project preparation | Orchestrator after upstream synchronization | Make the opening commit, then run the project’s `prepare` action when declared, before build-start evidence. |
| First-entry branch publication | Orchestrator branch-publication command | Publish with lease protection; report the outcome rather than assuming publication. |
| Build-start evidence | Record-writing command at builder admission | Capture base, plan and checklist without recording a workflow phase. |
| Build dispatch and plan reconciliation | Orchestrator action using configured session capabilities | Reuse an active, unreleased builder attempt or start a new attempt at the current todo head; present the current plan or reconciliation obligation. |
| Build completion and role handoff | Completion stamp, then `instrukt todo handoff <slug>` | Record the role head, advance the todo branch by fast-forward only, retain released attempt evidence, invalidate the previous build verdict and reset finalization. |
| Failed synchronization and recovery | `instrukt todo handoff <slug> --failed --reason <text>`, then builder dispatch with `--recover-attempt <role/attempt>` | Release the failed writing attempt with its reason, branch/head/path and workspace evidence; leave the todo branch unchanged. The recovery builder starts at that preserved head and completes through normal handoff. |
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
Cleanup enumerates every retained attempt path and role branch. Each
`worktree.remove` observes its selected path with `worktree.exists`; after all
paths are absent, `role-branch.remove` observes each selected role branch with
`role_branch.exists`. Absence satisfies the corresponding `done_when`.

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

A new recovery builder is the explicit exception to the current-todo-head base:
`instrukt sessions run --todo <slug> --recover-attempt <role/attempt> …` creates
its own worktree at that failed attempt's recorded committed role head. The
source must be a confirmed failed writing attempt with no outstanding successor;
all ordinary claim, hold and preparation checks still apply. An unresolved start
or handoff remains outstanding until its own recovery confirms release.

A code-writing role completes by committing its work and recording the required
completion stamp bound to its branch head. The orchestrator runs
`instrukt todo handoff <slug>` after setup/finalizer
`--role-complete` or builder/fixer `--build-complete`. The command resolves
the todo's outstanding completed attempt from the work record, checks its
stamped head and clean worktree, and advances the todo branch only when its
current head is an ancestor of that stamped head. The ref update compares
the expected old head; a concurrent ref change requires re-observation.
This moves only the todo
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

Git refs and SQLite have no shared transaction. `todo handoff` persists the
attempt's stamped source head and expected todo-branch head as pending intent
before changing the ref. After the compare-and-swap fast-forward, it confirms
the resulting head and releases the attempt in one work-record transaction.
Retry observes that recorded intent: the exact source head finishes confirmation;
the unchanged expected head permits the same ref update. A different observed
head must pass a fresh fast-forward check to the same immutable source head;
record the new expected head before retrying the ref update. Divergence records
the finding and automatic hold while preserving the outstanding attempt. No
retry substitutes a different source or advances beyond it. A confirmed retry
reports the recorded handoff and changes no ref. A stamp alone never releases
an unfinished handoff. Released bindings retain branch, base, head and path.

An outstanding writing attempt first resolves its own synchronization conflict
when it can. When it cannot, the orchestrator runs
`instrukt todo handoff <slug> --failed --reason <text>`. This alternative requires
an observed committed role head, not a completion stamp or clean worktree. It
records failure and retained workspace evidence, releases the attempt and queues
reflection atomically, without advancing the todo branch. A potentially executed
pending ref update must be reconciled before this failed release. A retry with
no outstanding attempt reports the confirmed failed outcome without a mutation.

Failed setup and finalizer attempts use this same route. Only after confirmed
failed release may the orchestrator admit a recovery builder at the recorded
failed head. Retain any uncommitted work or unfinished merge in the failed path
as recovery evidence; the new builder brings applicable work into its own
worktree and resolves synchronization there against accepted upstream and the
current todo head so its normal handoff can fast-forward. For failed setup, finish its missing
opening commit, project preparation and publication before implementation;
record missing build-start and checklist evidence there. Preserve the original
build base; when no build-start exists, use the original setup's captured base,
not the recovery head. This recovery dispatch is admitted for synchronization before
those build-admission records; implementation still waits for both. The builder
then completes the approved build, stamps `--build-complete` and hands off
normally. Its confirmed handoff satisfies the failed setup's outstanding setup
obligations as well as settling recovery. Independent review follows, then a
new finalizer; a failed finalizer's binding never supplies readiness. A failure
in the recovery builder itself is released by the same failed route, retaining
the chain and selecting the latest failed role head for its successor.

The command's statuses and caller boundary are owned by
`project/instrukt/spec/feature/work/todos`; it performs no runtime start,
publication, merge, rebase, readiness stamp or canonical-branch update.

### Finalization and integration handoff

Deferrals are settled before finalizer dispatch. Readiness requires a clean
finalizer worktree and equality of its head, the advanced local todo branch,
the remote todo branch and the commit named by the finalize-ready stamp. A
finalizer first checks equality of its clean worktree head, local role branch
and published remote head, then records a `role_complete` stamp. Completion
requires consistency of this finalized candidate, independently of whether the
earlier reviewed head still matches. Its handoff advances the todo
branch and confirms attempt release before the finalize-ready stamp and
`workflow.finalized` commit together. The finalizer records readiness against
that retained binding; this record-only write admits no further workspace work.
Further workspace changes require a new finalizer attempt at the current todo
head and its own completion and confirmed handoff.
When finalization changes todo content or its review report, finish and hand off
that finalizer attempt before requesting independent review. Its release
permits a new review attempt at the advanced todo head; the review sees that
head in its own worktree. The verdict releases the reviewer. Current approval
then permits record-only readiness against the retained finalizer binding if
all candidate identities still match, or a new finalizer attempt for further
workspace reconciliation. Changes requested route through a new fixer attempt
and its handoff before review resumes. An undetermined binding is resolved
through the evidence route before another role is admitted.

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
- `instrukt todo handoff <slug>` is the orchestrator action after every writing
  attempt completion, including setup, builder/fixer and finalizer. Pending
  completion or ref-advance evidence selects this action before another role.
  Read-only verdict and deferral-processing stamps release directly.
- `instrukt todo handoff <slug> --failed --reason <text>` releases an attempt
  unable to finish synchronization while preserving its work. The explicit
  recovery-builder dispatch captures that failed role head as its base; normal
  completion and handoff remain required before later roles.
- `instrukt todo stamp <slug> --finalize-ready` follows the finalizer branch
  handoff and records readiness with `workflow.finalized` in one transaction,
  idempotently by project/todo/SHA.
- Release with `instrukt todo update <slug> --unclaim`; confirm `claim.held`
  is false and record the handoff. The runtime owns session ending and
  cleanup; preserve conversations still needed for review or human input.
- The integrator runs `instrukt docs fold <slug>` before candidate verification,
  then `instrukt todo deliver <slug> --commit <sha>` after confirmed promotion.
  Delivered worktree cleanup covers every retained role-attempt path with
  `git worktree remove <path>` and `done_when` the path is absent, followed by
  safe deletion of every retained role branch from the project checkout.
  Preserve failed removals as owed obligations. Returned preparation work retains its
  working resources.

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
When the workflow dispatches a fixer on T3
Then the fixer works in its own worktree at the todo branch's current head
When the fixer commits the corrections and stamps completion
And the orchestrator runs instrukt todo handoff for that completed attempt
Then the todo branch advances by fast-forward to the fixer's stamped head
And the next independent review sees those corrections in its own worktree
And the released fixer branch and worktree remain recorded as evidence
But when that command stops after advancing the branch and before confirming release
Then retrying it confirms the recorded handoff without advancing the branch again
```

### UC-DC3: Divergent work is preserved for reconciliation

```gherkin
Given a developer's completed role branch diverges from the current todo branch
When the orchestrator runs instrukt todo handoff for that role
Then the todo branch is unchanged and the todo is held
And the finding identifies both branches and commits for reconciliation
And the role's work is preserved without dispatching the next role
```

### UC-DC4: The builder receives the completed workspace setup

```gherkin
Given a developer's prepared change needs its workspace initialized on T3
When setup stamps role completion and the orchestrator runs instrukt todo handoff
Then its committed setup reaches the todo branch by confirmed fast-forward handoff
And the builder starts in a new worktree at that advanced head only after setup is released
```

### UC-DC5: Deferral disposition permits finalization without changing the code

```gherkin
Given a developer's approved change has deferred items awaiting disposition
When the deferrer records dispositions covering the current deferrals artifact and stamps processing
Then that attempt is released without changing the todo branch
And a finalizer can start in its own worktree at the current todo head
But missing or stale processing evidence keeps the deferrer outstanding and finalization pending
```

### UC-DC6: Integration receives only a candidate whose finalizer handoff is confirmed

```gherkin
Given a developer's approved change has been synchronized and published by a finalizer
When the finalizer verifies that its clean worktree head, role branch and published head match each other
And records role completion even though its content differs from the earlier reviewed candidate
Then integration readiness remains pending until instrukt todo handoff confirms that stamped head and releases the finalizer attempt
And a new independent review starts at the advanced todo head in its own worktree after that release
And readiness waits for its current approval and release
When the finalizer records readiness against matching clean worktree, local todo branch and remote branch evidence
Then the readiness stamp and finalized event are recorded together for that candidate
```

### UC-DC7: Recover failed setup without losing its work

```gherkin
Given a developer's setup attempt has committed work on its own role branch
And synchronization cannot be completed within that attempt
When the orchestrator records a failed handoff with the reason
Then the failure, role branch, committed head and workspace evidence are retained and that attempt is released
And the todo branch remains unchanged
When a recovery builder starts after that release
Then its new worktree starts at the failed setup's recorded head with the committed work preserved
And it completes synchronization and the remaining setup obligations before implementing the approved plan
When the recovery builder stamps build completion and hands off normally
Then the todo branch advances by confirmed fast-forward to the recovery builder's head
And independent review sees the recovered work before a new finalizer is admitted
```

## Known caveats

- T3's host CLI creates worktrees but has no attach-existing-worktree option.
  The shipped sdlc delivery workflow uses sequential per-role worktrees and
  needs creation only. An upstream attach option would let roles share one
  worktree again; it is to be requested from T3, alongside pingdotgg/t3code#31621.

## See Also

- [sdlc/principle/design-fundamentals](../../../../domain/sdlc/principle/design-fundamentals.md)
- [general/principle/trust-and-guardrails](../../../../general/principle/trust-and-guardrails.md)
- [general/principle/intelligence-boundaries](../../../../general/principle/intelligence-boundaries.md)
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
- [project/instrukt/spec/feature/work/todos](../work/todos.md)
- [project/instrukt/spec/feature/work/stamps](../work/stamps.md)
