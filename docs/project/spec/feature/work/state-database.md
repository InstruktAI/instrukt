---
description: The one SQLite database holding a computer's work record, runner state and conversations — its tables, its migrations, and instrukt as its single, transactional writer.
visibility: public
---
# State Database — Spec

## What it is

A computer keeps instrukt's operational state in one SQLite database: the
local work record that stands in front of its GitHub reflection, the
runner's state and the company context's conversations. instrukt is its only
writer and its only interface: agents work with the record through instrukt's commands. A write interrupted
part-way leaves the state as the last completed change left it.

The schema below describes instrukt's internal implementation, versioned by its migrations. It is not a reader API and promises no stable columns to external readers. Agents use instrukt commands; instrukt owns changes to these internal tables. The work context owns the file,
its migrations, the writer rules and the work tables; the runner context owns
the meaning and the writes of the runner tables; the company context owns the
meaning and the writes of the conversation tables.

## Canonical fields

### Database file

- **Path:** `~/.instrukt/state/instrukt.db`, one per home, in WAL journal
  mode; SQLite keeps `instrukt.db-wal` and `instrukt.db-shm` beside it.
  instrukt creates it, at the current schema version, on the first command
  that needs it.
- **Holds:** the work tables, the runner tables and the conversation tables
  below. Nothing else is stored in it.
- **Protection:** the database and the files SQLite keeps beside it are
  readable and writable by their owner only.
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
- The state directory is resolved to its canonical location once, and the database,
  its sidecars and every lock — writer, reflection, runner, runtime-dispatch, history
  and conversation locks — derive from it, so a home or a state directory reached
  through a symbolic link keeps one authority. A symbolic link at the database file
  or one of its sidecars is refused, naming the door (link the home or `state/`
  instead): it alone would put one database under two lock domains
  (`general/principle/trust-and-guardrails`: interference is a harm restoring state
  cannot repair).
- A separate per-computer reflection lock
  `~/.instrukt/state/reflection.lock` serializes GitHub reflection. It is held
  across GitHub requests and takes the writer lock only briefly for each local
  transaction that records what reflection learned. Lock order is always
  reflection → writer; a local writer releases the writer lock before acquiring
  the reflection lock. Current-schema local readers take neither lock. Readers
  of an older database enter the same migration path under the writer lock
  before consuming it.
- A change that spans several records commits in one transaction or not at
  all: a todo with its new stamp and revision, a finalize-ready stamp with
  its `workflow.finalized` event, a dependency or group change across several
  todos, a delivery's outcome with its handled marks and cursor.
- A committed transaction survives a process crash and a power loss
  (`synchronous = FULL`). An interrupted transaction leaves nothing of itself
  behind.
- Foreign keys are enforced. Removing a todo's row removes every row that
  belongs to it.
- The runner lock `~/.instrukt/state/runner.lock` serializes runner
  lifetimes. Runtime dispatches also take `state/runtime-dispatch.lock` for
  admission to an invocation or runner wake. They release it before calling
  the runtime and use an OS lock per exact runtime key for ownership.
  The machine locks own no stored data.

### Scope of what is stored

The database holds this computer's local items, and the last known copy of
each other todo of its projects as the runner's pickup listing last read it
(`project/instrukt/spec/feature/work/github-record`). A workflow phase
is derived, never stored: `instrukt todo list` and `instrukt todo show` give
it.

Values are stored as follows: timestamps as RFC 3339 text in UTC; booleans as
`0` or `1`; a session as `<runtime>:<id>` with native endpoint/incarnation encoded by its
adapter; an actor as JSON text, one of
`{"session": "<runtime>:<id>"}`, `{"github": "<login>"}` or
`{"person": "<name>"}`; other structured values as JSON text. An empty optional value is `NULL`.

### Schema version and migrations

- `PRAGMA user_version` is the schema version. instrukt ships an ordered
  sequence of migrations and owns every one of them.
- Opening the database for writing applies each pending migration in its own
  transaction under the writer lock, and sets `user_version` in that same
  transaction. Opening older owned state for reading first performs that same
  locked migration, then opens a current-schema reader. Current-schema reads
  do not migrate or take the writer lock.
- A database whose version is newer than the running instrukt knows is
  refused for writing; the refusal names both versions and prints the command
  that installs the current instrukt. A status command reads an older database
  through a migrated in-memory image and changes nothing on disk, and reports a
  newer one as a finding while still reporting everything else.

Version 2 preserves version-1 session bindings while introducing role attempts. Their placement and assignment stay explicitly unknown when the older record supplied no such facts; no native key or successful completion is inferred. Record-envelope schema 2 carries the native attempt evidence and role output bindings.

Version 13 canonicalizes retained native assignments with an omitted dispatcher
to explicit unknown evidence. Its admission semantics are owned by
`project/instrukt/spec/feature/runner/session-continuity`.

Version 18 gives each addressed message its sender's connector identity, name
and email address. A message stored before it keeps the member key it was
recorded with as its `sender_name` and has no `sender` and no `sender_email`:
it is delivered as it was admitted, and a tool call during its turn finds no
login of the speaker's own. No identity or address is derived for it.

Version 20 records when the post of each answer piece was begun
(`conversation_answers.attempted_at`). Pieces stored before it have none: an
outstanding one is posted as before, and a posted one keeps its `posted_at`.

Version 21 gives each addressed message its admission time, its no-work
interval, the newest message the agent has written in its turn and when that
turn ended (`conversation_messages.admitted_at`, `unavailable_since`,
`turn_update`, `turn_ended_at`).
Messages stored before it have none of them: none has an interval open, and
the relay-wide failure counts for one from that failure's start.

