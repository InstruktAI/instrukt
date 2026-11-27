---
description: Runner session orchestration — keyed native bindings, automatic succession, placement verification, runner quota marking and fallback, execution observation, approval attention, todo worktree lifecycle, and operational sessions listing.
visibility: public
---
# Session Continuity — Spec

## What it is

The runner orchestrates unattended sessions over the runtime-neutral adapter
facade in `project/instrukt/spec/feature/runtime/adapter`: todo orchestrator
pickup, event-driven standing agents, recurring tasks, keyed singleton binding,
event and approval delivery, execution observation, and operational session
listing.

An agent operating in its own conversation uses runtime-native operations via
read-time `@config` instructions; only a capability absent natively uses an
explicit adapter fallback. Native child reports and endings reach the parent
through the runtime without instrukt observing children or composing a second
parent wake. The runtime owns lifecycle, input delivery, restart, revival,
handover, live transcripts, and cleanup; instrukt owns its work facts, durable
bindings, events, schedules, and operational state.

## Canonical fields

### Runner capability requirements

Runner features require the following neutral capabilities from the selected
runtime (`project/instrukt/spec/feature/runtime/adapter`; how global
`runner.runtime` is selected and validated belongs to
`project/instrukt/spec/feature/configuration/layered-configuration`):

| Runner feature | Required runtime capabilities |
| --- | --- |
| Standing agents (`modules.events`) | `start_or_wake`, `send`, `read_state`, and the delivery/continuation capabilities required by the subscription. Steering is optional; missing steering is a supported degradation, not a validation finding. |
| Todo pickup (`modules.todos`) | Keyed `start`, `send`, `read_state`, `caller_identity`, and the workspace capabilities declared by the workflow's typed `requires` (`project/instrukt/spec/feature/workflow/workflow-definition`). |
| Recurring tasks (within `modules.todos`) | Keyed `start`, `read_state`, and start/recovery evidence. |
| Requested review | Keyed `start` in the project's registered checkout, idempotent `dispatch_send`, and `read_state` reporting whether a turn is in progress (`project/instrukt/spec/feature/runner/requested-review`). |
| History extraction (`modules.history`) | Optional cold text source (`project/instrukt/spec/feature/runner/history`); an unsupported runtime reports extraction unavailable without blocking configuration. |
| Thread retention | Optional thread archiving (`finished_threads` and `archive`, `project/instrukt/spec/feature/runner/thread-retention`). |
| Default anomaly notification | Optional `notify`; an explicit `runner.notify` command is independent (`project/instrukt/spec/feature/runner/anomaly-notification`). |

Receiving and publishing external webhook facts require no agent runtime on an
ingress-only computer
(`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`).
Enabling any other runner feature while the selected runtime lacks a required
capability is refused without changing configuration, naming the runtime and
missing capability. A temporarily unreachable runtime is an execution failure,
never a reason to select another adapter or create a replacement session.

### Assignment facts and opening briefs

Standing assignment input retains machine-readable procedure, home, subject,
effective role (declared or `event-handler`), attendance, and interruptibility
facts beside its system-origin loader. Role-aware help and briefing use
`project/instrukt/spec/feature/configuration/roles-and-capabilities`; the
binding's recorded role, home, and placement — never native provider identity —
resolve its scope. Native child reservations retain their dispatcher reference;
later claim changes do not rewrite that relationship. Retained assignments whose
producer recorded no dispatcher migrate to explicit unknown evidence, which
neither refuses legitimate commands nor infers a parent from the current claim.

The native binding is durable authoritative identity; display titles never
rebuild it. Assignment facts retained by the owning domain remain authoritative
through retries and succession. Unknown or contradictory ownership evidence
fails visibly; publication never replaces unknown initialized settings.

The opening brief carries the procedure's closure read, bound parameters,
optional task `occurrence: <RFC 3339>`, and supplied briefing text. The runner
binds procedure parameters before dispatch. User text travels as separate
process arguments or standard input, never as executable shell text.

