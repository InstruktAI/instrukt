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
  revision, as files under `~/.instrukt/state/blueprint/`), the history store
  (`state/history.db`), publication state, the global index, eval
  fingerprints, hook outcomes and the machine id. Their owners' contracts
  place them.
- **Review evidence stays outside it too:** head verdicts and exemptions live
  in `state/review.db`, and derived approvals and publication records in
  `state/review-publication.db`, each under its own lock and never under the
  writer lock (`project/instrukt/spec/feature/work/github-apps`, Review
  evidence storage). The pages the runner reads for automatic review live in
  `state/review-observation.db` under its own lock
  (`project/instrukt/spec/feature/runner/pull-request-review`, Observation
  store).


- **Requested reviews stay outside it as well:** the asks
  `instrukt review start` admits live in `state/requested-review.db` under
  `state/requested-review.lock`, never under the writer lock
  (`project/instrukt/spec/feature/runner/requested-review`, Requested review
  records).


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
  its sidecars and every lock — writer, reflection, withdrawal, runner,
  runtime-dispatch, history and conversation locks — derive from it, so a home or a state directory reached
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
  before consuming it, except status inspection and documentation reads.
- A withdrawal holds the per-computer withdrawal lock
  `~/.instrukt/state/withdrawal.lock` from before it records its intent
  (`withdrawal_intents`) until it has committed or abandoned it, or returned
  with the intent kept by a refusal. The lock tells a withdrawal in progress
  from a recorded intent no command is acting on; it is never requested
  while the writer lock is held, and it is released before the withdrawal's
  reflection acquires the reflection lock. A withdrawal's git and filesystem
  work runs under it and outside the writer lock.
- A change to a todo is computed from the record its command read and is
  written only while that record is unchanged. When another command changed it
  in between, the later write is refused whole, naming the todo and its door:
  run the command again, which acts on the current record
  (`general/principle/trust-and-guardrails`, interference).
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
  newer one as a finding while still reporting independently readable sections.
  Neither `instrukt status` nor `instrukt events status` takes the writer lock
  or changes the database, WAL or shared-memory files.
- Documentation reads — `instrukt docs validate`, `docs get`, `docs index`
  and `docs schema`, with their caller's admission, the todo-existence lookup
  and a `--target` lookup — read an older database the same way, through a
  migrated in-memory image: they take no writer lock and leave the database
  and its sidecars as found. The todo-existence lookup reads that way for
  every caller.

<!-- planned:fix-runner-snapshot-read-corrupts-state-db -->

- A process that has opened the database through SQLite, such as the runner,
  reads it through SQLite from then on, also where a status or documentation
  read takes the in-memory image elsewhere.

<!-- /planned:fix-runner-snapshot-read-corrupts-state-db -->

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

Version 32 removes the history tables (`history_conversations`, `history_fts`,
`history_projection_records`, `history_source_positions` and `history_sources`)
when none of them holds a row, so a database created at the current schema
holds none. Tables that hold rows stay until the history store has copied them
(`project/instrukt/spec/feature/runner/history`); it then drops them in one
transaction and has this database reclaim the freed space, in one hold of the
writer lock. Nothing but that move invokes reclaiming. A database past version
31 is refused by earlier builds, so no earlier build writes history here once
the move has begun, and the history store creates a missing database at the
current schema for the same reason.

Reclaiming rewrites the database without its free pages and then runs one
checkpoint; it is not a transaction, and the writer lock is held across both.
The rewrite commits to the write-ahead log: from then on the database holds no
free pages, while the database file keeps its length until a checkpoint has
applied every committed change to it. The checkpoint that follows the rewrite
does that and shortens the file, unless another connection is then reading a
snapshot taken before the rewrite; it does not wait for that reader. Where
such a reader prevents it, the file shortens at the next checkpoint that
reaches the end of the log, which SQLite runs after a commit once the log
passes its automatic threshold and when the last connection to the database
closes. Reclaiming does not truncate the log file `-wal`, which keeps its
length and is reused by later writes.

Persisted runtime requests and manual standing reservations enter domain code
with a required captured title. Migration fills a missing or null title with
the request's original key and preserves supplied titles. It covers unresolved
starts, bound requests, current and retired standing assignments, and manual
reservations. Native driver placement evidence likewise retains its captured
title; earlier placements recover it from retained title evidence, including
verified runtime bindings, or from their recorded role and original phase branch.
An omitted placement title does not establish a key-only native title. Remote
conversion is owned by `project/instrukt/spec/feature/work/github-record`.

Version 34 evolves the conversation tables for named company connectors:
`connected_conversations` adds `role` (default `'company-agent'`),
`session_scope` (default `'conversation'`), and nullable `opening_conversation`;
`conversation_messages` adds nullable `conversation_display`; and
`conversation_refusals` widens its `reason` check constraint to admit
`'not_admin'` alongside `'not_own'` and `'unavailable'`, preserving all
existing rows verbatim.

