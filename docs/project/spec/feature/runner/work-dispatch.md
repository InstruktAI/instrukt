---
description: Module-aware runner execution and automatic or manual todo dispatch without a separate consumer daemon.
visibility: public
---
# Work Dispatch — Spec

## What it is

The runner connects available work to agent sessions. A developer adopting
instrukt chooses when todos are picked up while retaining independent event
reactivity. The runner executes the work of enabled modules in one process.

## Canonical fields

### Execution contract

`instrukt run` and `instrukt tick` are entry points to the runner. The OS
scheduler supplies wake-ups. The runner has no consumer daemon beside it and no
separate state service; with a company's connector declared, an owned service keeps
`instrukt run` running
(`project/instrukt/spec/feature/runner/module-installation`). Enabled project webhook sources use the independent loopback ingress
receiver under `project/instrukt/spec/feature/runner/single-ingress-webhook-termination`;
that narrow intake outlives runner invocations and exposes no session-control API.
The runner's local state is the state database
(`project/instrukt/spec/feature/work/state-database`), written by
whichever instrukt process holds the writer lock for a local write transaction.
Network waits and reactions run outside that lock. A running process can read
events and react to them; the scheduled tick supplies time-based wake-ups.
These are execution forms of the
same runner, not separate services per module.

**Admitted configuration.** The runner works from one admitted generation of its
configuration: the machine layer, `agents.yaml`, each company home's document with
the secrets its connector references resolved, and each considered project's layer,
validated together. It is built when `run` or `tick` starts — an invalid machine
layer or `agents.yaml` refuses the start, with the finding — and again at every
wake. A valid result replaces the current generation for every part of the runner at
once; an invalid one leaves the current generation in force and its findings are
reported in `instrukt events status`. A project layer with a finding against a
module withholds only that module's own work: with a `modules.todos` finding (for
example, a project whose code is not on GitHub) the project gets no todo dispatch.
Whether the runner considers the project, and its canonical checkout
synchronization (the `main` fast-forward that feeds the blueprint), still follow
its switches; its other work is unaffected. No part
of the runner reads those files on its own
(`project/instrukt/policy/checks-and-guards`: a prerequisite is checked where it
enters; nothing running handles a missing one).

Company connections and the conversation relay follow the admitted generation
as it changes. Adding a connector starts it, changing its settings or tokens
renews its connection, and withdrawing it ends that connection. The
connector checks its provider permissions before taking member questions.

**Parts that depend on a service outside instrukt.** The conversation relay (on
T3's conversation source), the connectors (on the provider), the event readers (on
Redis) and history's extraction (on T3's database) and query responder (on cluster
Redis) each retry inside themselves until their service returns, and report their
state in `instrukt events status`; a part never ends the runner because its
service is away. A wake that cannot take the runtime-dispatch lock within its wait
is skipped and the next wake tries again; `instrukt events status` shows the last
skipped wake with its time and cause until a wake that is not skipped. Lock waits are measured on the clock that
measures the limits of started programs
(`project/instrukt/spec/feature/configuration/layered-configuration`). The
runner ends only when it cannot start: no home, the runner lock held by another
runner, a state database it cannot open or one newer than it knows
(`project/instrukt/policy/checks-and-guards`: an outside failure is retried, a
defect is fixed in the code).

| Capability | Runner responsibility |
| --- | --- |
| `core` | Always available; documentation, configuration and publication do not require background work. |
| `todos` enabled | Start eligible todo orchestrators through the compiled adapter; they delegate workers natively. |
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
it through `instrukt todo work`; any other caller is refused as "claimed
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

What each enabled module, declared source and declared connector installs is
owned by `project/instrukt/spec/feature/runner/module-installation`. Core
installs none, and local finalize-ready recording needs no Redis.

The configuration agent changes desired settings through revision-checked config
commands, invokes `instrukt install`, and verifies `instrukt install --check`
with `ticks status` and, for events, `events status`. Config writes and ordinary
publication install nothing. Disabling stops that module's admissions on the
next effective-config evaluation; explicit convergence withdraws only unneeded
instrukt-owned units across all projects, retaining data and pending work.
A receiver or wake-up another enabled project needs stays installed.

### Canonical checkout synchronization

Before dispatch on each tick, the runner observes and fetches remote canonical
main and fast-forwards a clean canonical project checkout when it is behind.
Its post-merge hook publishes docs and hands the revision to enabled blueprint.
Dirty or non-fast-forward checkouts are reported and preserved; no force, stash
or branch switch is used. The operation runs for considered todo/event projects
with events independently switchable and consumes no GitHub API quota.
Every new phase branch starts only after a fresh origin fetch, from origin/main;
local main is never its base. An outstanding runner.main_failing event wakes
the integrator for the existing revert-first obligation, and creates no new
runner admission predicate or todo hold.

### Todo pickup

`runner.autostart` controls todos only. It is a boolean or a map
`{<kind>: bool}`, set globally and overridable in a project. These settings
do not enable or disable standing-agent event activation.

In automatic mode the runner dispatches eligible work: for a todo's lane it
starts the procedure that declares the flavor's `orchestrator` role (`todo-orchestrator`,
`project/instrukt/spec/feature/workflow/workflow-definition`), and that
session holds the claim and routes primitives in prose from the factual ledger.
In manual mode a todo awaits deliberate dispatch. Unclaimed active todos waiting
on configured human approval or a standing `--finalize-ready` handoff are
suppressed from re-dispatch until approval arrives or a kickback/new build resets
the handoff. The workflow supplies eligibility and assignment facts; the runner
does not replace workflow facts with a second state machine. A project executes
through either instrukt's workflow engine or another engine, never both.

