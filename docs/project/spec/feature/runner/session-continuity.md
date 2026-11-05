---
description: Runtime adapter primitives, T3 thread dispatch, recoverable keyed start identity, session observation and record-backed completion.
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
| `start` | Project, durable start key, opening brief and optional worktree request (base revision and branch); returns the session and, when requested, its worktree path. | Todo-role dispatch, setup-only worktree dispatch and scheduled task occurrences. |
| `send` | Native session id and text; acknowledges acceptance by that runtime. | Follow-up work, event delivery and one-send recovery. |
| `read_state` | Native session id; returns one of the five states below, session identity and available runtime metadata. | Reuse decisions, dispatch observation and task completion checks. |
| `caller_identity` | Selected adapter plus caller environment; returns the native session id and any verified agent identity, or no caller. Performs no runtime I/O. | Read-time `runtime.session_id` and commit attribution. |
| `read_conversation` | Native session id; returns the runtime's recent messages with their attribution and the read's extent. | Independent intent checking and worker-evidence inspection. |
| `link` | Two session ids and add/remove; returns acknowledgement or unsupported. Optional capability. | The released linked-collaborator goal and peer-discussion doctrine. |
| `notify` | Session or work-item reference and an available deep link; returns acknowledgement or unsupported. Optional capability. | Anomaly notification when no explicit `runner.notify` command is configured. |

Worktree creation and attachment to an existing worktree are distinct explicit
adapter capabilities, alongside linking, notification and agent/model selection.
Availability information, when the runtime supplies it, is adapter metadata;
there are no configurable `available` or `model` commands. A
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
| `instrukt sessions run [--procedure <id>] [--args <text>] [--brief <file\|->] [--worktree <name>] [--todo <slug>] [--start-key <key>]` | Call `start` and return the native id and requested worktree path. With `--todo`, record the role-attempt binding through work. `--start-key` resumes an existing recorded request and accepts no changed start inputs. |
| `instrukt sessions send <id> <text>` | Call `send` for the identified session; return the id after acceptance. |
| `instrukt sessions alive <id>` | Call `read_state`; print its semantic state or report observation failure. |
| `instrukt sessions conversation <id>` | Call `read_conversation` for an identified conversation; report that this is a bounded recent read. |
| `instrukt sessions link [--remove] <a> <b>` | Call optional `link`, or report unsupported. |
| `instrukt sessions resolve-start <key> --session <runtime:id> \| --none` | Resolve any ambiguous start under its key lock: verify and bind the named keyed thread, or record the operator's confirmation that no thread was created and no start remains in flight. The standing-agent shorthand `<procedure>@<project>` remains accepted. |

`--procedure` is required except for setup-only `run --worktree <name> --brief
<file|->`. A setup run takes no `--args`, sends only the setup brief, and
returns a worktree path without an implementation instruction. With `--todo`,
it binds the `setup` role attempt; without it, it is a standalone run.
The returned evidence includes the start key. A retry addresses the recorded
key rather than allocating another request. Worktree paths remain in todo-role
attempt bindings after a run ends; filesystem observation supplies
`worktree.exists`.

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
| Todo dispatch (`modules.todos`) | Keyed `start` with worktree creation, `send`, `read_state`, `read_conversation`. The shipped sdlc workflow uses per-role worktrees. A workflow declaring shared-worktree attachment additionally needs that capability. |
| Recurring tasks (within `modules.todos`) | Keyed `start`, `read_state`, `read_conversation`. |
| Linked-session input | Optional `link`. |
| Default anomaly notification | Optional `notify`; an explicit `runner.notify` command is independent. |

Enabling a module without a selected runtime, or with an adapter lacking a
required capability, is refused without changing configuration. The refusal and
`config validate` name the runtime and missing capability, or state that no
runtime is chosen. A temporarily unreachable runtime is an execution failure,
never a reason to select another adapter or create a replacement session.
Linked-session input and notification remain unavailable on T3's published CLI;
instrukt does not rebuild them.

### Start keys and locks

Every start instrukt makes has a durable key carried as the thread's exact
`--title`, including setup-only and ad-hoc starts. Keys are scoped to the
selected runtime and registered project:

