---
description: Module-aware work dispatch, event activation, session capabilities, task timing and anomaly reporting without a consumer daemon, with opt-in signed ingress.
visibility: public
delivered_by: [instrukt-first-build]
---
# Runner — Spec

## What it is

The runner bounded context connects eligible work, received events and due
times to agent sessions. It owns dispatch, the runtime adapter, the sessions facade, event-source
translation and time eligibility, including anomaly reporting. It executes
the enabled modules' work in one process, entered through `instrukt run` or
`instrukt tick` and woken by the OS scheduler. It has no consumer daemon. A separate opt-in loopback webhook receiver accepts signed external events on the designated ingress computer. The work
context owns work content, artifacts and stamps; workflow owns
authorization and derived workflow state.

## Ubiquitous language

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

**Runtime adapter**
: A Rust trait implemented in code, selected explicitly by global
`runner.runtime`. T3 is shipped first over its published host CLI. The
adapter translates start-or-wake, start, send, state and conversation reads,
recognises caller identity without runtime I/O,
and reports optional capabilities. Configuration contains no runtime verbs.

**Singleton key**
: The standing procedure and project identity carried in its runtime thread
 title; the database binding is a cache. A per-key database lock admits one
 starter or deliverer at a time.

**Linked-session input**
: Human words typed in one session also reaching a linked session with their
  human attribution, when the runtime adapter supports that capability.

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

## Ownership boundary

- **Owns:** Module-aware runner execution; todo dispatch policy; procedure
  activation from events; the runtime-adapter trait, the sessions facade and session reuse behavior;
  runtime capability translation, including optional linked-session input; time
  eligibility and missed-run decisions; anomaly-notification routing; the
  meaning and writes of its tables in the state database (the
  event log, consumer cursors and handled marks, request validators,
  watched-ref observations, executions, cached standing-agent bindings, pending delivery targets, start markers and per-key locks,
  held task failures and the last wake-up), whose schema is published in
  `project/instrukt/spec/feature/work/state-database`.
- **Does not own:** Work content, approval, workflow derivation or step
  completion evidence, owned by work and workflow;
  configuration storage and interpolation, owned by configuration;
  subscription document validation and delivery, owned by docs and publication; pushed-revision graph generation, owned by
  blueprint;
  session lifecycle, input delivery, restart, revival, handover,
  transcripts and their search, cleanup and subscriptions to session
  activity, all owned by the runtime; instrukt calls only the adapter's
  released primitives. Events, schedules and dispatch decisions are the
  runner's own; a runtime's task, schedule or event features are not used.
- **Published contracts:** `instrukt run`, `instrukt tick`, the runtime-neutral
  `instrukt sessions run|send|alive|conversation|link|resolve-start` surface,
  `instrukt ticks status`, and wake-up registration by `instrukt install`;
  structured adapter operations, five-state observation, bounded conversation
  reads, optional linked input and notification; singleton key/title discovery
  and database locking. Every facade command accepts `--json`.
  Worktree creation returns the runtime path; reattachment requires a distinct
  runtime capability. Events include `runner.upstream_changed`,
  `runner.ci_completed`, `runner.main_ahead`, `runner.main_failing` and
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

## Canonical fields

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

- **Root entity:** The continuing responsibility's runtime binding.
- **Contains:** Canonical procedure, exact home, subject when identity is subject,
  and the configured runtime's session identity.
- **Invariants:** Activation acceptance is distinct from runtime resolution and
  event delivery. Exact runtime key-title discovery repairs the cached binding
  under its per-key database lock; status observation determines reuse. Domain
  completion requires its owning work record's evidence. Linked input is an
  optional adapter capability.
- **Identifier:** Procedure and exact home, plus subject when declared; todo-role
  bindings remain work-owned. A3's project-keyed adapter mapping is extended at
  the S6 home/subject-delivery boundary.
- **Lifecycle:** A3 owns serialized discovery/start-or-wake, pending fixed targets
  and ambiguous-start recovery. List/send failure leaves work pending and starts
  no replacement. S6 realizes continuing home/subject responsibility and eligible
  event delivery; routing itself resolves no session.

#### Event activation

- **Root entity:** An event's activation of a subscribed responsibility.
- **Contains:** The event's meaning and the subscribed procedure it selects.
- **Invariants:** Subscriptions use instrukt's vocabulary. Event enablement
  determines activation independently of todo pickup. An upstream event is
  not authorization to reconcile the fork.
