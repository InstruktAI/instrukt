---
description: Runtime-neutral adapter facade — universal live spine for unattended start, send, coarse state, execution roots, and caller identity, plus declared optional progressive capabilities without vendor coupling or live runtime-database reads.
visibility: public
delivered_by:
  - thin-live-runtime-boundary
---
# Runtime Adapter — Spec

## What it is

The `runtime` bounded context defines the runtime-neutral `RuntimeAdapter`
facade, neutral session and capability types, and caller-identity recognition.
One compiled runtime adapter is selected explicitly by global `runner.runtime`
(or, when unset, the single adapter compiled into instrukt); no code detects a
runtime from installed tools or session environment variables.

The adapter acts where instrukt has no agent: todo orchestrator pickup,
event-driven standing agents, recurring tasks, requested reviews, company
conversations, publication of configured agent runtimes, and periodic cold or
retention sweeps. An agent operating in its own conversation uses
runtime-native operations through read-time `@config` instructions; only a
capability absent natively uses an explicit adapter fallback. Native child
reports and endings reach their parent through the runtime without instrukt
polling children or composing a second wake.

The runtime owns session lifecycle, input delivery, restart, revival, handover,
live transcripts, and cleanup. instrukt owns its work record, events,
schedules, active-speaker tracking, and delivery bookkeeping. Live operational
paths never read a runtime's internal database.

## Canonical fields

### Universal live spine

Every runtime adapter implements this minimal live boundary using structured
inputs and structured results. A session reference is `<runtime>:<native-id>`;
instrukt invents no replacement session identity.

| Primitive | Input and result | Released consumer |
| --- | --- | --- |
| `start` / `start_or_wake` | Execution checkout, durable semantic key, readable title, opening brief, optional todo-worktree placement, and assignment facts; returns the existing or newly started session and acceptance, separately from domain completion. | Todo orchestrator pickup, standing procedure activation, recurring tasks, requested reviews, and company conversations. |
| `send` | Native session reference, structured input, and delivery intent (`as soon as possible` or `only at entry`); acknowledges runtime acceptance separately from turn execution. | Event, approval, recovery, requested-review, and company conversation delivery. |
| `read_state` | Scoped native session reference; returns coarse operational state (`running`, `needs-input`, `input-ready`, `failed`, `gone`), presence (`present`, `archived`, `absent`), and available native metadata (project, branch/worktree, settled flag, native activity timestamp with provenance). | Keyed reuse, delivery-boundary checks, succession, and runner-owned execution observation. |
| `caller_identity` | Selected adapter plus caller environment; returns the native session reference and any verified agent identity, or no caller. Performs no runtime I/O. | Read-time `runtime.session_id`, recorded role/assignment lookup, capability scope, and commit attribution. |
| `execution_root` | An execution root path and a mode (`check` or `ensure`); returns whether the selected runtime holds a live project rooted exactly at that root, creating one in `ensure` mode, together with the runtime's own door for adding one. Never selects, alters, or removes an ancestor or descendant project. | Session launch and module installation (`project/instrukt/spec/feature/runner/module-installation`). |
| `list` | Registered project checkout; returns operator-facing session summaries with native identity, title, state, and available activity timestamps with provenance. Never selects an actor for dispatch. | `instrukt sessions list`. |

A runtime's idle or exited process supplies no work outcome; `running`,
`needs-input`, `input-ready`, `failed`, and `gone` remain distinct. An
unrecognised response shape, command failure, or unreachable runtime is an
observation error, never `gone` or success. Live operational paths — runner
dispatch, standing-agent delivery, company conversation relay, active-speaker
resolution, MCP access, configuration admin-DM patching, knowledge-proposal
recording, and review admission — never read a runtime's internal database.

### Declared optional progressive capabilities

Every capability beyond the universal live spine is explicitly declared by the
adapter. An unsupported capability is distinct from a transient runtime failure:
an unsupported operation reports the runtime and missing capability, while an
unavailable runtime leaves work pending for retry.