Version 36 rewrites a stored Antigravity manual standing reservation that
holds `thinking_level` to carry it as T3's `thought_level`.

Version 37 adds `withdrawal_intents` (Work tables), the durable intent of a
withdrawal whose working resources are being released. It starts empty.

Version 38 adds lookups of `session_bindings` by `worktree` and by `branch`,
of `todos` by `branch`, and of `holds` and `github_label_actions` by todo, so
the todos a computer's worktrees and work branches name, and their records,
are read without reading the rest of the record
(`project/instrukt/spec/feature/runner/worktree-reclamation`). It changes no
table, column or row.

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
| `fact` | The fact kind, in snake case (`preparation_verdict`, `build_verdict`, `approval`, `intent_checked`, `kickback`, `build_started`, `build_complete`, `role_complete`, `verification_disposition`, `finalize_ready`, `task_firing_started`, `task_outcome`, …). |
| `at`, `actor` | Time and acting session, GitHub account or person. |
| `source_comment` | GitHub decision occurrence, or `NULL` for other stamp forms. Unique per todo when present; retained in reflected stamps so reconstruction does not reapply the same decision. |
| `role` | The role the acting session played, or `NULL`. |
| `model`, `model_version`, `model_provenance` | Model identity, or `NULL` with `outside_session` or `unknown`. |
| `eval_fingerprint` | The published fingerprint copied at write time. |
| `human_involved` | The audit-only declaration, or `NULL`. |
| `evidence` | The fact's bound evidence (digests, commit, phase, round, reason, occurrence, carried-from stamp, disposition and follow-up todo). |

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
| `worktree`, `branch`, `base_head` | The todo's single shared `<slug>/work` worktree, branch, and captured starting head (`NULL` for worktree-less `kind: task` assignments; legacy prepare rows remain readable for history). Retained after release. |
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
| Verifier | `--verification` disposition bound to the complete `verification-findings` and current captured shared head. |
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

**`withdrawal_intents`** — a withdrawal of a local item whose working
resources are being released (`project/instrukt/spec/feature/work/todos`,
"Parking, withdrawal and delivery"). Key: `todo_id`. Local-only operational
intent: it is never reflected to GitHub and never copied to another computer.

| Column | Meaning |
| --- | --- |
| `todo_id` | The withdrawn todo (`todos.id`); removed with it. |
| `branch` | The work branch the withdrawal releases, or `NULL` when there is none. |
| `branch_head` | The commit that branch pointed at when the withdrawal last read it, or `NULL`. It is recorded before the branch is deleted, against that commit, and kept as the deleted branch's recovery commit. |
| `worktrees` | The worktrees the withdrawal releases, as JSON: each with why it belongs to the todo (`bound`, named by a session binding, or `on_branch`, with the work branch checked out), the checkout it had when the withdrawal checked it (its branch, or the commit it was detached at), whether it is released yet, its git directory (its administrative directory in the repository, which a move keeps), and its stage: how far the withdrawal's own moves of it have come, each recorded before the `git worktree move` it names — not moved by the withdrawal (left out of the JSON), `moving_to` its staging location, `staged` at the place that move put it, or `restoring_from` that place back to its path. The staging location is a sibling of its path in the same directory, of bounded length whatever the worktree is called and distinct for each worktree; the place a move put it is its destination or, when a directory took that destination meanwhile, the worktree's directory name inside it. |
| `workspace_path` | Where the todo's workspace is in the checkout the withdrawal was admitted from (`<checkout>/workspace/<slug>`): the one workspace the withdrawal checks and releases, whichever checkout of the project runs it again. |
| `workspace` | The workspace at `workspace_path`: `absent` when there was none to release when the withdrawal checked, `present` while it is there, empty, to release, `released` once removed or found gone. |
| `branch_state` | How far the work branch's release has come: `pending` while it is not deleted; `partial_marker` once a failed deletion is known to have left only the withdrawal's marker beside the branch it did not delete; `deleted` once it is deleted by this withdrawal, or found gone or recreated at a commit that does not descend from `branch_head`. |
| `started_at` | When the withdrawal began. |

