---
description: Disk held by todo worktrees is reclaimed by instrukt itself — a project's declared regenerable output from idle todo worktrees and on a working session's request, and a delivered todo's worktree — with no agent running a deletion command.
---
# Worktree Reclamation — Spec

## What it is

Worktree reclamation returns the disk a todo's worktree holds once that disk
no longer serves work. The runner removes a project's declared regenerable
output from an idle todo worktree and has a delivered todo's worktree
released; a session working a todo reclaims its own worktree's regenerable
output with `instrukt repo reclaim`, which is how disk is returned while the
todo's work is still under way. Every removal is instrukt's own mechanical
work: no agent issues a deletion command and no agent runtime is asked to
approve one.

## Canonical fields

### Regenerable output

Regenerable output is what a project declares under `regenerable` in the
project layer of its configuration, as rebuilt from its committed sources:
paths inside a checkout of that project
(`project/instrukt/spec/feature/configuration/layered-configuration`). The
runner knows no language, build tool or directory name: the declaration is
its only source for what may be removed. A project that declares nothing has
no regenerable output, and reclamation removes nothing from its worktrees
short of a delivered worktree as a whole.

The declaration reclamation acts on is the one admitted for the project on
that computer, from the project's own checkout. It applies alike to every
todo worktree of the project there, whatever that worktree's branch holds. A
declaration that reaches outside a checkout is a configuration finding at
each point where configuration enters, and removes nothing
(`project/instrukt/policy/checks-and-guards`).

In a worktree, a declared path is removed only where it is reached through
that worktree's real directories and git ignores it there. A declared path
git tracks or reports as untracked, and one a symbolic link leads to, are
left as they are and named with that reason; a declared path that is itself
a symbolic link loses the link and nothing it points to. Reclaiming
regenerable output therefore leaves the worktree at the same commit, on the
same branch, with the same status git reported before.

### Idle worktrees

The runner reclaims the regenerable output of the idle todo worktrees on the
computer it runs on, for each project whose work tracking is admitted there.
A worktree is idle while its todo's candidate waits for integration, the todo
is parked or held, or its work has gone back to preparation. Workflow answers
each todo worktree's standing — delivered, idle or under way — and the runner
derives none of it (`project/instrukt/spec/domain/workflow`). A worktree whose
todo is under way — between a build and its review, between a review and its
fix — keeps its output for the role that works there next.

The runner reclaims nothing in a worktree where a role attempt is open
(`general/principle/trust-and-guardrails`). It moves the output out of the
worktree, to a staging location beside it, inside a section of the work
record in which no role attempt opens
(`project/instrukt/spec/domain/work`), and deletes it from there afterwards,
outside that section. An attempt that opens finds its worktree either still
holding its output or no longer holding it, never while it is being emptied.

Reclamation follows an invocation's dispatch and event work. No pickup, event
reaction or command waits on a deletion, and output a stopped invocation left
at a staging location is deleted by a later one.

### Delivered worktrees

After a todo is delivered, the runner has work release that todo's worktree
and work branch on each computer that holds any of them
(`project/instrukt/spec/feature/work/todos`): the cleanup
`project/instrukt/spec/feature/workflow/delivery-cycle` requires of a
delivered todo. The worktree's regenerable output leaves with it. A worktree
that holds uncommitted changes, is locked or has a role attempt open stays,
and its cleanup stays owed until a later invocation finds it releasable. A
delivered worktree that stays is reclaimed as an idle one.

A withdrawn todo's worktree is released by the withdrawal itself
(`project/instrukt/spec/feature/work/todos`).

### Reclaiming on request

`instrukt repo reclaim` reclaims the regenerable output of the worktree the
calling session's todo assignment records. It takes no location: neither an
argument nor the caller's working directory selects what it acts on. It
removes exactly what the runner removes from that worktree, and it answers
once that output is deleted, naming each path it removed and each declared
path it left with the reason, or stating that the project declares nothing.
Where the project's declaration is not admitted on that computer, it refuses
with that configuration finding and removes nothing.

It is a write to that worktree. Its action is the one a commit in the
assigned authoring workspace takes, and its object is the workspace the
caller's assignment records
(`project/instrukt/spec/feature/configuration/roles-and-capabilities`). The
worktree's own state decides who may write. A caller the assignment binds is
refused only while another session holds the worktree: a role attempt open
there that is not the caller's, or, with no role attempt open, an execution
claim the caller does not hold. The refusal names that holder. While nothing
holds the worktree, any caller the assignment binds reclaims it. A caller no
todo assignment binds to a worktree is refused, and the refusal names the
assignment that would bind it.

### Mechanical work

Reclamation starts no agent session, sends no agent a message and emits no
event. It keeps no record of its own: what is owed is read again on every
runner invocation from the work record, git and the filesystem
(`sdlc/policy/state-storage`). A removal that cannot complete changes nothing
else and is retried on a later invocation.

The runner's log names each worktree whose reclamation stays owed, with its
cause, when that cause appears or changes. A delivered todo's remaining
worktree and work branch show in that todo's own ledger
(`project/instrukt/spec/feature/work/todos`).

## Use cases

### UC-WR1: Idle todo worktrees stop holding build output

```gherkin
Given a developer's project declares its regenerable output
And one of its todos has an idle worktree holding that output
When the runner reclaims on that computer
Then the declared output is gone from that worktree
And the worktree is at the same commit on the same branch, as clean as before
And no agent session took part in the removal
But a worktree whose todo is between its rounds of building, reviewing and fixing keeps its output
And a worktree in which a role attempt is open keeps its output
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
- Reclaimed output is rebuilt by the next build in that worktree, so work that
  returns to an idle worktree, or continues after a reclamation on request,
  pays a full build.
- A worktree whose todo's work is still under way holds its output for as long
  as that work runs; reclaiming on request is the only reclamation it sees.
  While a review role's attempt is open, no session is admitted to reclaim
  there: the review role holds no write in the worktree, and the session
  holding the claim is refused until that attempt is released.
- A structural holder has no lifecycle, so its worktree, where it had one
  before it became a holder, is neither idle nor delivered: it is not
  reclaimed and not removed.
- A declared path names one location. A project whose regenerable output sits
  at many locations declares each of them.
- A computer acts on what its own work record holds. It learns of a delivery
  made elsewhere because work reads GitHub for the todos whose worktree or
  work branch the computer holds
  (`project/instrukt/spec/feature/work/github-record`); until it has, the
  worktree keeps the standing the record last gave it.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [general/policy/shared-machine-resources](../../../../general/policy/shared-machine-resources.md)
