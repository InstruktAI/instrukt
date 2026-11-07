---
eval: true
description: 'Finalize the bound candidate, report role completion, and record readiness after atomic completion and current review.'
visibility: 'public'
role: finalizer
parameters:
  - name: slug
    required: true
dispatch:
  agent: [gemini, claude, codex]
  tier: light
---

# Finalize — Procedure

## Required reads

## Goal

Prepare the candidate in your bound finalizer worktree, report role completion
to the work orchestrator, and record finalize readiness after atomic completion
and current independent approval. Report blockers to that orchestrator with
the affected evidence and preserved work location.

## Preconditions

- `instrukt todo artifacts read {slug} review-findings` exists with APPROVE.
- At admission, the bound build-review stamp is current for the target and report.
  Changes made during finalization are reviewed after completion and release.
- `instrukt todo artifacts read {slug} quality-checklist` exists.
- Deferrals are resolved and their processing is recorded.

## Steps

### Prepare the bound candidate

1. Read the current approval and processed deferrals. Work in your bound shared
   todo worktree on the todo branch.
2. Fetch and merge accepted upstream main. Resolve conflicts preserving both
   accepted upstream behavior and the approved intent. Run only checks warranted
   by affected use cases under the testing policy and commit resolutions.
3. Publish with `git push origin HEAD`. Verify the clean worktree head,
   local todo branch and published remote todo head match.
4. Record `instrukt todo stamp {slug} --role-complete` at that exact head. The
   stamp releases this attempt atomically. Report completion and any changed
   todo content or review evidence to your parent. A changed candidate is
   independently reviewed at the current shared head after release.
5. After your parent confirms current independent approval and no outstanding
   workspace role, recheck matching candidate identities and record
   `instrukt todo stamp {slug} --finalize-ready` against the retained binding.
   This record-only write commits readiness and `workflow.finalized` together
   for the exact project/todo/SHA. Further workspace changes need a new attempt.

Report candidate SHA, synchronization/publication outcomes, completion stamp and
readiness evidence to the orchestrator; a mismatch or blocker retains its cause
and preserved shared-work location.

## Outputs

- Branch merged with latest upstream main and pushed to the captured `origin/<slug>/work` branch.
- Readiness bound to the matching clean worktree, local branch and remote head.
- Completion evidence and readiness reported to the work orchestrator; readiness
  records the exact candidate handoff to the standing integrator.

## Recovery

- Resolve synchronization within the outstanding attempt. A blocker is reported
  to the parent with reason, native status, shared head and workspace evidence.
  Dirty or conflicted work remains outstanding for in-place recovery. Native stop
  evidence and a clean tree permit failed release under the todo-attempt contract;
  a successor continues in the same worktree from its current head. Failure
  establishes no completion or readiness.
- A changed intent or reviewed-contract defect returns through the orchestrator's
  kickback path. Publication and record-write failures remain explicit obligations.
- A readiness-only retry uses the retained successful binding after current
  review and release; further workspace changes require a new assignment.
