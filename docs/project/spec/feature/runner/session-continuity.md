---
description: Runtime adapter primitives, T3 thread dispatch, recoverable standing-agent identity, session observation and record-backed completion.
visibility: public
delivered_by: [instrukt-first-build]
---
# Session Continuity — Spec

## What it is

The runner defines a Rust runtime-adapter trait and ships the T3 implementation
first. The machine chooses it explicitly with `runner.runtime: t3`. The adapter
translates instrukt's dispatch requests into the runtime's published host CLI;
its implementation is code, never configured command templates.

The runtime owns session lifecycle, input delivery, restart, revival, handover,
transcripts, cleanup and subscriptions to session activity. instrukt owns
events, schedules, dispatch decisions and session bindings. Runtime task or
event facilities do not replace instrukt's runner.

## Canonical fields

### Adapter primitives

The trait's operations take structured inputs and return structured results. A
session reference is the selected runtime name and its native id; instrukt
creates no replacement session identity.

| Primitive | Input and result | Released consumer |
| --- | --- | --- |
| `start_or_wake` | Project, stable singleton key and opening/event brief; returns the existing or newly started session and delivery acknowledgement. | Event activation of a standing procedure. |
| `start` | Project, opening brief and optional worktree request (base revision and branch); returns the session and, when requested, its worktree path. | Todo-role dispatch, setup-only worktree dispatch and scheduled task occurrences. |
| `send` | Native session id and text; acknowledges acceptance by that runtime. | Follow-up work, event delivery and one-send recovery. |
| `read_state` | Native session id; returns one of the five states below, session identity and available runtime metadata. | Reuse decisions, dispatch observation and task completion checks. |
| `caller_identity` | Selected adapter plus caller environment; returns the native session id and any verified agent identity, or no caller. Performs no runtime I/O. | Read-time `runtime.session_id` and commit attribution. |
| `read_conversation` | Native session id; returns the runtime's recent messages with their attribution and the read's extent. | Independent intent checking and worker-evidence inspection. |
| `link` | Two session ids and add/remove; returns acknowledgement or unsupported. Optional capability. | The released linked-collaborator goal and peer-discussion doctrine. |
| `notify` | Session or work-item reference and an available deep link; returns acknowledgement or unsupported. Optional capability. | Anomaly notification when no explicit `runner.notify` command is configured. |

Worktree support, linking, notification and agent/model selection are explicit
adapter capabilities. Availability information, when the runtime supplies it, is
adapter metadata; there are no configurable `available` or `model` commands. A
runtime operation failure is distinct from an unsupported capability.
Unsupported operations report the runtime and capability; an unavailable runtime
leaves work pending.

The opening brief carries the procedure's closure read, bound parameters,
optional task `occurrence: <RFC 3339>`, and supplied briefing text. The runner
binds procedure parameters before dispatch. User text travels as separate
process arguments or standard input, never as executable shell text.

### Public sessions surface

These commands remain because released workflows dispatch workers, inspect
intent evidence and continue recorded conversations through a runtime-neutral
surface. All accept `--json`; no command prints an instruction for an agent to
execute in place of its operation.

| Command | Contract |
| --- | --- |
| `instrukt sessions run [--procedure <id>] [--args <text>] [--brief <file\|->] [--worktree <name>] [--todo <slug>]` | Call `start` and return the native id and requested worktree path. With `--todo`, record the procedure-role binding through work. |
| `instrukt sessions send <id> <text>` | Call `send` for the identified session; return the id after acceptance. |
| `instrukt sessions alive <id>` | Call `read_state`; print its semantic state or report observation failure. |
| `instrukt sessions conversation <id>` | Call `read_conversation` for an identified conversation; report that this is a bounded recent read. |
| `instrukt sessions link [--remove] <a> <b>` | Call optional `link`, or report unsupported. |
| `instrukt sessions resolve-start <procedure>@<project> --session <runtime:id> \| --none` | Resolve an ambiguous standing-agent start under its key lock: verify and bind the named keyed thread, or record the operator's confirmation that no thread was created and no start remains in flight. |

`--procedure` is required except for setup-only `run --worktree <name> --brief
<file|->`. A setup run takes no `--args` or `--todo`, sends only the setup
brief, and returns a worktree path without an implementation instruction.
Worktree paths remain in todo-role bindings after a run ends; filesystem
observation supplies `worktree.exists`.

Session ending, self-ending and history search are outside this surface. The
runtime handles ending and cleanup; instrukt supplies no ending verb, agent-kind
instruction or distinct "yours to do" exit status.

