---
eval: true
description: 'Orchestration loop for the derived lifecycle obligations and worker session handling.'
visibility: 'public'
---

# Orchestration — Procedure

## Required reads

- [sdlc/procedure/lifecycle/overview](../../domain/sdlc/procedure/lifecycle/overview.md)
- [general/procedure/ai-to-ai-operations](ai-to-ai-operations.md)

## Goal

Drive the configured preparation or work flow, execute its actions and owed
effects, and keep recovery with its responsible owner until a grounded handoff.

### Dispatch Rules

- **Reviewer tier follows the review's scope.**
  - `strong` for commit reviews containing code, including mixed code-and-doc
    scope; for a documentation-only scope that changes what agents are directed
    to do, the process layer included; and for other reviews.
  - `standard` for a documentation-only scope that changes what a snippet
    establishes and directs no agent differently.

  The session dispatching a documentation-only review classifies its scope by
  what the change does (`general/policy/documentation-discipline`). Context
  procedures state the resolved mode for their fixed scope; they do not choose a
  different mode.

Intelligence maintains the preference order for performance and then cost;
availability is the first selection condition. The parent or orchestrator
uses the strongest available entry, with the next as fallback. Valid agent
names come from the configured runtime rather than a fixed catalog.

### Review Quality Gate (Orchestrator-Owned)

The workflow boundary refuses every preparation or build verdict unless the
phase-specific report exists and binds its current digest together with the
current reviewed target. The orchestrator verifies the report's substance:

1. Open the current phase-specific report and confirm every required review
   lane or preparation-review section is present with substantive analysis.
2. When the review reports zero Important-or-higher findings, verify the
   required `Why No Issues` evidence is specific to the target.
3. If either check fails, reject the review and re-dispatch at the mode the
   reviewer thinking-mode rule resolves for the scope.

This is a document-quality check, not a second code or artifact review. Digest
binding proves which report exists; the orchestrator verifies that the reviewer
actually did the work recorded there.

### Orchestration Loop

1. Call `instrukt todo prepare {slug}` or `instrukt todo work {slug}` for the
   current target. Work orchestration is slug-scoped; future pickup belongs
   to the runner.
2. Read the derived state, transition, action, rendered template and owed
   effects. The engine evaluates facts; it stores no lifecycle phase and owns
   no session or worktree mutation.
3. Execute the configured commands in their listed order. Effects are
   idempotent. An effect remains owed while its optional `done_when` condition
   is absent or does not hold; the next call re-derives what is still owed.
   Record failures and their recovery rather than treating a silent skip as success.
4. Reserve a new worker through the selected `instrukt todo attempt` action.
   Use its rendered brief, role, attempt, semantic key, display title and captured shared head with the
   configured runtime delegation instruction under
   `general/procedure/ai-to-ai-operations`; immediately record `todo bind` with
   the native child id and matching shared workspace evidence. Continue active attempts
   with native child messaging. The runtime supplies parent linkage and result
   attention. Yield while valid work continues; read the native result and current
   record before choosing the next contribution. Selection follows procedure
   `dispatch` or `agents.order` where natively supported. A proven rejection
   before creation permits fallback; ambiguous native starts retain their attempt.

5. Assess current session and work evidence under the sections below. A step
   is complete only when the record carries its required artifact and stamp,
   with current target bindings. A reply or ended run alone supplies neither.
6. Re-enter the flow after a material outcome or recovery. Terminal handoff
   closes this assignment; remaining session obligations follow
   `general/principle/session-lifecycle`.

The orchestrator routes workspace preparation, upstream merges and publication
to the admitted shared child. It coordinates stale evidence, findings relay,
kickback and lane release through configured record commands. A work team shares
its orchestrator's one todo worktree and branch; preparation children share the
preparation orchestrator's `<slug>/prepare` worktree. Completion stamps release workspace attempts
atomically before the next role is admitted. The runtime adapter supplies capabilities rather than workflow policy.
The record supplies facts rather than a second stored phase.

