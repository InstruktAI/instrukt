---
description: Execute an authorized task, preserve continuity and outputs, and record each firing outcome on the task record. Non-SDLC; no commit.
role: task-runner
parameters:
  - name: slug
    required: true
dispatch:
  agent: [codex, claude, gemini, deepseek]
  thinking_mode: med
---

# Run Task — Procedure

## Required reads

@~/.instrukt/docs/general/principle/continuity.md
@~/.instrukt/docs/sdlc/spec/todo-kind-task.md

## Goal

Execute an authorized `kind=task` todo against its durable `task-plan`, carry
continuity through `log`, deliver filesystem outputs to the predictable
workspace and record an outcome for each firing on the task record. This is
non-SDLC work: no build, review, finalize or repository commit.

## Preconditions

- The todo is `kind=task`, prepared, authorized and not held.
- The runner dispatched an eligible occurrence under `runner.autostart`
  (boolean or per-kind map), or work was deliberately dispatched. Due time
  does not replace authorization or pickup policy.
- The plan passed its configured approval moment against the checked `input`;
  task creation alone authorizes nothing.
- `task-plan` exists. `log` may already carry prior firing evidence.
- The recorded session binding and current firing are available. The runner
  does not fire an in-flight occurrence again; a failed dispatch is not fired.

## Steps

1. **Stamp the start, then read the evidence.** Record
   `instrukt todo stamp {slug} --task-firing-started --occurrence <occurrence>`
   for this occurrence, with the `occurrence:` value from the opening message.
   Preserve any failure to record it; no failed stamp is completion evidence.
   Read the task record and `log` for prior outcomes and continuity. A completed
   occurrence is not repeated. A resumed session or a fresh execution does not
   manufacture another scheduled instant.
2. **Read the plan.** Read `task-plan` and the checked `input`. The approved
   plan governs what to produce, operations, result handoff, timing and log.
   Approval may be human or peer under the configured preparation gate; it is
   not inferred from an agent's suggestion or from the schedule.
3. **Execute the firing, unbounded in method.** Do what the plan describes with
   available capabilities: query data, use services, research or generate
   content. The plan bounds the outcome; this procedure supplies no fixed menu.
   Repository implementation work remains outside the task kind.
4. **Hand off as the plan directs.** If the plan says to report results to a
   person, channel or external recipient, do it through the appropriate
   available capability. If the plan is silent on reporting, send nothing outward.

   **Deliver to the workspace.** Filesystem outputs go under
   `{project_root}/workspace/{slug}/`, reused across firings. Organize within
   it as the plan requires, including dated subfolders for distinct snapshots.
   Record the output references on the task.
5. **Write the durable record.** Append a concise dated `log` entry: what the
   firing did, output references, recoverable obstacles and the scheduled
   instant satisfied. Preserve platform-written firing entries verbatim,
   in place. The task record and outputs carry continuity across executions.
6. **Keep the log lean.** Roll older entries of your own into a summary when
   useful. Never roll up, rewrite, reorder or remove recorded firing lines.
7. **Record the firing outcome.** Run
   `instrukt todo stamp <slug> --task-outcome success|error --occurrence <occurrence> [--reason <text>]`
   to write the outcome stamp on the task record,
   bound to this execution: `success` when the authorized intent is achieved,
   `error` with its reason when it is blocked. Success delivers a one-shot task;
   success on a recurring task records this occurrence and leaves the task live.
   An error holds the task with its reason and leaves its unresolved evidence
   in this session. A recoverable obstacle worked around is not an error.
   A turn without the required record evidence is incomplete, not success.
8. **Resolve, then rest.** After success, end the current execution; the runner
   owns the next due occurrence and recorded-session reuse under its continuity
   contract. A held task does not refire. Resolve an error with its responsible
   agent owner; if missing intent or a human-owned decision remains, ask here
   and leave the thread waiting as `needs-input`. A failed task with no waiting
   thread is an anomaly. The runner may notify only a reference to this session,
   never its content. No task runner invokes a separate notification command.

## Outputs

- Updated `log` and filesystem deliverables under
  `{project_root}/workspace/{slug}/`.
- A task-record outcome for the firing: success delivers a one-shot or records
  a recurring occurrence; error retains an explicit hold and reason.
- A waiting session for any human-owned question, or an anomaly reference for
  an unattended failure, subject to runner notification policy.

## Recovery

- **Plan absent or unreadable:** run
  `instrukt todo stamp <slug> --task-outcome error --reason <text>` with the
  blocker; do not
  improvise a task. Preserve the session for recovery.
- **A capability fails:** recover within the plan where possible. If the intent
  cannot be achieved, run
  `instrukt todo stamp <slug> --task-outcome error --reason <text>` rather
  than fabricating success. Technical
  failure alone does not make a human its owner.
- **Outcome write fails:** retain the execution and evidence. An unrecorded
  outcome establishes no completion. The runner's one-send recovery for missing
  evidence applies; continued absence is an anomaly.
- **Handoff ambiguity:** send nothing outward beyond what the plan sanctions.
  Record the result and its references; ask for genuinely
  missing intent in this thread. No notification substitutes for that question.

## Discipline

Read the record, execute the approved intent per occurrence and record its
outcome. Do not mistake a prior conversation for remembered execution, a start
stamp for completion, or a recurrence for permission to repeat an in-flight
firing. Keep repository work outside tasks, keep deliverables in their workspace
and keep ordinary human interaction in the session thread. Never send anything
outward the plan did not sanction.