For subscribed procedures, the opening input establishes the assignment before
its separately identified event data. Delivery intents, waiting-slot combining,
delivery-log composition, and input consumption observation are owned by
`project/instrukt/spec/feature/runner/procedure-event-delivery`. Native agent
messages and child reports belong to the runtime and never occupy this
instrukt-owned waiting slot. Recovery here concerns only instrukt-started
orchestrators and tasks, never their natively delegated workers.

### Start keys, locks, and human-facing titles

Every instrukt-owned start has a stable semantic object key independent of its
readable title. The runner retains one current binding per object and a history
of retired generations in instrukt-owned durable state. Each generation
captures its fixed native command and thread identities, resolved execution
root, model selection, and launch request before creation; retries replay the
captured request without recomputing it.

| Assignment | Semantic key |
| --- | --- |
| Standing procedure | `instrukt:standing:<encoded-procedure>@<encoded-home>[:subject:<encoded-subject>][:generation:<n>]` |
| Runner-owned todo orchestrator | `instrukt:todo:<encoded-project>/<todo-id>:<encoded-role>:<attempt>` |
| Native child assignment | The same role-attempt semantic key supplied to native delegation and retained by work; not an adapter-discovery key. |
| Task occurrence | `instrukt:task:<encoded-project>/<todo-id>:<encoded-occurrence>` |
| Requested reviewer | `instrukt:review:<encoded-project>:<head>` |
| Other unattended starts | `instrukt:run:<encoded-project>:<request-id>` |
| Conversation session (per conversation) | `instrukt:conversation:<encoded-company>:<connector>:<encoded-conversation>` |
| Conversation session (company-scoped) | `instrukt:conversation:<encoded-company>:<connector>` |

For standing keys, the canonical home token is `project:<slug>`,
`company:<name>`, or `platform`; a person token retains its kind and cannot
execute. A subject token is the complete typed reference from the event
envelope. Components are UTF-8 percent-encoded leaving only ASCII letters,
digits, `-`, `_`, `.`, `~` unescaped, with uppercase hex digits. Occurrences
normalize to RFC 3339 UTC with `Z`, omitting a zero fractional part and
trimming trailing fractional zeros; retries and catch-up retain that instant.
Attempts are positive integers increasing per todo and role; request IDs are
generated 32-character lowercase hex identities allocated and persisted before
dispatch. A retry retains its key; an unresolved attempt cannot be bypassed by
allocating another attempt or request ID. Matching is exact, and the execution
scope is independent of optional native project identity.

The `todo-id` is the immutable todo identity, not the mutable slug. A todo
rename keeps its keys and any outstanding startup's captured branch and opening
input; a later same-slug todo has a different identity and cannot collide with
retained earlier keys. A project-slug or procedure-id change requires
reconciliation of outstanding keys and bindings before dispatch under the
changed address. Completed task occurrences retain their key and are never
started again.

Every start or wake claims an exact-key lock and retains OS ownership through
native operations and local acceptance. Short writer transactions persist
identity before creation; none spans runtime I/O. Native delegated children
retain the work-owned reservation and native delegation path without acquiring
a second launcher. Elapsed time never changes the generation.

Every start freezes a readable title without an object-key suffix: `<role> ·
<todo-slug>` for todo roles, procedure name and optional typed subject for
standing procedures, `task · <task-slug>` for tasks, `review · <branch> @
<first 12 characters of the head>` for requested reviewers, `run ·
<procedure-name>` for ad-hoc runs, and company-formatted titles for
conversations (`project/instrukt/spec/feature/company/conversations`). Titles
are presentation only; existing native titles are never rewritten by startup or
migration.

### Keyed dispatch, placement verification, and succession

The binding owns native identity; listings serve people and never choose an
actor. Live start and delivery tracking rely on instrukt-owned durable state,
adapter acceptance receipts and idempotency keys where supported, and live
runtime state observation — never on reading a runtime's internal database.
When a pending start's reply is lost, replay reuses the captured native command
and thread identities and opening request without allocating another generation
or sending the opening brief separately.