The row is written, after the todo's record and its working resources were
checked, before any resource is released. While it exists the todo stays live,
with its claim as it was, or unclaimed when it had none. A change of the todo's
record committed while the row exists is refused, naming its doors:
`instrukt todo remove <slug>`, which completes the withdrawal, or the command
run again once it has ended. Nothing read from GitHub replaces the record while
the row exists: a pull or pickup listing keeps the local record, and
`instrukt todo refresh` refuses naming `instrukt todo remove <slug>`. What
reflection learns of its own receipts is still kept: a pull that finds this
computer's own revision on GitHub records it as confirmed, setting the revision
comment and remote revision to the ones GitHub holds. A conflict mark is kept
too, and confirms nothing: it marks the todo conflicted and leaves the revision
comment and remote revision as they were. A pull while the row exists answers
no `/instrukt` command and applies no label action it finds on the issue: each
stays outstanding, and the first pull after the withdrawal commits gives it the
outcome a withdrawn todo gives. An answer to a command, or a label action
recorded as ignored, is recorded only while the todo's live record is still the
one it was decided on and no row exists, in the transaction that records it; one
decided on a record that changed before it was recorded, such as an active
record the withdrawal committed over meanwhile, is not recorded and stays
outstanding. Whenever a command or label action is left outstanding this way,
and when the row is removed, the issue's stored `ETag` is dropped, so the next
pull reads the issue in full rather than being told nothing changed, even when
the issue has not changed again since.
Dispatch admission, read back after a reservation's reflection, refuses while
the row exists, in the same read that confirms the reservation, claim and
status. The row is updated as each step whose outcome must be recognized
afterwards happens: the branch's commit before the branch is deleted, a
worktree's stage before each move of it, and each worktree, the
workspace and the branch once released; recording a partial marker is
attempted before the marker is taken back, which goes ahead whether or not
that write succeeded (see the marker-only outcome below). It is removed in the
transaction that commits the withdrawn record, with `removal` naming exactly
what was released, or on its own only when a refusal rests on what git or the
filesystem positively shows — a resource that must be kept, or an attempted
release proven to have changed nothing or taken back — while the command
attempted no other release and the row records nothing released and nothing
under way: no worktree the withdrawal is moving, has moved or is moving back,
and no `partial_marker`. Then the
todo is left as it was.
Otherwise a refusal keeps the row with everything it records, `branch_head`,
worktree stages and `partial_marker` included, whatever refused the command,
such as files that appeared in the workspace before a partial marker was taken
back. The refusal names what the row records as released and what it records
under way; one that rests on an observation git or the filesystem could not
make, or on a release that returned a failure without its outcome proven, says
that the rest could not be established, claiming nothing beyond it.

The work branch is deleted by one `git update-ref --stdin` call, against the
commit the row holds, that first creates the withdrawal's marker ref
`refs/instrukt/withdrawals/<todo_id>` in the project's repository at that commit
and then deletes the branch. The call ends in one of four ways:

- **Confirmed deletion.** The call succeeds. The row records the branch deleted
  at that commit, then the marker is removed. A marker found while the row
  records the branch `pending` shows the branch was deleted at the marker's
  commit by a call whose success was not recorded.
- **Attempt proven ineffective.** The call returns a failure, and the refs read
  back show no marker and the branch still at that commit, or moved on from it:
  nothing changed, and the attempt does not count as one.
- **Marker only.** The call returns a failure, and the refs read back show the
  marker and the branch both still at that commit. Recording `partial_marker`
  in the row is attempted before the marker is removed, and the marker is
  removed whether or not that write succeeded: once it is gone there is nothing
  to misread. Once it is removed the row records `pending` again. A marker
  removal that fails leaves `partial_marker` recorded, written again with the
  refusal when the first write failed. While the row records `partial_marker`
  the marker shows nothing: the withdrawal run again removes it, records
  `pending`, and only then looks at the branch and deletes it.
- **Unsettled failure.** The call returns a failure and the refs read back show
  anything else; the marker is left.

The branch is never recorded deleted on a returned failure: the command
refuses with git's failure. An attempt proven ineffective counts as no attempt:
when the command attempted no other release and the row records nothing
released and nothing under way, the row is removed and the todo left as it
was. After any other returned failure — a marker left, an unsettled outcome, or
an ineffective attempt following another release — the row keeps what it
records, for the withdrawal run again to recognize what happened.

A withdrawal interrupted while the row exists is completed by running
`instrukt todo remove <slug>` again, which continues from the row before reading
GitHub: an action that arrived there meanwhile, such as a hold label, stays
outstanding until the first pull after the withdrawal commits. It first records
what the interrupted command did without recording it, and what became of the
resources since: while the row records the branch `pending`, a marker records
the branch deleted at the marker's commit, and whatever branch carries its name
now is left as it is; each worktree is recognized by its git directory, never
at the project's primary checkout nor where git reports it prunable, and acted
on only at its path or where its stage says the withdrawal's own moves can have
put it: being moved, its staging location or inside a directory that took that
location; staged, where it was staged; being moved back, where it was staged or
inside a directory that took its path. Found at its path, its stage records it
not moved, even when it was staged and someone moved it back there, and it is
read and checked there as a worktree the withdrawal has not moved; found
anywhere else there, its stage records it staged at that place, and it is never
taken for released. Until the withdrawal tries to remove a worktree, that is in
every stage but staged, git keeps its git directory for it alone, so the
withdrawal also reads every other worktree git lists: one listed elsewhere with
its git directory was moved there by someone else, even while the withdrawal was
moving it or moving it back, and is left where it is, never moved back, and
leaves the withdrawal. One not staged that git lists nowhere is released only
once git no longer keeps its git directory; a staged one is released once git
lists it neither where it was staged nor at its path and nothing is where it
was staged, whatever git lists elsewhere. Removing a worktree lets git give its
git directory, which git names after the worktree's directory name, to the next
worktree added under that name, wherever it is, so a checkout git lists with
that git directory anywhere but its path or where it was staged, even of the
same commit, is never taken for a staged worktree. The workspace at `workspace_path` is released when
the row records it present and it is gone. A workspace at `workspace_path` holding
files refuses the withdrawal and is named, whatever the row records of it. Without a marker, a branch absent, or recreated at a commit that does
not descend from `branch_head`, is recorded as deleted with the commit the row
holds and a recreated branch is left as it is; a branch that moved on to a
descendant of `branch_head` is recorded at, and deleted against, the commit it
moved to. What the row names as released stays released.

