---
eval: true
description: 'Orchestration loop for prose-routed lifecycle primitives and worker session handling.'
visibility: 'public'
---

# Orchestration — Procedure

## Required reads

- [general/procedure/ai-to-ai-operations](ai-to-ai-operations.md)

## Goal

Drive a todo's prose-routed lifecycle primitives from the factual ledger, manage
worker sessions and recovery, and keep ownership clear until a grounded handoff.

### Dispatch Rules

- **Classify the review's scope for selection, never inside the brief.** Code,
  mixed code and docs, directive documentation and other substantive reviews
  select for full depth; documentation that changes what a snippet establishes
  and directs no agent differently selects for the lighter scope under
  `general/policy/documentation-discipline`. Select through the reviewer's role
  mapping in `~/.instrukt/agents.yaml` (with any per-task agent/tier override
  taking precedence first); shipped `strong` and `standard` choices are
  configurable heuristics, not procedure-level model or tier defaults. Pass
  coordinates and optional widening source pointers only in every delegated
  review assignment (`prd-reviewer`, `architecture-reviewer`, `plan-reviewer`,
  `build-reviewer`, and `commit-reviewer`) — never carry classifications,
  narrative summaries, or asserted constraints into a review brief
  (`sdlc/procedure/lifecycle/review`). For todo reviews, those coordinates are
  the slug, review phase, target (`artifact@digest`, or exact candidate `head`
  and `base` for builds), report artifact, on re-review or recovery the prior
  findings digest and any missing obligation, and optional widening pointers
  (including `log` when carrying an orchestrator challenge); commit-review
  coordinates live in `sdlc/procedure/lifecycle/work/review-commits`. Producer
  briefs (such as `builder` or `planner`) remain free to carry orchestration
  direction.

Intelligence maintains the preference order for performance and then cost;
availability is the first selection condition. The parent or orchestrator uses
the strongest available entry, with the next as fallback. Valid agent profile
names come from `~/.instrukt/agents.yaml`, whose role mappings and typed chains
own model choice independently of procedure content. A proven rejection before
native session creation (`instrukt todo attempt --not-started`) permits fallback
to the next available entry in the configured order; an ambiguous native start
retains its attempt identity for reconciliation rather than starting a
duplicate session.

### Review Quality Gate (Orchestrator-Owned)

The workflow boundary refuses every preparation or build verdict unless the
phase-specific report exists and binds its current digest together with the
current reviewed target. The orchestrator verifies the report's substance:

1. Open the current phase-specific report and confirm every required review
   lane or preparation-review section is present with substantive analysis.
2. When the review reports zero Important-or-higher findings, verify the
   required `Why No Issues` evidence is specific to the target.
3. If either check fails, reject the review and re-dispatch with the reviewer
   role's configured selection, passing only coordinates naming the prior
   report digest and the missing sections or lanes
   (`sdlc/procedure/lifecycle/review`).

This is a document-quality check, not a second code or artifact review. Digest
binding proves which report exists; the orchestrator verifies that the reviewer
actually did the work recorded there.

### Orchestration Loop

1. Call `instrukt todo work {slug}` (or `instrukt todo show {slug} --json`) to
   claim the active todo and inspect its current factual ledger. Orchestration
   is slug-scoped; future pickup belongs to the runner.
2. Read the factual ledger: artifacts, digests, stamps, claims, bindings, git
   state, open findings, and gate status. Code enforces only the three hard
   integrity invariants; choose the next composable primitive from the prose
   playbook in `sdlc/procedure/lifecycle/overview`.
3. Reserve every new round through `instrukt todo attempt {slug} --role <role> --procedure <id> --args {slug} --brief -`,
   preserving its rendered brief, role, attempt, semantic key, display title,
   captured shared head, and any reported `prior_session`. Then route the round
   under **Continuing vs. Starting Fresh** below through the configured runtime
   operations in `general/procedure/ai-to-ai-operations`. Yield while valid work
   continues; read the native result and current factual ledger before choosing
   the next move.
4. Assess current session and work evidence. A step is complete only when the
   record carries its required artifact and stamp with current target bindings.
   A reply or ended run alone supplies neither.
5. Re-read `instrukt todo show {slug}` after every material outcome or recovery.
   At handoff, stamp `--finalize-ready` and release the claim with
   `instrukt todo update {slug} --unclaim`.

A todo shares its unified `todo-orchestrator`'s single `<slug>/work` worktree
and branch. Completion stamps release workspace attempts atomically before the
next role is admitted. The runtime adapter supplies capabilities rather than
workflow policy; the record supplies facts rather than a stored phase machine.

#### Continuing vs. Starting Fresh

- **Reserve every new round first:** Start every new round with
  `instrukt todo attempt`, which reports any retained `prior_session` and the
  resolved model selection for that role.