Persisted runtime requests and manual standing reservations enter domain code
with a required captured title. Migration fills a missing or null title with
the request's original key and preserves supplied titles. It covers unresolved
starts, bound requests, current and retired standing assignments, and manual
reservations. Native driver placement evidence likewise retains its captured
title; earlier placements recover it from retained title evidence, including
verified runtime bindings, or from their recorded role and original phase branch.
An omitted placement title does not establish a key-only native title. Remote
conversion is owned by `project/instrukt/spec/feature/work/github-record`.

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
| `refreshed_at` | Latest successful read of this local item from GitHub (including a not-modified response); `NULL` before a read. Listing age uses this local observation time, never its revision or creation time. |
| `revision` | Incremented by every accepted change; the revision reflection writes. |
| `group_id` | The structural holder this todo is grouped under (`todos.id`), or `NULL`. |
| `group_slug` | The holder's last-known slug, for a holder not stored on this computer, or `NULL`. |
| `schedule` | A task's schedule: an ISO-8601 instant with offset, or `cron:<five fields>`; `NULL` for none. |
| `catch_up` | `once` or `skip`. |
| `agent`, `tier` | A task's agent preference and tier, or `NULL`. |
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
| `fact` | The fact kind, in snake case (`preparation_verdict`, `build_verdict`, `approval`, `intent_checked`, `kickback`, `build_started`, `build_complete`, `role_complete`, `finalize_ready`, `task_firing_started`, `task_outcome`, …). |
| `at`, `actor` | Time and acting session, GitHub account or person. |
| `source_comment` | GitHub decision occurrence, or `NULL` for other stamp forms. Unique per todo when present; retained in reflected stamps so reconstruction does not reapply the same decision. |
| `role` | The role the acting session played, or `NULL`. |
| `model`, `model_version`, `model_provenance` | Model identity, or `NULL` with `outside_session` or `unknown`. |
| `eval_fingerprint` | The published fingerprint copied at write time. |
| `human_involved` | The audit-only declaration, or `NULL`. |
| `evidence` | The fact's bound evidence (digests, commit, phase, round, reason, occurrence, carried-from stamp). |

**`holds`** — the current dispatch hold and the ordered hold audit. Row order
retains distinct actions even when source timestamps tie. Columns: `todo_id`, `source` (`manual`, `automatic` or `github`),
`reason`, `set_by`, `set_at`, `released_at`, `released_by`. The current hold
is the row with `released_at IS NULL`; at most one per todo.

**`session_bindings`** — retained sessions per todo, role and attempt. Key:
`(todo_id, role, attempt)`; unique `(runtime, semantic_key)`.

| Column | Meaning |
| --- | --- |
| `todo_id`, `role`, `attempt` | Assignment and monotonic positive attempt, reserved before native delegation and reused on retry. |
| `runtime`, `semantic_key`, `display_title` | Selected runtime, stable assignment identity and independently mutable display title. Native child keys are assignment facts, not adapter lookup entries. |
| `session` | Scoped native identity from `todo bind`, or `NULL` while unresolved. |
| `worktree`, `branch`, `base_head` | The work phase's one shared `<slug>/work` worktree and branch and captured starting head; preparation instead records its separate `<slug>/prepare` worktree and branch. Retained after release. |
| `completion_sequence`, `completed_head` | Releasing stamp and its exact shared head; read-only roles retain their captured head. `NULL` until completion. |
| `failure_reason`, `failure_head`, `failure_workspace`, `failed_at` | Failure evidence and current shared head, not a recovery source branch. |
| `released_at` | Completion, verdict, disposition or confirmed failure release time; `NULL` while outstanding. |
| `evidence` | Rendered assignment, native task identity, matching placement and any no-start proof; deliberate driver entry retains its supplied native placement/source evidence; runner-owned task acceptance retains its own claim and task-start stamp transaction. |

One todo has one work-phase workspace binding owned by its orchestrator and
at most one outstanding workspace-role attempt. All its children inherit that
workspace and commit directly on its todo branch. Reserving the next role
captures the current shared head. An unresolved native start blocks admission.
The orchestrator coordinates work and keeps the workspace clean between roles.

| Role | Evidence and release in the same record transaction |
| --- | --- |
| Setup | `--role-complete` at the clean shared todo head after setup. |
| Builder/fixer, including plan reconciliation | `--build-complete` at the clean shared todo head. |
| Build reviewer | Verdict bound to the complete findings and current captured shared head. |
| Deferrer | `--deferrals-processed` covering the current dispositions artifact at the captured clean head. |
| Finalizer | `--role-complete` at the clean, published shared candidate; readiness is a later record-only write against the retained binding. |
| Failed attempt | Record-only `todo attempt --failed --reason` after native stop evidence and a clean shared tree; no completion stamp. |

The stamp or failure, release, new revision and reflection obligation commit
together. A retry reads that retained outcome. Git refs have no per-role advance
intent: every commit is already on the checked-out todo branch. Missing or stale
evidence leaves the attempt outstanding; native idleness supplies no release.
Dirty or conflicted work remains in the same workspace for recovery within that
attempt. A released failure permits a successor at the current shared head.
Failure creates neither a different starting base nor setup/build completion.
The original build base remains fixed. Native child lifetime remains runtime-owned. Preparation doc writers release
with `--role-complete` bound to required artifacts and exact authored local-main
commits; preparation reviewers release with their verdict stamps. Direct commit
review is still owed before preparation release.

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

**`inbound_commands`** — processed `/instrukt` comment identities and their
confirmation replies. Key: `(todo_id, comment_id)`. `reply` retains the fixed
response and `reply_pending` distinguishes a committed decision from its remote
confirmation. Their acceptance commits with any stamp and pending attention.
Retries reconcile the source-comment reply marker before posting again.

