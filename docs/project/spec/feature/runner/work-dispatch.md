---
description: Module-aware runner execution and automatic or manual todo dispatch without an instrukt daemon.
visibility: public
delivered_by: [instrukt-first-build]
---
# Work Dispatch — Spec

## What it is

The runner connects available work to agent sessions. A developer adopting
instrukt chooses when todos are picked up while retaining independent event
reactivity. The runner executes the work of enabled modules in one process.

## Canonical fields

### Execution contract

`instrukt run` and `instrukt tick` are entry points to the runner. The OS
scheduler supplies wake-ups. The runner has no consumer daemon or separate state
service. Enabled project webhook sources use the independent loopback ingress
receiver under `project/instrukt/spec/feature/runner/single-ingress-webhook-termination`;
that narrow intake outlives runner invocations and exposes no session-control API.
The runner's local state is the state database
(`project/instrukt/spec/feature/work/state-database`), written by
whichever instrukt process holds the writer lock for a local write transaction.
Network waits and reactions run outside that lock. A running process can read
events and react to them; the scheduled tick supplies time-based wake-ups.
These are execution forms of the
same runner, not separate services per module.

| Capability | Runner responsibility |
| --- | --- |
| `core` | Always available; documentation, configuration and publication do not require background work. |
| `todos` enabled | Dispatch eligible todos through the configured workflow and sessions facade. |
| `events` enabled | Resolve event subscriptions and start or wake standing agents independently of todo pickup mode. |
| `blueprint` enabled | Its pushed-revision generation follows its own hook contract; enabling it does not create another runner. |

One runner lock per machine, `~/.instrukt/state/runner.lock`, serializes
invocations. A tick exits at once while `instrukt run` holds the lock. Publication state supplies each project's
checkout path beside its published commit; the runner considers projects
whose effective configuration (project over global) enables `todos` or
`events`, without a separate registry. A project whose code is not on GitHub
gets no todo pickup: todos need the project's code on GitHub
(`project/instrukt/spec/feature/work/github-record`).
Before starting a session for a todo, the runner refreshes the todo from
GitHub, so work starts from its current state, then commits a claim on the
item and reflects it to GitHub; a revision conflict means another computer
claimed it first, and this runner leaves the item. When GitHub cannot be
reached, the claim stays local and pending and no session starts; the next
tick refreshes the todo and reflects the claim, then dispatches, or drops the
claim on a conflict.

A claim is held by a computer, identified by the id created once in
`~/.instrukt/state/machine-id`. Like the rest of the home it is re-creatable;
deleting it orphans this computer's live claims until they are released. On that
computer the holder is a session bound to the todo, or the session that claimed
it through `instrukt todo prepare|work`; any other caller is refused as "claimed
elsewhere". After the claim, `instrukt todo update <slug> --unclaim` releases
it. The dispatcher records the handoff and retains conversations needed for
later rounds or human questions. The runtime owns session ending and cleanup.
Every tick also polls GitHub for the todos this computer has an interest in,
with conditional requests: the todos it holds claims on, one pickup listing per
project with `todos` on, and the prerequisites its own work is blocked on
(`project/instrukt/spec/feature/work/github-record`). Pickup considers the todos
of that listing. Every tick flushes pending work-record reflection under the
per-computer reflection lock; commands that write the record flush it under that
lock too. Reflection holds that lock across GitHub requests and takes the writer
lock only briefly for each local commit of what it learned. Lock order is always
reflection → writer; the local transaction releases the writer lock before a
flush acquires the reflection lock. Local readers take neither lock.

### Enablement and infrastructure

Infrastructure follows
`project/instrukt/spec/feature/runner/module-installation`. Core installs none.
Enabling todos demands only its shared wake-up and existing work/runtime
prerequisites; local finalize-ready recording needs no Redis. Enabling events
adds scoped Redis setup/checks and prepared retained readers. Only enabled
nonempty project sources on the designated ingress demand the independent
socket-held receiver. No consumer daemon is installed.

The configuration agent changes desired settings through revision-checked config
commands, invokes `instrukt install`, and verifies `instrukt install --check`
with `ticks status` and, for events, `events status`. Config writes and ordinary
publication install nothing. Disabling stops that module's admissions on the
next effective-config evaluation; explicit convergence withdraws only unneeded
instrukt-owned units across all projects, retaining data and pending work.
A receiver or wake-up another enabled project needs stays installed.

