---
description: Evidence-bound workflow facts with accountable actors and model, version and evaluation provenance.
visibility: public
---
# Stamps — Spec

## What it is

A stamp records a workflow fact against the evidence that earned it. It makes
reviews, approvals and handoffs inspectable without turning the displayed
lifecycle position into an independently editable source of truth.

## Canonical fields

### Evidence and attribution

Every stamp carries its fact, timestamp, acting session or approving person,
target evidence, model identity, model version and eval fingerprint. A stamp's
actor and its model provenance are distinct. A GitHub approval retains the
identity GitHub reports; a human approval stated in a session is attributed
to that session’s runtime and id and recorded by its agent. No identity is
guessed. An agent session's model and version come from the active runtime adapter's
read of that session's state (on T3, the provider and model `t3 thread show`
reports for the caller's thread); when the adapter reports none, or the read
fails, the stamp records the model as explicitly unknown, never a guess. Outside an agent session, model
and version are explicitly absent and the actor is named. Every stamp copies the current published eval
fingerprint at write time.

An artifact-bound stamp identifies the reviewed or approved content digest.
A commit-bound stamp identifies the reviewed or handed-off commit. A review
also binds its complete report's digest, verdict, review phase and round.
Required reasons are retained with skip, hold, failure and re-derivation facts.
Project, todo, phase, role, review findings and human intervention remain
associated with the fact they describe, so evaluation can derive outcomes from
the work record.

A stamp counts only for the evidence it names. Rewriting an artifact does not
transfer approval to its replacement. Rebuilding after a review does not turn
the earlier review into a verdict on a different commit. Earlier decisions
remain available as history when current evidence changes.

### Preparation head review

`--verdict approve|needs_work --phase preparation_head` binds the current clean preparation-worktree head and `preparation-review-findings`. It releases the independent `commit-reviewer` attempt atomically. Approval appends `workflow.preparation_reviewed` in the same transaction for immediate integrator attention. A later head or report invalidates that review. Preparation writers cannot review their own head.

### Mutation boundary

`instrukt todo stamp <slug>` records an allowed fact. The whole resulting
record is validated before its transaction in the state database commits;
the stamp, the todo's new revision and its pending reflection commit
together. An unknown stamp,
missing target, unresolved required evidence, missing required attribution or
invalid resulting state is refused without a partial local stamp.

| Fact family | Bound evidence |
| --- | --- |
| Review verdict | Current target and complete review report. |
| Approval | Current reviewed artifact, source attribution, and whether a human or peer approved. |
| Intent-checked | Intent version, originating conversation and checking session. |
| Kickback | Affected evidence and recorded reason. |
| Task-firing started | Particular scheduled occurrence and firing start. |
| Architecture skip | Applicable work item and recorded reason. |
| Drift judgment | Affected artifact and the evidence examined. |
| Settled diagnosis | Current diagnosis content. |
| Build start | Original base commit and current approved plan digest. |
| Build completion | Current build evidence and clean shared todo-branch head; completion releases the attempt atomically. Any later todo-branch change clears completion until restamped. |
| Role completion | Setup/finalizer or preparation doc-writer role and attempt, exact clean branch head and required artifact/commit references; releases the attempt atomically, independently of build completion or readiness. |
| Deferrals processed | Dispositions covering the current deferrals artifact; `deferrals.pending` is false only while this binding is current. |
| Finalize readiness | Candidate commit handed to integration. |
| Task-firing outcome | Particular execution and its result or failure reason. |

The command retains the verdict and readiness forms, including
`--kickback --reason` and `--task-firing-started`, and adds `--intent-checked`.
Approval forms record whether a human or a peer gave the approval.

The workflow engine derives its next state from these facts. The stamp
surface retains the fact; it does not independently perform the transition
or repeat runtime side effects. The finalize-ready write also records its handoff
and idempotent event-log append as specified above. Workflow owns phase-specific admissibility.

Approval attribution and intent-check binding are owned by
`project/instrukt/spec/feature/work/approvals`. Reflection and replay of
saved evidence are owned by
`project/instrukt/spec/feature/work/github-record`.

Task execution records its result with
`instrukt todo stamp <slug> --task-outcome success|error [--reason <text>]`,
bound to the particular task execution; the reason records failure evidence.

`instrukt todo stamp <slug> --deferrals-processed` records the disposition
bound to the current deferrals artifact. `deferrals.pending` becomes false
only while that binding covers the current artifact. For a bound deferrer
attempt, the stamp requires its clean worktree at the captured todo head and
releases the attempt in the same transaction without advancing a branch.

### Decision attention and completion evidence

An applied approval or request-changes decision retains pending attention for
the todo's current live claim-holding orchestrator with the as-soon-as-possible
intent. With no live orchestrator it is only a record update for next pickup or
a deliberate start. The recording transaction precedes runtime delivery;
session-continuity owns retry and claim revalidation. Replaying the same
applied decision creates no second notice. This applies to CLI recording and
GitHub intake alike.