**`github_label_actions`** — applied inbound hold-label actions. Key:
`(project, issue_number, action_id)`. Retains todo ID, add/remove, acting GitHub
login, verified write access, source time and accepted/ignored outcome. Its
application and hold audit commit together. Reflection-originated output
changes are recognized separately and do not become inbound commands. A
snapshot alone supplies no new action identity or attribution.

**`provider_deferrals`** — per project, the time until which GitHub deferred
requests (rate limiting). Key: `project`.

**`project_boards`** — per project, the GitHub project board
(`<owner>/<number>`) reflection set up or found when the project layer names
none (`project/instrukt/spec/feature/work/github-record`). Key: `project`.

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

Owned by the runner and written through its interface, in the same
transactions as the reactions they record. Work invokes that interface for
finalized publication and approval attention committed with their stamps.

**History tables** — `history_sources` records selected-source legacy import
completion. `history_source_positions` stores each runtime source's opaque
committed position; history does not interpret adapter cursor fields.
`history_projection_records` retains normalized native records by source, kind
and native id, including records delivered by earlier batches. Completed
conversation views, archive aliases, original source documents and FTS5 changes
commit with the source position. Migration from the previous history shape
preserves retained native records before the new source position begins.
`history_conversations` and `history_fts` serve local search and document lookup.
These tables are owned by `project/instrukt/spec/feature/runner/history`, survive
source removal and are never reflected to GitHub or stored centrally in Redis.

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

**`consumer_sources`** — per logical consumer, each Redis source it reads and
whether its group was prepared. `state` is `active`, `retiring` (a former source
being drained), `retired`, or `abandoned`: a former source no longer in the
configuration, whose pending events stay unread in that Redis and whose
`prepared` fact is kept, so configuring it again drains its own group
(`project/instrukt/spec/feature/runner/event-transport`).

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

**`consumer_outcomes`** — a parked outcome can carry an open announcement
intent with the destination Redis identity, deterministic receipt key and
complete encoded announcement. The worker records the intent before sending.
A partial SQLite index covers only open intents, with a limit of 1,024
on the computer. At the limit, the newly parked outcome remains unannounced
and no append is attempted. Recovery takes up to 32 entries per sweep, in
oldest-settled order. The runner keeps the last position in
`state/announcement-recovery-cursor` across scheduled ticks and wraps
after the final page. Recording a confirmed append retires the intent before
the worker tries to delete its Redis receipt. Receipt deletion is best effort.
The receipt expires seven days after the append or the most recent retry that
finds it. A destination that is no longer configured retires the intent as
abandoned and keeps that fact on the outcome for status.

**`runner_workers`** — current worker failures, deferred consumer source
transitions and keyed-start findings. A keyed-start finding keeps its first
observation on the operator clock and is cleared when reconciliation ends it.
The process remembers which runtime keys it has notified during its lifetime. Each deferred transition retains its first reported time across
retries. Completing the transition removes the row.

Redis itself owns stream payloads, group pending entries, transport progress,
source-scoped ingress claims and dead letters. These are not alternate work
records or database tables. Redis and SQLite have no shared transaction.

**`procedure_activations`** — durable acceptance keyed by `(event_id, procedure,
exact_home)`, where home includes kind and slug or platform. Retains immutable
event, accepted identity/attendance/delivery declaration, procedure-read references,
acceptance state and visible failure/outcome. A duplicate key returns its existing
acceptance. Submission failures leave the consumer unsettled; accepted rows precede the final consumer-outcome, handled-mark and cursor transaction before Redis ACK. Later runtime handover and entry consumption are separate boundaries under
`project/instrukt/spec/feature/runner/procedure-event-delivery`.

**`procedure_responsibilities`** — continuing procedure/home[/subject] assignments.
Key: `(procedure, exact_home, subject_or_none)`, with typed home and subject
identity, next enqueue position and current session generation. One per-procedure/
home resolution guard covers all its subjects. Generation history retains the
runtime key, native session, execution checkout, system-origin assignment evidence,
initialized role, attendance and interruptibility, and confirmed
terminal/continuation disposition. Human settling does not retire a T3 standing
assignment; later input retains its initialized settings;
a successor adopts current publication. This is assignment bookkeeping, not a
runtime session, transcript or lifecycle store. Conflicting semantic-key evidence or uncertain
observation cannot advance the generation.

**`procedure_generations`** — retired standing assignment evidence keyed by
responsibility and generation, retaining native identity, key, initialized
declaration, immutable start request and confirmed closure observation. Retirement
and successor admission commit together. A successor's pre-creation marker proves
that no creation had begun before that decision. The current responsibility
retains its last observed activation position, so close alone suspends old work;
a later activation can admit a home successor.

**`standing_deliveries`** — one durable procedure-event record per activation.
Key: `(event_id, procedure, exact_home)`, referencing `procedure_activations`.
Retains responsibility key, ordered enqueue position, immutable envelope,
accepted declaration, pending/selected/delivered/failed state, selected boundary,
target generation/session, input identity, handover outcome/time, entry-consumption evidence and visible failure evidence.
Uniqueness is by activation, never `(consumer, seq)`; local sequence and transport
coordinates are accounting references only. The initial row exists before
runtime startup. Selection records one oldest pending event or an ordered
snapshot, leaving unselected rows pending. Runtime acceptance of selected data
records handover; queued entry consumption remains separate. A content-free
wake alone marks no event handed over. Retryable pre-handover
failures retain pending work; terminal failure remains distinct and visible.

