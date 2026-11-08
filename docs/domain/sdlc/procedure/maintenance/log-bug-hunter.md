---
description: 'Proactive log-scanning task that discovers a project''s runtime errors and files a minimal bug report for each untracked signal.'
visibility: 'public'
role: bug-hunter
---

# Log Bug Hunter — Procedure

## Required reads

- [sdlc/concept/defect-evidence](../../concept/defect-evidence.md)
- [sdlc/policy/todo-creation-discipline](../../policy/todo-creation-discipline.md)
- [sdlc/spec/todo-kind-task](../../spec/todo-kind-task.md)

## Goal

Scan the current project's logs, reject warnings and exclude errors attributable to
congestion, a computer restart, or an upstream dependency's own failure. File only errors
whose records support a failure in the scanned project's own handling through
the todos module as a todo of kind `bug`. The bug lifecycle owns diagnosis and implementation.

## Preconditions

- This procedure is the work of an authorized, recurring `task` todo
  (`sdlc/spec/todo-kind-task`): the runner dispatches it in the target project
  when the task is due, and its `task-plan` names this procedure. The task's
  slug is its identity.
- The project's log name and the task's schedule are available.
- A project log-reading capability, `instrukt todo list`, `instrukt todo artifacts read`,
  `instrukt docs index`, and `git` are available.
- This procedure owns the checkpoint schema under Outputs; the previous
  firing's checkpoint is read when resuming.

## Steps

Each firing writes its replacement checkpoint to `{project_root}/workspace/{slug}/checkpoint.json` and records that output reference in the task's `log` under `run-task`. The outcome stamp records success or error separately. Retained known-issue
suppressions are consulted before filing; the scan checkpoint is not their store.

1. Establish the scan window for this project.
   - Read the previous firing's checkpoint using the schema under Outputs. An absent checkpoint uses the first-run defaults. A present checkpoint that cannot be read or does not match this procedure's schema follows Recovery.
   - If `incomplete_scan` is present, retry its recorded start/end window. Otherwise, fix the end at the current UTC time and start at `completed_until`; with no completed cursor, start one schedule interval of the task before that end. Process records in the half-open interval `[start, end)`. A completed window advances the next start to its end.

   - Collect that project's records for the window through the available
     log-reading capability.
     A successful empty result is a clean scan. A failed collection is an
     incomplete scan, reported under Recovery.

2. Establish which records are eligible to become signals.
   - Under `sdlc/policy/observability`, only `ERROR` and `CRITICAL`
     records, exceptions, and tracebacks are filing candidates. A `WARNING` never becomes
     a signal or bug report, however often it repeats. Adjacent warnings may be read only as
     context for an eligible error; they are not findings and are omitted from the report.
   - Group related eligible records into distinct error signals. Use the observed messages
     and context; do not investigate the codebase to establish a diagnosis.
   - Establish which system owns the failed operation. An upstream 5xx response, rate limit,
     timeout, or connection reset is an external failure and does not by itself establish a
     defect in the scanned project. Exclude the upstream failure unless records also show that
     the scanned project violated a supported contract in handling it, such as a required
     retry or recovery not occurring; cite that contract. When ownership or the local handling
     outcome is unknown, exclude the signal as causally inconclusive. An ERROR level or a failed
     delivery does not establish a local defect
     (`sdlc/policy/todo-creation-discipline`, `general/policy/human-contract`).
   - Before an error signal may proceed, falsify both operational explanations: congestion
     and a computer restart. Correlate its timing and surrounding records with the available
     restart evidence, resource-pressure or backlog evidence, and subsequent recovery. The
     filing burden is positive: absence of evidence for congestion or restart is not evidence
     against them.

   - When congestion or a computer restart explains the error, exclude it. When the available observations cannot rule out either cause, exclude it as causally inconclusive. Do not create a bug to obtain the missing causal evidence.

3. Deduplicate each error signal that passed the eligibility and causal gates.
   - Read the project's retained known-issue suppressions and apply their stated
     scope and reason before filing. A matching triaged signal is suppressed,
     not filed again; unrelated signals continue through these checks. Retain
     the disposition independently of this firing's checkpoint. When triage
     changes, update or remove the suppression through the owning work contract.
     An unreadable suppression set is incomplete classification, never an empty
     set or permission to file.
   - Read `instrukt todo list`. It returns slugs, and a
     slug names work rather than a mechanism, so a signal that matches no slug
     is not thereby untracked. Select candidates by token overlap between the
     signal — its message, its implicated module or file, and any subsystem or
     agent it names — and the returned slugs, then read those candidates'
     artifacts with `instrukt todo artifacts read <slug> <name>` before concluding
     a signal is untracked. Read the selected candidates only; the active set is
     never read in full. Record a matching active bug or todo by its slug
     instead of filing a duplicate.
   - Establish whether the implicated code has changed since the signal fired,
     keyed on the path the signal already carries. On the project's main branch,
     `git log --since=<last occurrence> -- <path>`: a change there after the
     signal last fired means the failure is likely already repaired, whether or
     not any todo records it. Do not file; carry the signal to the next run and
     file only if it recurs afterwards. On each active branch,
     `git log <branch> --since=<first occurrence> -- <path>`, together with
     `instrukt docs index --target <slug>` for a pending contract on that path: a
     match means in-flight work owns the mechanism. Record that slug and do not
     file a sibling.

- Delivered, closed, and removed work is not read. Whether a past failure was
  repaired is answered by the code and its history above, which is the source
  of truth for that question (`general/principle/grounding`); a record of
  completed work is neither, and its size grows without bound while its
  bearing on a current signal does not.

