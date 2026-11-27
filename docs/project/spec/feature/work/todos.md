---
description: Project work items with explicit kinds, durable intent, retained artifacts and recoverable withdrawal.
visibility: public
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

`instrukt todo create <slug> <content> --kind <kind> --approved` writes the
entry, initial state and first input version in one transaction. Omitting
`--approved` leaves the record unchanged. Invalid content, kind, dependency or
grouping refuses the write. `--kind bug` is refused with the address of the
bug-report command. `--human-involved true|false` records audit attribution.

`--after <slugs>` records prerequisites. `--hold --reason <text>` records a
dispatch hold in the creation transaction. `--icebox` sets the initial status
to iceboxed. For a task, `--agent <name> [--tier light|standard|strong|premium]`
sets an optional agent preference.

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

### Bug reports

`instrukt bugs report '<description>'` is the front door for filing a defect
(`general/procedure/bug-reporting`).

- **Without `--approved`** it files nothing. It prints the pre-filing
  questions to stderr and exits 0: serving the questions is that invocation's
  successful outcome. Only this output names how to complete the filing.
- **With `--approved`** it files a bug-kind todo in the owning project through
  the same creation transaction as `instrukt todo create`, with the
  description as the reported symptom and `--body <text>` beneath it. The bug
  enters preparation as every bug does
  (`project/instrukt/spec/feature/workflow/preparation`). `--slug <slug>`
  names it; without one the slug is `fix-` followed by the description's
  words, at most 40 characters. `--human-involved true|false` is the
  audit-only declaration `instrukt todo create` takes. The output names the
  filed slug and its reflection, as `instrukt todo create` does.
- **The owning project** is `--project <slug>`: a project publication state
  records on this computer (`project/instrukt/spec/feature/publication/instrukt-status`,
  `projects`), filed into its checkout. Every `--project` of the work reads
  (`instrukt todo list`, `instrukt bugs list`, `instrukt bugs suppressions`)
  resolves the same way and is refused the same way, so a filer outside the
  owning project, such as a company's agent in its home, faces every question
  from where it stands. Without `--project` it is the project
  the caller stands in. An unknown slug, or no `--project` outside an adopted
  project, is refused before anything is filed, naming `--project` and the
  projects this computer records.

The questions, each with the sources it names:

| Question | What the filer faces |
| --- | --- |
| Fix in context or file | Is the repair documentation-only, or a bounded code change of about five minutes: the approach known, the edit bounded, needing neither a plan, a builder nor a handoff (`sdlc/policy/todo-creation-discipline`)? Then the filer makes it at once in a short-lived one-off worktree and nothing is filed; finding it supplies the authority. A change whose correctness is contested belongs in the lifecycle. A company's agent never holds repair authority: for it the answer is always to report. |
| Bug or change | Which current contract does the behaviour break? A failure shows that something happened, not that it is wrong. Where no current contract states otherwise it is not a bug: file a change with `instrukt todo create <slug> <content> --kind change --approved` only when the behaviour is wanted, and otherwise file nothing. A report whose premise is that nothing followed up first establishes whether the sanctioned recovery was attempted. |
| Validity | Have the intended outcome and every contract it rests on converged, with no pending authoring, review or commit still expected to change them? A todo enters the lifecycle valid (`sdlc/policy/todo-creation-discipline`); work that has not converged is not filed through this door yet. |
| Evidence | An occurrence or a dangerous path in hand (`sdlc/concept/defect-evidence`). The report puts the observations and their addresses first, and keeps any causal account apart from them: each transition with its evidence, the alternatives checked and what remains unknown, or that the cause is not established. |
| Duplication | Already reported, already being fixed, or a triaged known issue? In the owning project: `instrukt todo list --project <slug>`, `instrukt bugs list --project <slug>` and `instrukt bugs suppressions --project <slug>` (`project/instrukt/spec/feature/work/known-issue-suppression`). |
| Ordering and dependency | Downstream of, or dependent on, work already in flight? Route it to that work under `sdlc/policy/todo-creation-discipline` instead of filing. |
| Owning project | The project that owns the defect, named with `--project`. A symptom that stops once the observing project's own configuration matches an instrukt default is that project's to fix, there. |