Every placement runs in the checkout's own runtime project. Whenever the runner
adopts, wakes, succeeds, or delivers input to an instrukt-owned session —
event, approval, recovery, and conversation input alike — it compares the
session's native project against the live runtime project rooted at the exact
execution checkout through a read-only lookup (`execution_root` in `check`
mode; comparison never registers a project). The binding's recorded project is
not proof of a match, and a checkout with no live project of its own never
matches. A misplaced session receives no input and is not adopted:

- When reported `input-ready` or `failed`, its owner rolls it over to one
  successor generation in the checkout's own project under the object lock,
  transferring pending work (unconsumed events, active work claims, or
  unanswered conversation messages) and leaving the misplaced session untouched
  as history. A todo successor attaches a retained worktree only when it is a
  registered worktree of the checkout on the captured branch; otherwise it
  starts with fresh worktree placement on its captured base and branch. Input
  delivery and approval attention never roll a session over themselves; their
  pending input waits for the owner's succession, taking the object lock before
  any input lock.
- When reported `running`, `needs-input`, or unobservable, it is left
  undisturbed; work stays pending and operator surfaces name the session and
  checkout, stating that it rolls over once idle or archived.
- A manual reservation never adopts a misplaced session and performs no
  rollover.

Confirmed native archive or deletion (`gone`) with eligible outstanding work
allocates one successor generation under the object lock: standing transfers
only unconsumed events under its existing generation accounting; work validates
eligible active claims and commits its binding/claim, runner acceptance,
opening receipt, and execution tracking atomically; company preserves retained
explicit answers and transfers unanswered messages in order. A released or
ineligible assignment receives no successor. Settled and idle sessions in their
checkout's own project remain reusable and wake on delivery. Observation or
send failures retain their target and never establish terminal presence.
Corrupt or contradictory binding or project evidence fails visibly without
arbitrary adoption.

### Runner quota marking and fallback policy

`project/instrukt/spec/feature/configuration/agent-selection` owns role-keyed
selection in configuration, and the selected runtime adapter supplies
background-refreshed provider readiness and usage-window facts
(`project/instrukt/spec/feature/runtime/adapter`).

An instrukt-owned start selects from the last known readiness facts and never
acquires them on the start path. The runner keeps those facts current in the
background on a wake of its own. A refresh that fails or runs past its limit
retains the last known facts and reports the cause in `instrukt events status`;
it never refuses a start. With no facts known yet, a start proceeds in the
configured order; a runtime refusal at launch is handled as any refused start.
Existing keyed sessions retain their captured selection without a new-start
probe.

Runner policy owns quota expiry, marking, and the ordered walk of configured
profiles and tier chains:

- Automatic starts skip unavailable and degraded marks until expiry at the
  command clock; an available mark clears the addressed restriction.
- A usage window at 100% restricts the profile's configured models it applies
  to until its reset. Missing resets default to one hour from the snapshot's
  observation time, recording that default.
- A snapshot's windows are applied once, when the snapshot is read; a start
  applies restrictions already in force and never re-applies a reused
  snapshot's windows, so a reused snapshot never renews a lapsed restriction.
- Overlapping exhausted windows use the latest active reset; a reset already
  past has lapsed.
- Applying a snapshot synchronizes the observed quota marks of every profile it
  covers in one transaction: models at a full window are marked and the
  profile's other observed marks are removed. A failed refresh changes no mark.
- Observed quota marks and manual restrictions remain independent: applying a
  snapshot never alters a manual restriction.
- Availability and skip diagnostics retain cause and expiry. Launch-error text
  supplies neither quota evidence nor no-start proof.
- An unstructured launch failure retains its key and captured inputs for
  recovery. Only proven pre-creation rejection or explicit no-start resolution
  permits chain fallback; capacity evidence never proves that startup created
  no session.

### Operational sessions surface

`instrukt sessions list [--since <duration> | --all] [--json]` inspects the
runtime sessions instrukt binds to its objects. It lists project sessions with
separate semantic key and display title, state, and available native activity
time with provenance, newest known time first. The default settled-thread
window is configured for settled dormant sessions; active non-settled sessions
and sessions with unknown activity time remain visible regardless of age.
`--since` accepts integer `s`, `m`, `h`, `d`, or `w` durations and is mutually
exclusive with `--all`. Filtering uses only native timestamps whose
adapter-declared meaning supports the activity window; local observation time
never substitutes for native activity, and an invalid supplied timestamp is an
observation error.

