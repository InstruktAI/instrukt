---
description: Unattended runtime adapter operations, native agent delegation and record bindings, native keyed launch, automatic succession and settled-thread inspection.
visibility: public
---
# Session Continuity — Spec

## What it is

The runner defines a Rust runtime-adapter trait and ships T3 first, selected
with global `runner.runtime` or, unset, as the one compiled adapter. It acts where instrukt has no agent:
todo orchestrator pickup, event-driven standing agents, recurring tasks,
keyed singleton binding, event/approval delivery and conversation-link relay. Its implementation is code.

An agent operating in its own conversation uses runtime-native operations via
read-time `@config` instructions for delegation, child messages, peer messages,
conversation reads, current-thread listing, status inspection and steering.
Only a runtime capability absent natively may use an explicit adapter fallback.
Native child reports and endings reach the parent through the runtime, without
an instruction, instrukt child observation or a second parent wake.
The runtime owns lifecycle, input delivery, restart, revival, handover,
transcripts and cleanup, and performs the archives instrukt's thread retention
decides (`project/instrukt/spec/feature/runner/thread-retention`). instrukt
owns its work facts, events and schedules.

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
| `conversation_source` | Optional durable changes since an opaque source position, through two reads, each returning the next position; also supplies a native change-notification subscription. The turn read returns, for the conversations a caller names, normalized conversations/messages and native records, native ids and parent links, roles, available timestamps and typed runtime-neutral completed turns (native reference, ordered contributions, author kind/identity and text). The text read returns, over every conversation the source holds, each changed conversation by its native id with its own record, where the source holds one, and its finished messages, except a runtime's report of a background command it ran (for T3, a message whose `notification.source.kind` is `command` or `background_command`; a subagent's, a delegated task's or any other notification is returned), with native ids, roles, available timestamps, text and the native record without that text and without the members of its attached context that hold what the runtime captured; for T3, a `terminal` context record's `text`, a `review-comment` record's `diff`, an `element` record's `htmlPreview` and the `htmlPreview` of each of a `preview-annotation` record's `elements`, while every other member of those records, a review comment's `text` and an annotation's `comment` among them, stays. Each read validates, fingerprints and reads only the source tables it returns from; a read's fingerprint covers every row of those tables, the finished rows it leaves out of what it returns included, so a position of one read holds fingerprints the other does not share. | The independent conversation-link relay (turn read) and history extraction (text read), without owning their storage, indexing or relay behavior. |
| `read_conversation` | Optional native recent conversation/terminal read with source, attribution when available and extent; never synthesizes messages from terminal text. | Adapter recovery when supported; agents read natively. Consumption and assignment evidence are separate capabilities. |
| `notify` | Session or work-item reference and an available deep link; returns acknowledgement or unsupported. Optional capability. | Anomaly notification when no explicit `runner.notify` command is configured. |
| `finished_threads` | Registered project checkout; returns each thread the runtime lists for that project and reports finished, with its finishing time, its latest activity (a person reading or writing to it included), whether it is quiet (not running, queued, snoozed or waiting on a person's input or approval) and whether a person pinned it. A time the runtime does not report is unknown, never substituted. Optional capability: thread archiving. | Thread retention (`project/instrukt/spec/feature/runner/thread-retention`). |
| `archive` | Native session id and the finished-thread observation it was chosen from; archives the thread only while that observation still holds: present, finished, quiet, not pinned and without activity after the observed latest activity. Returns archived, already archived (which is done) or changed (not archived). Optional capability: thread archiving. | Thread retention. |

`execution_root` takes an execution root and a mode, check or ensure. It
returns whether the selected runtime holds a live project rooted exactly at
that root, creating one in ensure mode, together with the runtime's own door
for adding one. It never selects, alters or removes another project; an
unreachable or refusing runtime returns its cause and no answer. Its released
consumers are launch, for the root of the work it starts, and installation
(`project/instrukt/spec/feature/runner/module-installation`).

Worktree creation, shared native delegation and attachment to an existing
worktree are distinct runtime capabilities. T3's native MCP path supports shared delegation, existing-worktree attachment and explicit provider/model selection; its unattended native launch adapter supports existing-worktree attachment. Agent workflow requirements use typed native capability metadata;
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
The native binding is durable authoritative identity. Titles never rebuild it.
Assignment facts retained by the owning domain remain authoritative through
retries and succession. Unknown or contradictory ownership evidence fails
visibly; publication cannot replace unknown initialized settings.

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
runtime threads instrukt binds to its objects, not an agent's way to look
around. It lists project sessions with separate semantic key and display title,
state and available native activity time with provenance, newest known time first. The default settled-thread window is three days;
active non-settled threads remain visible regardless of age. `--since` accepts
integer `s`, `m`, `h`, `d` or `w` durations; `--all` includes all settled threads.
The flags are mutually exclusive. Filtering uses only native timestamps whose adapter-declared meaning supports
the activity window. Missing activity is explicitly unknown and remains visible
as unfilterable; local observation time is separate and never substitutes for it.
An invalid supplied timestamp is an observation error. T3 uses the later of
`updatedAt` and `settledAt`; other adapters expose their supported meaning.

Keyed starts replay their captured native command and thread IDs automatically.
There is no operator start-resolution or deleted-thread replacement command.
Manual standing reservations retain the separate `events bind` contract.

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
| History extraction (`modules.history`, default on) | Optional `conversation_source` with native change notifications; an unknown or unsupported runtime reports extraction unavailable, without selecting one or blocking configuration. |
| Conversation links | `conversation_source` (read-only durable projections and native change notifications) and native input delivery (`send`), per `project/instrukt/spec/feature/runner/conversation-links`. |
| Conversations (a company home's declared connector) | Keyed `start` in a company home, `send`, `read_state`, and `conversation_source` with native change notifications for the answer back, per `project/instrukt/spec/feature/company/conversations`. |
| Requested review (no module switch) | Keyed `start` in the project's registered checkout; `dispatch_send`, native input delivery under a command identity instrukt recorded before the frame, which the runtime accepts once, so sending it again opens no second turn and reports the acceptance (`project/instrukt/spec/feature/runner/t3-contract`); and `read_state` reporting whether a turn is in progress; per `project/instrukt/spec/feature/runner/requested-review`. Without them `instrukt review start` refuses, naming the runtime and the missing capability. |
| Default anomaly notification | Optional `notify`; an explicit `runner.notify` command is independent. |
| Thread retention (global `runner.archive_finished_after` set) | Thread archiving: `finished_threads` and `archive`. |


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

How configuration treats a retention window set while the selected runtime
lacks thread archiving is `project/instrukt/spec/feature/configuration/layered-configuration`'s.

### Start keys and locks

Every instrukt-owned start has a stable object key independent of its readable
title. Runner retains one current binding per object and a history of retired
generations. Each generation has a fixed native command key and thread ID,
derived from its execution scope, runtime endpoint/home, object key and generation.
Retries retain both IDs, the native project ID and the entire captured request.


| Assignment | Semantic key |
| --- | --- |
| Standing procedure | `instrukt:standing:<encoded-procedure>@<encoded-home>[:subject:<encoded-subject>][:generation:<n>]` |
| Runner-owned todo orchestrator | `instrukt:todo:<encoded-project>/<todo-id>:<encoded-role>:<attempt>` |
| Native child assignment | The same role-attempt semantic key is supplied to native delegation and retained by work; it is not an adapter-discovery key. |
| Task occurrence | `instrukt:task:<encoded-project>/<todo-id>:<encoded-occurrence>` |
| Requested reviewer | `instrukt:review:<encoded-project>:<head>`, the full id of the head commit asked for |
| Other unattended starts | `instrukt:run:<encoded-project>:<request-id>` |
| Conversation session | `instrukt:conversation:<encoded-company>:<connector>:<encoded-conversation>` |


For standing keys, the canonical home token is `project:<slug>`,
`company:<name>` or `platform`; a person token retains its kind but cannot execute.
A subject token is the complete typed reference from the event envelope. Encode
each whole component with the rule below. Home identity omits the subject suffix;
subject identity requires it. The object key stays fixed across succession;
its generation increases once under the object lock after confirmed native
archive or deletion, or after an idle thread is confirmed misplaced (T3
realization below), and only when outstanding or new work needs a session.
Settled and idle threads in their checkout's own project remain reusable.
Observation/send failures retain their target; they never establish terminal
presence. Native archive/deletion owns provider detachment; neither the create
receipt nor a misplacement rollover proves a process exit, and a rolled-over
thread is idle and never sent to again.

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

Every start or wake claims `runtime_key_locks` and retains exact-key OS ownership
through native operations and local acceptance. Short writer transactions persist
identity before creation; none spans runtime I/O. Native delegated children keep
the existing work-owned reservation and T3 delegation path. They acquire no
second launcher. Elapsed time never changes the generation.

### Human-facing thread titles


Every start captures a readable label without an object-key suffix. Todo roles
use `<role> · <todo-slug>`, standing procedures use their name and optional typed
subject, tasks use `task · <task-slug>`, requested reviewers use
`review · <branch> @ <first 12 characters of the head>`, and runs use
`run · <procedure-name>`.
Company owns conversation title formatting from optional connector display facts:
`<Company> · #<channel> · "<first words>"`, `<Company> · DM with <name>`, or
`<Company> · Slack thread` without enrichment. The preview uses the first eight
whitespace-separated words, capped at 80 Unicode characters with an ellipsis
when shortened. Titles are presentation only, frozen with each launch request.
Existing native titles are never rewritten by startup or migration.


### Keyed dispatch and recovery

The binding owns native identity. Native-ID reads establish presence, project
and placement; listings serve people and never choose an actor. When a pending
ID is absent, its exact native creation receipt distinguishes never-created from
created-then-deleted; that evidence survives projection deletion. A lost launch
reply replays the same command/thread IDs and opening message receipt. It neither
allocates another generation nor sends the opening assignment separately.

Confirmed native archive/deletion, or an idle thread confirmed misplaced, with
outstanding work allocates one successor under the object lock.
Standing transfers only unconsumed events using its
existing generation accounting. Work validates eligible active claims and commits
its binding/claim, runner acceptance, opening receipt and execution tracking
together. Company harvests available completed answers before adoption, preserves
answer posting identities/state, and transfers unanswered questions in order.
A released or ineligible assignment receives no successor.

### T3 realization of keyed dispatch

The adapter calls authenticated `orchestration.launchThread` over the existing
WebSocket transport with a fixed `commandId`, `threadId`, `projectId`, supplied
title (`generateTitle:false`), explicit model/modes and initial message. Scoped
short-lived tokens are revoked after use. T3 0.1.8 stamps WebSocket launches and
sends `createdBy:"user"`; that native stamp is accepted.

Before first launch, authenticated `GET /api/projects` supplies registered project
IDs and roots. Exclude deleted projects, compare canonical paths when available
and otherwise resolved original paths, and select the one live project whose
root is exactly the checkout. A project rooted at an ancestor or a descendant of
the checkout never stands in for it: `root` placement would run the session in
that other folder and `worktree` placement would branch another repository.
When no live project has that root, launch adds one through `execution_root` in
ensure mode and starts in it. The checkout is always the execution root instrukt
derives from the work's home — a registered project checkout, a company home or
the platform checkout — so only such a root is ever added. A retry of a
generation replays its captured project; a successor generation never inherits
a retired generation's project and resolves its checkout's project afresh. Two
live projects at one root are contradictory project evidence. An unreachable
runtime, a refused addition or malformed metadata leaves work pending before
launch with instrukt's own cause naming the checkout, which reaches operator
surfaces unchanged and is retried on later passes.

After model preparation, `server.getConfig` supplies normalized settings,
project overrides, managed limits and selected-provider mode capabilities.
Resolve the standalone CLI's `full-access/default` ceiling narrowed by managed
maximum runtime mode. Use effective global/project `defaultRuntimeMode` within
that ceiling, recording a ceiling fallback explicitly. If the selected instance
cannot support the inherited runtime mode, select its broadest supported mode
no broader than that mode, or `approval-required` if none qualifies. An absent
or empty native capability list preserves the inherited mode. Interaction stays
`default`. Unknown/malformed mode values fail before launch. Persist the typed
resolved pair and its native resolution reason with project/start/thread IDs
before native mutation; retries never recompute it. A present legacy thread
without captured modes supplies its actual pair through
`orchestration.getThreadProjection`.

Native placement uses `root`, `worktree` with the captured base/branch, or
`existing_worktree` with the retained verified absolute path/branch for a
successor. T3 owns preparation and release; a launch reply can precede completed
workspace preparation. Observe placement on later passes before accepting usable
worktree delivery, preserving base ancestry verification and never resending the
opening brief just because placement is preparing.

Every placement runs in the checkout's own project. Whenever instrukt adopts,
wakes or succeeds an instrukt-owned thread, and before every input it delivers
to one — event, approval, recovery and conversation input alike, for every
binding kind — it looks up the checkout's project in T3's project snapshot by
exact root, read-only, and compares it with the thread's native project. The
comparison never registers a project. The binding's recorded project is not
proof of a match, and a checkout with no live project of its own never
matches. A thread that does not match is misplaced: it is not adopted and is
sent nothing, and the input stays pending.

A misplaced thread reported `input-ready` or `ended failed` is rolled over by
its owner, at the same succession point and under the same object lock that
follow confirmed archive or deletion, with the same transfer of outstanding
work: a standing agent's delivery, a todo or task execution's observation or
follow-up, and a company conversation's succession. The new generation
launches as any successor does, adding the checkout's project if it is still
missing, and the misplaced thread is left as it is, as history; instrukt never
sends to it again. A todo successor attaches a retained worktree only when it
is a registered worktree of the checkout on the captured branch; a worktree
the misplaced thread holds elsewhere is left untouched, and the successor
starts with `worktree` placement on its captured base and branch. Input
delivery and approval attention never roll a thread over themselves; their
pending input waits for the owner's succession, and approval attention keeps
its own rule: once the claim moves to the successor, the notice is not routed
to the former holder, and the successor derives the decision from the work
record. Taking the object lock before any input lock, the owner never rolls
over from inside input delivery.

A misplaced thread reported `running`, `needs-input` or `unknown` is left
alone: a turn in progress, a pending request or an unobserved state is never
cut off. Its work stays pending, and operator surfaces carry instrukt's own
outcome naming the thread and the checkout, stating that the thread rolls over
once it is idle or archived.

A manual reservation never adopts a misplaced thread: a candidate a project
listing returns meets the same comparison and is refused, with no rollover.
A person writing to a company's agent is told only that the assistant is
unavailable for a while. No project root is retained beside the binding.

Service failures retain the captured request and actual cause for retry/status.
No unresolved-start permission state, five-minute start anomaly or operator
recovery command remains. Corrupt or contradictory binding/project evidence
fails visibly without arbitrary adoption. Acceptance is distinct from input
consumption and task completion.

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
each enabled provider instance in the response's server settings, its open
well-formed driver name (wire driver `claudeAgent` is reported as `claude`
for compatibility), without filtering through model-selection decoding. The
canonical selection name and the established native loader family remain separate:
an unfamiliar wire driver named `claude` does not acquire Claude loading merely
by colliding with the index alias. The adapter reports
registry-agent identity independently from the instance and driver names.
The adapter combines settings with the invocation's inherited native variables:
`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG_DIR`, `FX_HOME`, `KIRO_HOME`
and `PI_CODING_AGENT_DIR`. Settings translated into effective Codex/Claude
native variables and Codex shadow links retain their pinned precedence.
Otherwise a nonsecret instance value precedes an inherited native value.
Defaults use the inherited machine user home. Generic instance HOME/USERPROFILE,
OpenCode XDG, Grok variables and private Antigravity profile derivation do not
select instruction targets. Opaque/redacted overrides remain unknown.
Relocated Claude/fx targets carry `instruction_relocation_unverified`;
Antigravity's default target carries `runtime_home_differs` because pinned T3
forces a private profile. These facts qualify receipt separately from output.
A home or overlay folder its settings name by a relative path is not known either: T3
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

Every child command instrukt launches has a process group. Instrukt attempts
to stop that group on every outcome: success, failure, timeout and interruption.
It stops the descendants it can signal before reporting completion.
An absent process group is already stopped. On macOS, if the child has been
reaped when instrukt attempts cleanup, a group instrukt can no longer signal
counts as cleaned up. Instrukt logs the group ID, and the command keeps the
child's result. If instrukt records the reap during a refused signal attempt,
it tries once more with that evidence. A refused signal is an error while
instrukt has no reap recorded.
This concerns instrukt's own commands, not a machine-wide process janitor or
the runtime's standing agent lifecycle, which T3 owns.

### T3 command translation

Commands run on the computer hosting T3 and instrukt. `start` has no
remote-environment flag. The project operand is its registered checkout, not an
assumed current worktree: T3 may not recognise a linked checkout as `.`. The
adapter runs the installed `t3` on the host's executable search path; there are
no configured command templates in this adapter contract. Agent-native
instruction prose is a separate configuration surface.

| Operation | Native translation and evidence |
| --- | --- |
| Plain start | Authenticated `orchestration.launchThread` supplies fixed command/thread IDs, captured project/model/modes, title and opening message. Native acceptance and execution remain separate. |
| Todo orchestrator start | Native `worktree` carries the captured remote base and `<slug>/work` branch. A successor attaches its verified retained `existing_worktree`; native shared children inherit that workspace. |
| Send | `t3 thread send <id> -`, no `--wait`. A busy thread receives the text as its next turn under the shared waiting-slot rule. |
| State | `t3 thread show --json --messages 0 <id>`. Decode runtime status and available pending-input, archive, project and worktree metadata. |
| Conversation | `t3 thread show --json --messages 100 <id>`. Return recent messages and their runtime attribution; this is no full archive or search, and no evidence that a turn completed: completion comes only from the conversation source. |
| Operational listing | `t3 thread list --project <checkout> --json` supplies native identity, title, state and activity for people. Known semantic keys come only from retained bindings. Listing never selects an actor. |
| Linked-thread source | The adapter reads the T3 projection database read-only through the turn read, with per-conversation fingerprints over every record T3 holds of a conversation and native database change notifications. Each link owns its source/delivery positions. No T3 session token or `subscribeThread` is used; sends use native input delivery. |
| Notify | Unsupported by the published `t3 thread` CLI. Report the capability gap; use an explicit `runner.notify` command for notifications. |
| Finished threads | `t3 thread list --project <checkout> --json`. Finished is `settled` with `settledAt`. Quiet is a status of the `input-ready` or `failed` row below with `snoozed` false and no `pendingBackgroundTasks`; any other status, `unknown` included, is not quiet. A missing or malformed `status` or `snoozed`, or a malformed `pendingBackgroundTasks`, is not quiet; a listing that omits `pendingBackgroundTasks` reports none. Pinned is the projection row's `pinnedAt`. Latest activity is the latest of `updatedAt`, `settledAt` and the projection row's `opens.openedAt` (epoch milliseconds), read read-only from the projection database as for the linked-thread source, by bound parameters. A listed thread without a projection row is returned pinned, with unknown latest activity. |
| Archive | Re-reads the projection: the thread row present with no `archivedAt` or `deletedAt`, `settledOverride` `settled` with `settledAt`, no `pinnedAt`, no `snoozedUntil` later than now, and no `updatedAt` or `opens.openedAt` later than observed; no run of the thread whose status is other than `completed`, `failed`, `interrupted`, `cancelled` or `rolled_back`; no runtime request of the thread `pending`; and no delegated task of the thread whose status is other than `completed`, `failed`, `interrupted`, `cancelled` or `idle`. Anything else reports changed. A projection that cannot be read, or whose tables lack what the re-read consumes, fails the archive with its cause, and nothing is archived. Then authenticated `orchestration.dispatchCommand` with `thread.archive` and a fresh command ID, over the scoped-token connection runner-owned queued input uses, with a token issued and revoked for that command. T3's refusal that the thread is already archived is reported as already archived. |

Execution-root readiness reads the authenticated project snapshot and matches
canonical roots exactly, as launch does. Ensure mode adds a missing root with
`t3 project add <root>`, titled by T3 from the folder name, then reads the
snapshot again: the project it finds there is the result. T3's running server
accepts one active project per root, so a refusal because another addition
landed first is a reuse confirmed by the same read. The door reported for a
root that is still missing is `instrukt install`, or `t3 project add <root>`.

T3 project listing includes settled threads and supplies no server-side time
filter. The adapter reports their retained binding identities. Sending to a settled thread
wakes a new turn and un-settles it; runner automation never settles or archives a standing agent
(`project/instrukt/spec/feature/runner/procedure-standing-agent`), and settling one (whether
by a human or by the standing procedure itself when dormant) is never evidence of absence or
permission for a second keyed start. Archived or explicitly
unknown threads report `gone`, a released or terminal assignment receives no successor, and
archived transcripts remain extractable (`project/instrukt/spec/feature/runner/history`).
Agent-invoked settling instructions are governed by
`project/instrukt/spec/feature/configuration/layered-configuration`. The published CLI omits
subagent threads from project listing; this surface reports only the threads the runtime
exposes.

### One worktree per todo team

Each repository todo uses one T3-managed `<slug>/work` worktree and branch (agents never choose a bare slug or a role branch; `kind: task` creates no git worktree). The unified `todo-orchestrator` starts from canonical remote main through native keyed launch with `worktree` placement on `<slug>/work` (continuing from `origin/<slug>/work` on another computer, or from a legacy un-integrated `<slug>/prepare` branch during upgrade). Its orchestrator and native shared children — `intaker`, `architect`, `planner`, `builder`, `fixer`, reviewers, `verifier`, `deferrer`, and `finalizer` — all share that one worktree. A terminal successor uses native `existing_worktree` with its retained registered path and branch. A native follow-up never starts a replacement in the project root when it needs a retained worktree.
Retry or wake uses the retained native assignment and its worktree. Deliberate entry first returns placement and the configured native start instruction; after native startup, `todo work <slug> --placement-evidence <file|->` records supplied native session, key, title, recorded worktree, branch, head and source. The caller must be that native session in the matching registered tree. Git cwd alone never binds an orchestrator. Runner-created bindings instead use its verified adapter result.

A todo's recorded orchestrator resuming in its own placement needs no placement
evidence (`project/instrukt/spec/feature/work/dependencies-and-holds`).

Only the integrator writes main. Pending documentation authored in `<slug>/work` is folded into current contracts at work delivery, and after confirmed delivery the runner has the clean `<slug>/work` worktree released and `<slug>/work` deleted locally and on `origin`, on each computer that holds them (`project/instrukt/spec/feature/runner/worktree-reclamation`). Failure or dirty/in-use state retains the cleanup obligation and its cause; retained thread records and review evidence stay. The release goes through git's own worktree commands: the published `t3 thread` CLI has no operation that removes a worktree, and a delivered todo's thread keeps naming the path it was attached to. Ad hoc authoring uses its own one-off worktree, reviewed before integration.

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
| `running` | `running`, `preparing`, `starting`, `queued` or `waiting`; this operational state supplies no proof of input consumption. T3 reports `waiting` for a thread with no active run whose background work holds its completion — a delegated subagent, a background task, or an awaited command or monitor such as a pull-request watch — and for a run whose ended turn awaits checkpoint capture. Neither is a wait for input. |
| `needs-input` | A reported pending request (`pendingRequestCount` above zero), whatever the status. T3 reports a wait for input or approval only as a pending request. |
| `input-ready` | T3 `idle` or `completed`, with no outstanding input. A settled thread is dormant and reachable; turn idleness establishes no task success. Listings retain its native `settled` metadata. |
| `failed` | `failed`, `interrupted`, `cancelled` or `rolled_back`. |
| `gone` | An explicit unknown-thread response, or a confirmed archived/unavailable thread. The thread's presence keeps the two apart: absent for the explicit unknown-thread response, archived for a shown archived thread. Outstanding owner-admitted work after confirmed terminal presence uses the automatic succession contract above. |

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
step is complete only with its required artifact and stamp.

A runner-owned execution lasts while its session holds the todo's execution claim
(`project/instrukt/spec/feature/work/dependencies-and-holds`). Release of that
claim, or its transfer to another session, ends the execution. The runner acts on
that at the first observation that finds the claim released or held by another
session: from that observation on it sends the former session no recovery and
raises no anomaly for it, whether its thread is present, settled or gone
(`general/principle/session-lifecycle`: release of a claim, end of an execution
and loss of a session are separate facts). While the todo waits on a person, as
work decides for its `Needs you` marker
(`project/instrukt/spec/feature/work/github-record`), the session owes no evidence:
an ended turn receives no recovery send and raises no anomaly, whichever state its
thread reports (`general/principle/session-lifecycle`: runtime silence alone
establishes no missing work). For a todo that work offers for pickup, the offer
itself shows that no session holds the claim: that observation treats the
execution as ended without reading workflow's assignment. An observation that
reads workflow's current assignment for the todo
(`project/instrukt/spec/domain/workflow`) takes the claim holder and the person
wait from that one assignment and acts on what it read.

For every other runner-owned
execution, a present thread ending without evidence receives one recovery send;
a repeated ending raises an anomaly. Confirmed native archive/deletion with an
eligible active assignment uses the common successor and work-claim transaction.
Observation failure retries observation instead. Such an execution is listed in `instrukt events status` until its workflow
state advances, whether or not a notification command is configured; recovered means
that its state advanced (`project/instrukt/policy/checks-and-guards`: what instrukt
cannot resolve goes to the operator). Release claims through work's command contract; ending
conversations remains the runtime's responsibility.

### Fresh canonical base

Before creating `<slug>/work`, fetch origin and use
`t3 thread start <project> - --worktree origin/main --branch <slug>/work --title <display-title>`
(or carry forward an un-integrated legacy prepare branch when present).
Only the integrator writes and pushes gate-green exact commits to main; local
main can lag and supplies no new-worktree base. A fetch failure leaves startup
unperformed, with its cause visible. A retained published `<slug>/work` on
another computer is continuation of the existing branch, using freshly fetched
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

### UC-SS2: Operator start resolution (retired)

Retired: the former operator/title-discovery goal is removed.

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
Given a developer's selected runtime supports worktree creation, shared native delegation and existing-worktree attachment
When they enable a workflow requiring those workspace capabilities
Then configuration accepts those supported requirements
And an existing phase worktree remains available for native terminal succession
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
And instrukt retains its durable native binding
When an event arrives while T3 cannot be observed
Then the event remains pending and no thread is started
When T3 becomes reachable and dispatch is retried
Then instrukt reads the bound native conversation and sends it the event
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
When the runner resumes and replays the captured native launch
Then it receives the same native thread and binds it
And retries unconfirmed event delivery there without creating another conversation
```

### UC-SS12: Title identity conflicts (retired)

Retired: the former operator/title-discovery goal is removed.

### UC-SS13: Recover todo orchestrator pickup after its start reply was lost

```gherkin
Given T3 created a developer's todo orchestrator but instrukt lost its start reply
When the runner resumes pickup after a restart
Then it recovers that orchestrator's keyed thread without starting another
And replay preserves the same native command/thread IDs and phase worktree
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
And its native binding selects it without starting another thread
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
Then its durable native-ID binding selects the same conversation independently of title
And the existing conversation wakes without a new thread
And the listing window has no part in that decision
```


### UC-SS22: A developer who deleted a standing agent's thread has the subject's events handled in a new one

```gherkin
Given a developer's procedure keeps a standing agent for a subject
When the developer deletes its thread in the runtime and a later event for that subject arrives
Then one successor starts with a new generation and native start key
And it receives only pending and later events
And an archived thread with pending work also receives a successor
```


### UC-SS23: A developer keeps using a renamed standing agent

```gherkin
Given a developer renames the bound thread of a standing agent
When a later event needs that agent
Then the event reaches the same thread by its retained ID within the retained native project
But if the native project changed, the event stays pending and status reports the conflict
```

### UC-SS24: A developer replaces a deleted task session consistently

```gherkin
Given a task's bound thread is deleted or archived while its assignment is eligible
When ordinary runner pickup resumes its outstanding work
Then the runtime binding, work binding and claim move to one successor atomically
And a lost acknowledgement replays that successor with the same IDs
```

### UC-SS27: A handed-off orchestrator's conversation stays settled

```gherkin
Given a developer's orchestrator released its todo and settled its conversation
And another session has since taken that todo up
When the runner next considers the project's work
Then that conversation receives no recovery message and stays settled
And no stopped execution is reported for it
```

### UC-SS25: Keyed recovery commands (retired)

Retired: the former operator/title-discovery goal is removed.

### UC-SS21: Leave no instrukt-owned child processes behind (retired)

Retired: owned child-command cleanup is part of the prose contract above.

### UC-SS4: Conversation archive lookup (retired)

Retired on this operational session surface. Searchable history belongs to
`project/instrukt/spec/feature/runner/history`; the former adapter bindings
remain retired.

### UC-SS7: Agent-owned session ending (retired)

Retired: the runtime owns session ending; its test bindings have been removed.

### UC-SS26: Native delegation translates Antigravity effort to T3's thought_level option

```gherkin
Given a developer configures an Antigravity profile with xhigh effort in agents.yaml
When instrukt prepares native delegation inputs for a role using that profile on T3
Then the resolved native options map that effort to thought_level with value xhigh
And those options carry no other effort option
```

### UC-SS28: A developer's todo pickup is accepted once T3 finishes preparing its worktree

```gherkin
Given the runner picks up a developer's eligible todo on T3
And T3 answers the launch while it is still preparing that todo's worktree
When T3 finishes preparing the worktree during that pickup
Then the pickup accepts the orchestrator in that conversation and its managed worktree
And no failed start is reported for it
And the orchestrator has received its opening brief once
```

### UC-SS29: Start and deliver only in the checkout's own runtime project

```gherkin
Given an execution checkout has no runtime project rooted at itself
And an ancestor folder is registered as a runtime project
When the runner starts a session for that checkout
Then a runtime project is registered at the exact checkout
And the session starts in that checkout's project
And no session is started in the ancestor project
When new work arrives for an idle bound thread whose project is rooted elsewhere
Then that thread receives none of it
And a successor starts in the checkout's own project and receives the work
When new work arrives for a running bound thread whose project is rooted elsewhere
Then the work stays pending and that thread receives none of it
And the runner reports the thread and the checkout, and that the thread rolls over once idle
```

### UC-SS30: A developer's todo pickup moves to the checkout's own project when its created orchestrator idles elsewhere

```gherkin
Given T3 created a developer's todo orchestrator but the runner never accepted its start
And that orchestrator's thread is idle in a runtime project rooted elsewhere than the checkout
When the runner resumes the pickup
Then a successor orchestrator starts in the checkout's own runtime project, in that todo's managed worktree
And the todo's orchestrator is the successor
And the earlier thread receives nothing more
And no failed start is reported for the pickup
```

## Known caveats

- A legacy native admission whose original startup request is unknown remains
  pending after terminal presence until its owner supplies an authorized
  successor assignment. Existing worktree attachment does not invent its brief.

- T3 host-CLI ASAP delivery is queue-only. Consumption, assignment recovery and
  activity timestamps require their own supported evidence; a send acknowledgement
  or completed prior run proves no consumption.
- T3 native shared delegation inherits the parent's recorded workspace. Native
  children may be absent from project listings; their parent recovers them through
  native task evidence.
- UC-SS4 is retired on this session surface; searchable history is owned by
  `project/instrukt/spec/feature/runner/history`.
- UC-SS7 is retired: session ending belongs to the runtime.
- Pre-change unbound CLI starts cannot be retroactively deduplicated. Migration
  refuses an unresolved old start without a recoverable native ID and preserves
  all data. Settle it with the old version before upgrade.
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
- T3's `thread.archive` takes no condition on the thread's state. A message a
  person sends between the adapter's re-read and its archive command still
  reaches an archived thread, whose queued run T3 then cancels; the person
  unarchives the thread to continue.
- T3 does not record every opening of a thread in `opens.openedAt`. A reading
  it leaves unrecorded does not restart the thread's quiet time.

- T3 reports a turn that ended on a plain message, with no pending request, as
  `input-ready`; nothing in the thread tells a session that paused there from
  one that stopped. This bears only on an execution that still lasts: a session
  that an observation finds without the todo's execution claim is not weighed,
  whatever its thread shows. For a session that holds the claim, such an ending
  is left alone while the work record shows the todo waiting on a person. Outside that
  wait, when the workflow state has not advanced, it receives the one recovery
  send and a repeated ending raises the anomaly. A thread reported as `running`
  or `needs-input` is sent no recovery, and that state alone raises no anomaly.
  An earlier recovery input that ended unconfirmed is a separate ground
  (`project/instrukt/spec/feature/runner/anomaly-notification`): with the
  workflow state unmoved and outside a person wait, it raises the anomaly where
  the execution reaches the recovery decision, a thread reported as `running`
  or `needs-input` included. Confirmed native archive or deletion with an
  eligible active assignment does not reach that decision: the successor
  transaction comes first, and that input raises no anomaly there.
- An observation that reads the claim holder and the person wait acts on what
  it read. A release, a transfer or the start of a person wait that lands after
  that reading is taken up by the next observation; the observation under way
  can still send the recovery it decided on. The decision to send a recovery is
  taken at observation and is not taken again at delivery, so a recovery input
  already recorded can still reach a session whose claim was released, or whose
  todo began waiting on a person, after it was recorded.
  Whether and when it is delivered follows the ordinary delivery rules of
  `project/instrukt/spec/feature/runner/procedure-event-delivery`, those for a
  batch it shares with other entries included.

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
- https://github.com/pingdotgg/t3code/blob/bf469bb19669671d0df2efda74ed73867a7137c0/packages/shared/src/paletteSearch.ts — T3's `recordThreadOpen`, which decides which openings `opens.openedAt` records.