Commit the runtime target before dispatch. Failed or interrupted send retries
that same target without a replacement. Confirmed session closure is a separate
continuation decision: under the responsibility guard, retain undelivered work
through suspension and transfer it only to an eligible resumed session or
confirmed successor. Retain delivered activation identities to suppress repeats;
a crash after external handoff but before commit can still repeat data. No
second SQLite-cursor scheduler independently executes these rows.

Manual standing-entry reservations live with their guarded responsibility and
generation. They retain the reserving caller, token, initialized assignment,
key, native session when bound and any no-start proof/confirmation. Event-driven
activation waits on an unresolved manual reservation; it never adopts its unknown
session or initiates a competing generation. These record-only reservations
are distinct from instrukt-owned `runtime_starts`.

**`runtime_inputs`** — durable inputs to a native target session, shared across
procedure events, approvals and runner-owned recovery inputs. Key:
`(runtime, session, input_id)`. Retains intent (`asap` or `entry`), entry kind, source and arrival timestamps, ordered item
references and immutable selected content, pending/selected/accepted/entered/
failed outcome, native message identity when available, acceptance and entry
evidence with its runtime source and observation time, and last failure. Each item retains its own source identity; event entries are always supplied as a bounded ordered batch. One waiting
accepted message per target is permitted. A selected or uncertain send reserves
that slot too, so concurrent senders cannot exceed it. New items stay pending;
verified consumption normally clears the slot. Evidence retains the runtime
capability, native reference, source and observation time; unknown/unsupported
evidence leaves the slot occupied, rather than fabricating a transcript or run id,
until `INPUT_WAIT_BOUND`: then the input is `unconfirmed` (outcome
unknown, never sent again) and the slot is released. An item larger than the
delivery limit on its own is `set_aside` and later items proceed. The runner
compacts set-aside payloads and mirrored standing envelopes to a digest as soon
as it records that disposition. Identity and failure metadata suppress replay
for seven days (`SET_ASIDE_RETENTION`) from the input's arrival or the
activation's acceptance. On a wake after the bound, the runner settles any
dependent standing or approval record before retiring the input and its
set-aside mirrors in the same transaction. An input with an unsettled
reference is retained. Status reports the retained count and up to
`SET_ASIDE_STATUS_LIMIT` recent findings. Wake requests coalesce per target.
Only-at-entry selection rechecks idleness before handover. Runtime calls run
outside transactions; the fixed target and selected input commit before send.
A lost acknowledgement can repeat input and remains distinguishable from
confirmed acceptance. Domain completion is not an input state.

Native children have no observation/parent-wake table. `session_bindings`
retains reservation, native child identity, role/workspace facts and release
history. Their native parent link, status delivery and ending belong to the
runtime. The reservation records any proof/confirmation permitting a retry of
an unstarted native attempt; no `runtime_starts` marker or project-list absence
resolves that native attempt.

**`approval_attention`** — attention retained with an applied decision. Key:
`(todo_id, decision_sequence)`. Retains the decision's source identity, gate,
attribution and evidence, target claim/session when live, referenced input item
and pending/accepted/no-live-claim outcome. Creation commits with the decision;
retry revalidates the claim and never revives or starts an orchestrator. A known
absent claimant records `no-live-claim`; observation failure stays pending.
Replayed decision intake reuses the decision and its attention record. These
rows are local runtime delivery bookkeeping, not reflected approval evidence.

**`conversation_links`** — runner-owned links between two runtime-scoped native
threads, with active/closed state, member references and the opening positions.
An active member belongs to at most one link. Closure updates the whole link;
it preserves delivery evidence rather than deleting progress.

**`conversation_relay_positions`** — one record per link and source direction,
retaining the source stream position, completed-turn delivery position, fixed
native target, pending range/content identity, native acceptance receipts (or
explicit empty-content dispositions) and
last stream/handover failure. The source observation position is distinct from
accepted delivery progress. Link creation initializes both directions; target
acceptance or a recorded empty disposition advances delivery. Source occurrence
times are attribution, never cursors. Pending evidence survives runner restart.
Pending handovers distinguish native turns, opening contributions and final
contributions. Existing native handovers migrate to the native origin before
consumption. A final contribution's acceptance receipt commits with whole-link
closure; management receipts do not advance the native completed-turn position.

These tables realize the runner's contract in
`project/instrukt/spec/feature/runner/conversation-links`; work owns their file,
migration and writer boundary, not the relay decision. Native stream and send
operations run outside local writer transactions.

**MCP sign-in tables** — what signing in to a company's MCP servers keeps,
under `project/instrukt/spec/feature/runner/mcp-access`. A person is their
email address in lower case. These tables hold credentials; they are written
to no file other than this database, and their values appear in no log,
status or recorded cause.

**`mcp_registrations`** — one registration of a client. Key: `return_id`.

| Column | Meaning |
| --- | --- |
| `company`, `server`, `issuer` | The company home, the server and the authorization server it was made for. |
| `declared` | `1` for a client declared under `sign_in.client`, whose secret is read from the home's secrets file; `0` for a client instrukt registered. |
| `client_id` | The client's identifier, for a declared client as it was declared when the registration was made. |
| `client_secret`, `secret_digest` | A registered client's secret, `NULL` for a public client and for a declared one; for a declared client the SHA-256 digest of its secret, `NULL` when it has none. |
| `return_address` | The return address the client was registered with. |
| `registered_at`, `retired_at` | When it was made, and when a newer registration replaced it; `NULL` while current. |

**`mcp_generations`** — Key: `(company, person, server)`. Column:
`generation`, raised by every removal of that person's login or open sign-ins
for that server.

**`mcp_logins`** — one signed-in login. Key: `(company, person, server)`.