Each worktree is released through git's own worktree commands. It is read
again and checked at its path; a clean worktree still the todo's is moved with
`git worktree move` to its staging location, its stage recording that move
first, and read and checked again where the move put it. Still the todo's and
clean, it is removed there with `git worktree remove`; otherwise it is moved
back to its path, its stage recording that move first. One that holds
uncommitted changes or is locked by then refuses the withdrawal, naming its
path. One whose checkout is no longer the one the row records, or that has the
work branch checked out once that branch is recorded deleted, is no longer the
todo's: it is left at its path and leaves the withdrawal.
Whatever a move returns, git's worktree list is read back and the worktree
recognized by its git directory where that move can have put it, as when a
withdrawal is run again. Listed at its path, the move did not happen: its stage
records it not moved, and a move that returned a failure refuses the
withdrawal with git's reason, having changed nothing. Listed at its staging
location, or inside a directory that appeared there meanwhile, its stage
records it staged at that place before anything else is done there, and it is
released from there, even when the move returned a failure. Otherwise the row
keeps the move recorded and the refusal says where it is could not be
established. A worktree git lists where it is looked for whose git directory
cannot be read, when none listed there is recognized as this one, establishes
nothing: the refusal names where git lists it and git's diagnostic, and the
row keeps what it records. So does a worktree not staged that no worktree git
lists, apart from those it reports prunable, holds while git still keeps its
git directory: its directory is gone, or was moved without git recording
where, and the refusal names
`git worktree repair` and `git worktree prune` as its doors; and a staged one
git no longer lists where it was staged while something is still there, whose
refusal names that place.
A worktree is moved back only to a path nothing occupies: when git lists
another worktree there, or any file or directory is there, such as a checkout
created at the vacated path meanwhile, it is not moved back — git would move it
inside that directory. It stays staged where it is, and the refusal names that
place and says its path is taken; when that place is inside its path, the
refusal names moving it out with `git worktree move` before its path is freed,
rather than freeing the path. After a move back, the worktree is recognized by
its git directory in the same way: listed at its path, it is back and no
longer moved; listed where it was staged, the move back did not happen and it
stays staged there; listed inside a directory that took its path between the
look and the move, its stage records it staged there and the refusal names it;
listed anywhere else with its git directory, someone else moved it there: the
refusal names that place, and the withdrawal run again leaves it there;
when where it is cannot be established, the move back stays recorded and the
refusal says so, with git's diagnostic. A
`git worktree remove` that returns a failure keeps the row, and the refusal
says only what git's list and the filesystem show afterwards: git still listing
it where it was staged, or no longer listing it with the directory still
there or gone, or that this could not be established.
Once a worktree is moved, a git command started afterwards and addressed to its
path, such as `git -C <path> switch`, does not reach it: while the path is
vacant, it finds no worktree there and changes nothing; once a replacement
checkout occupies the path, it acts on that checkout and leaves the moved
worktree untouched. A process whose working directory was already inside it
is not stopped by the move (see Known caveats). A
withdrawal interrupted while it is moving a worktree, has staged it or is
moving it back continues from where it finds it when run again, removing it or
moving it back, with nothing to clean up
by hand unless its path is taken or git stopped inside its own move (see Known
caveats). This holds whatever ref storage the repository uses. A git
observation that fails decides nothing, and neither does a negative answer git
gives together with a diagnostic, such as for an object store it could not
read: the command refuses and the row keeps what it recorded, for the
withdrawal to be run again.

### Runner tables

Owned by the runner and written through its interface, in the same
transactions as the reactions they record. Work invokes that interface for
finalized publication and approval attention committed with their stamps.

**History** — retained history is not stored in this database. It lives in the
history store owned by `project/instrukt/spec/feature/runner/history`, which
survives source removal and is never reflected to GitHub or stored centrally in
Redis. History tables an earlier build left here with rows stay only until that
store has moved them (version 32).

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

