---
description: Unattended runtime adapter operations, native agent delegation and record bindings, keyed recovery and settled-thread inspection.
visibility: public
---
# Session Continuity — Spec

## What it is

The runner defines a Rust runtime-adapter trait and ships T3 first, selected
with global `runner.runtime` or, unset, as the one compiled adapter. It acts where instrukt has no agent:
todo orchestrator pickup, event-driven standing agents, recurring tasks,
keyed singleton discovery, event/approval delivery and conversation-link relay. Its implementation is code.

An agent operating in its own conversation uses runtime-native operations via
read-time `@config` instructions for delegation, child messages, peer messages,
conversation reads, current-thread listing, status inspection and steering.
Only a runtime capability absent natively may use an explicit adapter fallback.
Native child reports and endings reach the parent through the runtime, without
an instruction, instrukt child observation or a second parent wake.
The runtime owns lifecycle, input delivery, restart, revival, handover,
transcripts and cleanup. instrukt owns its work facts, events and schedules.

## Canonical fields

### Adapter primitives

The trait's operations take structured inputs and return structured results. A
session reference is the selected runtime name and its native id; instrukt
creates no replacement session identity.

| Primitive | Input and result | Released consumer |
| --- | --- | --- |
| `start_or_wake` | Execution checkout, exact procedure/home[/subject] responsibility, stable generation key and assignment facts; returns the existing or newly started session and runtime acceptance, separately from event input consumption. | Event activation of a standing procedure. |
| `start` | Project or company home, durable start key, opening brief and optional todo-worktree placement on the derived phase branch; returns the session and, when requested, its worktree path. | Runner pickup of a todo orchestrator, scheduled task occurrences and conversation sessions. |
| `send` | Native session id, structured input and delivery intent; acknowledges runtime acceptance separately from entry consumption. | Instrukt event/approval delivery and recovery of its own executions. |
| `read_state` | Scoped native session id; returns semantic state, identity and available metadata with provenance, preserving unknown and unsupported observations. | Keyed reuse, delivery-boundary checks and runner-owned execution observation. |
| `caller_identity` | Selected adapter plus caller environment; returns the native session id and any verified agent identity, or no caller. Performs no runtime I/O. | Read-time `runtime.session_id`, recorded role/assignment lookup, capability scope and commit attribution. |
| `conversation_source` | Optional durable changes since an opaque source position: normalized conversations/messages and native records, native ids and parent links, roles, available timestamps, typed runtime-neutral completed turns (native reference, ordered contributions, author kind/identity and text) and the next position; also supplies a native change-notification subscription. | History extraction and the independent conversation-link relay, without owning their storage, indexing or relay behavior. |
| `read_conversation` | Optional native recent conversation/terminal read with source, attribution when available and extent; never synthesizes messages from terminal text. | Adapter recovery when supported; agents read natively. Consumption and assignment evidence are separate capabilities. |
| `notify` | Session or work-item reference and an available deep link; returns acknowledgement or unsupported. Optional capability. | Anomaly notification when no explicit `runner.notify` command is configured. |

Worktree creation, shared native delegation and attachment to an existing
worktree are distinct runtime capabilities. T3's native MCP path supports shared delegation, existing-worktree attachment and explicit provider/model selection; its unattended CLI adapter lacks attachment and effort/options flags. Agent workflow requirements use typed native capability metadata;
unattended consumers use adapter capabilities. Conversation links use the
runner-owned contract in `project/instrukt/spec/feature/runner/conversation-links`
and require the durable conversation source with native change notifications and native input delivery. Notification and
agent/model selection retain their optional capability contracts.
Availability information, when the runtime supplies it, is adapter metadata;
there are no configurable `available` or `model` commands. A
runtime operation failure is distinct from an unsupported capability.
Unsupported operations report the runtime and capability; an unavailable runtime
leaves work pending.

Standing assignment input retains machine-readable procedure, home, subject,
effective role (declared or `event-handler`), attendance and interruptibility
facts beside its system-origin loader. Role-aware help and briefing use
`project/instrukt/spec/feature/configuration/roles-and-capabilities`; the binding's
recorded role/home/placement, not native provider identity, resolves its scope.
Native child reservations also retain their dispatcher reference; later claim
changes do not rewrite that relationship. Retained assignments whose producer
recorded no dispatcher migrate to explicit unknown evidence. Unknown dispatcher
provenance does not refuse otherwise legitimate commands or infer a parent from
the current claim.
After loss of local binding caches, semantic-key discovery recovers those fixed
facts through the selected adapter's supported assignment-evidence capability
with native provenance before reuse. Missing or
contradictory assignment evidence fails visibly; current publication cannot
replace unknown initialized settings. Any observed waiting delivery reserves
the shared slot until its entry is observed.

The opening brief carries the procedure's closure read, bound parameters,
optional task `occurrence: <RFC 3339>`, and supplied briefing text. The runner
binds procedure parameters before dispatch. User text travels as separate
process arguments or standard input, never as executable shell text.

For subscribed procedures, the opening input establishes the assignment before
its separately identified event data. Delivery has two intents: as soon as
possible (default) and only at entry. The adapter exposes steering, queue-only
or entry-only realization, as specified in
`project/instrukt/spec/feature/runner/procedure-event-delivery`. T3 uses queued
input rather than a required hook. Missing steering creates no finding.
Runtime acceptance is transport evidence, not entry consumption or completion.
One session-wide waiting slot combines instrukt event, approval and runner-owned recovery deliveries
while preserving their separate identities. Native agent messages and child
reports belong to the runtime and do not occupy this instrukt-owned slot.
Recovery here concerns only instrukt-started orchestrators/tasks, never their
natively delegated workers.

### The waiting message is a delivery log

The composer places the message composition time at the top. Each delivery is
one entry, ordered by arrival from oldest at the top to newest at the bottom.
Every entry retains its source identity, kind (`event`, `approval` or `recovery`), source occurrence time and arrival time. An unavailable
source time is explicitly unknown; the arrival time never substitutes for it.
Event entries mark their envelopes as untrusted data. Combining preserves every
entry and its timestamps; a retry retains the exact composed log and time.

### Runtime evidence capabilities

Native project identity is optional metadata, separate from instrukt project
identity and the scoped native session reference (runtime, endpoint and native
conversation incarnation). A missing native project id does not make an
assignment invalid. A runtime's idle or exited process supplies no successful
work outcome; unknown/unobservable, input-ready, running and terminal failure
remain distinct. T3-specific state translation is confined to its adapter.

