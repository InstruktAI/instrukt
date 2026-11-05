---
description: The one SQLite database holding a computer's work record and runner state — its tables, its migrations, and instrukt as its single, transactional writer.
visibility: public
delivered_by: [instrukt-first-build]
---
# State Database — Spec

## What it is

A computer keeps instrukt's operational state in one SQLite database: the
local work record that stands in front of its GitHub reflection, and the
runner's state. instrukt is its only writer and its only interface: agents
work with the record through instrukt's commands. A write interrupted
part-way leaves the state as the last completed change left it.

The schema below is instrukt's own contract, versioned by its migrations. The work context owns the file,
its migrations, the writer rules and the work tables; the runner context owns
the meaning and the writes of the runner tables.

## Canonical fields

### Database file

- **Path:** `~/.instrukt/state/instrukt.db`, one per home, in WAL journal
  mode; SQLite keeps `instrukt.db-wal` and `instrukt.db-shm` beside it.
  instrukt creates it, at the current schema version, on the first command
  that needs it.
- **Holds:** the work tables and the runner tables below. Nothing else is
  stored in it.
- **Stays outside it:** the blueprint graph store (one complete graph per
  revision, as files under `~/.instrukt/state/blueprint/`), publication
  state, the global index, eval fingerprints, hook outcomes and the
  machine id. Their owners' contracts place them.
- **No file stores:** the work record and the runner's state have no other
  home. Directories left by earlier builds (`state/work/`, `state/runner/`)
  are neither read nor migrated.

### Writing

- Only instrukt writes, and it validates every change before committing it.
  Writing the database with any other tool is outside this contract.
- Every write transaction runs while its process holds the per-computer
  writer lock `~/.instrukt/state/writer.lock` for its local transaction.
  Network waits and reaction execution run outside that section, so a blocked
  consumer holds no unrelated local write hostage. SQLite's own locking is not
  the serialization contract.
- A change that spans several records commits in one transaction or not at
  all: a todo with its new stamp and revision, a finalize-ready stamp with
  its `workflow.finalized` event, a dependency or group change across several
  todos, a delivery's outcome with its handled marks and cursor.
- A committed transaction survives a process crash and a power loss
  (`synchronous = FULL`). An interrupted transaction leaves nothing of itself
  behind.
- Foreign keys are enforced. Removing a todo's row removes every row that
  belongs to it.
- The runner lock `~/.instrukt/state/runner.lock` still serializes runner
  invocations. It guards no data.

### Scope of what is stored

The database holds this computer's local items, and the last known copy of
each other todo of its projects as the runner's pickup listing last read it
(`project/instrukt/spec/feature/work/github-record`). A workflow phase
is derived, never stored: `instrukt todo list` and `instrukt todo show` give
it.

Values are stored as follows: timestamps as RFC 3339 text in UTC; booleans as
`0` or `1`; a session as `<runtime>:<id>`; an actor as JSON text, one of
`{"session": "<runtime>:<id>"}`, `{"github": "<login>"}` or
`{"person": "<name>"}`; other structured values as JSON text, readable with
`json_extract`. An empty optional value is `NULL`.

### Schema version and migrations

- `PRAGMA user_version` is the schema version. instrukt ships an ordered
  sequence of migrations and owns every one of them.
- Opening the database for writing applies each pending migration in its own
  transaction under the writer lock, and sets `user_version` in that same
  transaction. A read-only open never migrates.
- A database whose version is newer than the running instrukt knows is
  refused for reading and writing; the refusal names both versions and the
  door: install the newer instrukt.

### Work tables

Owned and written by work through its command contract. Reflection to
GitHub reads from these tables; the runner writes the rows it is entitled to
(claims, reflection flushes, pickup-listing copies, last-fired stamps,
automatic holds, the purge of expired retained records and per-role session
bindings) only through work's command contract.

**`todos`** — one row per local item. Key: `id`. Unique while live:
`(project, slug)` among `active`, `iceboxed` and `structural_holder` rows.

