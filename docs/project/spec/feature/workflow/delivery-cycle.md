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
| Worktree creation or reattachment | Runtime through setup-only `instrukt sessions run --worktree <name> --brief -` | Create or reattach the todo worktree from the canonical branch or existing remote todo branch and return its path; wait without implementation instructions. |
| Merge of upstream canonical branch | Orchestrator merge command before initial build-start evidence | Expose skipped merges, fetch failures and conflicts. On conflict, abort the merge and route recovery to the live builder's own procedure. An absent builder is an explicit dispatch gap. |
| Opening commit and project preparation | Orchestrator after upstream synchronization | Make the opening commit, then run the project’s `prepare` action when declared, before build-start evidence. |
| First-entry branch publication | Orchestrator branch-publication command | Publish with lease protection; report the outcome rather than assuming publication. |
| Build-start evidence | Record-writing command at builder admission | Capture base, plan and checklist without recording a workflow phase. |
| Build dispatch and plan reconciliation | Orchestrator action using configured session capabilities | Reuse the recorded live builder or start the required worker; present the current plan or reconciliation obligation. |
| Build completion | Builder completion-stamp command | Record completion, invalidate the previous build verdict and reset finalization. |
| Review dispatch and rounds | Orchestrator dispatch and verdict stamps | Count rounds from verdict stamps; no admission write. Re-evaluation or session reuse alone counts no round. |
| Verdict currency | Provider over target, findings and repository evidence | Supply current, stale or undetermined binding facts without executing a hidden transition. |
| Findings relay and fixes | Orchestrator action addressing the recorded live builder | Deliver findings for disposition and revision; expose an unavailable builder. |
| Deferrals | Orchestrator deferrals action | Obtain a disposition per deferred item and record processing before finalizer dispatch. |
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
preparation when declared, first branch publication, build-start record, quality-checklist presence, then builder dispatch. Plan reconciliation
re-records `--build-started` against the current plan before resuming the build.
Cleanup uses `worktree.exists` with `done_when: {not: {exists: worktree.exists}}`.

A checklist-only admission gap does not repeat synchronization after the
build-start stamp. First branch publication follows the opening commit and
project preparation. Once finalize readiness is recorded, confirmation and
handoff owe no merge or branch-publication effect.

### Session reuse

Worktree creation and attachment are distinct adapter capabilities. T3's
published `--worktree <base>` and `--branch <name>` create a new worktree; they
cannot attach a new role's thread to the setup thread's existing worktree. Reuse
that bound thread where the assignment permits it. A new role requiring the same
worktree remains pending with the capability gap reported; the adapter neither
repeats setup nor runs that role in the project root.

Every role dispatch in this cycle uses recorded-session reuse under
`project/instrukt/spec/feature/runner/session-continuity`.

### Finalization and integration handoff

Deferrals are settled before finalizer dispatch. Readiness requires a clean
worktree and equality of its head, the local todo branch, the remote branch
and the commit named by the finalize-ready stamp. A readiness mismatch returns
an explicit reconciliation and restamping obligation; it does not hand off a
candidate under an earlier stamp.

The sole workflow event is `workflow.finalized`, addressed to the standing
integrator for the todo and finalized commit. Re-reading unchanged readiness
does not create a new logical handoff. Integration is governed by
`project/instrukt/spec/feature/workflow/integration-gate`.

### Settled command bindings

- Runtime setup uses setup-only `instrukt sessions run --worktree <name> --brief -`; the binding
  exposes `worktree.path`. After creation and upstream synchronization, make
  the opening commit and run the
  project's `prepare` action when declared, before implementation.
- `instrukt todo stamp <slug> --build-started` captures the original base and
  approved plan digest; write the quality checklist separately before admission.
- Review rounds count verdict stamps; dispatch requires no admission write.
- `instrukt todo stamp <slug> --finalize-ready` records the handoff and appends
  `workflow.finalized` idempotently by project/todo/SHA.
- Release with `instrukt todo update <slug> --unclaim`; confirm `claim.held`
  is false and record the handoff. The runtime owns session ending and
  cleanup; preserve conversations still needed for review or human input.
- The integrator runs `instrukt docs fold <slug>` before candidate verification,
  then `instrukt todo deliver <slug> --commit <sha>` after confirmed promotion.
  Delivered worktree cleanup is `git worktree remove <path>` with `done_when`
  the worktree is absent. Returned preparation work retains its working resources.

### Use cases

#### UC-DC1: Deliver a prepared change for integration

```gherkin
Given a developer has a prepared change whose dependencies are delivered
When the developer has the change implemented through the project's workflow
Then an integration candidate is handed off only after current independent approval, deferral disposition and matching finalization evidence
And findings requiring changes return to implementation before that handoff
```

## Known caveats

- T3's host CLI creates new worktrees but cannot attach a new role to an
  existing one. Such a dispatch remains pending with that capability gap;
  the setup and multi-role delivery sequence is not fully supported by this
  adapter until the runtime supplies attachment.

## See Also

- [sdlc/principle/design-fundamentals](../../../../domain/sdlc/principle/design-fundamentals.md)
- [general/principle/trust-and-guardrails](../../../../general/principle/trust-and-guardrails.md)
- [general/principle/intelligence-boundaries](../../../../general/principle/intelligence-boundaries.md)
