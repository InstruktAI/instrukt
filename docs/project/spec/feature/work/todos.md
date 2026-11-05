---
description: Project work items with explicit kinds, durable intent, retained artifacts and recoverable withdrawal.
visibility: public
delivered_by: [instrukt-first-build]
---
# Todos — Spec

## What it is

A developer records a project's work as todos, each with explicit intent and a
kind that selects its preparation and execution path. The work record retains
what was requested and the evidence produced while fulfilling it.

## Canonical fields

### Identity and creation

A todo has a durable identity and a project-local slug. The slug is unique
among addressable active, iceboxed and structural-holder entries. Retained
delivered or removed entries keep their identities even when a later todo
uses the same slug. An identity-addressed action acts on that exact entry: a
retained item is addressed as `<slug>@<identity-prefix>`, and a plain slug
always means the live item. Prerequisites are stored by durable identity and a
delivered one is shown as `<slug> (delivered)`.

`instrukt todo create <slug> <content> --kind <kind>` requires substantive
content and an explicit kind. Without `--approved`, it prints the admission
questions and creates nothing. With `--approved`, creation writes the entry, initial state and
first input-artifact version in one transaction of the state database
(`project/instrukt/spec/feature/work/state-database`). Invalid content,
kind, dependency or grouping rejects creation before partial work is recorded.
`--icebox` creates the todo in the icebox. For a task, `--agent <name>
[--thinking slow|med|fast]` sets its optional agent preference, which
dispatch uses before `agents.order`. Admission is the agent's judgment;
there is no machine proof gate.
`instrukt todo create` accepts `--human-involved true|false` as an action-scoped,
audit-only declaration under `general/policy/human-involvement-recording`.
It records observed human involvement and supplies no authorization.
GitHub reflection is owned by
`project/instrukt/spec/feature/work/github-record`.

### Kinds

| Kind | Purpose | Completion evidence |
| --- | --- | --- |
| Feature | Deliver a capability requiring intent development. | Confirmed delivery commit. |
| Change | Deliver bounded repository work whose intent is already settled. | Confirmed delivery commit. |
| Bug | Repair observed incorrect behavior. | Confirmed delivery commit. |
| Task | Carry out a prepared external action, once or repeatedly. | Recorded execution outcome. |

A task performs external operations and may write its own working material;
repository edits or commits belong to a repository-work kind. Preparation routing and approval requirements belong to
`project/instrukt/spec/feature/workflow/preparation`. Creation never
authorizes a task, even when a complete plan is supplied: the plan passes the
configured approval gate.

### Task schedule

A task record holds a one-off `at` or recurring `cron` schedule and its
missed-run policy, `catch_up: once` by default or `skip`, set at creation with
`--catch-up once|skip`. Creation sets the
schedule with `--schedule <value>`; `instrukt todo update <slug> --schedule <value>` changes it afterwards (an empty value clears it). Values use an
ISO-8601 instant for `at` or `cron:<minute hour day month weekday>` for `cron`.
Timing and firing decisions belong to the runner; work retains the schedule
and task-firing stamps.

A task record carries an optional agent/model preference. Dispatch consults
that task preference before procedure dispatch defaults and global `agents.order`.

### Reads and changes

`instrukt todo list` reads the caller's project's work. Active work is the
default view; iceboxed, removed and delivered work are available through the
corresponding listing selections. Ordering is derived from readiness and
dependencies. `instrukt todo show <slug>` exposes the record, current evidence,
holds, prerequisites, group context, execution claim and per-role session
attempt bindings (runtime, id, key, attempt, branch, base, returned worktree
path and release evidence), including released attempts. These bindings form the
assignment child set exposed as `sessions.bound`; `claim.held` exposes whether
the execution claim remains held. `instrukt todo list` reads only this
computer's state database and fetches nothing: todos held elsewhere show as
last known, labelled with where they run and how old the copy is.
`instrukt todo show <slug>` refreshes that one todo first. When and what is
read from GitHub is the interest-driven refresh contract in
`project/instrukt/spec/feature/work/github-record`.