Keyed starts replay their captured native command and session identities
automatically; there is no operator start-resolution or deleted-session
replacement command. Manual standing reservations retain the separate
`events bind` contract. Agent-native `run`, `send`, `alive`, `conversation`,
and `link` facades are absent from this surface. Session ending remains
runtime-owned, and extracted history belongs to
`project/instrukt/spec/feature/runner/history`.

### One worktree per todo team

Each repository todo uses one runtime-managed `<slug>/work` worktree and branch
(`kind: task` creates no git worktree). Before creating `<slug>/work`, fetch
`origin` and create the worktree from the canonical remote default branch (or
continue an already-published `origin/<slug>/work` branch without resetting it
to main). A fetch failure leaves startup unperformed with its cause visible.
The orchestrator and its delegated workers share that one worktree; a terminal
successor attaches its verified retained worktree and never starts a
replacement in the project root when a retained worktree is required.

Deliberate entry returns placement and the configured native start instruction;
after startup, `todo work <slug> --placement-evidence <file|->` records the
native session, key, title, worktree, branch, head, and source, verifying that
the caller is that native session in the matching registered tree. Git working
directory alone never binds an orchestrator. A recorded orchestrator resuming
in its own placement needs no placement evidence
(`project/instrukt/spec/feature/work/dependencies-and-holds`).

After confirmed delivery, the runner releases the clean `<slug>/work` worktree
and deletes `<slug>/work` locally and on `origin` on each computer that holds
them (`project/instrukt/spec/feature/runner/worktree-reclamation`). Failure or
dirty/in-use state retains the cleanup obligation and its cause while retaining
session records and review evidence.

### Approval attention and execution observation

Applying an approval or request-changes decision commits its work-record fact
before attempting attention delivery. If the todo's current execution claim
names a live orchestrator session, queue the decision, attribution, and bound
evidence to that session with the as-soon-as-possible intent. A failed liveness
read or send retains a pending notice with its cause; retry rechecks the
current claim, and a released or replaced claim never routes a stale notice to
its former holder. If no live claimant exists, the decision is only a state
update; approval attention neither starts an orchestrator nor publishes a
decision event.

Work records retain sessions per todo, role, and attempt. Artifact writers and
stamps retain `<runtime>:<id>` identity, and a step is complete only with its
required artifact and stamp.

A runner-owned execution lasts while its session holds the todo's execution
claim (`project/instrukt/spec/feature/work/dependencies-and-holds`). Release or
transfer of that claim ends the execution at the first observation that sees
it, after which the former session receives no recovery send and raises no
anomaly. While the todo waits on a person
(`project/instrukt/spec/feature/work/github-record`), the session owes no
evidence and an ended turn receives no recovery send and raises no anomaly. For
a todo offered for pickup, the offer itself shows no session holds the claim.
An observation that reads workflow's current assignment
(`project/instrukt/spec/domain/workflow`) takes the claim holder and person
wait from that one assignment and acts on what it read.

For every other runner-owned execution, a present session ending `input-ready`
without workflow advancement receives one recovery send; a repeated ending
raises an anomaly (`project/instrukt/spec/feature/runner/anomaly-notification`).
Confirmed native archive or deletion with an eligible active assignment runs
the successor and work-claim transaction. Observation failure retries
observation instead. Unrecovered executions remain listed in
`instrukt events status` until workflow state advances.

### Owned child-command cleanup

Every child command instrukt launches runs in its own process group and is
stopped on success, failure, timeout, and interruption. An absent process group
is already stopped; on macOS, a group that can no longer be signaled after the
child has been reaped counts as cleaned up while preserving the child's result,
and a refused signal without a recorded reap is an error. On SIGINT or SIGTERM,
instrukt forwards the signal to each process group before enforcing the grace
window (`STOP_GRACE` in `crates/instrukt-core/src/process.rs`). This concerns
only instrukt's own child commands, never a runtime's standing session
lifecycle.