### Terminal Child Cleanup

At terminal handoff, execute `instrukt todo update <slug> --unclaim` and the shipped
flavor's cleanup commands
and confirm their outcomes. The next owner waits on claim release, not on the
end of this session. An unfinished human conversation retains the session,
never the claim. A refused release remains unresolved.

Between rounds, retain available workers for follow-up through their recorded
session ids. Once the outcome is recorded and no later round or human
conversation needs the child, record the contribution and handoff. Close
discussion through the configured runtime discussion instruction.
Session ending and cleanup belong to the runtime. Preserve any unresolved
cleanup with its responsible owner.

### Container Slug Handling

- Treat holders as child-resolution entrypoints, never executable work.
- Resolve direct children by recorded group membership, never slug prefixes
  or prerequisites. Holder input is frozen seeding context, not a plan.
- Preparation considers eligible direct children needing preparation; work
  considers dependency-satisfied children prepared or already in progress.
- Each child uses its own evidence and claim. The runner owns later pickup.

### Conversation and Status Discipline

Read the recorded conversation through the configured runtime read instruction
when the observation needs it. Keep evidence already read distinct from new
input, without assuming incremental-tail options. Silence establishes only
that no new conversation evidence was observed.

Check actions against the brief even while work is running. If a worker acts
outside its lane, direct it back to the governing boundary and record the
departure in the todo `log`. A status or a worker's justification widens no brief.

| Observed state | Orchestrator response |
| --- | --- |
| `running` | Preserve the session and pending assignment; assess any concrete obstruction. |
| `needs-input` | Read the question in its thread and classify the required input. |
| `ended ok` | Check the artifact and stamp; a successful run is not completion by itself. |
| `ended failed` | Read the failure and record evidence; keep the step pending unless its evidence establishes completion. |
| `gone` | Keep the step pending, inspect confirmed absence and select recovery under the retained attempt. |

A failed native status observation remains unknown; the governing parent
retains its unresolved attempt. For a child ending without required evidence, read the
reported closing message and current record, then send the bounded recovery
to its retained session when it can continue. Preserve an unavailable attempt
and route its grounded failure under the recovery contract. The runtime delivers
child endings to this parent; this parent owns the recovery decision.

### When Attention Arrives

1. Read the native result's child id, then its role/attempt binding and report.
   Keep finished-turn, failed-turn and input-wait accounts distinct; a missing
   message is an evidence gap. Read later human input before using the observation. Apply current steering
   and preserve any explicit boundary; stale dispatch assumptions supply no authority.
2. Read native evidence, the session conversation and work artifacts under
   `general/procedure/ai-to-ai-operations`. Distinguish reported-time evidence
   from current evidence. An unchanged conversation does not dismiss a failure.
3. For an obstruction, use When Worker Needs Help. For apparent completion,
   verify the required report and stamp before re-deriving the next action.
4. Yield while valid work continues. Native results restore parent attention;
   this procedure adds no timer or terminal polling loop.
5. Send substantive next work or recovery, never acknowledgement-only guidance.

### When Worker Needs Help

1. Classify the remaining decision under `general/policy/human-contract`.
   Technical recovery goes to its agent owner with evidence and governing
   addresses. Do not author the worker's code or tests.
2. Record the next contribution and its owner. A plan defect returns through
   the work kickback path; an unresolved technical question returns to evidence.
3. The responsible session asks a human-owned question in its own thread.
   A child's native input-wait result reaches its parent for assessment; read that
   question and apply the existing mandate. A recorded approval or request-changes
   decision reaches the live claim-holding orchestrator as direct attention;
   re-derive the obligation from the updated record.
4. An unattended failure is an anomaly. The runner decides notification;
   only a reference to the anomaly session or dispatch hold may be delivered.

### Agent Degradation Handling (Orchestrator-Owned)

