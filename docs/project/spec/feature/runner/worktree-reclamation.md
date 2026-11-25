---
description: Disk held by todo worktrees is reclaimed by instrukt itself — a project's declared regenerable output from worktrees nobody is working in and on a working session's request, and a delivered todo's worktree — with no agent running a deletion command.
delivered_by:
  - runner-owned-workspace-cleanup
---
# Worktree Reclamation — Spec

## What it is

Worktree reclamation returns the disk a todo's worktree holds once that disk
no longer serves work. The runner removes a project's declared regenerable
output from a todo worktree nobody is working in and removes a delivered
todo's worktree; a session working a todo reclaims its own worktree's
regenerable output through one instrukt command. Every removal is instrukt's
own mechanical work: no agent issues a deletion command and no agent runtime
is asked to approve one.

## Canonical fields

### Regenerable output

Regenerable output is what a project declares, in the project layer of its
configuration (`project/instrukt/spec/domain/configuration`), as rebuilt from
its committed sources. The runner knows no language, build tool or directory
name: the declaration is its only source for what may be removed. A project
that declares nothing has no regenerable output, and reclamation removes
nothing from its worktrees short of a delivered worktree as a whole.

A declaration reaches only inside a checkout of the declaring project. One
that reaches outside it is a configuration finding where the declaration is
written and removes nothing (`project/instrukt/policy/checks-and-guards`).

Reclaiming regenerable output leaves the rest of the worktree as it was: the
same commit, the same branch, and a worktree that was clean stays clean.

### Unattended worktrees

A todo's worktree is unattended while no role attempt is open in it: after a
role attempt is released, while the todo's candidate waits for integration,
and while the todo is parked or held. Attempts and their release are facts of
the work record (`project/instrukt/spec/feature/work/todos`).

The runner reclaims the regenerable output of the unattended todo worktrees
on the computer it runs on. It reclaims nothing in a worktree where an attempt
is open: an attempt that opens first keeps the worktree, and reclamation waits
until the worktree is unattended again
(`general/principle/trust-and-guardrails`).

### Delivered worktrees

After a todo is delivered the runner removes its clean worktree, as the
cleanup `project/instrukt/spec/feature/workflow/delivery-cycle` requires of a
delivered todo. A worktree that is dirty or in use keeps its cleanup owed with
its cause (`project/instrukt/spec/feature/runner/session-continuity`).

A withdrawn todo's worktree is released by the withdrawal itself
(`project/instrukt/spec/feature/work/todos`).

### Reclaiming on request

A session reclaims the regenerable output of the worktree its own todo
assignment binds it to. The command removes exactly what the runner removes
from that worktree, and its caller cannot direct it at any other location. It
is a write to that worktree and is admitted as one
(`project/instrukt/spec/domain/work`). Its answer names what it removed, or
states that the project declares nothing.

A caller no todo assignment binds to a worktree is refused, and the refusal
names the assignment that would bind it.

### Mechanical work

Reclamation starts no agent session and sends no agent a message. A removal
that cannot complete changes nothing else, stays owed and is retried on a
later runner invocation.

## Use cases

### UC-WR1: Unattended todo worktrees stop holding build output

```gherkin
Given a developer's project declares its regenerable output
And one of its todos has a worktree holding that output with no role attempt open
When the runner reclaims on that computer
Then the declared output is gone from that worktree
And the worktree is at the same commit on the same branch, as clean as before
And no agent session took part in the removal
But a worktree in which a role attempt is open keeps its output
```

### UC-WR2: Reclaim disk in one's own worktree during work

```gherkin
Given a developer's agent is working a todo in the worktree its assignment binds it to
And the project declares its regenerable output
When the agent asks instrukt to reclaim its worktree
Then the declared output is gone from that worktree and the answer names what was removed
And everything else in that worktree and every other location on the computer is as it was
But when no todo assignment binds the caller to a worktree
Then the request is refused and names the assignment that would bind it
```

## Known caveats

- Checkouts no todo assignment records — ad hoc worktrees, clones, review
  copies, temporary homes and scratch directories — are not reclaimed; their
  creator removes them (`general/policy/shared-machine-resources`).
- Reclaimed output is rebuilt by the next build in that worktree, so a role
  that builds after a reclamation pays a full build.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [general/policy/shared-machine-resources](../../../../general/policy/shared-machine-resources.md)