### Runtime choice and feature availability

`runner.runtime` is global-only and names a compiled adapter. It has no default.
`t3` is the first shipped implementation. No code chooses a runtime from
installed tools or session environment variables. Unknown adapter names are
validation findings; runtime entries and verb templates are not settings.

| Feature | Needs |
| --- | --- |
| Standing agents (`modules.events`) | `start_or_wake`, `send`, `read_state`, `read_conversation`. |
| Todo dispatch (`modules.todos`) | `start` with worktree support, `send`, `read_state`, `read_conversation`. |
| Recurring tasks (within `modules.todos`) | `start`, `read_state`, `read_conversation`. |
| Linked-session input | Optional `link`. |
| Default anomaly notification | Optional `notify`; an explicit `runner.notify` command is independent. |

Enabling a module without a selected runtime, or with an adapter lacking a
required capability, is refused without changing configuration. The refusal and
`config validate` name the runtime and missing capability, or state that no
runtime is chosen. A temporarily unreachable runtime is an execution failure,
never a reason to select another adapter or create a replacement session.
Linked-session input and notification remain unavailable on T3's published CLI;
instrukt does not rebuild them.

### Standing-agent key and lock

A singleton is scoped to the selected runtime, registered project and
subscribing procedure. Its title is exactly
`instrukt:standing:<encoded-procedure>@<encoded-project>`, where the project
component is instrukt's project slug. Components are UTF-8 percent-encoded,
leaving only ASCII letters, digits, `-`, `_`, `.`, `~` unescaped; hex digits are
uppercase. Matching is exact, never title similarity or prefix matching. The
registered project's checkout selects T3's project independently of the title.
One registered project has one instrukt project slug on this computer. A project
slug or procedure-id change changes its key and requires explicit reconciliation
of the old binding before new activation; it is not an automatic request for
another standing agent.

The runner claims a row of `runtime_key_locks` in the state database before
reading, starting or waking that key. Only its owner invocation may perform that
sequence, through binding and delivery-result recording. A contending invocation
leaves the event pending. Lock changes use short transactions under the existing
writer lock; a transaction is not held over a T3 command. The machine runner
lock serializes eligible owners, including `resolve-start` while it resolves the
key. A successor that holds that lock can recover a dead invocation's key
ownership, retaining any unresolved start marker; elapsed time alone never
permits a second starter.

### T3 start-or-wake algorithm

1. Under the key lock, run `t3 thread list --project <checkout> --json`.
   The command must succeed and return a complete, valid project listing.
   Match `title` exactly and retain its `threadId` and `projectId`. The local
   `standing_bindings` mapping is a cache, never proof that a key is absent.
2. One match selects that thread and repairs the cache, including after loss
   of instrukt's database. Send the event brief with
   `t3 thread send <threadId> -`, supplying text on stdin. A busy thread
   queues it as its next turn; the adapter does not interrupt it. Commit the
   event's target in `standing_deliveries` before the external send.
3. Zero matches permits `start` only if no unresolved `standing_starts`
   marker or recorded target for this delivery exists. Commit that marker before running
   `t3 thread start --title <key-title> <checkout> -`. The initial message
   contains the procedure brief and event, so a successful start delivers
   that event without a second send. Bind the returned id and commit the
   acknowledged delivery outcome before releasing the key lock.
4. List failure, malformed output, an unrecognised listing shape or multiple
   exact matches leaves the event pending and starts nothing. Multiple
   matches additionally report a key conflict; selecting the newest is not
   a recovery rule. A failed send retains the same target and event for
   retry, never a request to create a new thread. Once a delivery has a
   recorded target, retry that same id; an absent or conflicting listed
   identity leaves it pending with an anomaly, not a new or retargeted start.

A missing, timed-out or unusable start result may follow successful runtime
creation. Keep its marker and reconcile by listing on a later invocation: one
keyed match recovers the session; any event whose acceptance was not recorded
may be resent to it. A successful empty list does not clear an ambiguous start:
report `resolve-start` once and wait for resolution. A provable rejection before
creation can clear the marker and retry later. A failed status read, renamed
thread or inaccessible session is never grounds for replacement of an unresolved
binding. Before starting after zero matches, any cached id must also be
confirmed absent, and no delivery may still target it; an extant cached thread
whose title changed is a key conflict.

Handled-event marks advance only after runtime acceptance and commit with the
delivery result and binding. Delivery is at least once across a lost
acknowledgement or crash between send and commit. Handlers reconcile their
subject idempotently; the adapter promises one keyed thread, not exactly-once
execution of an event.