A worker that produced no required artifact and stamp has not completed its
step. Read the failure evidence before selecting recovery. A failed send proves
only a delivery failure; silence proves neither failed startup nor an unusable
session. Preserve the recorded session and any work already produced.

Inspect through configured runtime status/conversation reads and work records.
Recover an executing worker within its brief through native child messaging. Availability selection belongs
to the runtime checks and ordered dispatch contract; it supplies no manual
provider-status mutation, restart or quota timer. Distinguish a start failure
from a failed observation and from execution that began and remains incomplete.
When evidence establishes rejection before any session was created, retry
through the native runtime's supported availability selection. An ambiguous start
retains its identity and pending work for reconciliation; session ending and
cleanup belong to the runtime. If recovery cannot complete, preserve the pending
step and report the anomaly.

### Review Threshold Handling (Orchestrator-Owned)

When a preparation or work review reaches its documented threshold, the count
is reported once and nothing is refused. Continuation stays with this
orchestrator: it requests another pass only when it can establish what remains
unresolved and why the next act resolves it within the frozen scope
(`sdlc/procedure/lifecycle/review`). The reviewer retains its verdict, the
producer retains its finding dispositions, and this orchestrator retains
lifecycle execution and state mutation.
`general/policy/human-contract` governs any escalation.

1. Inspect current evidence:
   - `instrukt todo artifacts read {slug} review-findings`
   - Fix commits since `review_baseline_commit`
2. Decide and act:
   - If any unresolved **Requirement-Violation**, **Critical**, or **Important**
     finding remains, keep `review=changes_requested`. Record the blocker summary
     in the phase-specific review artifact. Preserve the required sessions
     and name the next contribution's agent owner. No approve is stamped
     while a finding remains unresolved.
   - If every finding is resolved, have the responsible reviewer record the approving verdict. The verdict is
     arithmetic: zero unresolved findings means APPROVE.
   - If the loop is stuck or its remediation has become disproportionate to the
     concern, stop continuation, record the grounded cause, and route the next
     contribution under the shared review procedure. A technical recovery,
     return to an owning phase, or further bounded review does not require
     human approval. A decision in the closed human lane does.
3. Record rationale in todo artifacts so the closure is auditable.

The orchestrator never stamps an outcome on a worker's behalf. The responsible
worker writes the complete phase-specific report and stamps its own verdict.
If the stamp is missing, read the child's ending and current evidence, then
return that obligation to the retained worker through a bounded follow-up.
Preserve unresolved absence through the recovery path.

The orchestrator must not rebrand Requirement-Violation, Critical, or Important
findings as "non-blocking", "follow-up", "suggestion", or any other reduced
category to reach approve. Per the review procedure severity schema, such
findings block regardless of round count. The schema has three tiers and every
tier blocks the verdict — there is no fourth tier and no deferral path.

## Preconditions

- A target todo and configured flow are available.
- The record and runtime can supply the evidence needed for the next obligation.

## Steps

1. Derive the current obligation and execute its configured action and effects.
2. Observe workers through configured runtime operations and inspect their record-backed outputs.
3. Route recovery to the owning role and re-derive after material outcomes.
4. Confirm terminal handoff and claim release; retain other live obligations.

Confirm `claim.held` is false after `instrukt todo update <slug> --unclaim`,
then record the handoff and any remaining obligations. Preserve conversations
needed for later rounds or human input; the runtime owns session ending and
cleanup.

## Outputs

- Record-backed progress, with command failures and next ownership explicit.
- Current independent review evidence before finalization or preparation release.

## Recovery

- Apply Agent Degradation Handling to startup or session failures and When
  Worker Needs Help to an executing worker's obstruction.
- Execute owed effects through the shipped flavor's workflow bindings. A
  failed effect, unavailable observation or runtime-reported missing capability
  remains unresolved; no prose token or elapsed-time assumption discharges it.

## See Also

- [general/procedure/ai-to-ai-operations](ai-to-ai-operations.md)
- [general/policy/orchestrator-jurisdiction](../policy/orchestrator-jurisdiction.md)
