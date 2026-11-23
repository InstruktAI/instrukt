---
description: 'What an orchestrator writes: the process state of its slug, what its entry procedure assigns it, and human-directed ad-hoc process changes under process-change authority; every other change reaches the repository through the worker session it dispatches.'
visibility: 'public'
---

# Orchestrator Jurisdiction — Policy

## Rules

- **An orchestrator writes the process state of its slug.** It reads the
  factual ledger, dispatches and assesses worker sessions under its prose
  playbook, and maintains the todo's artifacts, stamps, log and lifecycle state.
- **Repository work reaches the worktree through the worker session the
  orchestrator dispatches.** A dispatched worker produces its role's code,
  tests, configuration and scripts; when that work needs a change, the
  orchestrator returns it to that worker with the evidence and governing
  addresses (`general/procedure/orchestration`).
- **An entry procedure may assign its orchestrator further writes.** What it
  names — authoring the orchestrator performs itself, or a change it
  executes on `main` — is inside the orchestrator's jurisdiction under that
  procedure's own review routing, and nothing beyond what it names is, except
  the ad-hoc process path below.
- **Human-directed ad-hoc process changes follow process-change authority.**
  When the human explicitly directs an ad-hoc process change in any
  orchestrator session covered by this policy (`todo-orchestrator`,
  `integrator`, `coordinator`), that orchestrator is authorized to author it
  outside its active slug or integration workspace under
  `sdlc/policy/process-change-authority`, leaving the active delivery or
  integration worktree untouched.
- **The active slug is the focus.** Changes elsewhere in the repository belong to
  their own owners and are taken as intended.

## Rationale

Every change keeps the session whose brief and review cover it. An orchestrator
that writes a worker's output places that change outside the review its role
routes it through, and leaves the worker reasoning over state it did not
produce. Where one session drives a whole lifecycle by design, its entry
procedure carries that authoring together with its independent review, so the
same chain of custody holds.

## Scope

- Every session holding an orchestrator role (`todo-orchestrator`, `integrator`,
  `coordinator`), in every project.

## Enforcement

- Build and commit reviewers read the todo log and the commits made under the
  claim; a change produced outside the dispatched worker session, outside what
  the entry procedure assigns, and outside a human-directed ad-hoc process
  change under `sdlc/policy/process-change-authority` is a scope finding.

## Exceptions

- None.

## See Also

- [general/procedure/orchestration](../procedure/orchestration.md)