| Column | Meaning |
| --- | --- |
| `resource` | The server's `url` the login was issued for. |
| `subject` | The OpenID Connect `sub` recorded at sign-in; `NULL` with a declared identity request. |
| `return_id` | The registration it was issued to, which names its authorization server and client. |
| `access_token`, `expires_at` | The access token and when it runs out; `expires_at` is `NULL` when the server gave no lifetime, which means unknown. |
| `checked` | `1` once who the access token identifies as has been confirmed; `0` while a renewed access token waits for that check, during which it is not used. |
| `refresh_token` | The refresh token; `NULL` when the server issued none. |
| `scopes` | The scopes granted, as the server gave them. |
| `signed_in_at`, `renewed_at` | When the person signed in, and when instrukt last renewed it. |
| `renewing_until` | While a renewal runs, when its claim lapses; `NULL` otherwise. |

**`mcp_sign_ins`** — one sign-in. Key: `state_digest`, the SHA-256 digest of
the state. At most one row per company, person and server is `open` or
`exchanging`.

| Column | Meaning |
| --- | --- |
| `company`, `person`, `server` | Whose sign-in it is, and to which server. |
| `return_id`, `resource`, `scopes` | The registration it uses, and what the link asked for. |
| `generation` | The generation of that company, person and server when it was made. |
| `connector`, `conversation`, `message`, `recipient` | The addressed message whose call started it, and the person's identity on that connector, to whom its direct messages go. |
| `code_verifier`, `nonce` | The PKCE verifier and, for an OpenID Connect sign-in, the nonce; `NULL` once it is closed. |
| `link` | The link while the connector has not accepted it; `NULL` afterwards. |
| `created_at`, `expires_at` | When it was made, and ten minutes after. |
| `state` | `open`; `exchanging`, its return claimed; or closed: `completed`, `declined`, `refused`, `failed`, `undelivered`, `expired` or `cancelled`. |
| `link_sent_at`, `told_at` | When the connector accepted the link, and the closing direct message; `NULL` while outstanding. |
| `closed_at` | When it closed; `NULL` while `open` or `exchanging`. |
| `failure` | The cause of its last failed step, without any value of the sign-in; `NULL` once that step succeeds. |

**`mcp_sign_in_waiters`** — the calls waiting on a sign-in. Key:
`(state_digest, waiter)`. Column: `waiting_until`, the end of that call's own
wait. A call removes only its own row when it stops waiting.

A sign-in and the call that starts it, with its waiter row, are written in one
transaction, only when the generation the start captured is still current. Claiming a return moves the sign-in from `open` to `exchanging`
in one transaction before anything is sent to the authorization server. A
sign-in's `completed` state and its login are written in one transaction,
only when its generation is still current; it reads the waiter rows to decide
whether waiting calls take the login or the person is told to ask again. A
renewal writes its new access token, lifetime and refresh token in one
transaction, only when the generation is unchanged. A removal raises the
generation, deletes the login and cancels the sign-ins that are open or being
exchanged in one transaction.

**`request_validators`** — conditional-request validators. Key:
`(project, request)`, `etag`, `body` (the cached answer an
unchanged `304` reuses).

**`poll_deferrals`** — per project, the earliest time the next poll of a
source may run, from `X-Poll-Interval` or an App-issuance retry boundary. Key:
`project`. Nullable `cause` retains an issuance failure independently of cached
CI responses; successful observation clears it. Credential failure never selects
an anonymous observation path. Database migration 14 adds this column while
preserving existing validator bodies and retry deadlines.

**`watched_refs`** — the last observation of each watched ref. Key:
`(project, remote, ref)`. Columns: `project`, `remote`, `ref`, `revision`,
`observed_at`.

**`executions`** — the runner-owned orchestrator or task execution currently
watched for each todo. Key: `(project, todo_id, role)`. Columns are `project`,
`todo_id`, `role`, `session`, `workflow_state`, `recovery_sent` and
`anomaly_raised`. Work's session bindings retain the authoritative attempt,
key and occurrence. Native child attempts are retained only by work; they
are not runner-observed executions.

Runtime observations retain each available activity timestamp with its meaning
and native provenance. Missing native activity time is `NULL`; local observation
time is separate and never substitutes for last activity. Acceptance, consumption
and initialized-assignment evidence are explicit adapter capabilities, with
unknown/unsupported outcomes instead of synthesized conversations or run ids.
The adapter is always selected by `runner.runtime`.