| Assignment | Exact title |
| --- | --- |
| Standing procedure | `instrukt:standing:<encoded-procedure>@<encoded-project>` |
| Todo role | `instrukt:todo:<encoded-project>/<encoded-slug>:<encoded-role>:<attempt>` |
| Task occurrence | `instrukt:task:<encoded-project>/<encoded-slug>:<encoded-occurrence>` |
| Other starts, including standalone setup | `instrukt:run:<encoded-project>:<request-id>` |

Components are UTF-8 percent-encoded, leaving only ASCII letters, digits, `-`,
`_`, `.`, `~` unescaped; hex digits are uppercase. Occurrences are the scheduled
instant normalized to RFC 3339 UTC with `Z`, omitting a zero fractional part
and trimming trailing fractional zeros; retries and catch-up retain that instant. Attempts are positive integers increasing per todo and role;
request ids are generated 32-character lowercase hex identities. Allocate and
persist them before dispatch. A retry retains its key; an unresolved attempt
cannot be bypassed by allocating another attempt or request id. Matching is
exact. The registered checkout independently selects T3's project.

A project-slug, todo-slug or procedure-id change requires reconciliation of
outstanding keys and bindings before dispatch under the changed address.
Completed task occurrences retain their key and are not started again.

Every start or wake claims `runtime_key_locks` before discovery and retains
ownership through binding and delivery-result recording. This applies to
manual session commands as well as runner dispatch. Lock changes are short
transactions under the writer lock; no transaction spans a runtime command.
The machine runner lock serializes eligible owners, including `resolve-start`.
A successor holding that lock may recover a dead invocation's key ownership
while retaining unresolved markers; elapsed time alone permits no new start.

### T3 keyed dispatch and recovery

The same discovery and start rules govern every key, for both `start` and
`start_or_wake`:

1. Under the key lock, run `t3 thread list --project <checkout> --json`.
   Require a complete, valid project listing. Match `title` exactly and retain
   `threadId` and `projectId`. The `runtime_bindings` mapping is a cache, never
   proof of absence.
2. One match selects that thread and repairs the cache, including after loss
   of instrukt's database. A recovered todo-role thread must use its
   recorded branch rooted at the captured base; recover any requested worktree
   path with `show`.
   Commit the binding before sending unconfirmed input. For standing events,
   commit the target in `standing_deliveries` before `send`. A busy thread
   receives the brief as its next turn.
3. Zero matches permits a new start only with no unresolved `runtime_starts`
   marker, bound session or recorded delivery target. Commit the marker,
   key, brief and worktree request before `t3 thread start --title <key>
   <checkout> -`, adding worktree creation flags where requested. Its initial
   message delivers the opening brief; an acknowledged start needs no second
   send. Commit the complete binding and acceptance before clearing the marker.
4. Listing failure, malformed output, an unknown listing shape or multiple
   exact matches leaves the assignment pending and starts nothing. Multiple
   matches report a key conflict. Failed sends retain the same target for
   retry. A bound id that is absent, renamed or inaccessible raises an anomaly;
   it is not a request for a replacement start or retargeted delivery.

A lost, timed-out or incomplete start result keeps its marker. On restart,
reconcile that marker by keyed discovery before any new start for its key.
One match recovers the same session; unconfirmed input may be resent there.
An empty listing alone never clears a marker. Report the key and
`instrukt sessions resolve-start <key>` once and keep the assignment pending.
`--session` verifies the exact key, project and any recorded worktree request
before binding; `--none` records the operator's confirmation that no thread
was created and no start remains in flight, permitting retry under the same
key. The resolved request retains its input and retry permission until retried.
A rejection proven to precede creation also permits that retry. Failure to
recover a requested worktree path leaves the result incomplete and preserves
the marker.

Runtime acceptance commits with the delivery result and binding. Consumer
handled marks instead track durable activation acceptance under
`project/instrukt/spec/feature/runner/procedure-subscription-routing`; they do not
wait for a runtime send. Input delivery is at least once across a lost acknowledgement
or crash between send and commit. Handlers reconcile their subject idempotently;
one keyed thread does not imply exactly-once execution of its brief.

