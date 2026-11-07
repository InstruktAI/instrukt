---
description: Project prerequisites, structural group holders, item-level execution claims and auditable dispatch holds.
visibility: public
delivered_by: [instrukt-first-build]
---
# Dependencies and Holds — Spec

## What it is

A developer arranges work into groups, orders it by prerequisites and pauses
individual todos with an inspectable reason. Group structure, prerequisite
readiness, an item-level execution claim and a dispatch hold remain separate facts.

## Canonical fields

### Prerequisites

Dependencies form a project-local directed acyclic graph. A dependency says
the dependent waits for evidence from its prerequisite. Unknown prerequisites,
self-dependencies and cycles are refused before any edge changes; a cycle
refusal identifies the cycle. An accepted change of edges or groups, across
however many todos it touches, commits in one transaction of the state
database. A todo cannot depend on its own structural
holder or an ancestor holder, including through a simultaneous group change.

`instrukt todo create` accepts prerequisites at creation.
`instrukt todo update <slug> --after <dependencies>` replaces the whole set.
An empty set clears prerequisites; omitting the option leaves them unchanged.

| Dependent's next lane | Prerequisite needed |
| --- | --- |
| Preparation, with a repository-work prerequisite | Current prepared evidence or delivery. |
| Preparation, with a task prerequisite | Delivery of the task's result. |
| Implementation or task execution | Delivery of every prerequisite. |

The workflow consumes these readiness facts both on initial pickup and on
later selection. A blocked todo does not prevent selection of other ready
work. Prepared evidence whose plan review has been invalidated no longer
satisfies preparation readiness. Changing dependency edges changes ordering;
it does not by itself rewrite prepared artifacts or establish drift.

An active todo may depend on iceboxed work. The mutation succeeds with a
warning identifying the parked prerequisite. It remains blocked until the
prerequisite satisfies its milestone or the edge is removed.

### Structural holders

Grouping through `instrukt todo create --group <parent>` or
`instrukt todo update <slug> --group <parent>` relates a child to its parent's
durable identity. The child retains its own kind, lifecycle, artifacts and
prerequisites.

A parent receiving its first child becomes a structural holder. Only a parent
still before prepared, or an existing holder, is eligible. Conversion removes
the parent's lifecycle state, claim and dispatch hold; its artifacts and
version history remain frozen evidence of the split. Pending documentation
ownership transfers to the first child through the documentation capability.
A holder remains a holder when its last child leaves; it has no independent
build, approval or delivery.

A prerequisite on a holder is satisfied when every direct child satisfies the
required milestone, recursively through nested holders. An empty holder does
not become satisfied merely by having no children. Group membership supplies
context in listings and in retained delivery history; dependencies do not
move a child to another group. A holder with no visible children contributes
no displayed group.

### Session bindings, claims and holds

The record retains each started session by todo and role, including its
runtime and id. The runner writes the binding after starting a session and
looks it up for reuse through the runtime’s alive capability. Workflow reads
these bindings and records role-associated execution evidence through work’s
mutation surface. A binding is distinct from a claim; different roles can
have different sessions without transferring execution ownership.

The runner records and reflects the todo's item-level execution claim before
starting any session. `claim.held` exposes that claim. Session identities are
retained only in per-todo, per-role bindings. A dispatch hold pauses automatic
pickup without establishing or transferring the claim.

`instrukt todo update <slug> --hold --reason <reason>` sets a hold with its
source, causal reason, time and recording session. A missing or blank reason
is refused. An automatic hold records the cause that produced it, such as a
failed task or unresolved diagnosis. `instrukt todo show` exposes the hold's
reason and source.

`instrukt todo update <slug> --release` releases the dispatch hold while
preserving the todo's intent, artifacts, dependencies and execution-claim
record. Release makes the todo eligible for the other workflow gates; it does
not approve evidence or satisfy prerequisites. A hold is a brake on future
automatic dispatch, not a command to terminate work already running. Structural
holders carry no dispatch hold; each child carries its own.

### Holds from GitHub

An attributable `hold` label addition by a collaborator with write access sets
the dispatch hold with source `github`, reason `held from GitHub by <login>`,
GitHub actor and time. An authorized removal releases the current hold and
retains the releasing GitHub login. These use the same item-level hold and
release semantics as the CLI, including preservation of work, claims and
running sessions. There is one current hold per todo, with its audit retained.
Inbound action authorization, replay accounting and output-label reflection
belong to `project/instrukt/spec/feature/work/github-record` (UC-GR9).

## Use cases

### UC-DH1: Sequence a dependent delivery

```gherkin
  Given a developer has a todo that needs other project work to be delivered first
  When they record those prerequisites
  Then the record retains every prerequisite needed for that delivery
  But when a prerequisite or a grouping would make the todo wait on itself, directly or indirectly
  Then the change is refused, names the cycle, and leaves the todo's prerequisites and group as they were
```

### UC-DH2: Pause work with a reason others can inspect

```gherkin
  Given a developer has active work they need to pause
  When they place it on hold with the reason it must wait
  Then the reason remains visible with the todo
  And the recorded work remains available with its dispatch hold
```

### UC-DH3: Grouping work under a todo keeps its pending documentation

```gherkin
  Given a developer has a todo whose planned documentation is not yet delivered
  When they file the first piece of work under that todo
  Then that documentation now belongs to the new piece of work
  And they are told which documentation files to commit
  But when filing the work under that todo is refused
  Then the documentation still belongs to the original todo
```

## Known caveats

- Readiness does not grant a session a claim. A claim does not satisfy an
  approval, prerequisite or integration gate.

## See Also
- [project/instrukt/spec/domain/work](../../domain/work.md)