### Todo pickup

`runner.autostart` controls todos only. It is a boolean or a map
`{<kind>: bool}`, set globally and overridable in a project. These settings
do not enable or disable standing-agent event activation.

In automatic mode the runner dispatches eligible work: for a todo's lane it
starts the procedure that declares the orchestrator role of the lane's flow
(`project/instrukt/spec/feature/workflow/workflow-definition`), and that
session holds the claim and executes the flow's own steps. In manual mode a todo
awaits deliberate dispatch. The workflow supplies eligibility and the next
assignment; the runner does not replace derived workflow state with a second
state machine. A project executes through either instrukt's workflow engine
or another engine, never both.

Time determines whether an authorized task is due; due status does not replace
the todo pickup policy. Task timing belongs to
`project/instrukt/spec/feature/runner/task-timing`.

### Agent selection and intent checks

Dispatch walks the task's optional agent/model preference first, then the
procedure's `dispatch` order, or global `agents.order`, including the chosen
thinking mode, and takes the first agent the configured adapter reports
available when it supports explicit selection. Availability checks are adapter
code. T3's CLI uses its project's defaults until the adapter can resolve a
provider/model pair; it reports that limitation rather than claiming to honour
agent order or thinking mode. Only a rejection proven to precede creation
permits fallback to another agent; an ambiguous start or failed send preserves
the existing dispatch for recovery. Exhausting a supported selection order is an
anomaly: the runner releases its own claim through work's command contract and
sends the anomaly through `runner.notify`; it sets no hold.

The runner on the computer holding the conversation dispatches its intent
check, where that conversation is readable; it is not copied to the remote
work store.

### Dispatch evidence

The runner starts or reuses a session through
`project/instrukt/spec/feature/runner/session-continuity`. The workflow
record's artifact and stamp establish completion of a step. A runtime reply,
successful process exit, or turn-completion event does not establish that
completion on its own.

Todo roles use the durable title key and start marker defined by
`project/instrukt/spec/feature/runner/session-continuity`. Allocate an attempt
once and retain it across retries. An unresolved marker is reconciled by exact
key discovery after restart; an empty listing leaves it pending for
`resolve-start`. T3 dispatches each role attempt into a new worktree from the
captured todo head. Roles of one todo run sequentially; completion is admitted
for the next role only after the fast-forward handoff defined by
`project/instrukt/spec/feature/workflow/delivery-cycle` is recorded. A review
releases its attempt without moving the todo branch.

### Deliberate pickup and switched-off modules

With `runner.autostart` false, a session picks up work deliberately with
`instrukt todo prepare <slug>` or `instrukt todo work <slug>`, which records the
claim as automatic dispatch does and makes that session the orchestrator; a due
task is picked up the same way.

A command of a switched-off module refuses and names the door: `todo` names
`modules.todos`; `sessions`, `run` and `tick` refuse only
when both `modules.todos` and `modules.events` are off, and then name both;
with either on, they serve that module's work. `install` and `ticks status` always
work; `eval report` needs no switch and reports only the record it can access.

### Use cases

#### UC-RD1: Delegate todo pickup at the project's chosen pace

```gherkin
Scenario Outline: Choose how development work is picked up
  Given a developer has adopted instrukt in a project with eligible development work
  And the developer has chosen <pickup> pickup for that project
  When the runner considers that work
  Then the work <outcome>

  Examples:
    | pickup     | outcome                                  |
    | automatic  | is dispatched to an agent                |
    | deliberate | awaits the developer's deliberate start  |
```

#### UC-RD2: A developer is told how to enable the runner

```gherkin
Given a developer's project has work tracking and event responses both switched off
When they ask the runner to run, to wake up, or to start a session
Then the command refuses and names both switches that would enable it
```

## Known caveats

- T3 supports worktree creation but not attachment. The shipped sdlc workflow
  uses per-role worktrees; a workflow requiring attachment is refused at
  enablement with that capability named.
- Preferred-agent selection on T3 has the limitation recorded in
  `project/instrukt/spec/feature/runner/session-continuity`.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/runner/task-timing](task-timing.md)
- [project/instrukt/spec/feature/workflow/delivery-cycle](../workflow/delivery-cycle.md)
- [project/instrukt/spec/feature/runner/module-installation](module-installation.md)
