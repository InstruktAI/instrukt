---
eval: true
description: 'Finalize the bound candidate, report role completion, and record readiness after confirmed handoff and current review.'
visibility: 'public'
role: finalizer
parameters:
  - name: slug
    required: true
dispatch:
  agent: [gemini, claude, codex, deepseek]
  thinking_mode: fast
---

# Finalize — Procedure

## Required reads

## Goal

Prepare the candidate in your bound finalizer worktree, report role completion
to the work orchestrator, and record finalize readiness after confirmed handoff
and current independent approval. Report blockers to that orchestrator with
the affected evidence and preserved work location.

## Preconditions

- `instrukt todo artifacts read {slug} review-findings` exists with APPROVE.
- At admission, the bound build-review stamp is current for the target and report.
  Changes made during finalization are reviewed after completed handoff and release.
- `instrukt todo artifacts read {slug} quality-checklist` exists.
- Deferrals are resolved and their processing is recorded.

## Steps

### Prepare the bound candidate

1. Read the approved findings and current verdict binding.
2. Integrate latest main inside the worktree:

   ```sh
   git fetch origin main
   git merge origin/main --no-edit --no-overwrite-ignore
   ```

3. Resolve conflicts where code context is available. Preserve accepted main
   behavior and reapply the reviewed branch intent. Confirm each changed bound
   test fails before its implementation change and passes afterward, per the
   testing policy. After resolving a conflict, rerun only the tests bound to
   the affected use cases. Commit resolutions and record the resulting evidence. A changed
   todo is reported to the orchestrator for independent review after this
   attempt's completed handoff and release; no full-suite admission gate
   is added here. If synchronization cannot be completed in this attempt,
   preserve its branch, worktree and local files and report the failure and role
   head to the orchestrator. It releases the failed attempt before admitting a
   recovery builder from that head (`sdlc/policy/version-control-safety`).
4. Publish the finalized branch:

   ```sh
   git push origin HEAD:{slug}
   ```

5. Confirm the worktree is clean and its head, local role branch and published
   remote todo branch match each other. Report that finalized commit and any
   changes to reviewed content; current approval of this new head is checked
   after handoff and release.
6. Stamp `instrukt todo stamp {slug} --role-complete`, binding this clean
   worktree's exact committed head to the finalizer attempt. Report that evidence
   to the work orchestrator for `instrukt todo handoff {slug}` under
   `project/instrukt/spec/feature/workflow/delivery-cycle`. Await its confirmed
   handoff, attempt release and current independent approval. A changed candidate
   is reviewed at the advanced todo head in the reviewer's own worktree after
   this attempt is released.
7. After that confirmation, recheck equality of the clean finalizer worktree
   head, local todo branch and published remote todo branch. Record
   `instrukt todo stamp {slug} --finalize-ready` against this retained finalizer
   binding. This record-only step admits no further workspace work. A readiness
   stamp names this exact commit, records the integration handoff and appends
   `workflow.finalized` idempotently keyed by project/todo/SHA; reporting readiness
   alone establishes nothing. Any further workspace changes require a new
   finalizer attempt at the current todo head and its own completion and handoff.

Worker report format:

```text
FINALIZE_READY: {slug}

Commit: <candidate SHA>
Main integrated: yes
Readiness stamp: <record reference>
```

## Outputs

- Branch merged with latest upstream main and pushed to `origin/{slug}`.
- Readiness bound to the matching clean worktree, local branch and remote head.
- Completion evidence and readiness reported to the work orchestrator; readiness
  records the exact candidate handoff to the standing integrator.

## Recovery

- Resolve synchronization conflicts within your outstanding attempt when you can.
  When you cannot, report the reason, committed role head and retained worktree
  to the orchestrator for `instrukt todo handoff {slug} --failed --reason <text>`.
  That release preserves the failed attempt's work for a recovery builder and
  leaves the todo branch unchanged. Resume finalization only under a new
  assignment after recovery, normal handoff and current independent approval.
  Return an intent, scope or reviewed-contract defect through the orchestrator's
  kickback path.
- Report branch-publication, handoff or record-write failures to the work
  orchestrator. A readiness-only retry uses the retained finalizer binding after
  confirmation; further workspace changes return for a new assignment.