- **Identifier:** Activation acceptance is keyed by occurrence ID, canonical procedure and exact home. Continuing responsibility identity includes procedure/home and subject when declared; runtime resolution belongs to A3/S6.
- **Lifecycle:** A source supplies a fact; one independent matcher durably accepts
  keyed activations from the current published index. Runtime execution follows
  that acceptance through A3/S6. Local publication, transport progress, activation
  acceptance and procedure delivery are distinct completion boundaries; handlers
  reconcile domain effects idempotently.

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

### Event-platform boundary

Event facts retain the complete project, company, person, none and unknown origin
vocabulary. Executable homes in this release are registered projects and the
platform; company/person resolution is an explicit unsupported capability, never
a conversion to another home. Unknown origin activates nothing. None selects a
platform responsibility. Only the emitting computer activates a matching
procedure: external events therefore activate on ingress, and a missing project
checkout there fails visibly without forwarding.

Published general/domain procedures reach every home of their declared kind.
A project-local subscribed procedure reaches only that project's origin; this
preserves instrukt's project-local reach as an adaptation of TeleClaude's tiers.

`instrukt run` hosts independent event readers. `instrukt tick` gives each a
bounded pending-then-new drain, retaining Redis groups between invocations.
A scheduled-only computer reacts on its next wake, within stream retention.
One consumer's slow handler or unavailable Redis does not block unrelated readers
or local work. SQLite writer-lock ownership is limited to local write sections;
network waits and reactions run outside it.

A1's local event log and local settled cursors remain. Redis owns transport
positions and bounded replay; SQLite occurrence marks protect a logical consumer
against the same event ID for seven days. Existing local event rows retain
retryable publication status and coordinates so a committed fact is published
later with its original ID. This supplies no separate outbox/inbox service and
no accepted ingress buffering during a cluster outage.

### Event naming and producer declarations

Existing vocabulary is retained explicitly: `runner.upstream_changed` →
`runner.upstream_changed`, `runner.main_ahead` → `runner.main_ahead`,
`runner.ci_completed` → `runner.ci_completed`, `runner.main_failing` →
`runner.main_failing`, and `workflow.finalized` → `workflow.finalized`.
The producing contexts remain owners of these five facts; subscriptions keep
these names. Event-platform types ported from `teleclaude.*` use `instrukt.*`.
No compatibility alias or second occurrence is emitted.

The canonical received boundary is the open event envelope, not a closed enum
of those five facts. The four runner declarations use project origin;
upstream changes name the observed repository revision as a commit reference,
main-ahead and main-failing name a repository-qualified commit, and CI completion
names the candidate commit (including the `no_ci` observation). Finalize-ready
uses project origin and a todo subject. The existing occurrence-key columns
below determine stable IDs, including producer type, independently of payload
serialization. Producer declarations expose the existing payloads as typed data.

### Domain events emitted

The first build reads watched upstream refs with `git ls-remote` and uses
conditional requests for an active integrate candidate's CI status per SHA.
Polling follows the cost contract in
`project/instrukt/spec/feature/runner/event-activation`. Missed
ticks are caught by the next state read. Signed native and GitHub requests also enter as ordinary cluster events; polling remains source reconciliation, with its own authoritative state.

| Event | Emitted when | Payload | Idempotency key | Consumer |
| --- | --- | --- | --- | --- |
| `runner.upstream_changed` | A watched upstream ref changes | Project, remote, ref, observed revision | Project, remote, ref, revision | `general/procedure/upstream-watch` |
| `runner.main_ahead` | Local main is ahead of remote main and no candidate is pending | Project, local main revision, remote main revision | Project, local main revision, remote main revision | Workflow's subscribed integrator |
| `runner.ci_completed` | The integrate candidate's CI result completes, or the project has no CI | Project, candidate SHA, CI result (`success`, `failure` or `no_ci`) | Project, candidate SHA, CI result | Workflow's subscribed integrator |
| `runner.main_failing` | The CI of remote canonical main's head commit completes red; a red result on a commit that is no longer the head is not observed, as the head's own CI covers the combined state | Project, failing head commit | Project, head commit: a commit's red result is reported once, and a new head is observed afresh | Workflow's subscribed integrator, woken to restore the canonical branch (revert first, repair as separate work); the event authorizes nothing by itself |

### Event-platform facts emitted

`instrukt.system.event.dead_lettered` ports
`teleclaude.system.event.dead_lettered`: subject is the failing stream entry,
origin is none, and data declares string fields `attempt_count`, `consumer`,
`consumer_id`, `dlq_destination`, `failing_stage`, `original_entry_id` and
`original_stream`. It reports a parked act, not a failing lifecycle state.
No signal/correlation or runtime-lifecycle producer is added by this declaration.

