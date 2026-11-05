---
description: Module-aware work dispatch, event activation, session capabilities, task timing and anomaly reporting without a daemon.
visibility: public
delivered_by: [instrukt-first-build]
---
# Runner — Spec

## What it is

The runner bounded context connects eligible work, received events and due
times to agent sessions. It owns dispatch, the sessions facade, event-source
translation and time eligibility, including anomaly reporting. It executes
the enabled modules' work in one process, entered through `instrukt run` or
`instrukt tick` and woken by the OS scheduler. It has no daemon. The work
context owns work content, artifacts and stamps; workflow owns
authorization and derived workflow state.

## Canonical fields

### Ubiquitous language

**Runner invocation**
: One execution that performs the available work of enabled modules.

**Todo pickup**
: Dispatch of eligible recorded work, governed by automatic or manual mode
  and per-kind autostart.

**Standing agent**
: An agent assigned a procedure that answers subscribed events.

**Subscription**
: Procedure frontmatter identifying the instrukt events that activate its
  responsibility.

**Event source**
: A capability that obtains an outside or runtime occurrence and translates
  it into instrukt's event vocabulary.

**Session binding**
: Work’s runtime and session id per todo and role, read and written by the
  runner through work’s interface; standing-agent bindings belong to runner.

**Runtime mapping**
: Global configuration connecting facade operations and declared capabilities
  to the runtime the developer chose, as command templates instrukt fills
  for the calls it makes, plus agent-facing history templates. A feature is
  available only when the chosen runtime fills every template it needs.
  instrukt ships an entry for T3 Code that a developer may choose; no code
  detects or chooses a runtime.

**Linked-session input**
: Human words typed in one session also reaching a linked session with their
  human attribution, when the runtime mapping supports that capability.

**Due task**
: A prepared, authorized task whose recorded schedule makes it eligible for
  execution at the observed time.

**Catch-up**
: A task's decision to execute once or skip occurrences missed while the
  runner was unavailable.

**Wake-up**
: An outside invocation that gives instrukt an opportunity to consider time
  and available work; it is not a task-specific scheduler decision.

**Anomaly**
: A failure requiring attention that waits in no agent thread.

### Ownership boundary

- **Owns:** Module-aware runner execution; todo dispatch policy; procedure
  activation from events; the sessions facade and session reuse behavior;
  runtime capability translation, including linked-session input; time
  eligibility and missed-run decisions; anomaly-notification routing; the
  meaning and writes of its tables in the state database (the event log,
  consumer cursors and handled marks, request validators, watched-ref
  observations, executions, standing-agent bindings and starts, held task
  failures and the last wake-up), whose schema is published in
  `local/projects/instrukt/spec/feature/work/state-database`.
- **Does not own:** Work content, approval, workflow derivation or step
  completion evidence, owned by work and workflow;
  configuration storage and interpolation, owned by configuration;
  subscription document validation and delivery, owned by docs and publication; pushed-revision graph generation, owned by
  blueprint;
  session lifecycle, input delivery, restart, revival, handover,
  transcripts and their search, cleanup and subscriptions to session
  activity, all owned by the runtime and reached only through the runtime
  mapping's templates. Events, schedules and dispatch decisions are the
  runner's own; a runtime's task, schedule or event features are not used.
- **Published contracts:** `instrukt run`, `instrukt tick`, `instrukt sessions
  run|send|alive|conversation|link|end`, `instrukt sessions run [--procedure <id>] [--args <text>] [--brief <file|->] [--worktree <name>] [--todo <slug>]`
  (procedure required except setup-only `run --worktree <name> --brief <file|->`;
  the runtime creates/reattaches it and the binding records its path as `worktree.path`), `instrukt sessions link --remove <a> <b>`, `instrukt ticks status`, and the wake-up
  registration performed by `instrukt install`; the five-state session
  contract; every sessions facade command accepts `--json`; linked-session input capability; instrukt event vocabulary,
  including `runner.upstream_changed`, `runner.ci_completed`, `runner.main_ahead`, `runner.main_failing` and
  `workflow.finalized`.