**`consumer_outcomes`** — parked coordinates and their `announced` confirmation
remain after durable settlement. The announcement is attempted once afterwards;
only a successful append response followed by a state transaction sets the
boolean. Failure or interruption leaves a conservative unannounced count and
never creates replay work. Migration retires the open-announcement index and
announcement source, receipt, payload and abandonment metadata, preserving base
parking coordinates, outcomes, cursors, failures and confirmation booleans. The
same normalization applies to in-progress parking records before consumption.
Old open announcements get no final replay. The former recovery cursor is inert.

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
| `company`, `server`, `issuer` | The company home, the server and the authorization server it was made for; `company` and `server` are `NULL` for a host-level shared OAuth client (`mcp_clients.<name>`), which shares one `return_id` across every company referencing it. |
| `host_client` | The global `mcp_clients.<name>` key for a shared host client registration; `NULL` for a company-scoped registration. |
| `declared` | `1` for a client declared under `sign_in.client` or global `mcp_clients.<name>`, whose secret is read from its owning secrets file; `0` for a client instrukt registered dynamically. |
| `client_id` | The client's identifier, for a declared client as it was declared when the registration was made. |
| `client_secret`, `secret_digest` | A registered client's secret, `NULL` for a public client and for a declared one; for a declared client the SHA-256 digest of its secret, `NULL` when it has none. |
| `return_address` | The return address the client was registered with. |
| `registered_at`, `retired_at` | When it was made, and when a newer registration replaced it; `NULL` while current. |

**`company_issuers`** — durable identity of each company's built-in OIDC issuer. Key: `company`.

| Column | Meaning |
| --- | --- |
| `company` | The company home name. |
| `issuer_id` | Stable 32-character lowercase hexadecimal identifier from 128 random bits, forming `<ingress.url>/hooks/inbound/issuers/<issuer_id>`. Generated once and retained across restarts. |
| `public_jwks_json` | Public ES256 JWK Set JSON matching the private signing key stored in the company home's `secrets.env`. Never contains private key material. Authoritative source for rebuilding the derived discovery and JWKS documents in cluster Redis (republished idempotently without key rotation on ingress receiver startup, on `ingress.url` change, or via `instrukt mcp issuer publish`; rebuild cost is one Redis write per enabled company). |
| `updated_at` | RFC 3339 UTC timestamp when the public key set was last updated. |

**`mcp_generations`** — Key: `(company, person, server)`. Column:
`generation`, raised by every removal of that person's login or open sign-ins
for that server.

**`mcp_logins`** — one signed-in login. Key: `(company, person, server)`.

| Column | Meaning |
| --- | --- |
| `resource` | The server's `url` the login was issued for. |
| `subject` | The OpenID Connect `sub` recorded at sign-in; `NULL` with a declared identity request. |
| `return_id` | For `authorization-code` logins, the registration it was issued to (`mcp_registrations.return_id`, naming its authorization server and client). For `sign_in.mode: issuer` logins, the sentinel `issuer:<issuer_id>` tying the cached token to the company's active `company_issuers.issuer_id` (no `mcp_registrations` row). |
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

**`runtime_bindings`** — runner-owned authoritative current object identity,
scoped to computer/runtime/endpoint and keyed by `(runtime, semantic_key)`.
The row retains generation, fixed native start key/thread ID, native project,
captured request (including resolved model/modes and resolution source), checkout,
worktree/branch/base, display title and acceptance. Identity/request are durable
before native mutation. Retry replays them; acceptance commits with owner-local
work facts and opening receipt. Titles neither establish identity nor rebuild rows. A legacy binding whose
original request is unknown keeps that request unknown and stores its observed
native mode pair in nullable `legacy_modes`; known requests carry the pair only
in their request. Retired history preserves either form.

**`runtime_generations`** — runner-owned append-only retired-generation evidence,
including native identities, immutable request/selection/placement, acceptance and
replacement lineage. One current binding stays separate from history. Historical
assignment lookup joins the retired native ID to its retained owner role-attempt
or conversation facts; it never selects that ID as a current worker. This
reference does not recreate owner facts after their own retention removes them. Migration
preserves legacy bindings and external effects; joins missing IDs only from
existing company, standing and work owner records, refusing contradictory IDs.
A legacy unresolved start without a recoverable native ID refuses the entire
migration atomically. Release preparation inventories an authorized copy and
settles those starts using the old version. Explicitly proven unstarted legacy
requests can receive their first fixed-key reservation. Only after every old row
is accounted for is the active `runtime_starts` table removed; resolution-only
information remains inert audit evidence. No continuing retry-permission state
or operator start-resolution subsystem remains.



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
preserving captured current-generation identity and request. Every instrukt-owned start and wake,
including lane orchestrators and task occurrences, uses this table. Native
worker attempts do not enter this keyed-discovery table.