### Domain events consumed

| Event | Producer | Payload | Idempotency key | Reaction |
| --- | --- | --- | --- | --- |
| `workflow.finalized` | work’s finalize-ready stamp, expressing workflow’s handoff | Project, todo, finalized revision | Project, todo, finalized revision | Wake the subscribed standing integrator for finalize-ready work |

The local event log preserves source facts and retryable publication with stable
occurrence IDs. Scoped Redis groups own transport progress; settled SQLite
cursors retain local accounting. A consumer's occurrence mark commits with its
accepted outcome before ACK. Replay and duplicate protection are bounded, and
handlers reconcile their subject against current state. Runtime session observations supply the five
facade states; they do not expand the first-build event vocabulary or replace
record-backed completion.

## Event subjects and origins

The source events above concern a watched ref, a main revision or an
integrate candidate in their project origin. The event declarations define
their typed subjects (`project/instrukt/spec/feature/runner/event-envelope`).
`instrukt.system.event.dead_lettered` concerns the failing stream entry and
has no origin, as declared in Canonical fields.

## Use cases

- **Work dispatch** — `project/instrukt/spec/feature/runner/work-dispatch`
  — choose automatic or deliberate pickup of eligible work.
- **Session continuity** — `project/instrukt/spec/feature/runner/session-continuity`
  — session reuse, five-state observation, sharing a human clarification
  with linked collaborators, working on T3 Code once it is chosen, looking up
  earlier conversations, refusing a feature the chosen runtime cannot
  carry, and an agent carrying out an ending handed to it as its to do.
- **Event activation** — `project/instrukt/spec/feature/runner/event-activation`
  — learn about upstream changes before deciding to reconcile a fork.
- **Task timing** — `project/instrukt/spec/feature/runner/task-timing`
  — arrange future work and recover missed recurring occurrences.
- **Anomaly notification** — `project/instrukt/spec/feature/runner/anomaly-notification`
  — notice unattended failures while keeping questions in their threads.

- **Event facts and producer declarations** — `project/instrukt/spec/feature/runner/event-envelope`.
- **Discover declared event shapes** — `project/instrukt/spec/feature/runner/event-discovery`.
- **Scoped transport and publication recovery** — `project/instrukt/spec/feature/runner/event-transport`.
- **Independent retained reactions** — `project/instrukt/spec/feature/runner/event-consumers`.
- **Signed project sources** — `project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework`.
- **Single cluster intake** — `project/instrukt/spec/feature/runner/single-ingress-webhook-termination`.
- **GitHub provider translation** — `project/instrukt/spec/feature/runner/github-webhook-normalization`.
- **Published subscriptions and durable activation acceptance** — `project/instrukt/spec/feature/runner/procedure-subscription-routing`.

## Governing designs and decisions

- `project/instrukt/design/architecture/overview` — module boundaries
  and the CLI's execution shape.

## Context map position

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

The runtime adapter and event-source boundary form an **Anti-corruption layer
(ACL)** translating session operations and external observations. These are
external boundaries, not additional bounded contexts. The runtime side is a Rust
trait and code implementations; T3 is the first, over its host CLI. The explicit
configuration name selects the implementation
(`project/instrukt/spec/feature/runner/session-continuity`).

Event activation carries the exact home and optional subject through this seam.
S6 establishes that handoff's delivery feasibility; runtime lifecycle remains
runtime-owned.

Todo dispatch reads work and workflow through runner-declared interfaces,
implemented by the todos crate and wired by the CLI. Work’s command contract
covers every runner mutation: recording and reflecting execution claims,
flushing pending reflection on ticks, writing last-fired stamps, setting
automatic holds, purging expired retained records, and reading and writing
per-role session bindings. Work owns validation and persistence of those writes.

## Known caveats

- The target delivery contract requires durable acceptance, sequential/batched
  selection, procedure-first briefing and untrusted activation provenance. Its
  realization through A3's published runtime boundary is the S6 feasibility gate;
  ordinary conversational JSON is not evidence of equivalence.
- Company/person home execution, conversation channels and help desks are unavailable in this port; runtime lifecycle remains runtime-owned.


- UC-HR1 is retired: machine maintenance is outside instrukt; its id is not reused.
- UC-HR2 is retired: machine maintenance is outside instrukt; its id is not reused.
- UC-HR3 is retired: machine maintenance is outside instrukt; its id is not reused.
- UC-HR4 is retired: machine maintenance is outside instrukt; its id is not reused.
- Other caveats are recorded in the feature specs.
