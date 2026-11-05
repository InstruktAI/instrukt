---
eval: true
description: 'Finalize phase. The finalizer records the handoff; the work orchestrator confirms it and exits.'
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

Prepare a reviewed branch for integration and close the work assignment after
the finalize-ready handoff is recorded. The standing integrator owns advancement
of main; the finalizer records readiness and the handoff, and the work
orchestrator confirms them and releases its lane.

## Preconditions

- `instrukt todo artifacts read {slug} review-findings` exists with APPROVE.
- The bound build-review stamp is current for the target and report.
- `instrukt todo artifacts read {slug} quality-checklist` exists.
- Deferrals are resolved and their processing is recorded.

## Steps

### Stage A — Worker: finalize-prepare (worktree)

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
   todo requires current review before readiness; no full-suite admission gate
   is added here. If Git refuses to overwrite
   an ignored local file, leave it intact and report the failed merge for
   builder recovery (`sdlc/policy/version-control-safety`).
4. Publish the finalized branch:

   ```sh
   git push origin HEAD:{slug}
   ```

5. Confirm the worktree is clean and its head, local role branch and remote todo branch
   match the reviewed candidate. Report the commit prepared for handoff.
6. Stamp `instrukt todo stamp {slug} --role-complete`, binding this clean
   worktree's exact committed head to the finalizer attempt. Report that evidence
   to the work orchestrator. The orchestrator checks the stamped head,
   fast-forwards the local todo branch to it and confirms the recorded handoff
   and attempt release under `project/instrukt/spec/feature/workflow/delivery-cycle`.
7. After that confirmation, recheck equality of the clean finalizer worktree
   head, local todo branch and published remote todo branch. Record
   `instrukt todo stamp {slug} --finalize-ready` against this retained finalizer
   binding. This record-only step admits no further workspace work. A readiness
   stamp names this exact commit, records the integration handoff and appends
   `workflow.finalized` idempotently keyed by project/todo/SHA; reporting readiness
   alone establishes nothing. Any further workspace changes require a new
   finalizer attempt at the current todo head and its own completion and handoff.

### Stage B — Work orchestrator: confirm handoff and end

The finalizer's readiness stamp records the handoff; integration belongs to the standing
integrator in its own worktree under `sdlc/procedure/lifecycle/integrate`.

1. Re-run `instrukt todo work {slug}` and read the derived obligation and effects.
2. Confirm the finalizer role-completion stamp, fast-forward handoff and attempt
   release before accepting readiness. Check readiness against the clean worktree,
   local todo branch, remote and stamped SHA.
   A mismatch returns to reconciliation and restamping; no earlier stamp admits it.
3. Confirm the handoff and `workflow.finalized` event recorded by the stamp, then
   `instrukt todo update <slug> --unclaim` for lane release
   and terminal cleanup commands supplied by the flavor. Confirm release before
   leaving this assignment. Session lifecycle follows
   `general/principle/session-lifecycle`; this handoff does not end an unfinished
   human conversation.
4. If the handoff cannot complete, preserve the blocker and route recovery
   inside work scope. No work-role call performs integration.

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
- Recorded handoff to the standing integrator and confirmed lane release.

## Recovery

- Resolve conflicts in the worktree before recording readiness. Return an intent,
  scope or reviewed-contract defect through the work orchestrator's kickback path.
- Repair branch publication or record-write failures and re-run the slug-scoped
  handoff. Missing evidence or failed cleanup remains an explicit obligation.
