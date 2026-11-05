---
description: Evidence-bound workflow facts with accountable actors and model, version and evaluation provenance.
visibility: public
delivered_by: [instrukt-first-build]
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
guessed. An agent session's model and version come from the active runtime's
optional `model` template, called with the session id, which prints
`<model> <version>`; without that template, or when it fails, the stamp records
the model as explicitly unknown, never a guess. Outside an agent session, model
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
| Build completion | Current build evidence and stamped role-branch head; branch handoff must reach that exact head before the next role is admitted. Any later todo-branch change clears completion until restamped. |
| Role completion | Setup or finalizer role, attempt, base and committed role-branch head; handoff evidence only, independent of build completion or finalize readiness. |
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
only while that binding covers the current artifact.

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
| `--build-complete` | Bound builder/fixer attempt's committed role head; clears the previous build-review verdict and finalize readiness. |
| `--role-complete` | Bound setup/finalizer attempt's committed role head, base and attempt. Records `role_complete` for fast-forward handoff without marking the build complete or emitting a finalization event. |

Approval forms with `--by` and optional `--session` are owned by the approvals
spec; build start, finalize readiness, deferral processing and task outcome
forms are specified here. These forms are the admitted stamp set, rather than
instructions to infer additional stamps.

### Build start, handoff and review rounds

`instrukt todo stamp <slug> --build-started` records the original base commit
and approved plan digest before builder admission; recovery preserves the original base.

Completion commands resolve the caller's current todo-role attempt, require a
clean worktree and bind its exact committed head. The completion stamp permits
the orchestrator's fast-forward handoff under
`project/instrukt/spec/feature/workflow/delivery-cycle`; it does not itself
move a Git ref. The expected promotion of that exact role head to the todo
branch preserves the completion binding. A different head requires fresh
evidence. A review verdict targets the captured todo head and never advances
a branch.

`instrukt todo stamp <slug> --finalize-ready` requires the finalizer's completed
handoff and equality of its clean worktree head, local todo branch and published
todo branch. It records the exact-commit handoff
and appends `workflow.finalized` through the runner's event-log interface idempotently, keyed by
project, todo and SHA, in the stamp's own transaction: the stamp is never
recorded without its event, nor the event without its stamp. It does not
release the execution claim.

Review rounds are counted from verdict stamps, including a new verdict in a
reused session. There is no review-occurrence admission write; repeated
evaluation alone counts no round.

### Use cases

Stamp capture and validation support the goals in the approval, GitHub-record
and evaluation-report specs. They introduce no separate stakeholder goal.

## Known caveats

- Role-completion evidence does not establish a successful Git handoff. The
  retained attempt's confirmed advance establishes that handoff under
  `project/instrukt/spec/feature/workflow/delivery-cycle`.

## See Also

- project/instrukt/spec/domain/work
- project/instrukt/spec/feature/workflow/delivery-cycle
- project/instrukt/spec/feature/work/state-database