### T3 command translation

Commands run on the computer hosting T3 and instrukt. `start` has no
remote-environment flag. The project operand is its registered checkout, not an
assumed current worktree: T3 may not recognise a linked checkout as `.`. The
adapter runs the installed `t3` on the host's executable search path; there are
no T3-specific configuration keys in this contract.

| Operation | CLI translation and evidence |
| --- | --- |
| Plain start | `t3 thread start <checkout> -`; stdout supplies the native id. No `--wait`: execution and acceptance are separate. |
| New worktree start | Add `--worktree <base-revision> --branch <name>`. The flag creates a new runtime-owned worktree; it is not an existing-worktree path. Read the resulting thread with `show --json` for the path. |
| Send | `t3 thread send <id> -`, no `--wait`. A busy thread receives the text as its next turn. |
| State | `t3 thread show --json --messages 0 <id>`. Decode runtime status and available pending-input, archive, project and worktree metadata. |
| Conversation | `t3 thread show --json --messages 100 <id>`. Return recent messages and their runtime attribution; this is no full archive or search. |
| Recover keyed identity | `t3 thread list --project <checkout> --json`; top-level array entries carry `threadId`, `projectId`, `title`, `status`, `link`, `providerInstanceId`, `model`, `settled`, `snoozed`, and timestamps. `settled` alone does not make a thread gone. |
| Link and notify | Unsupported by the published `t3 thread` CLI. Report the capability gap; use an explicit `runner.notify` command for notifications. |

For an already prepared worktree, reuse its bound thread via `send`. The CLI
provides neither an attach-existing-worktree flag nor a worktree path in the
observed list output. A fresh role that needs that existing worktree reports the
unsupported attachment rather than making a second worktree or silently running
in the project root. Losing a requested path after start is an incomplete start
result, not permission to repeat start.

### Observation, identity and completion

| State | Runtime observation |
| --- | --- |
| `running` | `running`, `starting`, `resuming` or `queued`. |
| `needs-input` | `waiting`, `waiting_for_input`, `waiting_for_approval` or a reported pending request. |
| `ended ok` | `idle`, `completed` or `stopped`, with no outstanding input. |
| `ended failed` | `failed`, `interrupted`, `cancelled` or `rolled_back`. |
| `gone` | An explicit unknown-thread response, or a confirmed archived/unavailable thread. |

An unrecognised status or response shape, command failure or unreachable runtime
is an observation error, never `gone` or success. Status spellings and optional
metadata must be verified against the installed T3 response before translation;
a fake runtime fixture is not evidence of the live shape.

Work records retain sessions per todo and role; subsequent work reads that
binding and observes it before `send`. An ended turn can leave a reusable
conversation. Artifact writers and stamps retain `<runtime>:<id>` identity. A
step is complete only with its required artifact and stamp. After an ended run
without either, or a gone session, the runner attempts one recovery send to that
same id; continued absence raises an anomaly. Observation failure retries
observation instead. Release claims through work's command contract; ending
conversations remains the runtime's responsibility.

### Use cases

#### UC-SS1: Share a clarification with a linked collaborator

```gherkin
Given a developer is working with linked conversations
And the configured runtime adapter supports sharing human input between them
When the developer types a clarification in one conversation
Then the clarification reaches the linked conversation identified as the developer's words
```

#### UC-SS2: A developer settles a standing agent whose start was interrupted

```gherkin
Given a standing agent's start result was lost and listing cannot recover its thread
When the developer confirms that no thread was created and no start remains in flight
Then the next wake-up may start that standing agent
But when the developer identifies the keyed thread that was created
Then instrukt verifies and binds that thread without starting another
```

#### UC-SS3: A developer on T3 Code has agents work after choosing T3 as their runtime

```gherkin
Given a developer has chosen T3 Code as the runtime on this computer
When instrukt dispatches work in a registered project
Then the compiled adapter starts its T3 conversation without configured command templates
And subsequent work reaches that recorded conversation
And an agent can read its recent messages through instrukt
```

#### UC-SS5: A developer switches on a feature their runtime cannot carry

```gherkin
Given a developer's selected runtime adapter lacks a capability a feature needs
When the developer enables that feature
Then instrukt refuses and names the runtime and missing capability
And configuration remains unchanged
But when the adapter provides all the required capabilities
Then the feature can be enabled
```

#### UC-SS6: A developer who has chosen no runtime is never given one

```gherkin
Given T3 Code is installed but the developer has chosen no runtime
When the developer enables work dispatch or standing agents
Then instrukt refuses saying that no runtime is chosen
And no runtime is picked for them
```