The output ends with the approved form:
`instrukt bugs report --approved '<description>' [--slug <slug>] [--body <text>] [--project <slug>] [--human-involved true|false]`.
The command verifies that the questions were faced, never that the answers
are true: judging the answers stays with the filer and with preparation.

`instrukt bugs list [--project <slug>] [--json]` lists the open bug-kind todos
of the project the caller stands in, or of the named project: every bug that
is neither delivered nor removed, with its slug, status and phase. Like
`instrukt todo list` it reads only this computer's state database.

### Task schedule

A task record holds a one-off `at` or recurring `cron` schedule and its
missed-run policy, `catch_up: once` by default or `skip`, set at creation with
`--catch-up once|skip`. Creation sets the
schedule with `--schedule <value>`; `instrukt todo update <slug> --schedule <value>` changes it afterwards (an empty value clears it). Values use an
ISO-8601 instant for `at` or `cron:<minute hour day month weekday>` for `cron`.
Timing and firing decisions belong to the runner; work retains the schedule
and task-firing stamps.

A task record carries an optional agent/model preference. Dispatch consults
that task preference before the role mapping in `~/.instrukt/agents.yaml`.

### Reads and changes

`instrukt todo list [--project <slug>]` reads the work of the caller's
project, or of the named project resolved as for a bug report (Bug reports,
above). Active work is the
default view; iceboxed, removed and delivered work are available through the
corresponding listing selections. Ordering is derived from readiness and
dependencies. `instrukt todo show <slug>` exposes the record, current evidence,
holds, prerequisites, group context, execution claim and per-role session
attempt bindings (runtime, id, key, attempt, branch, base, returned worktree
path and release evidence), including released attempts. These bindings form the
assignment child set exposed as `sessions.bound`; `claim.held` exposes whether
the execution claim remains held. `instrukt todo list` and
`instrukt todo show <slug>` read only this computer's state database and never
contact GitHub; they show the todos as this computer's record holds them. When
and what is read from GitHub is the contract in
`project/instrukt/spec/feature/work/github-record`.

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

### Native role-attempt recording

`instrukt todo attempt <slug> --role <role> [--procedure <id>] [--args <text>]
--brief <file|->` records an assignment for the claim holder. It includes the
supplied role, attempt number, stable semantic key and display title.
Repository assignments capture the recorded worktree, branch and current
clean head. Task child assignments omit Git coordinates.

At most one workspace or child reservation is open per todo. An identical
reservation retry returns its retained inputs. Unresolved dispatch suppresses
another start until `--not-started --attempt <n> --reason <text>` records
no-start evidence. The reply can include `prior_session`, the latest earlier
session bound for the same role on that runtime, with the retained attempt
number, dispatching session and selection when recorded.

`instrukt todo bind <slug> --role <role> --attempt <n> --session <runtime:id>
[--worktree <path>] [--branch <branch>] [--base <sha>]` associates a native
session with the reservation. The claim holder supplies matching identity
and placement. Repository bindings require the captured clean head and branch.
Task child bindings omit worktree, branch and base. Changed coordinates or a
changed session on a bound attempt are refused. A repeat of the same binding
succeeds. The claim and Git refs stay in place.

### Role completion and failed attempts

Successful completion and attempt release commit together under
`project/instrukt/spec/feature/work/stamps`. For a workspace attempt, the
recorded head is already on the bound branch.

`instrukt todo attempt <slug> --role <role> --attempt <n> --failed --reason <text>
--evidence <file|->` records a stopped child's failure and releases its attempt.
The evidence must match the bound session and placement. A workspace requires
a clean tree and the recorded branch and perceived head. Replay returns the
retained failure. A refusal leaves the attempt open.