**`runtime_bindings`** — runner-owned native identities scoped to this computer
and runtime. The pending row is the authoritative record of the native ID
returned by a keyed start. Key:
`(runtime, semantic_key)`. Columns are `runtime`, `semantic_key`, `kind`, `project`,
`subject` (the assignment's durable semantic address), `session`, optional `native_project_id`, `display_title`,
`checkout`, `worktree`, `branch`, `base_head`, `accepted`, original structured
`request` when known. These are instrukt-owned starts only. Standing scope,
initialized facts and generation are normalized in `procedure_responsibilities`
and `procedure_generations`, joined by `semantic_key`. The native project id is nullable; instrukt project
identity and runtime endpoint/placement scope remain separate. Display-title
changes do not change the semantic key or authorize a new start. Work's `session_bindings`
retains todo/role/attempt facts, and task occurrence stamps own completion.
One pending row per key keeps the exact returned ID and opening acceptance
from the native start result, before thread inspection or placement verification,
until the start commits. Retry reads that ID directly. The pending row becomes
accepted when the owning records commit, or is retired by explicit recovery.
During a todo or task replacement, the start marker retains the original claim's
session and the latest confirmed stopped session. Retiring a pending successor
updates that marker and retires its binding in one write. The work claim moves
only when a verified replacement commits.
An accepted replacement retains its most recent stopped ID so the same
operator command can finish pending reflection and return its result.
Discovery can recover a missing accepted binding. Removing a row proves no
absence.

**`manual_standing_reservations`** — runner-owned reservation keyed by standing
responsibility. The request can retain one native ID observed for the reserved
key, even before binding succeeds. Confirmation of no start clears that
observation after native verification. Binding clears it when the session is
recorded. Explicit replacement retires the reservation in the same transaction
that stages the successor.

**`runtime_key_locks`** — one starter/deliverer per runtime and exact key.
Key: `(runtime, semantic_key)`.

| Column | Meaning |
| --- | --- |
| `runtime`, `semantic_key`, `project` | Adapter, semantic key and execution scope; project is nullable for platform, whose registered checkout is retained with the standing assignment. |
| `owner` | Owning invocation token, including operational start resolution; `NULL` when released. |
| `acquired_at` | Acquisition time for diagnosis; elapsed time permits no new start. |

Acquisition writes the row under the writer lock after acquiring an OS lock
for this exact runtime and key. Only that owner performs the list/start/send
sequence and releases the row. No transaction spans a runtime command. A
successor holding the same key's OS lock can recover its ownership row while
preserving unresolved `runtime_starts`. Every instrukt-owned start and wake,
including lane orchestrators and task occurrences, uses this table. Native
worker attempts do not enter this keyed-discovery table.

**`runtime_starts`** — starts recorded before calling the runtime and not yet
completely bound. Key: `(runtime, semantic_key)`.

| Column | Meaning |
| --- | --- |
| `runtime`, `semantic_key`, `project`, `kind`, `subject`, execution checkout | Same assignment address as the binding. Owning work/standing records retain the orchestrator attempt, occurrence or generation before instrukt starts it. Native worker reservations use `session_bindings`, not this table. |
| `brief`, `request` | Opening input and structured start request, including requested branch, captured base SHA and worktree requirement. Persisted before the external start. |
| `begun_at`, `reported` | Start time and whether the unresolved-start finding was reported. |
| `resolved_at`, `resolved_by`, `resolution` | Resolution time, actor and evidence. A todo replacement retains the operator-confirmed stopped ID until the replacement commits. |
| `state` | `unresolved` while creation may be in flight; `retry_allowed` after `resolve-start --none` or proof of rejection before creation. The request and key are retained for retry. |

One design covers every instrukt-owned start kind under
`project/instrukt/spec/feature/runner/session-continuity`. Commit the marker
before calling start; bind and clear it in the same transaction only after
recovering all requested evidence. On restart, discover its exact key before
any new start. An empty listing alone never clears it. `resolve-start` or proof
of rejection before creation permits retry of the same key; the marker returns
to `unresolved` before that retry calls the runtime. Removing a todo or
releasing its execution never discards an unresolved marker. When the runtime
reports a bound session absent, its binding and marker are retired together
under the key lock and the start proceeds under the same key; an archived session
retires nothing (`project/instrukt/spec/feature/runner/session-continuity`).

**`task_failures`** — tasks held after a firing recorded `error`, and whether
the developer was notified. Key: `(project, todo_id)`. Columns: `project`,
`todo_id`, `session`, `notified`, `occurrence` (the failed run the hold was
set for).

**`wakeups`** — one row: `last_at`, the last wake-up.

### Conversation tables

Owned by the company context and written only through its code, in the
transactions stated below; no other context reads them, and other contexts
obtain conversation facts through that context's published contracts
(`project/instrukt/spec/domain/company`). They keep what conversations through
connectors keep, under
`project/instrukt/spec/feature/company/conversations`. A conversation's address
is `(company, connector, conversation)`.

**`connected_conversations`** — one row per conversation. Key: the address.

| Column | Meaning |
| --- | --- |
| `semantic_key` | The key its one session is started and woken by. |
| `opening`, `request` | The addressed message whose delivery starts the session, and the start request captured for that message before the runtime is called. |
| `session` | The bound session; `NULL` until a start is bound. An operator-confirmed replacement can rebind the conversation and reset its source position. |
| `source_identity`, `position_json` | The identity of the session's native conversation source, and the opaque position its finished turns are read to. A change of identity restarts the read of the bound session at the beginning, preserving existing answers. |
| `failure` | Cause of a source read failure or a blocked bound session, with its recovery door where applicable. Cleared when work can resume. |

**`conversation_messages`** — one row per addressed message. Key: the address
and `message`, the connector's identity of that message.

| Column | Meaning |
| --- | --- |
| `sender` | The connector identity of the person who wrote it, stored with the message before it is acknowledged. It decides nothing: admission reads the provider's record for it. |
| `sender_name`, `sender_email` | The name and email address the provider gave for the sender when the message was admitted: `NULL` until then, and the address `NULL` when the provider gave none. Once written they do not change. |
| `written_us`, `written_at` | When it was written: in microseconds, which orders a conversation's messages, and as a timestamp. |
| `arrived_at` | When it was stored. |
| `state` | `received` once stored; `refused` once the provider's answer showed that the sender is not one of the company's own; `pending` once the sender is admitted, what accompanies the message is captured and its input is framed; `sent` once the runtime accepted it; `failed` once its turn ended unsuccessfully, keeping its status and notice coverage without blocking later messages; `answered` once its turn finished with an answer or completed successfully with nothing to post. |
| `text` | The message's text as written. |
| `context_json`, `boundary`, `content` | While `received`, admission keeps the prior addressed message ID (or null for the start of the range) in `context_json`. The other fields are null until capture. On capture, `context_json` holds the accompanying messages and whether earlier ones were left out or unread. The other fields hold the delivery boundary and framed input. |
| `attempted_at` | When the current attempt began, recorded with the boundary that identifies it before the runtime can accept it. `NULL` before an attempt or when its turn settles. An acknowledgement applies only while its boundary identifies the outstanding attempt. |
| `source_uncertain` | The current attempt is missing from a moved conversation source. The company relay records this with the new source position. It blocks repeated delivery until the source identifies that attempt or the binding is replaced through the operator door. Migration 24 initializes earlier rows without this finding. |
| `retry_at`, `failed_turns` | The next eligible redelivery time on the operator clock and the failed-turn count used for its bounded delay. The company relay records them with the failure, updates the attempt boundary before sending, and clears retry eligibility when the message is served or superseded. Migration 24 leaves earlier rows intact. An earlier failed row without a schedule first receives one when the relay reads it. |
| `turn` | The native turn that answered it; `NULL` until then. |
| `failure` | Cause of its last failed capture, delivery, or turn. `NULL` once the message is served. A superseded message keeps its cause. |
| `admitted_at` | When its sender was admitted; `NULL` until then, and for a message admitted before it was recorded. |
| `admission_position` | Durable sequence assigned at admission. Migration 23 seeds older admitted messages in retained row order. |
| `superseded_at` | The operator clock time when a later-admitted message from the same sender in the same conversation settles and supersedes this failed row. `NULL` until then. Supersession closes the interval and removes the row from status and unfinished work while retaining its failure record. |
| `unavailable_since` | Start of the current interval in which no observable turn does the work this message waits on, `NULL` outside one; started and ended as `project/instrukt/spec/feature/company/conversations` states, for this message only. |
| `turn_update` | The newest message the agent has written in its turn, as the runtime records it, which its status shows; `NULL` until the agent has written one, and once its answer is posted or its turn completed with nothing to post. |
| `turn_ended_at` | When its turn was read as ended, recorded before any post of its answer; a post of that answer that fails opens its interval from this time. `NULL` until then. |

Superseded failed rows are retained for at least seven days from
`superseded_at`, measured on the runner's operator clock. Each conversation
relay pass prunes at most 100 eligible rows older than that bound, oldest first.
Only rows in state `failed` with a recorded `superseded_at` are eligible, and
only while the same conversation retains a delivered message with a greater
`written_us`. Until then the failed row is kept as the context boundary while
remaining absent from status and notice eligibility.

**`conversation_answers`** — the pieces of an answer, in order. Key: the
address, `turn` and `piece`. Columns: `text`, the piece's text;
`admission_position`, the message this piece answers;
`attempted_at`, the earliest unconfirmed post attempt, and
`latest_attempted_at`, the latest unconfirmed attempt. Both are recorded
before the provider can accept a post. A single attempt kept by an earlier
version supplies both bounds. `post_id` is the piece's opaque identity,
allocated before its first post and kept across retries. An attempted piece
from before migration 23 has no id. `legacy_read_at` records when its one
read-back began. `receipt_failure_deadline` records when a metadata read
failure can settle the attempt as unknown if receipt reads remain unavailable.
`delivery_unknown_at` records resolution after
the settle window without confirmation. The piece leaves the retry queue,
and a matching read-back that returns later replaces the unknown result.
`posted_at`, when the connector
accepted its post, `NULL` while outstanding; `failure`, the cause of its last
failed post, `NULL` once posted. A piece with `attempted_at` and no `posted_at`
may have been accepted. A piece resolved with unknown delivery keeps its
cause and no longer counts as pending. The conversation core looks for
unresolved attempts at the connector
before it is posted again. A rejected retry preserves earlier attempts unless
a read started after their latest request's timeout and found no post
(`project/instrukt/spec/feature/company/conversations`).

**`conversation_refusals`** — the platform's own replies owed to a sender in a
conversation (`project/instrukt/spec/feature/company/conversations`). Key: the
address, `sender` and `reason`: `not_own`, the one fixed reply to a sender the agent
does not answer, or `unavailable`, the one notice of the sender's current epoch.
Columns: `received_at`; `state` for `unavailable`: `owed`, `posting` (a post was
sent, its effect possibly in flight) with `attempted_at`, or `posted`; `post_id`,
the notice's opaque identity, allocated before its first post and reused
until the epoch ends; `covered_positions`, the admission positions that
made the notice due; `replied_at`,
when the connector accepted the reply, `NULL` while outstanding; `failure`. The row
records the decision, never proof of delivery. An `unavailable` row is retired
only when every covered position has settled and any attempted notice is
reconciled. A posted answer, a successful empty turn, a delivery-unknown
resolution or a superseded failure can settle a position. `not_own` rows stay.

A notice's `anchor_message` retains the provider id at the start of its
read-back range. Its admission coverage remains available to reconcile the
notice after a superseded failed message is pruned. Pruning applies only to
settled positions.

**`connector_states`** — one row per declared connector. Key:
`(company, connector)`. Columns: `state`, one of `connecting`, `connected`,
`failed` or `stopped`; `failure`, its cause; `updated_at`.

**`conversation_relay`** — one row: `failure`, the cause while the relay that
serves conversations is not running or cannot acquire its source, `NULL` while it
reads; `updated_at`, the start of the current failure, kept while the failure
persists across retries.

**`connector_read_back`** — one row per declared connector, owned by the
conversation core. Key: `(company, connector)`. Columns: `retained_until`, where the
last completed read-back started less a margin, `NULL` until one completes, written
only by a completed read-back in the transaction of its last retention and never
from a connection state or a retained message's time; `first_seen`, when the runner
first saw the connector declared; `missed`, the addressed messages older than the
answering bound that a read-back found and did not admit (conversation, message,
time, link), kept for the reporting period and one per message; both are declared
with the conversation core (`project/instrukt/spec/feature/company/conversations`). The row lives as long as the declaration. On
migration an existing connector's `first_seen` is its company's earliest retained
message, or the migration time.

**`runner_workers`** — one row per background worker of the runner: the history
workers (extraction, query responder) and the readiness refresh (`readiness`,
which keeps the runtime's provider snapshot current,
`project/instrukt/spec/feature/runner/session-continuity`). `failure` and `since`,
its current outside failure — for `readiness`, a refresh that failed or ran past
its limit — written by that worker alone on a change and cleared by its next
success; current state only. A row lives while its worker exists on this computer:
the history rows while history is on, `readiness` while the runner runs with a
runtime that reports provider facts.

A message is stored, with its sender's connector identity, before the
connector acknowledges it to its provider, so admission is taken up again
after a restart. A message's `refused` state and the reply owed to its sender
are written in one transaction.
Reading a finished turn and the connector accepting a post are separate
boundaries: an answer's pieces are stored in the same transaction as the
position that read them and the message's `answered` state, and `posted_at`
is set only when the connector accepted that piece. Recording the final
piece's receipt closes the message's no-work interval and clears its turn
update in the same transaction. These tables hold no
credential.

**`connector_private_messages`** — the messages a connector posted for one
person alone, such as a sign-in link
(`project/instrukt/spec/feature/company/slack-connector`). Key: `(company,
connector, channel, message)`, the provider's identity of the posted message.
Column: `posted_at`. These messages, and every message that carries the
connector's private-message marker whether or not its identity was recorded,
are left out of every range of accompanying messages.

### Retention

A removed todo's rows are purged with it when its recovery window expires.
An execution's row goes when its todo is released; a runtime start's when it
is completely bound. Explicit no-thread resolution retains the request for retry.
Pending runtime input, unresolved waiting slots and approval attention
remain until acceptance/consumption or a visible terminal disposition accounts
for them. Native child result retention belongs to the runtime, independently
of work-record attempt release.
Input identity remains while needed for source replay suppression. GitHub label
action identities remain with the todo's retained inbound evidence.
Standing deliveries retain pending work until eligible handoff or visible
terminal failure, and retain terminal activation identity while the local event
and activation records are retained. Session close deletes no responsibility
work. Responsibility generation evidence remains while needed by pending work,
start recovery or retained delivery identities. Runtime record retention is
separate and may limit resumption. A task failure's row is kept after its hold is
released, so the same failed run never sets the hold again, and is replaced by
the task's next failed run. Validators, deferrals, watched-ref observations
and listing copies are replaced in place. Key-lock rows are reused with
ownership cleared on release; they are not event history. Runtime bindings
remain as identity caches, including completed occurrences and standalone runs.
Local events and settled cursors are retained as operational accounting. Redis
stream retention separately bounds replay; occurrence duplicate marks expire
after seven days. Local history does not backfill a newly created Redis group.
Conversation bindings, addressed messages, answer pieces and refusal receipts
are kept with their text, captured context and framed input after delivery and
posting; a `failure` cause is the only value cleared, when its operation
recovers. A signed-in login is kept until it is signed out, removed by the
operator, refused at renewal or unusable. A closed sign-in loses its code
verifier, nonce and link at once and its row a day after `closed_at`. A
retired registration is kept while a login or a sign-in uses it. A
connector's private message is kept as long as its conversation.

### Agent capacity

The internal `todo_aliases` table retains prior slugs by immutable todo id for renamed pending-document lenses; aliases travel in the reflected todo record. The internal `agent_availability` table holds profile/model subjects, typed
available/unavailable/degraded status, optional absolute expiry and reason.
Migration moves the prior todo preference column and its fast/med/slow values
to tier light/standard/strong. Absence of capacity marks means available;
expired marks lapse at the command clock. These are internal storage details;
agents use `instrukt agents availability|status`, never direct database queries.

## Use cases

### UC-SD2: A write interrupted part-way leaves the last completed state

```gherkin
  Given a developer's computer is recording a change that spans several records, such as a stamp and the todo it advances
  When instrukt stops part-way through that write
  Then the next command finds the work as the last completed change left it
  And nothing of the interrupted change is present
```

## Known caveats

- Work's per-computer writer lock remains held during point refresh and stamp model observation; slow provider or runtime reads can delay other work commands.
- UC-SD1 is retired: reading the database directly is not a feature; agents use instrukt's commands. Its id is not reused.

- The local event log grows with activity; retaining it does not extend Redis replay or occurrence duplicate protection.
- A signed-in login and its refresh token lie in this database for as long
  as the login is kept; any process of the same operating-system user can
  read them.
- Conversation text is kept without a limit: addressed messages, what
  accompanied them, their framed input and the answers stay in the database,
  with the names and email addresses of the people who addressed an agent.
- Work recorded only in an earlier build's file store is not read. Reflect
  it to GitHub with that build before upgrading; after the upgrade it enters
  the database again by pull-through.
- Deleting the database loses work-record writes not yet reflected to
  GitHub. Reflected work returns by pull-through; the runner's state starts
  empty, so watched refs are observed afresh. Before any new
  start, the adapter discovers its semantic key through the adapter selected by `runner.runtime`; existing keyed
  threads are rebound rather than duplicated. A lost ambiguous start marker
  cannot be reconstructed when the runtime has no discoverable matching thread.

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/work/github-record](github-record.md) — reflection of the work tables to GitHub
- [project/instrukt/spec/feature/runner/procedure-standing-agent](../runner/procedure-standing-agent.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](../runner/procedure-event-delivery.md)
- [project/instrukt/spec/feature/runner/event-activation](../runner/event-activation.md) — the event log's events and consumers
- [sdlc/policy/state-storage](../../../../domain/sdlc/policy/state-storage.md)