#### UC-SS8: Events keep one standing agent through a runtime outage

```gherkin
Given a standing agent already has a keyed T3 conversation
And instrukt's cached binding is missing
When an event arrives while T3 cannot be listed
Then the event remains pending and no thread is started
When T3 becomes reachable and dispatch is retried
Then instrukt finds the keyed conversation and sends it the event
And a second conversation is not started
```

#### UC-SS9: A standing agent receives new work in its existing conversation

```gherkin
Given a standing agent has an existing keyed conversation
When two runner invocations contend to deliver events to that agent
Then only the key lock's owner dispatches at a time
And the events reach the existing conversation, including as a next turn when busy
And no additional conversation is started for that key
```

#### UC-SS10: Failed delivery retains the standing agent's identity

```gherkin
Given instrukt has found a standing agent's keyed conversation
When sending an event fails
Then the event remains pending for delivery to that same conversation
And no replacement conversation is started
```

#### UC-SS11: Recover a created standing agent after its start reply was lost

```gherkin
Given T3 created a standing agent's keyed conversation but instrukt lost the start reply
When the runner resumes and lists the project's threads
Then it binds the single keyed conversation it finds
And retries unconfirmed event delivery there without creating another conversation
```

#### UC-SS12: Conflicting standing-agent identities are made visible

```gherkin
Given the runtime lists two conversations with one standing-agent key
When an event needs that agent
Then instrukt reports the identity conflict and leaves the event pending
And neither conversation is chosen arbitrarily and no third conversation is started
```

#### UC-SS4: An agent looks up what an earlier conversation said (to retire)

UC-SS4 is to retire: the superseded scenario remains only while its existing
test bindings are removed by the builder.

```gherkin
  Given a developer's runtime mapping fills how their runtime searches and shows past conversations
  When an agent following doctrine needs what an earlier conversation said about a subject
  Then instrukt searches that runtime for it, or tells the agent how to search with its own runtime's tools
  And shows a conversation the search finds
  But when the runtime it is running in leaves history search empty
  Then it is told which verb is missing, and no search is made
```

#### UC-SS7: An agent told that an ending is its to do ends the sessions itself (to retire)

UC-SS7 is to retire: the superseded scenario remains only while its existing
test bindings are removed by the builder.

```gherkin
  Given a developer's runtime fills ending as an instruction for the agent
  And an agent has released a todo whose child sessions are bound to it
  When the agent ends the todo's sessions and then its own through instrukt
  Then instrukt tells it, for each session, what to do with its own runtime's tools, and that doing it is the agent's
  And once the agent has done so, the child sessions and its own are ended in the runtime
  But when the developer's runtime needs no ending
  Then instrukt says so and nothing more is owed
```

## Known caveats

- UC-SS4 is to retire: history search and archive lookup are not released in A3.
- UC-SS7 is to retire: session ending and agent-kind instructions belong to the runtime.
- T3 has no atomic server-side keyed start. Request pingdotgg/t3code#31621
  would replace list-before-start inside the adapter; it is not implemented
  here. The local lock protects instrukt starters on this computer, not
  external actors creating duplicate titles. Preserve keyed titles in T3;
  renaming or deleting them can require operator resolution.
- T3's list JSON supplies the array and fields above, without a worktree
  path. The `show` response schema is not established by CLI help. Its decoder
  needs real-response verification; the existing fixture's `thread.worktreePath`,
  `pendingRequestCount` and `archived` fields alone establish no T3 guarantees.
- `--worktree` creates a new worktree; attaching another role to an existing
  one is not supported by this CLI. This is a delivery-cycle capability gap,
  not permission for instrukt to take over runtime worktree lifecycle.
- T3 accepts `--provider` only with `--model`; instrukt agent names and
  thinking modes alone do not select that pair. Until an adapter can resolve
  the requested pair, starts use the T3 project's defaults and report that
  limitation; preferred-agent fallthrough is not claimed on T3.
- Conversation reads are limited to the latest 100 messages. A reader needing
  older evidence reports the gap; no history service, transcript store or
  search implementation is added.

## See Also

- project/instrukt/spec/domain/runner
- project/instrukt/spec/feature/work/state-database
- project/instrukt/spec/feature/runner/event-activation
- project/instrukt/spec/feature/runner/work-dispatch
- project/instrukt/spec/feature/runner/anomaly-notification
- project/instrukt/spec/feature/configuration/layered-configuration
- project/instrukt/spec/feature/configuration/interpolation