## Use cases

### UC-SS1: Share a clarification with a linked collaborator

```gherkin
Given a developer is working with linked conversations
And the configured runtime adapter supports sharing human input between them
When the developer types a clarification in one conversation
Then the clarification reaches the linked conversation identified as the developer's words
```

### UC-SS2

### UC-SS3: A developer has runner pickup into one managed worktree

```gherkin
Given a developer has selected a compiled runtime supporting todo workspaces
When the runner picks up an eligible prepared todo for work
Then instrukt derives the todo branch and starts the orchestrator in that todo's one managed worktree
And instrukt records durable role-attempt bindings attached to that managed worktree and todo branch
And at most one workspace claim is held for that todo at a time
```

### UC-SS4

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

### UC-SS7

### UC-SS8: Events keep one standing agent through a runtime outage

```gherkin
Given a standing agent already has a keyed runtime conversation
And instrukt retains its durable native binding
When an event arrives while the runtime cannot be observed
Then the event remains pending and no session is started
When the runtime becomes reachable and dispatch is retried
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
Given the selected runtime created a standing agent's keyed conversation but instrukt lost the start reply
When the runner resumes and replays the captured native launch
Then it receives the same native session and binds it
And retries unconfirmed event delivery there without creating another conversation
```

### UC-SS12

### UC-SS13: Recover todo orchestrator pickup after its start reply was lost

```gherkin
Given the selected runtime created a developer's todo orchestrator but instrukt lost its start reply
When the runner resumes pickup after a restart
Then it recovers that orchestrator's keyed session without starting another
And replay preserves the same native command/session IDs and phase worktree
```

### UC-SS14: A builder's ending does not cause instrukt child polling or a second parent wake

```gherkin
Given a developer's orchestrator has a bound native builder child
And the orchestrator is idle
When the builder finishes in the runtime
Then instrukt does not poll the child or compose a second ending delivery to the orchestrator
And the recorded attempt remains uncompleted until explicit workflow evidence is stamped
```

### UC-SS15: A busy orchestrator receives no duplicate instrukt waiting-slot entry for a child's result

```gherkin
Given a developer's orchestrator is working while its bound native reviewer child runs
When that child finishes in the runtime
Then no instrukt runtime-input entry or waiting slot is created for that child result
And the recorded role-attempt binding stays intact for inspection
```

### UC-SS16: Native child failure never mutates workflow completion automatically

```gherkin
Given a developer's orchestrator has a bound native child
When native task evidence reports a failure, input wait or confirmed absence
Then instrukt neither polls that child nor produces a parent notice
And no completion stamp or attempt release is inferred in the work record
```

### UC-SS17: Dispatch a later todo that reuses a slug

```gherkin
Given a developer retains runtime sessions for an earlier todo
And a later todo reuses that todo's slug with a different immutable ID
When the later todo dispatches a role or task occurrence
Then its runtime key uses the later todo's immutable ID
And it neither collides with nor adopts the earlier todo's conversation
And renaming the later todo preserves its existing keys
```

### UC-SS18: Wake a settled orchestrator in its retained conversation

```gherkin
Given a developer's orchestrator is settled but remains reachable in the selected runtime
When instrukt delivers an event or approval to its recorded session
Then the runtime starts its next turn and un-settles that conversation
And its native binding selects it without starting another session
```

### UC-SS19: Find recent project conversations with truthful activity evidence

```gherkin
Given a developer's selected runtime exposes active and dormant conversations
When the developer lists sessions within an activity window
Then each result has separate semantic key and display title, state and available activity evidence with provenance
And old dormant sessions with verified activity times are excluded unless all sessions are requested
And active sessions and sessions with unknown activity time remain visible
And local observation time never substitutes for native activity
And the JSON form carries the same facts
```

### UC-SS20: Wake an older standing agent by key

```gherkin
Given a developer's standing agent has been settled longer than the sessions-list window
When a later activation needs that responsibility
Then its durable native-ID binding selects the same conversation independently of title
And the existing conversation wakes without a new session
And the listing window has no part in that decision
```