Acceptance and consumption are distinct runtime capabilities. Each observation
retains its native reference when supplied, source, meaning and observation
time. Transport submission or a completed prior turn proves no consumption.
Unknown or unsupported consumption keeps the one waiting slot occupied and
reports the capability gap. Structured transcript/run ids are one adapter's
evidence, not shared prerequisites. Assignment recovery likewise uses an explicit
capability; unreadable initialization evidence stays a visible gap.

For T3, a persisted user message or assigned run can precede provider entry.
Preparing and queued runs supply no consumption evidence. A queued CLI send can
be accepted while its user message is absent from the recent CLI transcript.
Observation preserves accepted or unknown state until native entry evidence is
available; a run id alone never upgrades that state to consumed. The real-release
check in `project/instrukt/spec/feature/runner/t3-contract` exercises these
provider-free states alongside the fake.

### Operational sessions surface

`instrukt sessions list [--since <duration> | --all] [--json]` inspects the
runtime threads instrukt uses for keyed lookup, not an agent's way to look
around. It lists project sessions with separate semantic key and display title,
state and available native activity time with provenance, newest known time first. The default settled-thread window is three days;
active non-settled threads remain visible regardless of age. `--since` accepts
integer `s`, `m`, `h`, `d` or `w` durations; `--all` includes all settled threads.
The flags are mutually exclusive. Filtering uses only native timestamps whose adapter-declared meaning supports
the activity window. Missing activity is explicitly unknown and remains visible
as unfilterable; local observation time is separate and never substitutes for it.
An invalid supplied timestamp is an observation error. T3 uses the later of
`updatedAt` and `settledAt`; other adapters expose their supported meaning.

`instrukt sessions resolve-start <key> --session <runtime:id> | --none` resolves
instrukt's own ambiguous keyed starts under the key lock. It verifies and binds
the named keyed thread, or allows a fresh start when no thread or creation
remains. A retained first thread must be proven absent. Native delegated-child
attempts use the work record's `todo attempt`/`todo bind` contract. Standing home
shorthand `<procedure>@<project>` remains available; subjects and successors use
the full key.

`runtime.discuss` directs the agent to `instrukt discuss open <thread> [--message <text>]`
and `instrukt discuss close [--message <text>]`, using the native caller identity
and runner-owned links. This is the instrukt route for a runtime without native
linking; automatic turn-end relay remains with the runner.

Agent-native `run`, `send`, `alive`, `conversation` and `link` facades are absent
from this surface. Explicit capability-gap fallbacks, if a runtime needs one,
are named in its configuration instruction; no fallback is enabled for a native
operation. Session ending remains runtime-owned. Extracted history and merged
search belong to `project/instrukt/spec/feature/runner/history`, independently
of this operational session surface. The runner's automatic starts return acceptance
to its own dispatch path, never an instruction for an agent to execute.

### Runtime choice and feature availability

`runner.runtime` is global-only and names a compiled adapter. Unset, it is the
one adapter compiled into this instrukt.
`t3` is the first shipped implementation. No code chooses a runtime from
installed tools or session environment variables. Unknown adapter names are
validation findings. Runtime instruction entries are configuration prose, not
adapter verb templates.

