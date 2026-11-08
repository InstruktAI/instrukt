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
starts the procedure that declares the orchestrator role of the lane's flow
(`project/instrukt/spec/feature/workflow/workflow-definition`), and that
session holds the claim and executes the flow's own steps. In manual mode a todo
awaits deliberate dispatch. A released approved preparation handoff awaits integration and preparation-resource cleanup without reclaiming its preparer. After those obligations are confirmed, ordinary work pickup resumes. The workflow supplies eligibility and the next
assignment; the runner does not replace derived workflow state with a second
state machine. A project executes through either instrukt's workflow engine
or another engine, never both.

Time determines whether an authorized task is due; due status does not replace
the todo pickup policy. Task timing belongs to
`project/instrukt/spec/feature/runner/task-timing`.

### Agent selection and intent checks

Dispatch resolves the procedure's role through `agents.yaml`, with per-task
overrides first, under `project/instrukt/spec/feature/configuration/agent-selection`.
Walk each ready profile's tier chain before moving to the next profile. Report
skipped profiles/models and reasons; skip unavailable/degraded marks until
expiry. Before instrukt's own starts, check installation, enablement, login and
reported models. Runner starts and native assignment facts carry the resolved
profile, kind, provider instance, model, effort and options. T3 CLI starts pass `--provider` and `--model`;
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

instrukt derives the separate phase branches; T3 creates and owns their worktrees.
Preparation starts with `t3 thread start <project> - --worktree origin/main
--branch <slug>/prepare --title <key>`. After reviewed preparation integrates and
its local and origin branch are deleted, work starts from updated main with
`--worktree origin/main --branch <slug>/work`. Another computer uses
`--worktree origin/<slug>/work --branch <slug>/work`. Creation refuses an existing
local phase branch. Recorded native placement is verified before binding.

The orchestrator delegates workers through `runtime.dispatch` with shared
isolation. Work children inherit the work orchestrator's `<slug>/work` worktree; preparation
children inherit the preparation orchestrator's `<slug>/prepare` worktree. Reserve before native dispatch and bind
the native id and matching placement before work. One outstanding workspace-role
attempt is the work workflow's single admission. Completion and release commit
together; the next role reviews or writes the current shared todo head. The
orchestrator coordinates only and keeps that worktree clean between roles.

The runtime owns native parent linkage and results. Ambiguous child starts stay
outstanding for recovery from native task evidence. Tick and run never observe
children, compose their ending notices or turn stamps/verdicts into dispatch events.

### Deliberate pickup and switched-off modules

With `runner.autostart` false, a session picks up work deliberately with
`instrukt todo prepare <slug>` or `instrukt todo work <slug>`, which records the claim as automatic dispatch does. Preparation retains the
project-folder orchestrator. Deliberate work entry ensures the todo branch and
uses configured `runtime.start_thread` to place its orchestrator in the todo's
one managed worktree, or wakes its existing bound orchestrator; invoking the
record command from an unrelated checkout does not place a workspace. A due task
keeps its own task execution placement.

A command of a switched-off module refuses and names the door: `todo` names
`modules.todos`; `sessions`, `run` and `tick` refuse only
when both `modules.todos` and `modules.events` are off, and then name both;
with either on, they serve that module's work. `install` and `ticks status` always
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
Given a developer's project has work tracking and event responses both switched off
When they ask the runner to run, to wake up, or to start a session
Then the command refuses and names both switches that would enable it
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