### UC-SS21

### UC-SS22: A developer who deleted a standing agent's session has the subject's events handled in a new one

```gherkin
Given a developer's procedure keeps a standing agent for a subject
When the developer deletes its session in the runtime and a later event for that subject arrives
Then one successor starts with a new generation and native start key
And it receives only pending and later events
And an archived session with pending work also receives a successor
```

### UC-SS23: A developer keeps using a renamed standing agent

```gherkin
Given a developer renames the bound session of a standing agent
When a later event needs that agent
Then the event reaches the same session by its retained ID within the retained native project
But if the native project changed, the event stays pending and status reports the conflict
```

### UC-SS24: A developer replaces a deleted task session consistently

```gherkin
Given a task's bound session is deleted or archived while its assignment is eligible
When ordinary runner pickup resumes its outstanding work
Then the runtime binding, work binding and claim move to one successor atomically
And a lost acknowledgement replays that successor with the same IDs
```

### UC-SS25

### UC-SS26

### UC-SS27: A handed-off orchestrator's conversation stays settled

```gherkin
Given a developer's orchestrator released its todo and settled its conversation
And another session has since taken that todo up
When the runner next considers the project's work
Then that conversation receives no recovery message and stays settled
And no stopped execution is reported for it
```

### UC-SS28

### UC-SS29: Start and deliver only in the checkout's own runtime project

```gherkin
Given an execution checkout has no runtime project rooted at itself
And an ancestor folder is registered as a runtime project
When the runner starts a session for that checkout
Then a runtime project is registered at the exact checkout
And the session starts in that checkout's project
And no session is started in the ancestor project
When new work arrives for an idle bound session whose project is rooted elsewhere
Then that session receives none of it
And a successor starts in the checkout's own project and receives the work
When new work arrives for a running bound session whose project is rooted elsewhere
Then the work stays pending and that session receives none of it
And the runner reports the session and the checkout, and that the session rolls over once idle
```

### UC-SS30: A developer's todo pickup moves to the checkout's own project when its created orchestrator idles elsewhere

```gherkin
Given the selected runtime created a developer's todo orchestrator but the runner never accepted its start
And that orchestrator's session is idle in a runtime project rooted elsewhere than the checkout
When the runner resumes the pickup
Then a successor orchestrator starts in the checkout's own runtime project, in that todo's managed worktree
And the todo's orchestrator is the successor
And the earlier session receives nothing more
And no failed start is reported for the pickup
```

## Known caveats

- A legacy native admission whose original startup request is unknown remains
  pending after terminal presence until its owner supplies an authorized
  successor assignment; existing worktree attachment never invents its brief.
- An observation that reads the claim holder and the person wait acts on what
  it read. A release, transfer, or person wait landing after that reading is
  taken up by the next observation; a recovery input already recorded can still
  reach a session whose claim was released or whose todo began waiting on a
  person after it was recorded, governed by
  `project/instrukt/spec/feature/runner/procedure-event-delivery`.
- An earlier recovery input that ended unconfirmed raises an anomaly where the
  execution reaches the recovery decision with workflow state unmoved and
  outside a person wait (`project/instrukt/spec/feature/runner/anomaly-notification`);
  confirmed archive or deletion runs the successor transaction first.
- Searchable history belongs to `project/instrukt/spec/feature/runner/history`,
  and session ending belongs to the runtime.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runtime/adapter](../runtime/adapter.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
- [project/instrukt/spec/feature/runner/event-activation](event-activation.md)
- [project/instrukt/spec/feature/runner/procedure-standing-agent](procedure-standing-agent.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](procedure-event-delivery.md)
- [project/instrukt/spec/feature/runner/work-dispatch](work-dispatch.md)
- [project/instrukt/spec/feature/runner/anomaly-notification](anomaly-notification.md)
- [project/instrukt/spec/feature/runner/history](history.md)
- [project/instrukt/spec/feature/runner/thread-retention](thread-retention.md)
- [project/instrukt/spec/feature/configuration/agent-selection](../configuration/agent-selection.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/configuration/interpolation](../configuration/interpolation.md)