Time determines whether an authorized task is due; due status does not replace
the todo pickup policy. Task timing belongs to
`project/instrukt/spec/feature/runner/task-timing`.

### Agent selection and intent checks

Dispatch resolves the procedure's role through `agents.yaml`, with per-task
overrides first, under `project/instrukt/spec/feature/configuration/agent-selection`.
Walk each ready profile's tier chain before moving to the next profile. Report
skipped profiles/models and reasons; skip unavailable/degraded marks until
expiry. instrukt's own starts use the last known local T3 provider snapshot, kept
current in the background (`project/instrukt/spec/feature/runner/session-continuity`),
to check installation, enablement, login, reported models and quota windows; a
failed or slow refresh never refuses a start, and the runtime's own refusal at
the start is the signal. Runner
starts and native assignment facts carry the resolved profile, kind, provider instance, model, effort and options. T3 CLI starts pass `--provider` and `--model`;
effort/options cannot be expressed there and remain explicit start evidence.
A rejection proven before creation permits the next chain entry; ambiguous
starts and failed sends preserve the original request for recovery. Exhaustion before a new runtime invocation retires that freshly reserved unbound attempt, retaining its no-start evidence; an uncertain existing request is never retired by absence. Exhaustion
releases the runner's own claim and reports an anomaly, setting no hold.

The responsible orchestrator delegates the intent checker natively on the
computer where the originating conversation is readable, using the configured
native instruction. Work records the attempt and native child id through
`todo attempt` and `todo bind`. The runtime delivers its result to its parent;
GitHub carries the checked record and stamp, not the conversation.

### Dispatch evidence

The runner starts or reuses lane orchestrators and due tasks through the adapter
selected by `runner.runtime`. Event activation, keyed singleton discovery and
instrukt event/approval delivery use that same selection.

instrukt derives the single todo branch `<slug>/work`; T3 creates and owns its
worktree. A repository todo starts with `t3 thread start <project> - --worktree
origin/main --branch <slug>/work --title <key>` (continuing from
`origin/<slug>/work` on another computer, or from a legacy `<slug>/prepare`
branch during upgrade). Creation refuses an existing local `<slug>/work` branch.
Recorded native placement is verified before binding. `kind: task` creates no
git worktree.

The orchestrator delegates workers through `runtime.dispatch` with shared
isolation. Repository workers inherit `todo-orchestrator`'s `<slug>/work`
worktree (`task-preparer` runs as a worktree-less child attempt on `kind: task`).
Reserve before native dispatch and bind the native id and matching placement
before work. One outstanding workspace/child attempt is the interference lock.
Completion and release commit together; the next role reviews or writes the
current shared todo head. The orchestrator coordinates only and keeps that
worktree clean between roles.

The runtime owns native parent linkage and results. Ambiguous child starts stay
outstanding for recovery from native task evidence. Tick and run never observe
children, compose their ending notices or turn stamps/verdicts into dispatch events.

### Deliberate pickup and switched-off modules

With `runner.autostart` false, a session picks up work deliberately with
`instrukt todo work <slug>`, which records the claim as automatic dispatch does.
Deliberate work entry ensures the `<slug>/work` branch and uses configured
`runtime.start_thread` to place its orchestrator in the todo's one managed
worktree, or wakes its existing bound orchestrator; invoking the record command
from an unrelated checkout does not place a workspace. A due task keeps its own
task execution placement.

<!-- planned:in-thread-orchestrator-self-resume -->
Once the work record passes this computer's unassigned claim to a resuming
recorded orchestrator (`project/instrukt/spec/feature/work/dependencies-and-holds`),
the runner dispatches that todo no further, and a reservation it already made
resolves to that retained conversation.
<!-- /planned:in-thread-orchestrator-self-resume -->

A command of a switched-off module refuses and names the door: `todo` names
`modules.todos`; `sessions` and `tick` refuse only
when both `modules.todos` and `modules.events` are off, and then name both;
with either on, they serve that module's work. Conversation demand, a
company home's declared connector
(`project/instrukt/spec/feature/company/conversations`), admits `sessions` as
well, to list the sessions conversations start and resolve their starts, and
its refusal then names that declaration too; it does not admit `tick`. `run`
serves those two modules, history and conversations: it refuses only when
todos, events and history are off and there is no conversation demand, and
then names the three switches and the connector declaration. `install` and `ticks status` always
work; `eval report` needs no switch and reports only the record it can access.

## Use cases

### UC-RD1: Delegate todo pickup at the project's chosen pace

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

### UC-RD2: A developer is told how to enable the runner

```gherkin
Given a developer has work tracking, event responses and history switched off and no company has declared a connector
When they ask the runner to run, to wake up, or to start a session
Then the command refuses and names what would enable it
But when a company declares a connector
Then the runner runs with work tracking and event responses off
```

### UC-RD3: A developer's todos are picked up while their project's event responses are not admitted

```gherkin
Given a developer has switched on automatic pickup and event responses for their project on a computer without the event transport
When their project has a todo ready for pickup
Then the todo is picked up
And the developer sees in status that the project's event responses are not admitted, with the setting they need
```

## Known caveats

- Re-creating a deleted orchestrator while its todo worktree remains requires
  safe worktree removal first or the runtime's existing-worktree start where
  available (T3 MCP); T3's CLI cannot bind it. Waking a settled retained thread
  uses its existing workspace.
- T3 host CLI cannot express effort/options; native delegation receives them as resolved facts.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/runner/task-timing](task-timing.md)
- [project/instrukt/spec/feature/workflow/delivery-cycle](../workflow/delivery-cycle.md)
- [project/instrukt/spec/feature/runner/module-installation](module-installation.md)