The old active start journal is removed after migration 29 accounts for its effect
evidence in current bindings and immutable generation history.

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

**`connected_conversations`** — one row per conversation session binding. Key:
the address (`(company, connector, conversation)`, with `conversation = ''` for
the shared standing session of a `session_scope: company` connector).

| Column | Meaning |
| --- | --- |
| `semantic_key` | The key its one session is started and woken by (`instrukt:conversation:<company>:<connector>:<conversation>` for `conversation` scope; `instrukt:conversation:<company>:<connector>` for `company` scope). |
| `role` | The role recorded from the connector declaration when the binding was created (default `'company-agent'`); caller assignment lookup reads this column to grant the connector's role. |
| `session_scope` | `'conversation'` (default) or `'company'`. |
| `reply_mode` | `'turn_output'` (default) or `'explicit'`, recorded from the connector declaration when the binding was created so a running session stays aligned with its opening assignment. Migration 35 adds the column defaulting existing rows to `'turn_output'`. |
| `opening`, `opening_conversation`, `request` | The addressed message (and its originating conversation when `session_scope = 'company'`) whose delivery starts the session, and the start request captured for that message before the runtime is called. |
| `session` | The bound session; `NULL` until a start is bound. An operator-confirmed replacement can rebind the conversation and reset its source position. |
| `source_identity`, `position_json` | The fixed identity of the bound native source and its opaque read position. A mismatch or missing identity on an existing binding is a configuration error; it advances neither the position nor native work. Initial binding and confirmed replacement establish the identity. |
| `display_title` | Readable title captured for the session. |
| `failure` | Cause of a source read failure or a blocked bound session, with its recovery door where applicable. Cleared when work can resume. |

**`conversation_messages`** — one row per addressed message. Key: the address
and `message`, the connector's identity of that message.

| Column | Meaning |
| --- | --- |
| `sender` | The connector identity of the person who wrote it, stored with the message before it is acknowledged. It decides nothing: admission reads the provider's record for it. |
| `sender_name`, `sender_email` | The name and email address the provider gave for the sender when the message was admitted: `NULL` until then, and the address `NULL` when the provider gave none. Once written they do not change. |
| `sender_is_admin`, `is_direct` | Booleans (`0` or `1`) recorded at admission: whether the provider reported the sender as a workspace administrator (`sender_is_admin`) and whether the conversation is a direct message (`is_direct`). Migrated rows default to `0`. Once written they do not change. |
| `conversation_display` | Optional provider-supplied channel or DM label captured for this message (e.g. `#channel` or `DM with Name`), used to tag turns delivered to a company-scoped shared session. |
| `thread` | Optional connector-supplied thread identifier when the message belongs to a thread (`NULL` for direct messages or unthreaded conversations). Keeps conversation IDs opaque to the company layer. Migration 35 adds the column. |
| `written_us`, `written_at` | When it was written: in microseconds, which orders a conversation's messages, and as a timestamp. |
| `arrived_at` | When it was stored. |
| `state` | `received` once stored; `refused` once admission refused the sender (`not_own` or `not_admin`); `pending` once the sender is admitted, what accompanies the message is captured and its input is framed; `sent` once the runtime accepted it; `failed` once its turn ended unsuccessfully, keeping its status and notice eligibility without blocking later messages; `answered` once its turn finished with an answer or completed successfully with nothing to post. |
| `text` | The message's text as written. |
| `context_json`, `boundary`, `content` | The captured earlier messages with provider IDs and the context limit, how far an early read was complete, whether more context must be read, the delivery boundary, and the framed input. These fields are null until capture. The framed input is set to the selected context before an attempt and retained through uncertain acceptance. |
| `context_delivery_ids` | The provider IDs selected for the most recent attempted input. The delivery transaction uses these IDs to write the receipts. |
| `attempted_at` | When the current attempt began, recorded with the boundary that identifies it before the runtime can accept it. `NULL` before an attempt, after confirmed non-acceptance, or when its turn settles. An acknowledgement applies only while its boundary identifies the outstanding attempt. |
| `dispatch_command_id` | The durable T3 command ID for the current send attempt, recorded before dispatch. An uncertain retry reuses its ID and framed input. The ID is cleared with the attempt when it settles. A retained CLI attempt without an ID cannot be replayed. Migration 25 adds the column. |
| `dispatch_write_at` | When writing the first request byte could have begun under the recorded command ID. Recorded before writing and cleared if zero bytes were written and no earlier write made delivery uncertain. The first byte starts the reconciliation bound. Cleared when the attempt settles. Migration 25 adds it. |
| `send_unknown_at` | When an unresolved send stopped blocking later work at `SEND_RECONCILE_BOUND`. The original frame, attempt boundary and time remain for a late match. The row is not dispatched again and produces no further waiting indication or notice. A match clears this field. Migration 25 adds it. |
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
Only rows in state `failed` with a recorded `superseded_at` are eligible.