### Supported metadata and listing selections

`instrukt todo update <slug>` admits `--slug <new-slug>`, `--kind <kind>`,
`--schedule <value>`, `--group <parent>`, `--after <dependencies>`,
`--hold --reason <text>`, `--release`, `--unclaim` and, for a task,
`--agent <name> [--tier light|standard|strong|premium]`. Relationship and hold
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

<!-- planned-change:project-working-dirs-under-dot-instrukt -->
`instrukt todo remove <slug>` withdraws work from default reads while retaining
its identity, state, artifacts and versions during a fixed seven-day recovery
window. Working resources (worktree, branch and workspace) are released
immediately. A todo's workspace is `<project>/workspace/<slug>/`, the
directory run-task writes to; an empty workspace is removed. Withdrawal never
deletes a worktree holding uncommitted changes or a workspace holding files:
it is refused and names that location. The withdrawal records what it is about to
release before it releases anything; until it ends, other changes to the todo
are refused, and a withdrawal interrupted part-way is completed by running
`instrukt todo remove <slug>` again. The next runner tick or todo command after the window expires
purges the retained local record; recovery after that window is not promised.
`instrukt todo reactivate` restores retained work by identity or an unambiguous
slug; a live same-slug entry prevents ambiguous restoration. Removal does not
erase the associated GitHub issue or its history. The withdrawing agent
removes that slug's pending documentation markers in a doc commit; withdrawal
does not remove them automatically.
<!-- change:project-working-dirs-under-dot-instrukt -->
`instrukt todo remove <slug>` withdraws work from default reads while retaining
its identity, state, artifacts and versions during a fixed seven-day recovery
window. Working resources (worktree, branch and workspace) are released
immediately. A todo's workspace is `<project>/.instrukt/workspace/<slug>/`, the
directory run-task writes to; an empty workspace is removed. Withdrawal never
deletes a worktree holding uncommitted changes or a workspace holding files:
it is refused and names that location. The withdrawal records what it is about to
release before it releases anything; until it ends, other changes to the todo
are refused, and a withdrawal interrupted part-way is completed by running
`instrukt todo remove <slug>` again. The next runner tick or todo command after the window expires
purges the retained local record; recovery after that window is not promised.
`instrukt todo reactivate` restores retained work by identity or an unambiguous
slug; a live same-slug entry prevents ambiguous restoration. Removal does not
erase the associated GitHub issue or its history. The withdrawing agent
removes that slug's pending documentation markers in a doc commit; withdrawal
does not remove them automatically.
<!-- /planned-change:project-working-dirs-under-dot-instrukt -->

`instrukt todo deliver <slug> --commit <sha>` records the confirmed completion evidence.
Repository-work delivery requires the actual delivered commit, supplied by the
integrator after the integration gate succeeds. Delivery evidence belongs in
the record; deciding whether a candidate passes the integration gate belongs
to workflow. Delivered entries are read-only. `instrukt todo work <slug>` derives and
returns what a delivered todo still owes of its worktree and work branch from
retained evidence; it changes no delivered record.
Claim release belongs to the work lane before integration handoff. After delivery
or withdrawal, the work record retains its recorded `sessions:` bindings and
attempt evidence readable by identity (`instrukt todo show <slug>@<id-prefix>`)
for audit and for identifying the todo's recorded orchestrator sessions
(`project/instrukt/spec/feature/configuration/layered-configuration`).

Work releases a delivered todo's worktree and work branch when the runner
asks, on the computer whose work record holds the todo
(`project/instrukt/spec/feature/runner/worktree-reclamation`). The worktree
goes first, through the release a withdrawal uses
(`project/instrukt/spec/feature/work/state-database`): read and checked at
its path, moved aside with git's own worktree move, read and checked again,
then removed with git's own worktree remove or moved back. A worktree that
holds uncommitted changes, is locked, has a role attempt open or is no longer
the todo's stays where it is. The work branch is then deleted locally and on
origin, each only where it holds no commit beyond the head the todo handed to
integration; a branch that holds more stays. The cleanup is owed while the
worktree, the local work branch or the origin work branch remains, whichever
of them the computer holds, and an origin branch already gone counts as
removed. This release records no intent and writes no marker:
git shows of the worktree and of each branch whether it is still there, and a
release that stopped part-way is continued by the next from what git shows
(`sdlc/policy/state-storage`).