| Feature | Needs |
| --- | --- |
| Standing agents (`modules.events`) | `start_or_wake`, input handover and consumption observation, `read_state`, assignment-evidence recovery, plus the assignment/continuation capabilities required by the subscription. Steering is optional. |
| Todo pickup (`modules.todos`) | Keyed orchestrator `start`, `send`, `read_state`, required delivery evidence and caller identity. Worker operations use native instruction entries. Workflow workspace requirements describe native delegation capabilities, separately from unattended adapter capabilities. |
| Recurring tasks (within `modules.todos`) | Keyed `start`, `read_state` and required start/recovery evidence. |
| History extraction (`modules.history`, default on) | Optional `conversation_source` with native change notifications; an unknown or unsupported runtime reports extraction unavailable, without selecting one or blocking configuration/import. |
| Conversation links | `conversation_source` (read-only durable projections and native change notifications) and native input delivery (`send`), per `project/instrukt/spec/feature/runner/conversation-links`. |
| Conversations (a company home's declared connector) | Keyed `start` in a company home, `send`, `read_state`, and `conversation_source` with native change notifications for the answer back, per `project/instrukt/spec/feature/company/conversations`. |
| Default anomaly notification | Optional `notify`; an explicit `runner.notify` command is independent. |

Receiving and publishing external facts need no agent runtime. A computer that
is the ingress and not the reacting computer (`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`) validates with events
enabled for a declaring home whether or not its runtime is installed there: the project's effective
switch for a project source, the machine's global switch for a company source.
Its receiver installs and serves as on any ingress. The standing-agent row
applies where activation happens: on the reacting computer, and for the events
a computer emits itself; a procedure activation the ingress accepts for an event
of its own fails visibly at its execution boundary while that runtime is not installed there.
Every other feature in the table keeps its requirement.

Enabling a module while `runner.runtime` names no compiled adapter, or with its native/adapter path
lacking a required capability, is refused without changing configuration, with
the ingress exception above. The refusal and
`config validate` name the runtime and missing capability, or the unknown
runtime name. A temporarily unreachable runtime is an execution failure,
never a reason to select another adapter or create a replacement session.
Enablement reads the selected workflow's typed `requires` declaration under
`project/instrukt/spec/feature/workflow/workflow-definition`; it does not infer
requirements from action prose. Missing steering is a supported degradation,
not an enablement failure. Conversation links read T3's durable projection database without a session token and use
native input delivery, not on a CLI link command. Notification remains
unavailable on T3's published CLI; instrukt does not rebuild it.

### Start keys and locks

Every instrukt-owned start has a stable semantic key distinct from its display
title. The selected adapter establishes exact discoverable key evidence; T3
encodes the key as the final component of its `--title`, while another adapter may use a native label.
Renaming display text never changes semantic identity or proves absence. Missing
or conflicting key evidence stays unresolved. Keys are scoped to the
selected runtime and execution checkout. Todo/task/run keys retain registered project scope; a conversation key carries its company:

| Assignment | Semantic key |
| --- | --- |
| Standing procedure | `instrukt:standing:<encoded-procedure>@<encoded-home>[:subject:<encoded-subject>][:generation:<n>]` |
| Runner-owned todo orchestrator | `instrukt:todo:<encoded-project>/<todo-id>:<encoded-role>:<attempt>` |
| Native child assignment | The same role-attempt semantic key is supplied to native delegation and retained by work; it is not an adapter-discovery key. |
| Task occurrence | `instrukt:task:<encoded-project>/<todo-id>:<encoded-occurrence>` |
| Other unattended starts | `instrukt:run:<encoded-project>:<request-id>` |
| Conversation session | `instrukt:conversation:<encoded-company>:<connector>:<encoded-conversation>` |

For standing keys, the canonical home token is `project:<slug>`,
`company:<name>` or `platform`; a person token retains its kind but cannot execute.
A subject token is the complete typed reference from the event envelope. Encode
each whole component with the rule below. Home identity omits the subject suffix;
subject identity requires it. New standing keys omit the generation suffix. A confirmed successor keeps
the exact retained key, including a suffix on keys recorded by earlier builds.
The generation number is kept separately.
T3 standing threads remain unsettled under automation. A human may settle
one; it remains findable, wakes on a message and can be unsettled, for either
home or subject identity and either attendance. Settling never permits succession.
An actual terminal removal requires reconciliation of retained assignment and
unresolved input before a successor can be admitted. A failed send, failed observation or
unresolved start never does. Prior generations remain evidence; one current
generation is admitted under the procedure/home guard and key lock. Discovery
checks all generation-family semantic keys for that responsibility before selection;
missing evidence of which generation is current is a conflict, never permission
to reuse a retired generation or start another. The four keyed-dispatch rules
below apply to the established current generation.

Components are UTF-8 percent-encoded, leaving only ASCII letters, digits, `-`,
`_`, `.`, `~` unescaped; hex digits are uppercase. Occurrences are the scheduled
instant normalized to RFC 3339 UTC with `Z`, omitting a zero fractional part
and trimming trailing fractional zeros; retries and catch-up retain that instant. Attempts are positive integers increasing per todo and role;
request ids are generated 32-character lowercase hex identities. Allocate and
persist them before dispatch. A retry retains its key; an unresolved attempt
cannot be bypassed by allocating another attempt or request id. Matching is
exact. The execution scope is independent of optional native project identity.

The `todo-id` is the immutable `todos.id` (32 lowercase hex characters), not
the mutable slug. A todo rename keeps its keys and any outstanding startup's captured branch and opening input; a later same-slug todo has a
different identity and cannot collide with the retained earlier keys.
A project-slug or procedure-id change requires reconciliation of outstanding
keys and bindings before dispatch under the changed address.
Completed task occurrences retain their key and are not started again.

Every start or wake claims `runtime_key_locks` before discovery and retains
ownership through binding and delivery-result recording. This applies to
operational start resolution and runner dispatch, not native child delegation. Lock changes are short
transactions under the writer lock; no transaction spans a runtime command.
The machine runtime-dispatch lock gates entry to dispatch and is released
before any runtime command. Each owner holds an OS lock for its exact key
through the operation, including `resolve-start`. An owner taking that key's
lock can recover the prior owner's row while retaining unresolved markers.
Elapsed time alone permits no new start.

### Human-facing thread titles

Every instrukt-owned start supplies `<label> · <semantic key>`. Labels use:

| Assignment | Label |
| --- | --- |
| Todo orchestrator or native child | `<role> · <todo-slug>` (including `prepare-orchestrator`, `work-orchestrator`, `builder`, reviewer and other declared roles) |
| Standing procedure | procedure name (last id component), followed by ` · <typed-subject>` when present |
| Task occurrence | `task · <task-slug>` |
| Other unattended run | `run · <procedure-name>` |
| Conversation session | `conversation · <connector>` |

The complete title is captured with the start request and survives retries and
renames. T3 retains supplied titles. Discovery reads the complete native title
from CLI listing/show only: the whole legacy key-only title or the suffix after
the last ` · ` must be an instrukt start key with a recognized assignment prefix
and no whitespace. The prefix is opaque;
nothing interprets its label. Recovery verifies the complete captured display
title or the whole legacy key. An unexpected relabel that still claims the key
is a conflict. A rename that removes the key from its title follows the
retained-ID case below. Exact full-title evidence is retained alongside
the decoded key. Both old key-only and new label-first threads participate in
the same exact-key match set; one is reused, two conflict, neither permits a
replacement for an unresolved start. Persisted start requests and manual standing reservations carry a required
captured display title. The state migration normalizes earlier requests without
a title to their original key before domain code reads them; retry uses that
captured key-only title. Deliberate driver placement evidence includes the exact
native display title, retained with the work binding and supplied on later
pickup even after a todo rename. Older deliberate placements recover their
title from retained evidence first, including a verified runtime binding even
when its request is unknown, then from the recorded role and original phase
branch. The current todo slug supplies no historical title evidence. Missing
recovery facts refuse conversion; omission alone establishes no captured title. The CLI entry boundary converts an older
public placement proof that omitted its title from its role and original phase
branch, while retaining the whole-key native allowance;
domain and stored placement evidence always carry the current required field.
Bound standing delivery verifies the captured
full title or the whole legacy key before accepting more events. No startup path
reads T3 databases. Titles are never rewritten on existing threads.

### Keyed dispatch and recovery

Every unattended path selects its adapter through `runner.runtime`. Under the
key lock, perform successful discovery by semantic key before creation; a single
exact match repairs the binding, zero matches permit creation only without an
unresolved start, and failed/conflicting discovery permits none. Failed sends
retain the target. Native assignment, acceptance and consumption observations
carry their own supported evidence and provenance.

### T3 realization of keyed dispatch

The same discovery and start rules govern every key, for both `start` and
`start_or_wake`:

1. Under the key lock, run `t3 thread list --project <checkout> --json`.
   Require a complete, valid project listing. Decode only the canonical trailing
   semantic key (or whole legacy key-only title), match that key exactly and retain
   `threadId` and any available `projectId`. Native project id is optional
   metadata, independent of instrukt project identity. The `runtime_bindings` mapping is a cache, never
   proof of absence.
2. One match selects that thread and repairs the cache, including after loss
   of instrukt's database. A recovered runner-owned thread must match its
   retained assignment; recover any requested worktree path with `show`.
   Native delegated children are recovered from native task evidence instead;
   T3 project listing need not expose them.
   Commit the binding before sending unconfirmed input. For standing events,
   commit the activation target in `standing_deliveries` before any runtime
   handover. A busy thread receives ASAP input as its next turn. Only one
   instrukt message waits there; additional input stays durable for the next
   combined message. Only-at-entry input remains in instrukt until idle.
3. Zero matches permits a new start only with no unresolved `runtime_starts`
   marker, bound session or recorded delivery target. Commit the marker,
   key, brief and worktree request before `t3 thread start --title <display-title>
   <checkout> -`, adding worktree creation flags where requested. Its initial
   message delivers the opening assignment brief, with any initial event retained
   separately; an acknowledged start needs no second send. Commit the complete binding and acceptance before clearing the marker.
4. Listing failure, malformed output, an unknown listing shape or multiple
   exact matches leaves the assignment pending and starts nothing. Multiple
   matches report a key conflict. Failed sends retain the same target for
   retry. An explicit unknown-thread answer means the thread record is absent.
   The binding and pending work stay in place until the operator confirms that
   the deleted thread's provider session stopped. A bound id that is archived is no replacement: nothing is started,
   its completed answers are still read, and the operator is told. A bound id
   that is alive but no longer lists under its key (renamed) keeps being used by
   its id after verifying the retained project and worktree. Another live
   match still makes the key conflicted. An inaccessible id is an observation error, retried
   (`project/instrukt/policy/checks-and-guards`: a new actor starts only where the
   earlier one is proven unable to act).

A lost, timed-out or incomplete start result keeps its marker. On restart,
reconcile that marker by keyed discovery before any new start for its key.
One match recovers a start whose native ID was never returned. A returned ID
is retained as a pending intent before reading the thread or verifying placement.
Verification and subsequent attempts use that exact ID. A native creation that
remains pending requires a fresh inspection of its actual semantic key and a
positive native project match. If the native project ID was not yet captured,
the exact ID must appear in the retained checkout's scoped listing with the
same opaque project ID as the inspection. The requested worktree is verified against
the observed placement. A mismatch keeps the input pending and status directs
the operator to restore the expected key and placement, then run the command
for the binding state in the **Recovery by case** table below. Native acceptance
evidence reconciles transport acceptance without another send.
Without that evidence, instrukt may resend the unconfirmed input there.
An empty listing alone never clears a marker. Keep the assignment pending and
show its key and the `instrukt sessions resolve-start <key>` door in status.
`--session` uses the operator's explicit ID. It requires a present thread
in the retained native project and any requested worktree, and refuses a
thread already bound to another assignment. An ID already returned by native
creation must pass the inspection described above. For an ID supplied by the
operator after a lost start result, the listing or thread state must identify
the native project if it was retained. The refusal directs the operator to
restore project evidence and retry the same explicit ID. With no retained ID,
`--none` records the operator's confirmation that no thread was created and no
start remains in flight. The resolved request retains its input and retry
permission under the same key until retried.
For a first start with a pending binding, a confirmed absent native ID directs
the operator to `sessions resolve-start <key> --none`. That command checks
absence by ID and atomically clears the pending binding and permits a fresh
start under the same key. A live thread or failed observation leaves the
binding in place. The dispatch reservation remains, and no replacement or
execution claim is created by this command.
A rejection proven to precede creation also permits that retry. Failure to
recover a requested worktree path leaves the result incomplete and preserves
the marker.

**Recovery by case.** Every place a start or delivery can stall is one of these;
whether a fresh start is made depends on the actor it would share the work with,
never on how long anyone has waited:

| Case | Established by | Outcome |
| --- | --- | --- |
| Nothing started | Listing failed, the runtime not ready, no permitted model, rejection proven before creation | Retried every pass until it succeeds. |
| Lost acknowledgement | An unresolved marker | Recorded; discovery reconciles every pass; `instrukt sessions resolve-start` is the operator's door. No fresh start. |
| First start pending, thread absent | A pending binding with no replacement intent and native confirmation of absence | `instrukt sessions resolve-start <key> --none` clears the pending ID and permits a fresh start under the same key. |
| First start pending, thread present | A pending binding with no replacement intent and a present native thread | `instrukt sessions resolve-start <key> --session <runtime:id>` verifies the exact thread for acceptance. |
| Committed binding, thread absent | Native confirmation that the committed thread is absent | Work stays pending. Once the operator knows the provider stopped, `instrukt sessions replace-deleted <key> --stopped <runtime:id>` retires the committed binding. |
| Pending successor, thread present | An unaccepted successor is present under a retained replacement intent | `instrukt sessions replace-deleted <key> --stopped <runtime:id>` resumes acceptance using the stopped ID already authorized for that intent. Both forms of `resolve-start` direct the operator to this command. |
| Pending successor, thread absent | A pending binding under a replacement intent and native confirmation of absence | Once the operator knows the pending provider stopped, `instrukt sessions replace-deleted <key> --stopped <runtime:id>` retires that pending successor. |
| Live conflict | Two or more threads under one key | Nothing chosen, nothing started; the door is to archive or delete the extra thread. |
| Archived | The bound thread is shown archived | Nothing started; its completed answers are read; the door is to unarchive or delete it. |
| Renamed, alive | The bound thread reads, its key no longer lists | Used by its id. |

An unresolved start, a live conflict, an archived thread and a deleted thread awaiting confirmation that persist past the
runner's `START_OPERATOR_BOUND` on the operator clock raise one anomaly per
process per runtime key (`runner.notify`). `instrukt events status` shows the
current condition and its door as soon as it is observed. A conflict lists the
known competing threads. Reconciliation clears the status finding when the
condition ends, whether or not a notification command is configured (`project/instrukt/spec/feature/runner/anomaly-notification`).

`instrukt sessions replace-deleted <key> --stopped <runtime:id>` is the operator's
confirmation that the named provider session has stopped. It requires the retained
binding to match the given key and session and the adapter to report the thread
absent. It retires the binding and uses the ordinary recorded start path under
the same key. A standing replacement increments the retained generation number.
For a todo or task, the work record must still hold the stopped session's
assignment and execution claim before the start is attempted. The returned
native ID is recorded before applying the replacement. Retry reads that ID
directly. If the start reply is lost before an ID is recorded, the operator
can supply the actual session with `sessions resolve-start --session` before
retrying. Replacement checks the work record again before committing the
runtime binding, work binding, claim, opening receipt and execution tracking
together.

If a pending successor is deleted before acceptance, both forms of
`sessions resolve-start` report the exact pending session's replacement command.
After confirming that provider stopped, the operator runs that command with
the pending ID. Instrukt checks its absence before retiring it. The original
work binding and claim remain in force while the next successor is pending.
The command bearing the latest stopped ID resumes the same intent after an
interruption. The next verified successor commits against the original claim.

The command reports success once the local acceptance commits. Deferred
reflection appears as a warning, and the same command can retry reflection
and report the already-committed session.

Runtime acceptance commits with its dispatch result and binding. Selected event
handover commits with that acceptance, while queued entry consumption remains
separate. A content-free wake alone consumes no event. Consumer
handled marks instead track durable activation acceptance under
`project/instrukt/spec/feature/runner/procedure-subscription-routing`; they do not
wait for a runtime send. Input delivery is at least once across a lost acknowledgement
or crash between send and commit. Handlers reconcile their subject idempotently;
one keyed thread does not imply exactly-once execution of its brief.

### Model selection and adapter translation

`project/instrukt/spec/feature/configuration/agent-selection` owns role-keyed
selection in `agents.yaml`. The adapter translates a profile's kind, existing
T3 provider-instance id and typed option names; it owns no model catalogue,
role order or tier chains. Every T3 built-in harness has shipped defaults.
ACP discovery and configuration remain with the configuration skill; instrukt
creates no T3 provider instance and implements no ACP protocol.

An instrukt-owned start selects from the last known readiness facts and never
acquires them itself. The runner keeps those facts current in the background, on
a wake of its own and never on a start; the adapter translates them. Generic
runner policy owns quota expiry, marking and the ordered walk of configured
profiles and tier chains; those decisions are not adapter translation. A refresh
that fails or runs past its limit keeps the last known facts and is reported in
`instrukt events status`; it never refuses a start. With no facts known yet, a
start proceeds in the configured order. A refusal by the runtime at the start
itself is the signal, handled as any refused start. Existing keyed conversations
retain their captured selection without a new-start probe.

For T3, a refresh reads the local server's `server.getConfig` WebSocket RPC and
its `providers: ServerProvider[]` snapshot. The runner issues one read-scoped
session when it starts, with `t3 auth session issue --scope orchestration:read
--ttl <lifetime> --token-only --label instrukt-readiness`, keeps its token only in
memory, issues the next one before it expires and then revokes the earlier one,
and revokes the last one when it stops; the token's lifetime and the limit of each
CLI launch are declared in `crates/instrukt-runner/src/runtime/_t3_readiness.rs`.
Server discovery, token framing and RPC framing follow T3's internal protocol.
Runtime-descriptor version, negotiated orchestration protocol and consumed
response-shape validation admit a snapshot; CLI/server release strings are
diagnostics, not exact-release gates. An unknown descriptor version or
protocol/shape, a failed or slow token issue and a failed read are a failed
refresh, reported with the operation and a safe failure category, without raw
outputs or credentials. A token that cannot be revoked is logged: it is
read-scoped and lapses at the end of the TTL it was issued with. A hard process
stop before revocation leaves the session bounded by its short TTL.

The same read answers publication's question which agent runtimes are
configured (`project/instrukt/spec/feature/publication/agent-entrypoints`):
each enabled provider instance in the response's server settings, its driver,
for an ACP-registry instance the registry agent it names, and the home it
reports: the home its settings name, otherwise the runtime's home variable in
its instance environment. An instance naming neither reports no home; a home
T3 takes from its own process environment is not in the response. A home or
overlay folder its settings name by a relative path is not known either: T3
resolves it against its own process working directory, which the response
does not report (its `cwd` is the workspace it serves), so such an instance
is reported with that path as unknown, with no home and no folder, and
nothing is guessed. An
instance T3 failed to create is reported disabled in the provider snapshot;
its settings still decide whether it is configured, so publication can repair
what stopped its creation. A default instance T3 runs without a settings
entry is configured when the snapshot reports it enabled. A Codex
instance on an overlay home (`shadowHomePath`) reports its shared home as the
home and the overlay as a folder in which a link to the published file must
exist: at the pinned release Codex runs from the overlay, and T3 links into
it only the shared entries present when it creates the provider, makes those
links itself and refuses to create the provider over a file that is not a
link (`apps/server/src/provider/Drivers/CodexHomeLayout.ts` at the pinned
release). The adapter reads an instance's home as the pinned release does
(`project/instrukt/spec/feature/runner/t3-contract`). A server that cannot be reached or
read answers nothing, and publication leaves the global agent files as they
are.

The T3 adapter interprets combined probe facts: Claude warning/unknown from a
failed or incomplete authentication probe is unready; Codex ready/unknown from a
successful custom no-auth probe is usable. It reports missing, disabled,
uninstalled, unauthenticated, unavailable or error-state instances with reasons.
Reported model slugs and aliases are acquisition facts; T3's native `auto` and
`default` picks are exempt from reported-model membership.

Runner policy applies those facts in configured order. Automatic starts skip
unavailable and degraded marks until expiry at the command clock; available
clears the addressed restriction. A usage window at 100% restricts the profile's
configured models it applies to until its reset. The adapter reports a window's
scope: at the pinned T3 release a Claude window scoped to one model family is
reported under the id `seven_day_<name>`, labelled `Weekly · <Name>`, and applies
to the profile's configured models whose family is `<name>`. A configured model's
family is the second `-`-separated segment of its id, after the provider prefix
(`claude-fable-5-1` is `fable`), or the whole id when it has one segment,
compared lowercase; T3's reported aliases tie only the current model of a family
to that family's name, so they are not used. Every other window applies to all
of the profile's configured models, and a scoped window matching none of them
restricts nothing. Missing resets use one hour from the time the
snapshot was observed and record that default. A snapshot's windows are applied
once, when the snapshot is read; a start applies the restrictions in force and
never applies the windows of a snapshot it reuses again, so a reused snapshot
never renews a lapsed restriction. Overlapping exhausted windows use the latest
active reset; a reset already past has lapsed. Applying a snapshot makes the
observed quota marks of every profile it covers match that snapshot: models it
reports at a full window are marked, and the profile's other observed marks are
removed in the same transaction. A failed refresh changes no mark. Observed quota marks and manual
restrictions remain independent: applying a snapshot never changes a manual one. Availability and skip diagnostics retain cause
and expiry. Launch-error text supplies no quota evidence or no-start proof.

The selected adapter supplies the selection-realization report: generic starts,
standing assignments and generations carry it without declaring native limits.
Configuration retains neutral effort and extra settings; the T3 boundary maps
effort to its native option ids for native delegation. Host CLI starts pass
instance/model and report their inability to express requested effort/options.

An unstructured launch failure retains its key and selected inputs for recovery.
Only proven pre-creation rejection or explicit no-start resolution permits chain
fallback. Capacity evidence never proves that startup created no thread.

### Owned child-command cleanup

Every child command instrukt launches has a process group whose descendants
are cleaned up on every outcome: success, failure, timeout and interruption.
Completion is not reported while instrukt-owned descendants remain orphaned.
This concerns instrukt's own commands, not a machine-wide process janitor or
the runtime's standing agent lifecycle, which T3 owns.

### T3 command translation

Commands run on the computer hosting T3 and instrukt. `start` has no
remote-environment flag. The project operand is its registered checkout, not an
assumed current worktree: T3 may not recognise a linked checkout as `.`. The
adapter runs the installed `t3` on the host's executable search path; there are
no configured command templates in this adapter contract. Agent-native
instruction prose is a separate configuration surface.

| Operation | CLI translation and evidence |
| --- | --- |
| Plain start | `t3 thread start --title <display-title> <checkout> -`; stdout supplies the native id. No `--wait`: execution and acceptance are separate. |
| Todo phase orchestrator start | Preparation: `--worktree origin/main --branch <slug>/prepare`. Work: `--worktree origin/main --branch <slug>/work`; resumption on another computer: `--worktree origin/<slug>/work --branch <slug>/work`. Creation refuses an existing local phase branch. Native shared children inherit the orchestrator's recorded phase worktree. |
| Send | `t3 thread send <id> -`, no `--wait`. A busy thread receives the text as its next turn under the shared waiting-slot rule. |
| State | `t3 thread show --json --messages 0 <id>`. Decode runtime status and available pending-input, archive, project and worktree metadata. |
| Conversation | `t3 thread show --json --messages 100 <id>`. Return recent messages and their runtime attribution; this is no full archive or search, and no evidence that a turn completed: completion comes only from the conversation source. |
| Recover keyed identity | `t3 thread list --project <checkout> --json`; top-level array entries carry `threadId`, `projectId`, `title`, `status`, `link`, `providerInstanceId`, `model`, `settled`, `snoozed`, and timestamps. `settled` alone does not make a thread gone. |
| Linked-thread source | The adapter reads the T3 projection database read-only, with per-conversation fingerprints and native database change notifications, as history does. Each link owns its source/delivery positions. No T3 session token or `subscribeThread` is used; sends use native input delivery. |
| Notify | Unsupported by the published `t3 thread` CLI. Report the capability gap; use an explicit `runner.notify` command for notifications. |

T3 project listing includes settled threads and supplies no server-side time
filter. The adapter matches their keys exactly. Sending to a settled thread
wakes a new turn and un-settles it; automation never settles a standing agent,
and a human settling one is never evidence of absence or permission for a
second keyed start. Archived or explicitly unknown threads are
gone. The published CLI omits subagent threads from project listing; this
surface reports only the threads the runtime exposes.

### One worktree per todo team

Preparation and work use separate T3-managed worktrees. instrukt derives `<slug>/prepare` and `<slug>/work`; agents never choose a bare slug or a role branch. Preparation starts from canonical main with `t3 thread start <project> - --worktree origin/main --branch <slug>/prepare --title <display-title>`. Its orchestrator and native shared children all use that preparation worktree. Creation refuses while the local phase branch still exists. This also applies
to replacement after proven thread absence. A native follow-up never starts
a replacement in the project root when it needs a retained phase worktree.
Retry or wake uses the retained native assignment and its phase worktree. Deliberate entry first returns placement and the configured native start instruction; after native startup, `todo prepare|work <slug> --placement-evidence <file|->` records supplied native session, key, title, recorded worktree, branch, head and source. The caller must be that native session in the matching registered tree. Git cwd alone never binds an orchestrator. Runner-created bindings instead use its verified adapter result.

Review binds the committed preparation-worktree head. An approving `instrukt todo stamp <slug> --verdict approve --phase preparation_head`, bound to `preparation-review-findings`, releases the independent reviewer. The preparer releases its claim before cleanup; that record operation appends `workflow.preparation_reviewed` atomically. The integrator merges those reviewed docs as soon as possible, before build, folds their pending markers in its integration worktree and publishes through the ordinary gate. Once integrated the docs are current on main; they do not wait for build delivery. Other todos that depend on them wait for that integration.

After confirmed preparation integration, the integrator removes the clean preparation worktree and deletes `<slug>/prepare` locally and on `origin`. Failure or dirty/in-use state retains the cleanup obligation and its cause; retained thread records and review evidence stay. Work starts later from the new main with `--worktree origin/main --branch <slug>/work`; on another computer its retained published work starts with `--worktree origin/<slug>/work --branch <slug>/work`. The work orchestrator and setup, builder, reviewer, fixer, deferrer and finalizer share that one work worktree through native shared delegation. Only the integrator writes main. Ad hoc authoring uses its own one-off worktree, reviewed before integration.

### Approval attention

Applying an approval or request-changes decision commits its work-record fact
before attempting attention delivery. If the todo's current execution claim
names a live orchestrator session, queue the decision, attribution and bound
evidence to that session with the as-soon-as-possible intent. A failed liveness
read or send retains a pending notice with its cause; retry rechecks the current
claim. A released or replaced claim cannot route a stale notice to its former
holder. If no live claimant exists, the decision is only a state update; next
pickup or a deliberate start derives from it. Approval attention neither starts
an orchestrator nor publishes a decision event. Re-reading an already applied
GitHub command creates no second attention item.

### T3 observation translation and work completion

| State | Runtime observation |
| --- | --- |
| `running` | `running`, `preparing`, `starting`, `resuming` or `queued`; this operational state supplies no proof of input consumption. |
| `needs-input` | `waiting`, `waiting_for_input`, `waiting_for_approval` or a reported pending request. |
| `input-ready` | T3 `idle`, `completed` or `stopped`, with no outstanding input. A settled thread is dormant and reachable; turn idleness establishes no task success. Listings retain its native `settled` metadata. |
| `failed` | `failed`, `interrupted`, `cancelled` or `rolled_back`. |
| `gone` | An explicit unknown-thread response, or a confirmed archived/unavailable thread. The thread's presence keeps the two apart: absent for the explicit unknown-thread response, archived for a shown archived thread. For an absent thread, the binding state determines the command in the recovery table above. |

An unrecognised status or response shape, command failure or unreachable runtime
is an observation error, never `gone` or success. Status spellings and optional
metadata must be verified against the installed T3 response before translation;
a fake runtime fixture is not evidence of the live shape.

Work records retain sessions per todo, role and attempt; subsequent work reads that
binding and confirms that its attempt is active and unreleased before sending
a native follow-up to it. An ended turn can leave a reusable
conversation within that active attempt. A released finalizer's record-only
readiness write uses its retained binding after confirmed completion and release; further
workspace work requires a new attempt. Artifact writers and stamps retain `<runtime>:<id>` identity. A
step is complete only with its required artifact and stamp. For a runner-owned
execution, after a terminal run without either, or a gone session,
the runner attempts one recovery send to that same id; continued absence raises an anomaly. Observation failure retries
observation instead. Such an execution is listed in `instrukt events status` until its workflow
state advances, whether or not a notification command is configured; recovered means
that its state advanced (`project/instrukt/policy/checks-and-guards`: what instrukt
cannot resolve goes to the operator). Release claims through work's command contract; ending
conversations remains the runtime's responsibility.

### Fresh canonical base

Before creating either `<slug>/prepare` or `<slug>/work`, fetch origin and use
`t3 thread start <project> - --worktree origin/main --branch <slug>/<phase> --title <display-title>`.
Only the integrator writes and pushes gate-green exact commits to main; local
main can lag and supplies no new-phase base. A fetch failure leaves startup
unperformed, with its cause visible. A retained published `<slug>/work` on
another computer is continuation of the existing phase, using freshly fetched
`origin/<slug>/work` with the same local branch name; it is never reset to main.
For repositories whose canonical branch has another name, use that remote ref.

## Use cases

### UC-SS1: Share a clarification with a linked collaborator

```gherkin
Given a developer is working with linked conversations
And the configured runtime adapter supports sharing human input between them
When the developer types a clarification in one conversation
Then the clarification reaches the linked conversation identified as the developer's words
```

### UC-SS2: A developer settles a standing agent whose start was interrupted

```gherkin
Given a standing agent's start result was lost and listing cannot recover its thread
When the developer confirms that no thread was created and no start remains in flight
Then the next wake-up may start that standing agent
But when the developer identifies the keyed thread that was created
Then instrukt verifies and binds that thread without starting another
And when the returned ID was recorded before a failed first inspection, the retry uses that same ID
And a different key or project keeps the work pending until the developer restores placement and resolves the exact ID
And when explicit resolution lacks evidence of a retained native project, it refuses until the evidence is restored
```

### UC-SS3: A developer has runner pickup after choosing T3

```gherkin
Given a developer explicitly chose T3 Code as this computer's runtime
When the runner picks up an eligible prepared todo for work
Then instrukt derives the phase branch and the adapter creates its orchestrator in that phase's one managed worktree
And the orchestrator delegates workers natively with shared isolation and records their role-attempt bindings
And all roles use that worktree and todo branch with one outstanding workspace role
And agent reads and follow-ups use configured native instructions
```

### UC-SS5: A developer switches on a feature their runtime can carry

```gherkin
Given a developer's selected runtime supports worktree creation and shared native delegation but not existing-worktree attachment
When they enable the shipped sdlc workflow
Then those capabilities permit its one-worktree todo teams
But a custom workflow requiring existing-worktree attachment is refused with the missing capability named
And a feature missing another required capability is refused without changing configuration
```

### UC-SS6: A developer who has chosen no runtime works with the compiled one, never a detected one

```gherkin
Given the developer has not set `runner.runtime`
When the developer enables work dispatch or standing agents
Then the one runtime compiled into instrukt is selected and named
And nothing about installed tools or the caller's session changes that choice
But when `runner.runtime` names no compiled adapter
Then instrukt refuses naming the unknown runtime
```

### UC-SS8: Events keep one standing agent through a runtime outage

```gherkin
Given a standing agent already has a keyed T3 conversation
And instrukt's cached binding is missing
When an event arrives while T3 cannot be listed
Then the event remains pending and no thread is started
When T3 becomes reachable and dispatch is retried
Then instrukt finds the keyed conversation and sends it the event
And a second conversation is not started
```

### UC-SS9: A standing agent receives new work in its existing conversation

```gherkin
Given a standing agent has an existing keyed conversation
When two runner invocations contend to deliver events to that agent
Then only the key lock's owner dispatches at a time
And the events reach the existing conversation, at eligible delivery boundaries when busy
And no additional conversation is started for that key
```

### UC-SS10: Failed delivery retains the standing agent's identity

```gherkin
Given instrukt has found a standing agent's keyed conversation
When sending an event fails
Then the event remains pending for delivery to that same conversation
And no replacement conversation is started
```

### UC-SS11: Recover a created standing agent after its start reply was lost

```gherkin
Given T3 created a standing agent's keyed conversation but instrukt lost the start reply
When the runner resumes and lists the project's threads
Then it binds the single keyed conversation it finds
And retries unconfirmed event delivery there without creating another conversation
```

### UC-SS12: Conflicting standing-agent identities are made visible

```gherkin
Given the runtime lists two conversations with one standing-agent key
When an event needs that agent
Then instrukt reports the identity conflict and leaves the event pending
And neither conversation is chosen arbitrarily and no third conversation is started
And status identifies the conflicting threads and how to resolve the conflict
And the operator receives one notification for the key in the running process after the bound
```

### UC-SS13: Recover todo orchestrator pickup after its start reply was lost

```gherkin
Given T3 created a developer's todo orchestrator but instrukt lost its start reply
When the runner resumes pickup after a restart
Then it recovers that orchestrator's keyed thread without starting another
But when a successful listing cannot find it
Then pickup remains pending under its unresolved key until explicit resolution or proof of rejection
And status gives the operator the resolve-start door
And the operator receives one notification for the key in the running process after the bound
```

### UC-SS14: A builder's ending wakes its idle orchestrator natively

```gherkin
Given a developer's orchestrator delegated its bound builder through the runtime
And the orchestrator is idle
When the builder reports its ending
Then the runtime wakes that parent with its native child result
And instrukt does not observe the child or compose a second ending delivery
And the parent checks the recorded attempt, completion stamp and release before deciding what follows
```

### UC-SS15: A busy orchestrator receives its child's native result

```gherkin
Given a developer's orchestrator is working while its native reviewer child runs
When that child reports its result
Then the runtime delivers native parent attention through its supported boundary
And no instrukt runtime-input entry or waiting slot is created for that result
And the parent relates the native child id to its recorded role attempt before deciding what follows
```

### UC-SS16: Assess native child failure without inferring workflow completion

```gherkin
Given a developer's orchestrator has a bound native child
When native task evidence reports a failure, input wait or confirmed absence
Then the parent assesses that evidence under its retained assignment
And instrukt neither polls that child nor produces a parent notice
And no completion stamp or attempt release is inferred
```

### UC-SS17: Dispatch a later todo that reuses a slug

```gherkin
Given a developer retains runtime threads for an earlier todo
And a later todo reuses that todo's slug with a different immutable ID
When the later todo dispatches a role or task occurrence
Then its runtime key uses the later todo's immutable ID
And it neither collides with nor adopts the earlier todo's conversation
And renaming the later todo preserves its existing keys
```

### UC-SS18: Wake a settled orchestrator in its retained conversation

```gherkin
Given a developer's orchestrator is settled but remains reachable in T3
When instrukt delivers an event or approval to its recorded session
Then T3 starts its next turn and un-settles that conversation
And keyed discovery still selects it without starting another thread
```

### UC-SS19: Find recent project conversations with truthful activity evidence

```gherkin
Given a developer's selected runtime exposes active and dormant conversations
When the developer lists sessions within an activity window
Then each result has separate semantic key and display title, state and available activity evidence with provenance
And old dormant sessions with verified activity times are excluded unless --all is given
And active sessions and sessions with unknown activity time remain visible
And local observation time never substitutes for native activity
And the JSON form carries the same facts
```

### UC-SS20: Wake an older standing agent by key

```gherkin
Given a developer's standing agent has been settled longer than the sessions-list window
When a later activation needs that responsibility
Then keyed discovery searches all project threads and finds its exact semantic key independently of display title
And old key-only titles and new readable titles are both discovered
And duplicate old/new titles for one key are reported as a conflict
And the existing conversation wakes without a new thread
And the listing window has no part in that decision
```

### UC-SS22: A developer who deleted a standing agent's thread has the subject's events handled in a new one

```gherkin
Given a developer's procedure keeps a standing agent for a subject
When the developer deletes its thread in the runtime and a later event for that subject arrives
Then no successor starts, the event stays pending and status gives the replacement command
When the operator confirms the old provider stopped through that command
Then the successor uses the same key and receives the pending and later events
But when the developer archived the thread instead
Then no successor starts, the event stays pending and status shows the developer the archived thread
```

### UC-SS23: A developer keeps using a renamed standing agent

```gherkin
Given a developer renames the bound thread of a standing agent
When a later event needs that agent and no other live thread claims its key
Then the event reaches the same thread by its retained ID within the retained native project
But if the native project changed, the event stays pending and status reports the conflict
```

### UC-SS24: A developer replaces a deleted task session consistently

```gherkin
Given a task's bound thread is deleted and the operator confirms the old provider stopped
When the operator replaces it
Then the runtime binding, work binding and claim move to the same successor
But if the work record cannot accept the binding
Then the command reports failure and reconciliation can recover the same successor
And if that pending successor is deleted, the operator can confirm its stop and replace it under the original claim
```

### UC-SS25: Recover a keyed assignment through the command for its binding state

```gherkin
Given a developer has a retained keyed assignment with an interrupted start or a deleted thread
When the first start is pending and its thread is present
Then the exact session resolution verifies the original thread for acceptance
But if that first thread is confirmed absent
Then no-start resolution clears the pending ID and permits a fresh start
When a committed binding or pending successor is confirmed absent
Then the printed replacement command retires the named ID after the operator confirms its provider stopped
And a successful replacement of a committed binding receives later work
```

### UC-SS21: Leave no instrukt-owned child processes behind (retired)

Retired: owned child-command cleanup is part of the prose contract above.

### UC-SS4: Conversation archive lookup (retired)

Retired on this operational session surface. Searchable history belongs to
`project/instrukt/spec/feature/runner/history`; the former adapter bindings
remain retired.

### UC-SS7: Agent-owned session ending (retired)

Retired: the runtime owns session ending; its test bindings have been removed.

## Known caveats

- Re-creating a deleted work orchestrator while its branch remains checked out
  in the retained worktree needs existing-worktree binding. T3's CLI lacks it;
  remove that worktree safely first, or start through the runtime's own existing-
  worktree binding where available (T3 MCP). Waking a settled retained thread
  uses its recorded workspace and has no such gap.
- T3 host-CLI ASAP delivery is queue-only. Consumption, assignment recovery and
  activity timestamps require their own supported evidence; a send acknowledgement
  or completed prior run proves no consumption.
- T3 native shared delegation inherits the parent's recorded workspace. Native
  children may be absent from project listings; their parent recovers them through
  native task evidence.
- UC-SS4 is retired on this session surface; searchable history is owned by
  `project/instrukt/spec/feature/runner/history`.
- UC-SS7 is retired: session ending belongs to the runtime.
- T3 has no atomic server-side keyed start. Request pingdotgg/t3code#31621 could
  replace list-before-start; the local lock protects only instrukt starters.
  Because T3 encodes semantic keys in titles, changing that encoding requires
  explicit identity reconciliation, never a replacement start from a lost match.
- T3's list JSON carries no worktree path. `show` decoder fields require verified
  runtime evidence; a fixture alone establishes no native response guarantee.
- T3 receives the resolved provider-instance/model pair. Its CLI has no
  effort/options flag; instrukt-owned starts get effort only from the T3
  project default until upstream adds support. No instrukt workaround is
  built. Native delegation passes supported resolved options; the full
  selection contract is `project/instrukt/spec/feature/configuration/agent-selection`.
- T3 conversation reads cover the latest 100 messages. Missing older evidence is
  a visible gap; the operational session surface is independent of extracted
  history under `project/instrukt/spec/feature/runner/history`.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/conversation-links](conversation-links.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
- [project/instrukt/spec/feature/runner/event-activation](event-activation.md)
- [project/instrukt/spec/feature/runner/procedure-standing-agent](procedure-standing-agent.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](procedure-event-delivery.md)
- [project/instrukt/spec/feature/runner/work-dispatch](work-dispatch.md)
- [project/instrukt/spec/feature/runner/anomaly-notification](anomaly-notification.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/configuration/interpolation](../configuration/interpolation.md)