- **Continue (`attempt` → `bind` → configured child-messaging instruction):**
  Record `instrukt todo bind` for the new attempt against `prior_session` and
  the recorded shared placement **before** sending the new brief through the
  configured child-messaging instruction. A round is continued, never started
  fresh, whenever `prior_session` is present, reachable on the same runtime,
  matches the resolved selection, and can be continued by the current
  orchestrator with native result delivery back to this orchestrator (not bound
  to a former claim holder), and the round is one of
  (`general/principle/session-lifecycle`):
  - A follow-up round of a producer (`intaker`, `architect`, `planner`,
    `builder`, `fixer`, `deferrer`, `finalizer`).
  - A finding-closure re-check of a reviewer (`prd-reviewer`,
    `architecture-reviewer`, `plan-reviewer`, `build-reviewer`) where **all**
    target changes address only that reviewer's findings (no unrelated changes,
    substantive scope rewrite, or design rewrite) and the review loop is not
    deadlocked. Its brief carries the re-review coordinates named under
    Dispatch Rules.
- **Start fresh (`attempt` → configured delegation instruction → `bind`):**
  Every other admitted new round starts fresh — delegate through the configured
  delegation instruction and immediately record `instrukt todo bind` with the
  new native child id and any recorded placement. This is the default for
  attempt 1, roles outside the continuation lists above (such as `diagnoser` or
  `drift-analyzer`), a confirmed unusable or former-orchestrator `prior_session`
  (chosen before binding), a changed selection, any reviewer target change that
  includes unrelated edits or a substantive scope or design rewrite, and a
  deadlocked review loop.
- **Mid-attempt recovery:** While an attempt is still active (such as a missing
  stamp or bounded recovery on a stopped turn), continue directly on that active
  attempt through the configured child-messaging instruction without reserving a
  new attempt.

### Terminal Child Cleanup

At handoff, execute `instrukt todo update <slug> --unclaim` and confirm its
outcome. The next owner waits on claim release, not on the end of this
session. An unfinished human conversation retains the session, never the
claim. A refused release remains unresolved.

Between rounds, retain available workers for follow-up through their recorded
session ids. Once the outcome is recorded and no later round or human
conversation needs the child, record the contribution and handoff. Close
discussion through the configured runtime discussion instruction. When handing
off a finalized todo to `integrate` (`--finalize-ready` and `--unclaim`) or
once a todo reaches a terminal state (`delivered` or `withdrawn`), finish its
recorded orchestrator threads through the configured thread-finishing
instruction under `sdlc/policy/todo-creation-discipline` (finished resting
orchestrators remain wakeable in place):

<!-- @config: runtime.finish_thread -->

Apart from finishing completed todo orchestrator threads, session ending and
cleanup belong to the runtime. Preserve any unresolved cleanup with its
responsible owner.

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
4. Raise what no session of this todo can settle, instead of waiting on it or
   asking it in a thread no person reads: a worker's question this
   orchestrator cannot answer, work of another live todo that converges,
   overlaps or conflicts with this one, a solution existing outside the
   project that already provides what this todo builds, or a decision owned by
   a person who is not reached in this todo's threads, its approval gates
   aside. Run
   `instrukt events raise <todo>... --obligation <text>` for the active todos
   it concerns, naming the obligation at stake and no conversation content.
   The project's coordinator returns it to the work's origin
   (`general/principle/origin-routing`); keep working what the condition does
   not touch, and follow the coordinator's pause when one arrives.
5. An unattended failure is an anomaly. The runner decides notification;
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
retains its identity and pending work for reconciliation; apart from finishing
completed todo orchestrator threads, session ending and cleanup belong to the
runtime. If
recovery cannot complete, preserve the pending step and report the anomaly.

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

- A target todo and configured workflow flavor (`workflow.yaml`) are available.
- The factual ledger (`instrukt todo show <slug> --json`) and runtime can supply
  the evidence needed for the next step.

## Steps

1. Read the factual ledger and select the next composable primitive from `sdlc/procedure/lifecycle/overview`.
2. Observe workers through configured runtime operations and inspect their record-backed outputs.
3. Route recovery to the owning role and re-read the factual ledger after material outcomes.
4. Confirm terminal handoff (`--finalize-ready`) and claim release (`--unclaim`); retain other live obligations.

Confirm `claim.held` is false after `instrukt todo update <slug> --unclaim`,
then record the handoff and any remaining obligations. Preserve conversations
needed for later rounds or human input; apart from finishing completed todo
orchestrator threads, the runtime owns session ending and cleanup.

## Outputs

- Record-backed progress, with command failures and next ownership explicit.
- Current independent exact-SHA review evidence before finalization.

## Recovery

- Apply Agent Degradation Handling to startup or session failures and When
  Worker Needs Help to an executing worker's obstruction.
- A failed git operation, unavailable observation, or runtime-reported missing
  capability remains unresolved; no prose token or elapsed-time assumption
  discharges it.

## See Also

- [general/procedure/ai-to-ai-operations](ai-to-ai-operations.md)
- [general/policy/orchestrator-jurisdiction](../policy/orchestrator-jurisdiction.md)