- Read `outstanding_signals` from the checkpoint and re-evaluate them against current logs, work, and code history. Retain only signals that still require a retry or the next window's recurrence check. A completed window with no recurrence after the recorded repair removes that signal from the checkpoint.
- New evidence for an already tracked failure updates its outstanding state when it remains unresolved; it does not create a second bug for the same failure. Route amendments per `sdlc/policy/todo-creation-discipline`.

4. File every remaining error signal through the todos module as a todo of kind `bug`.
   - State the operation and the observed consequence the records support,
     distinguishing the operation's outcome from how that outcome is surfaced.
     An operation the records identify as attempted is stated as the report's
     subject, and one report carries one mechanism
     (`sdlc/policy/todo-creation-discipline`). Where several
     operations are identified, preserve a relationship only where the records
     establish it rather than merely display it: failures observed close
     together, or within one session, are correlated by that observation alone
     and the correlation is recorded as such, never asserted as a causal
     sequence (`general/principle/grounding`). Where an operator-initiated or
     otherwise deliberate action sits in the sequence, establish it before
     attributing anything downstream of it to a failure upstream.
     State the expected outcome with its supporting source: the current
     contract the operation contradicts, located through `instrukt docs index` and
     stated in the report with how the operation breaks it. Preserve an unknown operation or outcome
     explicitly; never infer one from an error signature. A signal whose
     expected outcome no current contract states is not a bug
     (`sdlc/policy/todo-creation-discipline`):
     do not file it or retain it in the checkpoint; the session transcript remains the record of the observed operation and consequence.
     A
     reporting defect is stated as the discrepancy in reporting, without
     asserting that the underlying operation failed.
   - Submit the observed error summary and a representative log excerpt through
     the todos module as a todo of kind `bug`. Filing supplies no coding authority to a task.
   - Record the returned bug slug and confirm creation from the result.
   - If filing fails, record the signal and error as outstanding, then continue
     with the remaining signals.

5. Prepare the complete replacement checkpoint using the schema under Outputs.
   - A complete scan has successful collection, causal classification, and deduplication, with every eligible signal filed, excluded, matched to existing work or in-flight work, or carried solely for the recurrence check in Step 3. Set `completed_until` to the window's end and `incomplete_scan` to `null`. Keep only the signals that still require that recurrence check.
   - A collection, lookup, classification, or filing failure keeps the prior `completed_until`, sets `incomplete_scan` to the attempted window's start/end, and retains the affected signals with their retry evidence. An outstanding filing keeps the scan incomplete.
   - An empty successful collection is a complete scan. The checkpoint contains all fields, including explicit `null` values and an empty signal array when applicable.

## Outputs

- A bug filed for each observed error that passes the causal gate and is not already tracked.

- One latest checkpoint, using the schema below.
- Only the cursor, incomplete window, and unresolved retry evidence persist between runs. Resolved signals leave the checkpoint; session transcripts carry the completed dispositions (`sdlc/policy/todo-creation-discipline`).

### Checkpoint schema

The following schema belongs to this procedure.

| Field                 | Shape                                                            | Meaning                                                                          |
| --------------------- | ---------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `completed_until`     | UTC RFC 3339 timestamp or `null`                                 | Exclusive end of the last completely processed scan window.                      |
| `incomplete_scan`     | Object with UTC RFC 3339 `start` and `end` timestamps, or `null` | Fixed window to retry when the previous scan was incomplete.                     |
| `outstanding_signals` | Array of signal objects                                          | Only unresolved retries or post-repair recurrence checks needed by the next run. |

Each outstanding signal has these fields:

| Field          | Shape                  | Meaning                                                                                            |
| -------------- | ---------------------- | -------------------------------------------------------------------------------------------------- |
| `summary`      | String                 | Observed operation and failure requiring re-evaluation.                                            |
| `last_seen_at` | UTC RFC 3339 timestamp | Latest occurrence supported by the saved evidence.                                                 |
| `evidence`     | String                 | Addressable log evidence and representative excerpt, with the established contract when available. |
| `retry_reason` | String                 | Failed lookup or filing, or the pending post-repair recurrence check.                              |

First-run defaults are:

```json
{
  "completed_until": null,
  "incomplete_scan": null,
  "outstanding_signals": []
}
```

Each firing replaces the whole checkpoint. The task record supplies its slug,
schedule and project context.

## Recovery

- If logs cannot be collected, keep the saved `completed_until`, record the attempted window in `incomplete_scan`, retain unresolved evidence, and report the scan as incomplete with that replacement checkpoint.

- If active-work or repository-history lookup fails, preserve the
  affected signals as outstanding and report failure. Resume deduplication on the
  next run. An unavailable history query never promotes a signal to filing: an
  unanswered "was this already repaired" is an outstanding signal, not a new one.
- If the available observations cannot falsify congestion or a computer restart for an error,
  record the causal-evidence gap and exclude that error from filing. Uncertainty never promotes
  an operationally explainable error into a bug report.

- An absent checkpoint uses the first-run defaults and the task's schedule interval. A failed filing leaves the remaining signals eligible for processing; retain its evidence in `outstanding_signals` for re-evaluation before retrying.
- If a present checkpoint cannot be read or does not match the schema, stop the scan and report failure, leaving the saved checkpoint unchanged. Record the checkpoint problem in the session transcript rather than guessing a cursor.

- Tool failures do not authorize repairing scripts, editing code, or bypassing
  permissions. Record them and close the run as failed.

## See Also

- [sdlc/policy/observability](../../policy/observability.md)
- [sdlc/policy/todo-creation-discipline](../../policy/todo-creation-discipline.md)
- [general/policy/human-contract](../../../../general/policy/human-contract.md)