| Capability | Contract | Consumer |
| --- | --- | --- |
| Idempotent start and send keys | Fixed native command and thread identities captured before creation or dispatch (`dispatch_send` / keyed native launch), so retries after a lost reply or restart reconcile without creating a second session or a second turn. | Keyed startup, company conversations, and requested review. |
| Quota-aware model selection and readiness | Translates neutral profile, model, effort, and extra options, validates them against runtime-reported descriptors where available, and supplies background-refreshed provider readiness and usage-window facts while runner policy owns selection order, quota expiry, and fallback (`project/instrukt/spec/feature/configuration/agent-selection`). | Automatic dispatch and `instrukt agents`. |
| Configured agent runtimes | Reports enabled agent harness instances and their resolved homes so publication writes global agent files; an unreachable runtime leaves existing global files unchanged (`project/instrukt/spec/feature/publication/agent-entrypoints`). | `instrukt sync` and `instrukt status`. |
| Workspace isolation (`create_worktree`, `attach_worktree`) | Native worktree creation on a freshly fetched canonical remote base, attachment to an existing verified worktree for succession, and typed native delegation capability metadata. | Todo workflows declaring workspace requirements (`project/instrukt/spec/feature/workflow/workflow-definition`). |
| Cold text source | Optional read-only text source over retained runtime conversations and messages, isolated from live operational paths and never invoked during live dispatch or conversation relay. | Cold history extraction (`project/instrukt/spec/feature/runner/history`). |
| Thread retention / purge | Optional finished-thread inspection (`finished_threads`) and archiving or purging of long-quiet finished threads (`archive` / purge), checking that the observed quiet, unpinned state still holds before mutating. | Thread retention (`project/instrukt/spec/feature/runner/thread-retention`). |
| Multi-account provider isolation | Optional ability to host multiple isolated model-provider accounts on one computer so Tier-1 shared-host companies run on their own subscriptions (`project/instrukt/spec/feature/company/company-containers`). | Company home `profiles` (`project/instrukt/spec/feature/configuration/company-homes`). |
| Remote peer-linking | Optional ability for an operator's host runtime to link into an isolated Tier-2 company container's runtime over a private network boundary, and optionally allow a technical company owner to link their own client account (`project/instrukt/spec/feature/company/company-containers`). | Tier-2 isolated company hosting. |
| Notification (`notify`) | Optional runtime-native anomaly notification accepting a session or work-item reference and deep link. | Default anomaly notification when no explicit `runner.notify` command is set (`project/instrukt/spec/feature/runner/anomaly-notification`). |
| Conversation link (`link`) | Optional sharing of human clarifications across linked conversations, identified as the human's words. | Linked-conversation collaboration. |

### Runtime choice and feature availability

`runner.runtime` is global-only and names a compiled adapter. Unset, it selects
the single adapter compiled into instrukt. Unknown adapter names are validation
findings. Runtime instruction entries for in-session agents are configuration
prose, not adapter verb templates.