Stamps, verdicts and changes requested are tracking and evaluation evidence;
they are not published events to drive worker dispatch. A worker's ending
reaches its parent through native runtime delivery, and that parent
reads the stamps before deciding what follows. The exact-candidate
`workflow.finalized` handoff remains the existing integrator event, not a second
worker-ending or approval notice.

### Stamp forms

| Command suffix after `instrukt todo stamp <slug>` | Bound evidence and result |
| --- | --- |
| `--verdict approve\|needs_work --phase prd_review\|architecture_review\|plan_review` | Current phase target and complete phase-specific findings digest. |
| `--verdict approve\|changes_requested --phase build_review` | Reviewed commit, complete build findings digest and review round. |
| `--architecture-skipped <reason>` | Applicable preparation and the recorded skip reason. |
| `--diagnosis-settled` | Current diagnosis digest. |
| `--diagnosis-unresolved --reason <text>` | Current diagnosis and the unresolved reason; establishes the dispatch hold. |
| `--drift-reviewed <artifact> cosmetic\|substantive <reason>` | Artifact, classification, reason and examined evidence anchor. |
| `--intent-checked` | Current input version, originating conversation and checking session. |
| `--kickback --reason <text>` | Current input, invalidated plan and review evidence; sets `<r>.cleared_by_substantive_drift` for preparation reviews invalidated by the kickback. |
| `--task-firing-started` | Particular due occurrence and start; the runner's last-fired accounting. |
| `--build-complete` | Bound builder/fixer attempt's clean committed todo head; releases the attempt and clears the previous build-review verdict and finalize readiness in one transaction. |
| `--role-complete` | Bound setup/finalizer or preparation doc-writer attempt, required outputs and clean committed branch head. Records `role_complete` and releases the attempt atomically without build completion or finalization event. |

Approval forms with `--by` and optional `--session` are owned by the approvals
spec; build start, finalize readiness, deferral processing and task outcome
forms are specified here. These forms are the admitted stamp set, rather than
instructions to infer additional stamps.

### Build start, completion and review rounds

`instrukt todo stamp <slug> --build-started` records the original base commit
and approved plan digest before implementation. Reconciliation preserves that base.

Completion resolves the caller's bound outstanding attempt and verifies the
shared worktree is clean on the todo branch at the exact stamped head.
`--build-complete` releases builder/fixer attempts; `--role-complete` releases
setup/finalizer attempts. Stamp, release, new todo revision and pending reflection
commit in one transaction. Exact replay reports the retained outcome without a
second release. Missing or stale evidence keeps the attempt outstanding. These
commands change no Git ref: each role commits directly on the todo branch.

A build-review verdict binds the current shared worktree head and complete
findings digest and releases its reviewer in the same transaction. That tree
must remain clean at the head captured for review. Deferral processing likewise
releases its bound read-only attempt only against current dispositions. New
workspace work after release requires a new attempt with shared isolation.
Preparation doc writers (`intaker`, `architect`, `planner`) also record
`--role-complete` against the required artifact digests and authored commit SHAs
at the clean todo-worktree head. It releases that writer so the parent can dispatch independent
review. Preparation verdict stamps release their corresponding reviewer attempts;
commit reviews retain the exact SHA and report-note binding. Missing outputs keep
the writer outstanding. Commit review remains owed after writer release.

Review rounds count verdict stamps; repeated evaluation counts no round.

The finalizer's `--role-complete` requires a clean shared worktree whose head,
local todo branch and published todo head match. It may complete after changing
the earlier reviewed candidate. Release permits independent review of that
current head in the same worktree; readiness still waits for current approval.
A failed attempt records only failure under the todo-attempt contract, never
completion or readiness.

`instrukt todo stamp <slug> --finalize-ready` requires current independent
approval, no outstanding workspace-role attempt, the latest successful finalizer
completion and equality of its clean shared worktree head, local todo branch,
published todo branch and completed candidate. Readiness uses the caller's
retained finalizer binding after release and admits no further workspace work.
New changes require a new finalizer attempt and its completion.

The readiness stamp and `workflow.finalized` event append commit together,
idempotently by project/todo/SHA. It records the exact candidate for integration
and does not release the execution claim.

## Use cases

Stamp capture supports the approval, GitHub-record and evaluation-report goals.
Shared completion and release are exercised by UC-DC2, UC-DC4, UC-DC5, UC-DC6
and UC-DC7 in `project/instrukt/spec/feature/workflow/delivery-cycle`.

## Known caveats

- Role completion alone establishes neither current independent approval nor
  finalize readiness; each binds its own current evidence.

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
- [project/instrukt/spec/feature/workflow/delivery-cycle](../workflow/delivery-cycle.md)
- [project/instrukt/spec/feature/work/state-database](state-database.md)
- [project/instrukt/spec/feature/work/todos](todos.md)