- **Allowed dependencies:** configuration supplies effective machine and
  project settings; publication supplies project checkout paths; docs supplies subscribed procedures; work
  supplies records and session identity, and the state database's file,
  writer lock and migrations; workflow supplies assignments; the runtime and
  GitHub supply execution and external observations. These
  relationships are specified in the context map position below.
- **Implementation roots:** `instrukt-runner` realizes this context, shared by
  `todos` and `events`, and depends on `instrukt-core`. `instrukt-todos`
  depends on `instrukt-runner`; `instrukt-cli` wires these crates together.

`runner.autostart` is a boolean or a map `{<kind>: bool}`, available in both
configuration layers and controlling todos only. `runner.notify` is global-only.
Project-layer `events.watch: [{remote, ref}]` selects refs checked each tick.

### Aggregates

#### Runner invocation

- **Root entity:** The current invocation.
- **Contains:** Effective module enablement and todo pickup policy, and the
  work selected for dispatch during that execution.
- **Invariants:** Execution follows the enabled modules. Todo pickup mode
  controls todos only; event activation remains independently switchable.
  One process handles those enabled responsibilities.
- **Identifier:** The invocation itself; no new durable invocation identifier
  is established by this contract.
- **Lifecycle:** Begins at a runner entry point and ends when that invocation
  exits. One runner lock per machine, `~/.instrukt/state/runner.lock`,
  serializes invocations; a tick exits at once while `instrukt run` holds it.
  The last wake-up is a row of the state database.

#### Standing-agent session binding

- **Root entity:** The assignment's session binding.
- **Contains:** Subscribing procedure and project, configured runtime identity and
  runtime-native session identity.
- **Invariants:** Binding lookup plus `alive` determines reuse. Completion
  requires the work record's artifact and stamp. Linked-session input is a
  declared runtime capability.
- **Identifier:** (subscribing procedure, project). Todo-role bindings belong to work, not this aggregate.
- **Lifecycle:** A start records session identity; subsequent dispatch observes
  it before reuse. Missing completion evidence after execution ends, or a gone
  session, causes one send to the same session, then an anomaly if still missing.

#### Event activation

- **Root entity:** An event's activation of a subscribed responsibility.
- **Contains:** The event's meaning and the subscribed procedure it selects.
- **Invariants:** Subscriptions use instrukt's vocabulary. Event enablement
  determines activation independently of todo pickup. An upstream event is
  not authorization to reconcile the fork.
- **Identifier:** Event identity and subscribed responsibility; the standing-agent session is keyed by (subscribing procedure, project).
- **Lifecycle:** A source supplies an event, the runner resolves subscriptions
  and starts or wakes the responsible session. The event log in the state
  database retains idempotency keys, per-consumer cursors and per-consumer
  handled marks across restarts, so each consumer is handed an event once;
  handlers reconcile their subject idempotently.

#### Task timing

- **Root entity:** The task's timing decision.
- **Contains:** Its recorded schedule, missed-run policy and the occurrence
  being considered. The task record itself belongs to work.
- **Invariants:** Instrukt decides due times and catch-up; an outside scheduler
  supplies a generic wake-up. Authorization and todo pickup remain separate
  eligibility conditions.
- **Identifier:** The task and its scheduled occurrence; last-fired accounting is a stamp on the task record. An in-flight task
  is not fired again; failed dispatch counts as not fired.
- **Lifecycle:** Preparation supplies the task schedule; wake-ups evaluate
  due work. A recurring schedule supplies later occurrences without per-task
  external scheduler registration.

### Domain events emitted

The first build reads watched upstream refs with `git ls-remote` and uses
conditional requests for an active integrate candidate's CI status per SHA.
Polling follows the cost contract in
`local/projects/instrukt/spec/feature/runner/event-activation`. Missed
ticks are caught by the next state read. Later webhook receipt only prompts a
tick; it does not carry authoritative state or require reliable delivery.