`instrukt todo refresh <slug>` resolves a conflicted item: it takes the
todo as GitHub holds it, and the agent re-applies its change.

`instrukt todo update <slug>` changes supported metadata. A rename preserves
durable identity and updates references. A kind change reroutes preparation
and handles incompatible evidence explicitly. Rename and kind changes are
refused once implementation starts. Dependency and group changes follow
`project/instrukt/spec/feature/work/dependencies-and-holds`.

A lifecycle phase is derived from evidence by the workflow engine. Editing a
displayed phase does not establish approval, readiness or delivery.

`instrukt todo update <slug> --unclaim` releases the execution claim, the
counterpart of the claim recorded before dispatch. Session bindings remain
evidence of the assignment's conversations. The runtime owns ending those
sessions; release changes the claim, not their lifecycle.

### Completed role handoff

`instrukt todo handoff <slug>` is the claim-holding work orchestrator's command
for a completed setup, builder/fixer or finalizer attempt. It refreshes that
todo under the existing command contract, resolves the outstanding attempt,
and requires its bound completion stamp and clean worktree at the stamped
role head. It persists source and expected todo heads in the state database,
fast-forwards the todo branch with an expected-old-head comparison, then
confirms the advance and releases the attempt. Recovery and divergence follow
`project/instrukt/spec/feature/workflow/delivery-cycle`.

The durable attempt and completion sequence identify the operation. A retry
finishes its pending intent or reports its confirmed handoff without advancing
again. With no outstanding attempt, it reports the most recent confirmed
handoff; it never selects an incomplete or read-only attempt as new work.
Review verdicts and deferral-processing evidence release their attempts directly.

`instrukt todo handoff <slug> --failed --reason <text>` is the alternative
release for an outstanding writing attempt that cannot finish synchronization.
It requires the claim-holding orchestrator, a non-empty reason and an observable
committed role-branch head, but no completion stamp or clean worktree. It records
the failure, role branch, head, path and any unfinished workspace state, releases
the attempt and queues reflection in one local transaction. It changes no Git
ref, preserves the worktree and role branch, and establishes neither completion
nor readiness. A builder can then be admitted with
`instrukt sessions run --todo <slug> --recover-attempt <role/attempt> …` at that
recorded failed head. A completed recovery hands off through the normal command.

Failed release is refused while a recorded ref advance may already have happened;
reconcile that normal handoff's intent first. With no outstanding attempt,
`--failed` reports the most recent confirmed failed release and its reason,
without another mutation. With an outstanding attempt, either form addresses
that attempt; re-observe the binding before invoking the command after dispatch.

| Exit status | Meaning |
| --- | --- |
| `0` | The requested normal handoff or failed release is confirmed, including an unchanged replay. Normal output identifies attempt, role, source, expected head and confirmed head. Failed output explicitly names the failed outcome, reason, attempt, preserved role branch/head/path and unchanged todo head. |
| `1` | Observation, Git or persistence prevented confirmation of the requested outcome. Output names the failure and any recorded intent; retry resumes that operation. Neither handoff nor failed release is implied. |
| `2` | The requested operation is refused: wrong claim, read-only role, no matching outcome to replay, missing normal completion/clean-head evidence or non-fast-forward; for failed release, missing reason or observable role head, or an unresolved possible ref advance. Output names the condition. Normal divergence additionally records an automatic hold and both branches and heads, retaining the attempt and role branch. |

A pending intent keeps the attempt outstanding and blocks later workspace-role
admission. The command neither marks finalize readiness nor publishes a branch.

### Supported metadata and listing selections

`instrukt todo update <slug>` admits `--slug <new-slug>`, `--kind <kind>`,
`--schedule <value>`, `--group <parent>`, `--after <dependencies>`,
`--hold --reason <text>`, `--release`, `--unclaim` and, for a task,
`--agent <name> [--thinking slow|med|fast]`. Relationship and hold
semantics remain in the dependencies-and-holds spec. `--force` permits
permanent orphan-artifact handling during a kind migration; it grants no
general bypass of claims, holds or review.
Listing selections expose the iceboxed, removed and delivered sets described
above; the active set remains the default.

