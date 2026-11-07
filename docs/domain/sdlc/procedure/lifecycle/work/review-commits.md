---
eval: true
description: Review a committed worktree head independently before the integrator merges it.
role: commit-reviewer
parameters:
  - name: slug
    required: false
dispatch:
  agent: [codex, claude, gemini]
  tier: strong
---

# Review Commits — Procedure

## Required reads

- [sdlc/procedure/lifecycle/review](../review.md)
- [sdlc/policy/todo-creation-discipline](../../../policy/todo-creation-discipline.md)

## Goal

Independently review the exact committed preparation or one-off worktree head before the integrator merges it. The shared review procedure owns every review lane and its verdict arithmetic.

## Preconditions

- The assignment identifies the worktree, branch, exact candidate head and its base against canonical main. A todo assignment also names its slug and reserved reviewer attempt.
- The reviewer is independent of the author and works through native shared delegation in that phase's worktree. One-off work supplies its own reviewed-head report destination.
- The candidate is clean; no writer remains outstanding while it is reviewed.

## Steps

### Configured runtime operations

<!-- @config: runtime.read_thread -->

<!-- @config: runtime.message_child -->

### 1. Pin the candidate

Verify the recorded worktree, branch and head against Git. Record the merge base against canonical main and the complete candidate diff. Read the full changed-file context and its governing current and pending contracts. The target is the whole worktree head; no per-SHA subset or direct-main exemption substitutes for it.

### 2. Apply independent full-depth review

Run the shared review procedure over that candidate, applying every triggered lane. For directive text carry the problem, reader role, intended behavior, faithful human direction, rejected alternatives and what remains unchanged into the review context. Author-supplied evidence directs attention and never substitutes for reviewer judgment. Keep unrelated work outside the candidate's declared scope.

### 3. Retain the exact-head result

Write the complete report required by the shared review procedure, with every executed lane and its clean-pass evidence. Retain the worktree path, branch, reviewed base, exact head, report digest and verdict. For preparation, write `preparation-review-findings` and record `instrukt todo stamp <slug> --verdict approve|needs_work --phase preparation_head` from the bound reviewer in that tree. Approval and reviewer release append `workflow.preparation_reviewed` atomically; the integrator integrates those docs as soon as possible before build. For a one-off candidate, return its report and reviewed head to the authoring session and integrator through the configured native route.

Do not write per-commit git notes or author commits on main. A changed head or report requires a current independent result before integration.

### 4. Continue remediation

Follow the shared re-review boundaries. Read the producer's dispositions and independently judge every unresolved finding against the revised worktree head. An evidence-only rejection may require no new commit. Return a complete report and verdict; an acknowledgement is no verdict, and approving a fixing commit alone does not close the candidate's review.

## Outputs

- Complete review report, exact candidate head, report digest and verdict.
- For approved preparation, a retained stamp, released reviewer attempt and immediate integrator event.

## Recovery

An unresolvable head, changed placement, dirty tree or missing review evidence is a blocker, never permission to review another target. Pre-existing patterns excuse no violation in touched code. Retain the report and unresolved findings for the owning authoring session; preserve the worktree until integration and cleanup are confirmed.