| Feature | Needs |
| --- | --- |
| Standing agents (`modules.events`) | `start_or_wake`, `send`, `read_state`, and the delivery/continuation capabilities required by the subscription. Steering is optional; lacking steering is a supported degradation, not a validation finding. |
| Todo pickup (`modules.todos`) | Keyed `start`, `send`, `read_state`, `caller_identity`, and the workspace capabilities declared by the workflow's typed `requires`. |
| Recurring tasks (within `modules.todos`) | Keyed `start` and `read_state`. |
| Conversations (a company home's declared connector) | Keyed `start` in a company home, `send`, `read_state`, and `caller_identity` (plus idempotent send keys when supported), per `project/instrukt/spec/feature/company/conversations`. Requires no runtime conversation-source database read. |
| Requested review (no module switch) | Keyed `start` in the project's registered checkout, idempotent `dispatch_send`, and `read_state` reporting whether a turn is in progress (`project/instrukt/spec/feature/runner/requested-review`). |
| History extraction (`modules.history`) | Optional cold text source; an unsupported runtime reports extraction unavailable without blocking configuration. |
| Thread retention (`runner.archive_finished_after` set) | Optional thread retention / purge capability (`project/instrukt/spec/feature/runner/thread-retention`). |
| Default anomaly notification | Optional `notify`; an explicit `runner.notify` command is independent. |

Receiving and publishing external webhook facts require no agent runtime on an
ingress-only computer (`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`).
Enabling any other module while `runner.runtime` names no compiled adapter, or
while the selected adapter lacks a required capability, is refused without
changing configuration, naming the runtime and missing capability.

### Start keys, locks, and human-facing thread titles

Every instrukt-owned start has a stable semantic object key independent of its
readable title. The runner retains one current binding per object and a history
of retired generations in `instrukt.db` (`project/instrukt/spec/feature/work/state-database`).
Each generation captures its native start/thread identity, resolved execution
root, model selection, and launch request before creation; retries replay that
captured request without recomputing it.

| Assignment | Semantic key |
| --- | --- |
| Standing procedure | `instrukt:standing:<encoded-procedure>@<encoded-home>[:subject:<encoded-subject>][:generation:<n>]` |
| Runner-owned todo orchestrator | `instrukt:todo:<encoded-project>/<todo-id>:<encoded-role>:<attempt>` |
| Native child assignment | The same role-attempt semantic key supplied to native delegation and retained by work; not an adapter-discovery key. |
| Task occurrence | `instrukt:task:<encoded-project>/<todo-id>:<encoded-occurrence>` |
| Requested reviewer | `instrukt:review:<encoded-project>:<head>` |
| Other unattended starts | `instrukt:run:<encoded-project>:<request-id>` |
| Conversation session | `instrukt:conversation:<encoded-company>:<connector>:<encoded-conversation>` |

For standing keys, canonical home tokens are `project:<slug>`, `company:<name>`,
or `platform`; a person token retains its kind and cannot execute. Components
are UTF-8 percent-encoded leaving only ASCII letters, digits, `-`, `_`, `.`,
`~` unescaped, with uppercase hex digits. Occurrences normalize to RFC 3339 UTC
with `Z` and no trailing fractional zeros. `todo-id` is the immutable todo
identity, so renaming a todo preserves its keys and a later todo reusing a slug
cannot collide.

Every start or wake holds an exact-key lock (`runtime_key_locks`) across native
operations and short database transactions; no database transaction spans
runtime I/O. Every start freezes a readable title without an object-key suffix:
`<role> · <todo-slug>` for todo roles, procedure name and optional subject for
standing agents, `task · <task-slug>` for tasks, `review · <branch> @ <12-char head>`
for requested reviews, `run · <procedure-name>` for ad-hoc runs, and
company-formatted conversation titles for conversations. Existing native titles
are never rewritten.

### Keyed dispatch, placement verification, and succession

The durable binding owns native identity; listings serve people and never
choose an actor. Delivery and start tracking rely on instrukt's own durable
state records, native adapter acceptance receipts and idempotency keys where
supported, and coarse `working`/`idle` state — never on scraping a runtime's
internal database.

Every placement runs in the checkout's own runtime project. Whenever instrukt
adopts, wakes, succeeds, or delivers input to an instrukt-owned session, it
compares the session's native project against the live runtime project rooted
at the exact execution checkout (read-only; never registering a project during
comparison). A misplaced session receives no input:

- If reported `input-ready` or `failed`, its owner rolls it over to a successor
  generation in the checkout's own project under the object lock, transferring
  pending work and leaving the misplaced session untouched.
- If reported `running`, `needs-input`, or unobservable, it is left undisturbed;
  work stays pending and operator surfaces name the session and checkout.

Confirmed native archive or deletion (`gone`) with eligible outstanding work
likewise allocates one successor generation under the object lock, transferring
unconsumed events, unanswered conversation messages, or active work claims
atomically. A released or terminal assignment receives no successor. Settled
threads remain reusable and wake on their next delivery; only confirmed archive
or deletion proves a session gone.

### Operational sessions surface

`instrukt sessions list [--since <duration> | --all] [--json]` inspects the
runtime threads instrukt binds to its objects. It reports separate semantic key
and display title, state, and native activity timestamp with provenance, newest
known time first. The default settled-thread window is three days; active
non-settled threads and threads with unknown activity time remain visible.
`--since` accepts integer `s`, `m`, `h`, `d`, or `w` durations and is mutually
exclusive with `--all`. Local observation time never substitutes for native
activity.

Agent-native `run`, `send`, `alive`, `conversation`, and `link` facades are
absent from this surface.

### Delivery logs, todo worktrees, approval attention, and cleanup

- **Waiting delivery log:** One session-wide waiting slot combines pending
  `event`, `approval`, and `recovery` entries in arrival order (composition
  time at the top, each entry carrying source identity, kind, source occurrence
  time or explicit unknown, and arrival time; event envelopes marked untrusted
  data). Native agent messages and child reports bypass this slot
  (`project/instrukt/spec/feature/runner/procedure-event-delivery`).
- **One worktree per todo team:** Each repository todo uses one runtime-managed
  `<slug>/work` worktree created from freshly fetched canonical remote main (or
  `origin/<slug>/work` on another computer), shared serially by the orchestrator
  and its native children. Successors attach the verified retained worktree when
  present. After confirmed delivery, the runner releases the clean worktree and
  deletes `<slug>/work` (`project/instrukt/spec/feature/runner/worktree-reclamation`).
- **Approval attention:** Committed work-record approval or request-changes
  decisions queue an as-soon-as-possible notice to the current live execution
  claimant; released or replaced claims receive no stale notice.
- **Execution observation:** A runner-owned execution lasts while its session
  holds the todo's execution claim. While the work record holds the todo
  waiting on a person, silence earns no recovery send or anomaly. Otherwise, an
  idle ending without workflow advancement receives one recovery send, and a
  repeated ending raises an anomaly (`project/instrukt/spec/feature/runner/anomaly-notification`).
- **Owned child-command cleanup:** Every child command instrukt launches runs
  in its own process group and is stopped on success, failure, timeout, and
  interruption; SIGINT/SIGTERM propagates to each group before grace expiry.

## Use cases

### UC-RA1: Work with the compiled runtime without environment detection

```gherkin
Given a developer has not set `runner.runtime`
When the developer enables work dispatch or standing agents
Then the one runtime compiled into instrukt is selected and named
And nothing about installed tools or the caller's session changes that choice
But when `runner.runtime` names no compiled adapter
Then instrukt refuses naming the unknown runtime
```

### UC-RA2: Switch on a feature only when the selected runtime declares its capabilities

```gherkin
Given a developer's selected runtime declares worktree creation, shared native delegation, and existing-worktree attachment
When they enable a workflow requiring those workspace capabilities
Then configuration accepts those supported requirements
And an existing phase worktree remains available for native terminal succession
But when the selected runtime lacks a capability required by a feature
Then instrukt refuses naming the runtime and the missing capability
```

### UC-RA3: Dispatch a todo orchestrator into one managed worktree with native shared delegation

```gherkin
Given a developer has selected a compiled runtime supporting todo workspaces
When the runner picks up an eligible prepared todo for work
Then instrukt derives the todo branch and the adapter creates its orchestrator in that todo's one managed worktree
And the orchestrator delegates workers natively with shared isolation and records their role-attempt bindings
And all roles use that worktree and todo branch with one outstanding workspace role
And agent reads and follow-ups use configured native instructions
```

### UC-RA4: Keep a standing agent's identity through runtime outages and failed sends

```gherkin
Given a standing agent has a durable native session binding
When an event arrives while the runtime cannot be observed or a send fails
Then the event remains pending and no replacement session is started
When the runtime becomes reachable and dispatch retries
Then instrukt delivers the event to that bound session without starting a second session
```

### UC-RA5: Serialize concurrent deliveries to a standing agent under its key lock

```gherkin
Given a standing agent has an existing keyed session
When two runner invocations contend to deliver events to that agent
Then only the key lock's owner dispatches at a time
And the events reach the existing session at eligible delivery boundaries
And no second session is started for that key
```

### UC-RA6: Recover a created session after a lost start reply via idempotent replay

```gherkin
Given the runtime created a standing agent or todo orchestrator but instrukt lost the start reply
When the runner resumes and replays the captured launch request
Then it recovers the same native session and binds it without starting a second session
And for a todo orchestrator replay preserves the same native identities and managed worktree
```

### UC-RA7: Receive native child results and failures without instrukt child polling

```gherkin
Given a developer's orchestrator delegated a bound child through the runtime
When that child reports completion, failure, input wait, or absence while the parent is idle or working
Then the runtime delivers native parent attention through its own boundary
And instrukt does not observe the child, compose a second ending delivery, or infer workflow completion
And the parent checks the recorded attempt, completion stamp, and release before deciding what follows
```

### UC-RA8: Isolate a later todo that reuses a slug using immutable todo identity

```gherkin
Given a developer retains runtime threads for an earlier todo
And a later todo reuses that todo's slug with a different immutable identity
When the later todo dispatches a role or task occurrence
Then its runtime key uses the later todo's immutable identity
And it neither collides with nor adopts the earlier todo's session
And renaming the later todo preserves its existing keys
```

### UC-RA9: Wake a settled, renamed, or older bound session by durable identity

```gherkin
Given a developer's orchestrator or standing agent has a durable native binding
And its session is settled, renamed, or older than the sessions-list window
When instrukt delivers an event or approval to that binding
Then the runtime wakes that same session by its durable identity without starting another session
And the listing window and display title have no part in that decision
But if the session's native project changed, delivery stays pending and operator status reports the conflict
```

### UC-RA10: List recent project sessions with truthful native activity evidence

```gherkin
Given a developer's selected runtime exposes active and dormant sessions
When the developer lists sessions within an activity window
Then each result shows separate semantic key and display title, state, and available native activity with provenance
And old dormant sessions with verified activity times are excluded unless `--all` is given
And active sessions and sessions with unknown activity time remain visible
And local observation time never substitutes for native activity
And the JSON form carries the same facts
```

### UC-RA11: Replace an archived or deleted session with one successor generation

```gherkin
Given a standing agent or eligible task assignment has a bound runtime session
When that session is confirmed archived or deleted while work is outstanding
Then one successor starts under a new generation and native start key
And pending work transfers without duplicating completed work
And for a task assignment the runtime binding, work binding, and claim move atomically
```

### UC-RA12: Leave a handed-off orchestrator settled without recovery noise

```gherkin
Given a developer's orchestrator released its todo and settled its conversation
And another session has since taken that todo up
When the runner next considers the project's work
Then that former conversation receives no recovery message and stays settled
And no stopped execution is reported for it
```

### UC-RA13: Start and deliver only in the checkout's own runtime project

```gherkin
Given an execution checkout has no runtime project rooted at itself
And an ancestor folder is registered as a runtime project
When the runner starts a session for that checkout
Then a runtime project is registered at the exact checkout and the session starts there
And no session is started in the ancestor project
When new work arrives for an idle bound session whose runtime project is rooted elsewhere
Then a successor starts in the checkout's own runtime project and receives the work while the earlier session receives nothing more
When new work arrives for a running bound session whose runtime project is rooted elsewhere
Then the work stays pending, the session is undisturbed, and operator status reports that it will roll over once idle
```

### UC-RA14: Share a clarification with a linked collaborator when supported

```gherkin
Given a developer is working with linked conversations
And the selected runtime adapter supports sharing human input between them
When the developer types a clarification in one conversation
Then the clarification reaches the linked conversation identified as the developer's words
```

### UC-RA15: Operate live dispatch, company conversations, and caller inspection without reading a runtime's internal database

```gherkin
Given instrukt is configured with a compiled runtime adapter whose internal database is absent or unreadable on live paths
When the runner dispatches work, delivers standing or company conversation turns, resolves the active speaker for MCP or configuration commands, and checks session state
Then every live operation completes through the runtime adapter's live spine and instrukt's own state database
And no live operational path reads the runtime's internal database
```

## Known caveats

- A legacy native admission whose original startup request is unknown remains
  pending after terminal presence until its owner supplies an authorized
  successor assignment; existing worktree attachment never invents a brief.
- Pre-change unbound starts cannot be retroactively deduplicated; migration
  refuses an unresolved old start without a recoverable native identity while
  preserving all data.
- An observation that reads a todo's claim holder and person-wait flag acts on
  what it read; a release, transfer, or person wait landing afterward is taken
  up on the next observation.
- Searchable history belongs to `project/instrukt/spec/feature/runner/history`,
  and session ending belongs to the runtime.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/company/conversations](../company/conversations.md)
- [project/instrukt/spec/feature/company/company-containers](../company/company-containers.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](../runner/procedure-event-delivery.md)
- [project/instrukt/spec/feature/runner/procedure-standing-agent](../runner/procedure-standing-agent.md)
- [project/instrukt/spec/feature/runner/work-dispatch](../runner/work-dispatch.md)
- [project/instrukt/spec/feature/runner/anomaly-notification](../runner/anomaly-notification.md)
- [project/instrukt/spec/feature/runner/history](../runner/history.md)
- [project/instrukt/spec/feature/runner/thread-retention](../runner/thread-retention.md)
- [project/instrukt/spec/feature/configuration/agent-selection](../configuration/agent-selection.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