Migration retires the former source-uncertainty flag while preserving message
states, boundaries, command ids, attempts and failures. An old unmatched-input
finding is retained in the message failure record and its binding identity is
made unavailable without clearing its session or position. The original source
identity cannot be reconstructed from that flag; ordinary bindings keep their
identity and position.

**`conversation_context_receipts`** — the messages the agent has already
received, with one row per conversation address and provider message ID.
Each row records the addressed delivery that last carried the message.
Both the addressed message and its accompanying messages are registered
in the delivery transaction, including when reconciliation proves
acceptance. Receipts stay while the carrying delivery is retained. When
a superseded failed delivery is pruned, its receipts are kept for the
same seven-day retention window, measured from the operator clock time
of pruning. The relay prunes at most 100 expired receipts per pass.
Migration 25 seeds the IDs of already delivered addressed messages.
Older accompanying messages have no stored provider IDs to recover.

**`conversation_answers`** — retained ordered answer pieces (from finished
turns under `reply_mode: turn_output` and explicit replies queued by
`instrukt company reply` under `turn = 'reply:<id>'`). Key: the address,
`turn` and `piece`. Columns: `text`, `admission_position`, `attempted_at`,
`provider_post_id`, `posted_at` and `failure`. The attempt timestamp records the
most recent posting attempt for status; it imposes no uncertainty deadline.
A NULL `posted_at` keeps a piece pending regardless of its age or prior outcome.
Only a successful post response confirms acceptance, with the provider's optional
opaque id and its posting time or the observed response time. No local id is
invented when the response omits one. Failure remains visible until confirmed
posting. The normal relay retries unconfirmed pieces in order, allowing duplicates
when a prior successful effect was not recorded before a restart.

Migration preserves text, order, admission associations, failures and confirmed
posting times, with an unknown provider id for old confirmed pieces. Old reserved
metadata ids are not provider ids. Unconfirmed pieces, including formerly
expired uncertain deliveries, become pending; receipt lookup and attempt-window
fields are retired.

**`conversation_refusals`** — the platform's fixed replies and best-effort
unavailable-notice attempts (`project/instrukt/spec/feature/company/conversations`).
Key: the address, `sender` and `reason`: `not_own` for the non-member fixed
refusal, `not_admin` for the administrator-only fixed refusal, or `unavailable`
for the person's current conversation outage. Columns:
`received_at`, `attempted_at`, `replied_at` and `failure`. For an unavailable
notice, the attempt timestamp is recorded atomically before provider I/O and
suppresses every further attempt in that outage, regardless of the response.
Recovery of the person's no-work interval retires that marker. `not_own` rows
and their reply records remain. Migration preserves old posting/posted attempts
and known reply times while removing notice phases, reserved post ids, anchors
and admission coverage; an old unattempted notice remains eligible.

**`connector_states`** — one row per declared connector. Key:
`(company, connector)`. Columns: `state`, one of `connecting`, `connected`,
`failed` or `stopped`; `failure`, its cause; `updated_at`.

**`conversation_relay`** — one row: `failure`, the cause while the relay that
serves conversations is not running or cannot acquire its source, `NULL` while it
reads; `updated_at`, the start of the current failure, kept while the failure
persists across retries.

**`connector_read_back_seed`** — migration 26 records the earliest
retained message time for each existing connector. It supplies a first
observation when the scan store has none and establishes no scan coverage.

**`state/connector-read-back.sqlite`** — the company conversation core owns
this store for one home's connectors. Its saved window is authoritative and
durable when first learned. Losing it can skip an unread question the window
would have recovered. Provider positions and seen identities record external
bookkeeping. The store has its own SQLite writer outside live intake. It uses the
canonical state directory. Status reads existing records without creating
the store or touching its sidecars.

Each connector keeps one active window with its start and lower bound, the
provider position, the current fetched page until processed, and any
provider retry time. Restarts resume those records. Only completion advances
coverage to the window's start less the overlap. Page identities are kept by
provider channel and message ID. A page with new addressed messages writes
them in one live-intake transaction. This transaction reconciles previously
received messages and records missed identities in `conversation_missed_identities`.
Live intake checks those same identities within its write transaction. The
first committed decision wins. Already seen page identities skip intake.
Provider reads run outside intake transactions. Unseen missed identity markers
older than the reporting period retire on a later intake batch.

A completed scan advances overall coverage while keeping each unavailable
channel's uncovered span separately. These spans are authoritative, owned by
the conversation core, and persisted as soon as a channel reports unavailable.
Losing one can skip unread questions even when the overall cursor survives.
Later scans retry the saved span, including channels absent from enumeration.
A retry extends a still-unavailable channel's span only when enumeration
also establishes that the channel belongs to the new scan.