### T3 command translation

Commands run on the computer hosting T3 and instrukt. `start` has no
remote-environment flag. The project operand is its registered checkout, not an
assumed current worktree: T3 may not recognise a linked checkout as `.`. The
adapter runs the installed `t3` on the host's executable search path; there are
no T3-specific configuration keys in this contract.

| Operation | CLI translation and evidence |
| --- | --- |
| Plain start | `t3 thread start --title <key> <checkout> -`; stdout supplies the native id. No `--wait`: execution and acceptance are separate. |
| New worktree start | Add `--worktree <base-revision> --branch <name>`. The flag creates a new runtime-owned worktree; it is not an existing-worktree path. Read the resulting thread with `show --json` for the path. |
| Send | `t3 thread send <id> -`, no `--wait`. A busy thread receives the text as its next turn. |
| State | `t3 thread show --json --messages 0 <id>`. Decode runtime status and available pending-input, archive, project and worktree metadata. |
| Conversation | `t3 thread show --json --messages 100 <id>`. Return recent messages and their runtime attribution; this is no full archive or search. |
| Recover keyed identity | `t3 thread list --project <checkout> --json`; top-level array entries carry `threadId`, `projectId`, `title`, `status`, `link`, `providerInstanceId`, `model`, `settled`, `snoozed`, and timestamps. `settled` alone does not make a thread gone. |
| Link and notify | Unsupported by the published `t3 thread` CLI. Report the capability gap; use an explicit `runner.notify` command for notifications. |

### T3 per-role worktrees

The shipped sdlc delivery workflow dispatches roles of one todo sequentially.
Each role attempt gets a new T3-created worktree from the todo branch's captured
current head SHA: `--worktree <base-sha> --branch <role-branch>`. The branch name
is `<todo-branch>--role-<role-utf8-hex>-<attempt>`, with the role's UTF-8 bytes
encoded as lowercase hex. A branch collision is a finding, never permission to
reuse an unrelated branch. Record the base SHA, branch and returned path before
admitting work. Setup uses the same convention with role `setup`.

After a code-writing role's completion stamp, advance the todo branch to the
stamped role head by fast-forward only, then release that attempt as evidence.
The next role starts only after that handoff is recorded. A read-only review
runs in its own worktree at the same todo head; its verdict releases the attempt
without a branch advance. Branch handoff, refusal and crash reconciliation are
owned by `project/instrukt/spec/feature/workflow/delivery-cycle`.

An active attempt may receive follow-ups in its bound thread. After release,
a fixer or later round gets a new attempt at the current todo head; earlier
conversations, branches and paths remain evidence. Workflows requiring shared
worktree attachment are refused at enablement naming that missing capability;
T3's shipped delivery workflow requires creation only.

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

Work records retain sessions per todo, role and attempt; subsequent work reads that
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

Given a developer has chosen T3 with worktree creation but no attachment capability
When the developer enables the shipped sdlc delivery workflow
Then its per-role worktrees permit enablement
But when the developer enables a workflow requiring attachment to an existing worktree
Then instrukt refuses naming the attachment capability and leaves configuration unchanged
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

#### UC-SS13: Recover todo work after its start reply was lost

```gherkin
Given a developer's todo role was created in T3 but its start reply was lost
When instrukt resumes dispatch after a restart
Then it recovers that role's keyed thread and worktree without starting another
But when a successful listing cannot find that thread
Then the todo remains pending with its unresolved key and the resolve-start command
And only explicit resolution or proof of rejection permits another start for that key
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

- Runtime start/send acceptance supplies transport evidence only. Event-hook
  provenance and eligible delivery remain the S6 feasibility gate under
  `project/instrukt/spec/feature/runner/procedure-subscription-routing`.
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
- T3 cannot attach a fresh thread to an existing worktree. The shipped sdlc
  workflow uses per-role worktrees; attachment-dependent workflows remain
  unavailable. An upstream attach option would allow roles to share one
  worktree again; it is to be requested from T3, alongside keyed-start request
  pingdotgg/t3code#31621. instrukt does not take over runtime worktree lifecycle.
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