One such release runs at a time on a computer. Work holds the per-computer
lock `~/.instrukt/state/delivered-release.lock` for the length of a release,
across its git and network steps and outside the writer lock. It takes the
lock without waiting: a release asked for while another holds it does nothing,
the cleanup stays owed, and the runner asks again on its next invocation. No
command takes this lock.

Prior slugs are retained with the immutable todo identity in the work record. Pending-document target reads use that history and the recorded workspace assignment, preserving original `<slug>/work` (and legacy prepare) branches and ownership across successive renames.

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

### UC-TD6: Role-branch handoff (retired)

Retired: every role commits directly in the todo's single shared worktree.

### UC-TD7: Record a native session against a reservation

```gherkin
Given a todo has an outstanding workspace attempt
When the claim holder binds a native session and matching workspace coordinates
Then instrukt records that session on the attempt
But different bound coordinates or a different session are refused
```

### UC-TD8: Recover an interrupted attempt reservation

```gherkin
Given a todo has an outstanding unbound attempt
When the claim holder repeats the reservation
Then it returns the same attempt and assignment
And another start is admitted after recorded no-start evidence
```

### UC-TD9: An agent faces the pre-filing questions before reporting a defect

```gherkin
Given an agent has met behaviour of the system it works through that appears to break a current contract
When it reports the defect without confirming it has faced the questions
Then it is shown the questions to face: fix in context or file, bug or change, validity, evidence, duplication, ordering and owning project
And it is told how to complete the filing once they are faced
And nothing is filed
And the report counts as answered, not failed
```

### UC-TD10: An agent files a faced defect report into the project that owns it

```gherkin
Given an agent has faced the pre-filing questions for a defect owned by a project on its computer
When it reports the defect, confirming the questions were faced, and names that project
Then that project holds a new bug carrying the reported symptom and the agent's account
And the bug enters preparation as every bug does
But when it names a project this computer does not hold
Then nothing is filed and it is told which projects it can name
```

### UC-TD11: An agent checks which defects are already reported

```gherkin
Given a project holds open bugs beside other work and bugs already delivered
When an agent asks for that project's bugs
Then it sees each open bug with its state
And no other kind of work and no delivered or withdrawn bug is listed
```

### UC-TD12: An agent filing a bug as ordinary work is sent to the front door

```gherkin
Given an agent files a defect as ordinary work of the bug kind
When the filing is made, confirmed or not
Then nothing is filed
And the agent is told that a bug is reported through the bug front door, and how to reach it
```

### UC-TD13: Inspect the previous attempt's binding

```gherkin
Given an earlier attempt has a session bound on the selected runtime
When a later attempt for that role is reserved
Then the reservation exposes the previous session and attempt number
And the earlier attempt and its evidence stay readable
```

### UC-TD14: A worker bound in one role is not bound to another

```gherkin
Given a developer's todo whose worker is bound to an attempt of one role
When the orchestrator binds that worker's session to a reserved attempt of another role
Then the binding is refused, naming the attempt that session holds and that the reserved attempt binds a new session
And the reserved attempt stays outstanding and unbound
And that worker's instrukt commands still run in the role it holds
```

## Known caveats

None.

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
- [project/instrukt/spec/feature/runner/session-continuity](../runner/session-continuity.md)
- [project/instrukt/spec/feature/work/state-database](state-database.md)
- [project/instrukt/spec/feature/workflow/delivery-cycle](../workflow/delivery-cycle.md)