### Parking, withdrawal and delivery

`instrukt todo freeze <slug>` parks work in the icebox;
`instrukt todo unfreeze <slug>` returns it to active consideration. Parking
preserves intent, artifacts and their history. A dispatch hold is a separate
fact, owned by `project/instrukt/spec/feature/work/dependencies-and-holds`.

`instrukt todo remove <slug>` withdraws work from default reads while retaining
its identity, state, artifacts and versions during a fixed seven-day recovery
window. Working resources (worktree, branch and workspace) are released
immediately. A todo's workspace is `<project>/workspace/<slug>/`, the
directory run-task writes to; an empty workspace is removed. Withdrawal never
deletes a worktree holding uncommitted changes or a workspace holding files:
it is refused and names that location. The next runner tick or todo command after the window expires
purges the retained local record; recovery after that window is not promised.
`instrukt todo reactivate` restores retained work by identity or an unambiguous
slug; a live same-slug entry prevents ambiguous restoration. Removal does not
erase the associated GitHub issue or its history. The withdrawing agent
removes that slug's pending documentation markers in a doc commit; withdrawal
does not remove them automatically.

`instrukt todo deliver <slug> --commit <sha>` records the confirmed completion evidence.
Repository-work delivery requires the actual delivered commit, supplied by the
integrator after the integration gate succeeds. Delivery evidence belongs in
the record; deciding whether a candidate passes the integration gate belongs
to workflow. Delivered entries are read-only. `instrukt todo work <slug>` may derive and
return the delivered workspace-removal obligation from retained evidence; it
changes no delivered record. The integrator executes removal outside evaluation.
Claim release and session cleanup belong to the work lane before handoff.

## Use cases

### UC-TD1: Record a piece of project work

```gherkin
  Given a developer has a request for their project
  When they file it with an explicit kind of work
  Then the project retains the request as a todo with a durable identity
```

### UC-TD2: Recover withdrawn work

```gherkin
  Given a developer withdrew a todo that remains within its recovery window
  And no active work conflicts with its name
  When they restore that todo
  Then its intent and recorded evidence are available under its original identity
  But when the recovery window has passed
  Then the withdrawn todo is gone and can no longer be restored
```

### UC-TD3: A developer is told how to enable work tracking

```gherkin
  Given a developer's project has work tracking switched off
  When they ask to file, change, prepare or work on a todo
  Then the command refuses and names the switch that would enable it
```

### UC-TD4: A developer reaches earlier work whose name was reused

```gherkin
  Given a developer delivered a todo and later filed new work under the same name
  When they ask for the todo by its name
  Then they get the new work
  But when they ask for a name that only earlier delivered or withdrawn work carried
  Then they are refused and shown the address of each earlier todo with that name
  And asking for one of those addresses shows its retained record
```

### UC-TD5: A developer withdraws work without losing what it produced

```gherkin
  Given a developer has a todo with work committed on its own branch
  When they withdraw the todo
  Then its working copies and branch are released
  And the branch's last commit is kept as evidence for recovering the todo
  But when one of its working copies holds uncommitted changes, or its task workspace still holds files
  Then the withdrawal is refused and names where that work is
```

### UC-TD6: Resume a completed change after an interrupted handoff

```gherkin
Given a developer's completed role work is committed and stamped in its own worktree
When the agent acting for the developer runs instrukt todo handoff and it is interrupted
Then the recorded handoff intent retains the completed work and intended todo head
When the agent retries the handoff
Then the todo branch reaches that stamped work once and the attempt is released
And another retry reports the confirmed handoff without another branch advance
But a divergent todo branch is held for reconciliation with the role work preserved
```

## Known caveats

None.

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
- [project/instrukt/spec/feature/runner/session-continuity](../runner/session-continuity.md)
- [project/instrukt/spec/feature/work/state-database](state-database.md)
- [project/instrukt/spec/feature/workflow/delivery-cycle](../workflow/delivery-cycle.md)