| Event | Emitted when | Payload | Idempotency key | Consumer |
| --- | --- | --- | --- | --- |
| `runner.upstream_changed` | A watched upstream ref changes | Project, remote, ref, observed revision | Project, remote, ref, revision | `general/procedure/upstream-watch` |
| `runner.main_ahead` | Local main is ahead of remote main and no candidate is pending | Project, local main revision, remote main revision | Project, local main revision, remote main revision | Workflow's subscribed integrator |
| `runner.ci_completed` | The integrate candidate's CI result completes, or the project has no CI | Project, candidate SHA, CI result (`success`, `failure` or `no_ci`) | Project, candidate SHA, CI result | Workflow's subscribed integrator |
| `runner.main_failing` | The CI of remote canonical main's head commit completes red; a red result on a commit that is no longer the head is not observed, as the head's own CI covers the combined state | Project, failing head commit | Project, head commit: a commit's red result is reported once, and a new head is observed afresh | Workflow's subscribed integrator, woken to restore the canonical branch (revert first, repair as separate work); the event authorizes nothing by itself |

### Domain events consumed

| Event | Producer | Payload | Idempotency key | Reaction |
| --- | --- | --- | --- | --- |
| `workflow.finalized` | work’s finalize-ready stamp, expressing workflow’s handoff | Project, todo, finalized revision | Project, todo, finalized revision | Wake the subscribed standing integrator for finalize-ready work |

The state database's event log persists these events with per-consumer
cursors, handled marks and idempotency keys; a consumer's handled mark
commits with the record of its delivery. Consumers resume from their cursors
and reconcile their
subject against current state. Runtime session observations supply the five
facade states; they do not expand the first-build event vocabulary or replace
record-backed completion.

### Use cases (index of feature specs)

- **Work dispatch** — `local/projects/instrukt/spec/feature/runner/work-dispatch`
  — choose automatic or deliberate pickup of eligible work.
- **Session continuity** — `local/projects/instrukt/spec/feature/runner/session-continuity`
  — session reuse, five-state observation, sharing a human clarification
  with linked collaborators, working on T3 Code once it is chosen, looking up
  earlier conversations, and refusing a feature the chosen runtime cannot
  carry.
- **Event activation** — `local/projects/instrukt/spec/feature/runner/event-activation`
  — learn about upstream changes before deciding to reconcile a fork.
- **Task timing** — `local/projects/instrukt/spec/feature/runner/task-timing`
  — arrange future work and recover missed recurring occurrences.
- **Anomaly notification** — `local/projects/instrukt/spec/feature/runner/anomaly-notification`
  — notice unattended failures while keeping questions in their threads.

### Governing designs and decisions (index)

- `local/projects/instrukt/design/architecture/overview` — module boundaries
  and the CLI's execution shape.

### Context map position

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| configuration → runner | Customer / Supplier | Runtime mapping, module switches and runner settings |
| workflow → runner | Customer / Supplier; Published language | Computed state, transitions and finalize-ready event used for dispatch |
| work → runner | Customer / Supplier | Work reads and command-contract writes: claims and reflection flushes, last-fired stamps, automatic holds, expired-record purge and per-role session bindings |
| runner → work | Customer / Supplier | Event-log append used when the finalize-ready stamp records `workflow.finalized` |
| docs → runner | Customer / Supplier | Procedures and their event subscriptions |
| publication → runner | Customer / Supplier | Project checkout paths recorded in publication state |
| work → runner | Customer / Supplier | The state database's file, writer lock and migrations, holding the runner's tables |
| runner → workflow | Published language | `runner.ci_completed` and `runner.main_ahead` wake the integrator to assess candidates and direct commits; `runner.main_failing` wakes it to restore a failing canonical branch |

The runtime mapping and event-source boundary form an **Anti-corruption layer
(ACL)** translating session operations and external observations. These are
external boundaries, not additional bounded contexts. The runtime side of the
ACL is configuration only: each runtime, including the T3 Code entry instrukt
ships, is reached through its mapping entry's templates, never through a code
path of its own
(`local/projects/instrukt/spec/feature/runner/session-continuity`).

Todo dispatch reads work and workflow through runner-declared interfaces,
implemented by the todos crate and wired by the CLI. Work’s command contract
covers every runner mutation: recording and reflecting execution claims,
flushing pending reflection on ticks, writing last-fired stamps, setting
automatic holds, purging expired retained records, and reading and writing
per-role session bindings. Work owns validation and persistence of those writes.

## Known caveats

None beyond those recorded in the feature specs.