At runner start and each scan start, the store retires expired prefixes and
prunes old records across connectors. The horizon is measured from each
span's own lower bound. Recovery records any expired prefix and clears the
remaining gap in one transaction. A connector no longer declared retires
its gaps, scan state and seen identities. Each saved gap has one operator
report for its expired span. As the expired boundary advances, the same
report is updated. It remains visible for 30 days after its last advance.
A channel whose ID changed remains pending under the old ID until its saved
span expires.

Seen identities retire after 30 days. Missed records within that period keep
the written time and provider link. They carry no message text. The active
page holds text only until intake and the scan checkpoint commit.

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

### UC-SD3: Use a linked state directory without splitting write ownership

```gherkin
Given the operator reaches the state directory through a linked home or linked state directory
Then writes and status work through the link
And writes from different homes that share the state directory wait for the same writer
But a link to an individual database file or sidecar is refused with the directory-link recovery
```

### UC-SD4: Inspect the computer when its database schema differs

```gherkin
Given the computer has a state database from another release
When the operator inspects status
Then older state is read with its retained records and no disk changes
And newer state is reported with the installer command while configuration is still shown
And a write against newer state is refused with the installer command
```

### UC-SD5: Local work and runner writes proceed while GitHub reflection is in flight

```gherkin
Given a developer has a project reflecting work to GitHub
And a GitHub reflection request for one of its todos is in flight on their computer
When other commands on the same computer list the local work, read that todo and its artifacts, and record a local change to it
Then the local change completes without waiting for the in-flight GitHub request
And listing the work and reading an artifact complete without waiting for that request or for a local write that is still open
And the reflected change and the concurrent local change both reach GitHub intact, with no conflict between them
```

<!-- planned:fix-runner-snapshot-read-corrupts-state-db -->

### UC-SD6: A process that holds the database open keeps it intact while it inspects it

```gherkin
Given an instrukt process holds the state database open, as the runner does
When that process reads the database for inspection, as the runner's workers do
Then another process still finds that process's lock on the database file in place
And another instrukt command can write the database and exit
And the holding process goes on reading and writing the database, seeing that command's write
And the database passes its integrity check
```

<!-- /planned:fix-runner-snapshot-read-corrupts-state-db -->

## Known caveats

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
- A git process stopped between creating a withdrawal's marker and deleting
  the work branch leaves both. The withdrawal run again records the branch
  deleted at the marker's commit and leaves the branch, which still points
  there, as it is: nothing is lost, but the branch outlives the withdrawal.
- Without a marker, a work branch deleted and recreated at a descendant of the
  commit the withdrawal recorded is indistinguishable from the branch moving
  on; the withdrawal deletes it against the commit it points at.
- Moving a worktree to its staging location changes where it is addressed,
  not what a process already inside it can do. A process whose working
  directory was inside the worktree before the move, such as an open shell,
  still reaches it at the staging location and can change its checkout between
  the withdrawal's last look and its removal. Uncommitted work it leaves there
  is kept, because `git worktree remove` refuses a worktree holding it; a
  branch it switched to keeps its commits.
- A staged worktree that someone else moves away from where it was staged,
  to anywhere but its path, before the withdrawal runs again is recorded
  released, as a removal whose receipt was lost would be, and is left where
  they put it, once git lists nothing where it was staged and nothing is
  there. Anything still at the place where it was staged, listed or not,
  keeps the row open and the refusal names that place; one moved back to its
  path is read and checked there as one the withdrawal has not moved: still
  the todo's, it refuses the withdrawal naming its path when it holds
  uncommitted changes, and one no longer the todo's is left there and leaves
  the withdrawal.
- A worktree the withdrawal has not moved, removed by someone else, whose git
  directory git gave to a new checkout added elsewhere under the same name, is
  taken for one moved by someone else: the new checkout is left as it is and
  the worktree leaves the withdrawal without being recorded released.
- A git process stopped inside its own `git worktree move`, after renaming the
  directory and before recording its new location, leaves git listing the
  worktree where it was, as prunable, while the directory is at its
  destination. The withdrawal run again
  refuses, naming the worktree and its git directory, until
  `git worktree repair <where it is>` records where it is.
- A failed branch deletion that left only its marker, where recording the
  partial marker fails both before the marker removal and again with the
  refusal, and removing the marker fails too, leaves the row `pending` with the
  marker present. The withdrawal run again then reads the
  marker as the deletion and leaves the branch, which still points at the
  marker's commit, as it is: nothing is lost, but the branch outlives the
  withdrawal.

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/work/github-record](github-record.md) — reflection of the work tables to GitHub
- [project/instrukt/spec/feature/runner/procedure-standing-agent](../runner/procedure-standing-agent.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](../runner/procedure-event-delivery.md)
- [project/instrukt/spec/feature/runner/event-activation](../runner/event-activation.md) — the event log's events and consumers
- [sdlc/policy/state-storage](../../../../domain/sdlc/policy/state-storage.md)