| Column | Meaning |
| --- | --- |
| `id` | Durable identity: 32 lowercase hex characters. |
| `project` | The project's slug. |
| `slug` | Project-local address. |
| `kind` | `feature`, `change`, `bug` or `task`. |
| `status` | Record status: `active`, `iceboxed`, `structural_holder`, `delivered` or `removed`. Never a workflow phase. |
| `created_at`, `created_by` | Creation time and actor. |
| `human_involved` | The audit-only creation declaration, or `NULL`. |
| `revision` | Incremented by every accepted change; the revision reflection writes. |
| `group_id` | The structural holder this todo is grouped under (`todos.id`), or `NULL`. |
| `group_slug` | The holder's last-known slug, for a holder not stored on this computer, or `NULL`. |
| `schedule` | A task's schedule: an ISO-8601 instant with offset, or `cron:<five fields>`; `NULL` for none. |
| `catch_up` | `once` or `skip`. |
| `agent`, `thinking` | A task's agent preference and thinking mode, or `NULL`. |
| `claim_machine`, `claim_session`, `claim_at` | The execution claim: holding computer, claiming session, time; `NULL` when unclaimed. `claim.held` is `claim_machine IS NOT NULL`. |
| `branch` | The work branch recorded at build start. |
| `delivered_commit`, `delivered_at`, `delivered_by` | Completion evidence. |
| `removed_at`, `removed_from` | Withdrawal time, starting the recovery window, and the status restoration returns to. |
| `removal` | Released working resources kept as recovery evidence (branch and head, worktrees). |
| `sequence` | The last record-order number given to a version or stamp. |

**`prerequisites`** — the dependency edges. Key: `(todo_id, prerequisite_id)`,
both `todos.id`; `prerequisite_id` may name a retained delivered todo.
Columns: `todo_id`, `prerequisite_id`, `slug` (the prerequisite's last-known
slug, for showing it and finding it by slug when it is not stored on this
computer).

**`artifacts`** — one row per named artifact. Key: `(todo_id, name)`.
Columns: `todo_id`, `name`, `address` (the provider address once
reflected; the artifact's identity on GitHub).

**`artifact_versions`** — retained versions, never rewritten. Key:
`(todo_id, sequence)`; `(todo_id, name)` references `artifacts`.

| Column | Meaning |
| --- | --- |
| `todo_id`, `name` | The artifact. |
| `sequence` | Place in the todo's record order, shared with stamps. |
| `digest` | `sha256:<hex>` of the content. |
| `writer`, `created_at` | Writing actor and time. |
| `content` | The content, where this computer holds it; else `NULL`. |
| `derived_from`, `grounding` | Upstream digests and grounding at write time. |
| `remote` | The provider's version number once uploaded. |

**`stamps`** — the stamp ledger, oldest first. Key: `(todo_id, sequence)`.
Approvals and intent checks are stamps: `fact = 'approval'` and
`fact = 'intent_checked'`.

| Column | Meaning |
| --- | --- |
| `todo_id`, `sequence` | The todo and record order. |
| `fact` | The fact kind, in snake case (`preparation_verdict`, `build_verdict`, `approval`, `intent_checked`, `kickback`, `build_started`, `build_complete`, `finalize_ready`, `task_firing_started`, `task_outcome`, …). |
| `at`, `actor` | Time and acting session, GitHub account or person. |
| `role` | The role the acting session played, or `NULL`. |
| `model`, `model_version`, `model_provenance` | Model identity, or `NULL` with `outside_session` or `unknown`. |
| `eval_fingerprint` | The published fingerprint copied at write time. |
| `human_involved` | The audit-only declaration, or `NULL`. |
| `evidence` | The fact's bound evidence (digests, commit, phase, round, reason, occurrence, carried-from stamp). |

**`holds`** — the current dispatch hold and the hold audit. Key:
`(todo_id, set_at)`. Columns: `todo_id`, `source` (`manual` or `automatic`),
`reason`, `set_by`, `set_at`, `released_at`, `released_by`. The current hold
is the row with `released_at IS NULL`; at most one per todo.

**`session_bindings`** — started sessions per todo and role. Key:
`(todo_id, role)`. Columns: `todo_id`, `role`, `session`, `worktree` (the
returned worktree path, kept after the session ends).

**`reflections`** — what this computer knows of each item's GitHub
reflection. Key: `todo_id`.

| Column | Meaning |
| --- | --- |
| `todo_id` | The item. |
| `issue_number` | The reflected issue. |
| `remote_revision` | The confirmed remote revision: the record revision GitHub holds. |
| `pending` | Local changes not yet reflected: the pending revision is `todos.revision`. |
| `conflicted` | A newer remote revision met a pending local change; commands refuse until `instrukt todo refresh`. |
| `failure` | The last failure that left reflection pending. |
| `pull_number` | The linked pull request. |
| `issue_id`, `record_comment`, `etag`, `pull_outstanding`, `reviews_reflected`, `pull_closed`, `board`, `board_item`, `board_status`, `board_failure` | Reflection bookkeeping. |

**`inbound_commands`** — `/instrukt` comment commands already answered. Key:
`(todo_id, comment_id)`.

**`provider_deferrals`** — per project, the time until which GitHub deferred
requests (rate limiting). Key: `project`.

**`listing_copies`** — the last known copy of each todo of a project that is
not a local item here, as the latest pickup listing read it. Key:
`(project, issue_number)`.

| Column | Meaning |
| --- | --- |
| `project`, `issue_number` | The project and the todo's reflected issue. |
| `slug`, `kind`, `status` | The todo's address, kind and record status, as reflected. |
| `remote_revision` | The record revision the copy was read at. |
| `claim_machine` | The computer its reflected claim names, or `NULL` when unclaimed. |
| `record` | The reflected record as read, as JSON. |
| `copied_at` | When the pickup listing read it: the copy's age. |

A copy is replaced by each later pickup listing and removed when its todo
becomes a local item or no longer appears in the listing.

### Runner tables

Owned by the runner and written only by it, in the same transactions as the
reactions they record.

**`events`** — the local event log, oldest first. `seq` is a monotone local
position, never a transport or occurrence identity. Rows hold the stable event
`id`, `type`, full canonical envelope including opaque `data`, `logged_at` and
optional project association derived from explicit origin. Platform and unknown
origins require no project value. Locally emitted rows additionally hold their
producer occurrence key, target publication scope, pending/confirmed publication
state and last failure. Transport coordinates identify endpoint/database, stream
and entry ID; one occurrence can have multiple transport coordinates. Observed
records are distinct from local publication obligations and are not republished.
Confirmed publication records acceptance only, not reaction completion.

**`consumer_cursors`** — settled local accounting partitioned by logical consumer
and transport source. It records contiguous settled progress and its associated
transport coordinate. It neither substitutes for nor advances a Redis group's
position. A higher observed entry alone cannot move a cursor past unsettled work.

**`handled_events`** — occurrence protection keyed by `(consumer, event_id)` on
this computer, with `handled_at` and an optional local sequence reference. Two
stream entries carrying one ID share the mark. Successful local reaction outcome,
handled mark and settled cursor commit together before Redis ACK. Seven-day
retention is distinct from the local log and stream retention.

A parked outcome is distinct from a successful handled mark. Retry and parking
stage recovery retain enough identity to recognize completed stages after a
crash; no later event settles past unfinished parking. Raw malformed entries
remain diagnosable at the transport boundary.

Redis itself owns stream payloads, group pending entries, transport progress,
source-scoped ingress claims and dead letters. These are not alternate work
records or database tables. Redis and SQLite have no shared transaction.

**`procedure_activations`** — durable acceptance keyed by `(event_id, procedure,
exact_home)`, where home includes kind and slug or platform. Retains immutable
event, accepted identity/attendance/delivery declaration, procedure-read references,
acceptance state and visible failure/outcome. A duplicate key returns its existing
acceptance. Submission failures leave the consumer unsettled; accepted rows precede the final consumer-outcome, handled-mark and cursor transaction before Redis ACK. Later runtime execution is separate; session/delivery-table realization is
owned by S6, within this same database.

**`standing_deliveries`** — pending deliveries with a fixed runtime target.
Key: `(consumer, seq)`, referring to the event's consumer and event-log sequence.
Columns: `consumer`, `seq`, `session` (the selected `<runtime>:<id>`).
Commit the target before `send`; a failed or interrupted send retries that
same session. A later absent or conflicting runtime listing cannot retarget
it or start a replacement. Delete the row in the transaction that records
accepted delivery and its handled mark. A start with an unknown result keeps
its start marker until its session can be recovered, then records this target
before any recovery send.

**`request_validators`** — conditional-request validators. Key:
`(project, request)`, `etag`, `body` (the cached answer an
unchanged `304` reuses).

**`poll_deferrals`** — per project, the earliest time the next poll of a
source may run, from `X-Poll-Interval`. Key: `project`.

**`watched_refs`** — the last observation of each watched ref. Key:
`(project, remote, ref)`. Columns: `project`, `remote`, `ref`, `revision`,
`observed_at`.

**`executions`** — dispatched executions the runner watches until their todo
is released. Key: `(project, todo_id, role)`. Columns: `project`, `todo_id`,
`role`, `session`, `workflow_state` (the state at the last look),
`recovery_sent`, `anomaly_raised`.

**`standing_bindings`** — cached standing-agent sessions. Key:
`(procedure, project)`. Columns: `procedure`, `project`, `session`, `key_title`.
A binding is repaired from the runtime thread's exact key title before a new
start is considered; deleting this cache never establishes absence in T3.

**`runtime_key_locks`** — one standing-agent starter/deliverer per runtime
and singleton key. Key: `(runtime, project, procedure)`.

| Column | Meaning |
| --- | --- |
| `runtime`, `project`, `procedure` | The compiled adapter name and singleton scope. |
| `owner` | Owning runner invocation token; `NULL` when released. |
| `acquired_at` | Acquisition time, for diagnosis; not a timeout permitting takeover. |

Acquisition is an atomic conditional write under the existing writer lock. Only
the recorded owner can perform the key's list/start/send sequence and release
the row. No database transaction spans a runtime command. A successor holding
the machine runner lock can recover dead invocation ownership; unresolved
`standing_starts` remains in force after that recovery. Every standing
start-or-wake goes through this row, even with a cached binding.

**`standing_starts`** — standing-agent starts recorded before the runtime
was asked and not yet bound or resolved. Key: `(procedure, project)`.
Columns: `procedure`, `project`, `runtime`, `key_title`, `begun_at`, `reported`.
The marker commits before the external start. A later keyed runtime match can
resolve it without a replacement start; an empty list alone cannot clear an
ambiguous marker. Explicit resolution or proof of rejection before creation
is required before trying a new start.

**`task_failures`** — tasks held after a firing recorded `error`, and whether
the developer was notified. Key: `(project, todo_id)`. Columns: `project`,
`todo_id`, `session`, `notified`, `occurrence` (the failed run the hold was
set for).

**`wakeups`** — one row: `last_at`, the last wake-up.

### Retention

A removed todo's rows are purged with it when its recovery window expires.
An execution's row goes when its todo is released; a standing start's when it
is bound or resolved. A pending standing delivery's row goes with its accepted
delivery and handled mark. A task failure's row is kept after its hold is
released, so the same failed run never sets the hold again, and is replaced by
the task's next failed run. Validators, deferrals, watched-ref observations
and listing copies are replaced in place. Key-lock rows are reused with
ownership cleared on release; they are not event history. Local events and settled cursors are retained as operational accounting. Redis
stream retention separately bounds replay; occurrence duplicate marks expire
after seven days. Local history does not backfill a newly created Redis group.

### Use cases

#### UC-SD2: A write interrupted part-way leaves the last completed state

```gherkin
  Given a developer's computer is recording a change that spans several records, such as a stamp and the todo it advances
  When instrukt stops part-way through that write
  Then the next command finds the work as the last completed change left it
  And nothing of the interrupted change is present
```

## Known caveats

- A3's existing standing bindings, starts, fixed delivery targets and key locks
  below retain their project-keyed adapter contract. S6 reconciles these with
  exact home/subject identities and durable procedure delivery. They create no
  second path independently executing Redis events from SQLite cursors.


- UC-SD1 is retired: reading the database directly is not a feature; agents use instrukt's commands. Its id is not reused.

- The local event log grows with activity; retaining it does not extend Redis replay or occurrence duplicate protection.
- Work recorded only in an earlier build's file store is not read. Reflect
  it to GitHub with that build before upgrading; after the upgrade it enters
  the database again by pull-through.
- Deleting the database loses work-record writes not yet reflected to
  GitHub. Reflected work returns by pull-through; the runner's state starts
  empty, so watched refs are observed afresh. Before any new standing-agent
  start, the adapter discovers the thread's key title in T3; existing keyed
  threads are rebound rather than duplicated. A lost ambiguous start marker
  cannot be reconstructed when the runtime has no discoverable matching thread.

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/work/github-record](github-record.md) — reflection of the work tables to GitHub
- [project/instrukt/spec/feature/runner/event-activation](../runner/event-activation.md) — the event log's events and consumers
- [sdlc/policy/state-storage](../../../../domain/sdlc/policy/state-storage.md)
