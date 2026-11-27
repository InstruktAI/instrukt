---
description: Module-aware work dispatch, event activation, session capabilities, task timing and anomaly reporting without a consumer daemon, with opt-in signed ingress.
visibility: public
---
# Runner — Spec

## What it is

The runner bounded context connects eligible work, received events and due
times to agent sessions. It owns dispatch, the runtime adapter, operational
session inspection, event-source translation, conversation links and time
eligibility, including anomaly reporting. It executes the enabled modules' work
in one process, entered through `instrukt run` or `instrukt tick` and woken by
the OS scheduler. The long-lived `instrukt run` also hosts turn-end conversation
relay and, through hosting the company context implements, that context's
connectors (`project/instrukt/spec/domain/company`); scheduled `tick` does
neither. It has no consumer daemon beside the runner;
with a company's connector declared, an owned service keeps `instrukt run`
itself running.
A separate opt-in loopback webhook receiver accepts signed external events,
and the returns of people's sign-ins to MCP servers, on the designated ingress
computer. The work context owns work content, artifacts
and stamps; workflow owns authorization and derived workflow state.

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

**External fact**
: An event the ingress receiver publishes from a verified signed delivery;
  its source is the ingress computer and its subject an inbound source.

**Reacting computer**
: The one computer that activates procedures for external facts: the computer
  `ingress.reactions` names, or the ingress when it names none.

**Activating computer**
: Where an event's procedures activate: the emitting computer for its own
  events, the reacting computer for external facts.

**Session binding**
: Work’s runtime and session id per todo and role, read and written by the
  runner through work’s interface; standing-agent bindings belong to runner.

**Runtime adapter**
: A Rust trait implemented in code, selected explicitly by global
`runner.runtime`. T3 is shipped first over authenticated native keyed-launch RPC, native-ID CLI reads and native thread stream. The
adapter translates start-or-wake, start, send, state and conversation reads,
recognises caller identity without runtime I/O,
and reports optional capabilities. Its recognized native session is the key to
work/standing assignment lookup, not a role supplied by the caller.
Native agent-operation prose is supplied by configuration and rendered through `@config`, independently of this adapter.

**Singleton key**
: The standing procedure, exact home and optional subject identity carried as an
  object key, separate from display title. Its authoritative database binding
  captures each generation and fixed native launch identity. Exact-key ownership
  admits one starter or deliverer at a time.

**Conversation link**
: A runner-owned relationship between two agent threads in which the human
  participates from either thread and finished conversational turns reach the
  peer with their original authorship.

**Relay position**
: Per-link directional source-stream and completed-delivery progress, retained
  across ordinary runner restarts.

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

**Sign-in**
: A speaker's one-time authorization, in their own browser, of instrukt's
  calls to an MCP server under their own login; started by the first call
  that needs it and completed through the receiver's sign-in return.

**Signed-in login**
: The login a sign-in yields, kept per company, person and server in the
  state database and renewed by instrukt until it ends.


**Review request**
: The runner's fact `runner.review_needed`: a commit owed a review on
  instrukt's own initiative, answered by a standing reviewer
  (`project/instrukt/spec/feature/runner/pull-request-review`).

**Requested review**
: An independent review of an exact committed worktree head that an authoring
  session asks for with `instrukt review start`, naming coordinates only; the
  runner renders the reviewer's assignment and starts the requested reviewer
  on it. It is no review request and emits no event
  (`project/instrukt/spec/feature/runner/requested-review`).

**Proposal handoff**
: The runner's fact `runner.proposal_approved`: in a company home, an open
  pull request whose current head is an admitted proposal
  (`project/instrukt/spec/domain/company`) holds an approving verdict, and the
  company's standing integrator is woken to integrate that exact head.


## Ownership boundary


- **Owns:** Per-computer history extraction/search and merged history queries;
  module-aware runner execution; todo dispatch policy; procedure
  activation from events; the admission, rendering and start of requested
  reviews; the runtime-adapter trait, operational session inspection and session reuse behavior;
  runtime capability translation, conversation-link membership, identity-envelope
  vocabulary and deterministic turn-end relay;
  unattended event/approval delivery and runner-owned execution observation; time
  eligibility and missed-run decisions; anomaly-notification routing; the
  container each company's sessions run in on a host, its state, its tools,
  its shared defaults and the export and import of its irreplaceable state; resolving
  a session's assignment from work's, its own and the company context's facts;
  agents'
  access to declared MCP servers (which declarations a caller sees and the
  login a call carries), people's sign-ins to them and the logins kept from
  them; the
  meaning and writes of its tables in the state database (the
  event log, consumer cursors and handled marks, request validators,
  watched-ref observations, executions, authoritative object bindings and generation history, pending delivery targets and per-key locks,
  conversation links and directional relay positions, held task failures and the last wake-up, MCP client registrations, signed-in logins and their generations, and pending sign-ins and their waiting calls), whose schema is published in
  `project/instrukt/spec/feature/work/state-database`; the history store with
  its file, lock and migrations, holding retained history, source positions and
  import progress (`project/instrukt/spec/feature/runner/history`); the
  requested-review store with its file, lock and migrations, holding the
  admitted asks (`project/instrukt/spec/feature/runner/requested-review`).
- **Does not own:** Work content, approval, workflow derivation or step
  completion evidence, owned by work and workflow;
  configuration storage and interpolation, owned by configuration;
  subscription document validation and delivery, owned by docs and publication; pushed-revision graph generation, owned by
  blueprint;
  session lifecycle, input delivery, restart, revival, handover,
  live transcripts and lifecycle cleanup, owned by the runtime, which also
  performs the archives thread retention decides; connectors,
  conversations, the speaker of a conversation turn and the conversation tables,
  owned by company (`project/instrukt/spec/domain/company`); instrukt's
  extracted searchable history is a separate contract. instrukt calls only the adapter's
  released primitives. Events, schedules and dispatch decisions are the
  runner's own; a runtime's task, schedule or event features are not used.
- **Published contracts:** `instrukt run`, `instrukt tick`, the runtime-neutral
  operational `instrukt sessions list` surface,
  `instrukt history extract|search|show`,
  `instrukt ticks status`, `instrukt mcp list|list-tools|call|sign-out`,
  `instrukt review start`, the sign-in return path, and wake-up
  and runner-service registration by `instrukt install`;
  structured adapter operations, semantic-state observation, optional bounded native conversation
  reads and explicit delivery/assignment evidence capabilities, linked-thread
  streams and optional notification; instrukt-owned start semantic-key discovery
  and database locking; the hosting and the conversation assignment lookup that
  the company context implements and the CLI wires. Every operational sessions
  command accepts `--json`.
  Agents use configured native operations in their own conversation and
  record native child attempts through work. The adapter starts only without
  an agent or for an explicit native-capability gap. Native delegation returns
  shared workspace evidence. The unified `todo-orchestrator` and its native shared children
  use one `<slug>/work` worktree and branch, with one workspace role admitted at a time
  and atomic completion/release through the work and workflow contracts. Events include
  `runner.upstream_changed`,
  `runner.ci_completed`, `runner.dependency_failing`, `runner.main_failing` and
  `workflow.finalized`.
- **Allowed dependencies:** blueprint supplies its release-owned tool lifecycle to module installation; configuration supplies effective machine and
  project settings and company homes; publication supplies project checkout paths; docs supplies subscribed procedures; work
  supplies records and session identity, and the state database's file,
  writer lock and migrations; workflow supplies assignments; company supplies
  conversation hosting and conversation assignments; the runtime and
  GitHub supply execution and external observations. These
  relationships are specified in the context map position below.
- **Implementation roots:** `instrukt-runner` realizes this context, shared by
  `todos`, `events` and `history`, and depends on `instrukt-core` and `instrukt-blueprint`. `instrukt-todos`
  depends on `instrukt-runner`; `instrukt-cli` wires these crates together.

Fleet attention is the runner's: which facts of a project's active work are
attention conditions, the `runner.attention.*` vocabulary, each condition's
occurrence identity, a project's attention episode, the fleet-attention
contract work calls inside its record transactions, and the raise through
`instrukt events raise`
(`project/instrukt/spec/feature/runner/fleet-attention`). Each underlying fact
stays with its owner: work passes the facts of its own record changes, and the
runner decides which of them are conditions and appends their events.


The runner also owns worktree reclamation: that and when the disk todo
worktrees hold is reclaimed on a computer, the removal of a project's declared
regenerable output, and the reporting of what stays owed. Its published
contract there is `instrukt repo reclaim`. It does not own a todo worktree's
standing, owned by workflow; the attempt lock and the release of a delivered
todo's worktree and work branch, owned by work; or the declaration of
regenerable output and its validation, owned by configuration
(`project/instrukt/spec/feature/runner/worktree-reclamation`).

The runner also owns thread retention: which finished runtime threads of the
registered projects on a computer are archived, and when, under the computer's
retention window. The selected runtime performs each archive through the
adapter and owns everything else in the thread's lifecycle. It does not own
whether a todo has reached a terminal outcome, owned by work, or the window's
admitted value and its validation, owned by configuration
(`project/instrukt/spec/feature/runner/thread-retention`).

The runner also owns carrying a company's knowledge proposals to review and
integration, and what of them reaches the operator, in a company home
admitted for automatic review. Its review observation's fetch of a company
home prunes remote-tracking branches the remote no longer holds. On each
review observation's tick it considers only the heads of the home's
remote-tracking branches, never a branch that exists only on the computer,
and asks the company context's proposal lookups
(`project/instrukt/spec/domain/company`):

- **Opens pull requests.** For each such head that is an admitted proposal,
  is not contained in the remote-tracking canonical branch, and that no pull
  request, open or closed, has as its head, the runner has work's GitHub
  boundary open a pull request for its branch, as the identity the
  `company-agent` role selects. The pull request is then reviewed as every
  eligible pull request is.
- **Hands off.** When an open pull request's current head is an admitted
  proposal, holds an approving verdict that work answers is current and covers
  the canonical branch, and is not contained in the remote-tracking canonical
  branch, the runner emits `runner.proposal_approved`.
- **Reports.** `instrukt events status` lists, with their doors, the
  unestablished entries the company context names and a proposal store whose
  condition is not usable, derived through the company context's snapshot
  lookups each time it runs. The runner raises each as an anomaly through
  `project/instrukt/spec/feature/runner/anomaly-notification` as that
  contract stands, once per running process.

Opening proposal pull requests, and nothing else, needs the proposal
pull-request credential, whose readiness work answers as it answers automatic
review's: met, unmet with a finding, or GitHub cannot be asked
(`project/instrukt/spec/domain/work`). It is asked as automatic review's is:

- **At entry.** `instrukt install` and `instrukt install --check` ask it for
  each company home admitted for automatic review, beside automatic review's
  readiness, and report an unmet answer or one GitHub cannot give as a finding
  with its repair door
  (`project/instrukt/spec/feature/runner/module-installation`).
- **On each pass.** The review observation of a company home asks it once.
  - **Met:** pull requests are opened as above.
  - **Unmet:** only the opening of proposal pull requests is withheld, with a
    warning naming the finding and its door.
  - **GitHub cannot be asked:** only the opening is withheld for that pass,
    the source's cause is recorded and a warning is logged, as for automatic
    review's readiness; the next tick asks again.

Automatic review of the home's other pull requests, the handoff of heads
already approved and CI observation never wait on this credential. An opening
GitHub or the runtime cannot answer falls under
`project/instrukt/spec/feature/runner/pull-request-review`, Availability: a
warning is logged and the next tick tries again. An opening GitHub refuses
while the credential is met is a defect of the part
(`project/instrukt/policy/checks-and-guards`): a warning carries GitHub's
answer, as the review pass warns of its own failures. Neither writes any
state. The verdict is work's,
and what the integrator does with the head is workflow's integrator's.

`runner.autostart` is a boolean or a map `{<kind>: bool}`, available in both
configuration layers and controlling todos only. `runner.notify` is global-only.
Project-layer `events.watch: [{remote, ref}]` selects refs checked each tick.

## Canonical fields

### Role-bearing assignments

The runner records effective role and exact home/placement in its standing and
scheduled assignments; work owns todo-role attempts. The shared admission
boundary consumes these facts under
`project/instrukt/spec/feature/configuration/roles-and-capabilities`. An omitted
subscription role resolves to `event-handler`; unknown explicit role mappings
use `default`. Role grants and help do not depend on todo enablement. Class does
not replace claim, phase or single-writer object checks, and a standing
responsibility is distinct from its configured role class.

### Aggregates

#### Runner invocation

- **Root entity:** The current invocation.
- **Contains:** Effective module enablement and todo pickup policy, and the
  work selected for dispatch during that execution.
- **Invariants:** Execution follows the enabled modules. Todo pickup mode
  controls todos only; event activation remains independently switchable.
  One process handles those enabled responsibilities. It runs only with the
  prerequisites it admitted at its start or at a later wake; a part that depends
  on a service outside instrukt retries inside itself until that service returns
  (`project/instrukt/policy/checks-and-guards`: a prerequisite is gated, an
  outside failure is retried).
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
  event delivery. Retained owner-native identity restores a missing binding
  under exact-key ownership; native status determines reuse or succession. Domain
  completion requires its owning work record's evidence. Conversation links are
  a separate runner-owned aggregate.
- **Identifier:** Procedure and exact home, plus subject when declared; todo-role
  bindings remain work-owned. The adapter key carries the exact home and optional subject;
  confirmed successors retain separate generation keys.
- **Lifecycle:** A3 owns serialized discovery/start-or-wake, pending fixed targets
  and ambiguous-start recovery. List/send failure leaves work pending and starts
  no replacement. A bound session the runtime reports absent gets a successor
  generation, with or without a subject; an archived one gets none
  (`project/instrukt/spec/feature/runner/session-continuity`: only absence
  proves an actor gone). The standing-agent and procedure-event-delivery contracts own continuing
  home/subject responsibility and eligible event delivery; routing itself resolves no session.

#### Conversation link

- **Root entity:** One link between two runtime-scoped native threads.
- **Contains:** Its two members, active/closed state, opening positions and
  per-direction pending handover and accepted-delivery evidence.
- **Invariants:** Admission, attribution, closure and turn eligibility are
  owned by `project/instrukt/spec/feature/runner/conversation-links`. T3
  transcripts supply content; transport positions confer no speaker authority.
- **Lifecycle:** Opening establishes both directions; the long-lived runner
  follows native completions, resumes retained progress on startup and stops
  both directions when a member closes the link. State remains runner-owned
  in the existing state database.

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

### Events and conversations

Two paths bring the outside world to agents, and they never cross. Events carry
facts: signed webhooks and observed state become events, published procedures
subscribe to them, and standing agents answer in the home the event names.
Conversations carry a person talking to a company's agent and belong to the
company context (`project/instrukt/spec/domain/company`). A connector emits no
event and activates no subscribed
procedure: the conversation's session starts from the company-assistance procedure
(`project/instrukt/spec/feature/company/conversations`), and an event never
reaches a conversation session.

### Event-platform boundary

Event facts retain the complete project, company, person, none and unknown origin
vocabulary. Executable homes are registered projects, company homes
(`project/instrukt/spec/feature/configuration/company-homes`) and the platform;
person resolution is an explicit unsupported capability, never a conversion to
another home. Unknown origin activates nothing. None selects a platform
responsibility. Only the activating computer activates a matching procedure:
a computer's own events activate where they are emitted, external facts on the
reacting computer (`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`). A missing project checkout or company home on
the activating computer fails visibly without forwarding.

Published general/domain procedures reach every home of their declared kind:
one published procedure runs in each company's own home, wherever its publishing
project is checked out.
A project-local subscribed procedure reaches only that project's origin.

`instrukt run` hosts independent event readers. `instrukt tick` gives each a
bounded pending-then-new drain, retaining Redis groups between invocations.
A scheduled-only computer reacts to retained instrukt events on its next wake,
within stream retention. Conversation relay is a separate native-thread reader
in `instrukt run`, governed by
`project/instrukt/spec/feature/runner/conversation-links`. Redis is transport
only; it owns no conversation turns or link decisions.
One consumer's slow handler or unavailable Redis does not block unrelated readers
or local work. SQLite writer-lock ownership is limited to local write sections;
network waits and reactions run outside it.

A1's local event log and local settled cursors remain. Redis owns transport
positions and bounded replay; SQLite occurrence marks protect a logical consumer
against the same event ID for seven days. Existing local event rows retain
retryable publication status and coordinates so a committed fact is published
later with its original ID. This supplies no separate outbox/inbox service and
no accepted ingress buffering during a cluster outage.

### Mechanical canonical checkout maintenance

Each tick observes and fetches canonical main before dispatch for considered
projects. A clean canonical project checkout behind it is fast-forwarded to
the observed commit; post-merge publishes documentation and hands the revision
to enabled blueprint. A dirty, misplaced or diverged checkout is reported and
preserved, and a later tick retries. This is mechanical runner work, independent
of event enablement and without a GitHub API call or a main-ahead event.

### Mechanical worktree reclamation

On each invocation, after its dispatch and event work, the runner reclaims the
disk todo worktrees hold on its computer, for each considered project whose
work tracking is admitted there. It removes the project's declared regenerable
output from the worktrees workflow reports idle, and has work release the
worktree and work branch of each delivered todo. `instrukt repo reclaim` gives
a session working a todo the same removal in its own worktree; it is the
runner's command in the `repo` namespace, beside repository's commit
composition (`project/instrukt/spec/domain/repository`). The contract is
`project/instrukt/spec/feature/runner/worktree-reclamation`: this work starts
no session, emits no event and keeps no state of its own. It reaches only the
worktrees the work record assigns to todos; every other checkout stays with
its creator.

### Mechanical thread retention

While the computer's retention window is set, each runner invocation, after
its dispatch and event work, archives finished runtime threads of every
registered project whose checkout is on the computer, whatever modules that
project switches on. The contract is
`project/instrukt/spec/feature/runner/thread-retention`.

A pass reads, for each such project, the finished threads the selected runtime
lists for it through the adapter's finished-thread listing
(`project/instrukt/spec/feature/runner/session-continuity`). It keeps every thread a standing responsibility holds — the
session of its current binding, of its standing runtime binding, or of its
manual reservation, observed or bound — and every thread work associates with
one of the project's todos without a terminal outcome. When the standing
bindings or work's answers cannot be read, the pass archives nothing. A
project whose listing or whose work answer cannot be read in a pass is left
whole for a later pass: nothing is archived on a partial answer. Of the
remaining eligible threads across projects, the pass attempts the longest
quiet first, up to its bound on archive attempts whatever their outcome, each
archived only while the adapter finds the observation it was chosen from still
holds.

No pickup, event reaction or other command waits on a pass; only the
`instrukt tick` that runs one returns after it. In `instrukt run` it
runs on a thread of its own, started after a wake once the one an earlier wake
started has ended; in `instrukt tick` it runs after the tick has released the
runner and runtime-invocation locks, before the tick returns. Passes that
overlap converge. A pass
keeps no durable state: it reads the window from the admitted configuration
and eligibility from the runtime, work and the runner's standing bindings
every time. Each archive is a structured log event. A failed archive, a
finished thread whose latest activity the runtime does not report, a project
left for later and a pass that archived nothing for want of an answer
are logged with their cause when that cause appears or changes for its thread,
project or pass, as worktree reclamation reports what stays owed
(`project/instrukt/spec/feature/runner/worktree-reclamation`).

Retention runs where the runner already runs: the window adds no wake-up
demand (`project/instrukt/spec/feature/runner/module-installation`).

### Event naming and producer declarations

Existing vocabulary is retained explicitly: `runner.upstream_changed` →
`runner.upstream_changed`,
`runner.ci_completed` → `runner.ci_completed`, `runner.main_failing` →
`runner.main_failing`, `runner.dependency_failing` → `runner.dependency_failing`,
and `workflow.finalized` → `workflow.finalized`.
The producing contexts remain owners of these facts; subscriptions keep
these names. Event-platform types use `instrukt.*`.
No compatibility alias or second occurrence is emitted.

The canonical received boundary is the open event envelope, not a closed enum
of these facts. Runner declarations use project origin;
upstream changes name the watched ref of the project's remote and main-failing
names the project's main, each as a repository ref with the observed revision
in its data, so a newer observation of the same ref takes the place of an
undelivered older one; CI completion names the candidate commit (including the
`no_ci` observation). Finalize-ready uses project origin and a todo subject. The existing occurrence-key columns
below determine stable IDs, including producer type, independently of payload
serialization. Producer declarations expose the existing payloads as typed data.

### Domain events emitted

The first build reads watched upstream refs with `git ls-remote` and uses
conditional requests for integrate candidates, canonical main and configured dependency-update branch heads.
Polling follows the cost contract in
`project/instrukt/spec/feature/runner/event-activation`. Missed
ticks are caught by the next state read. Signed native and GitHub requests also enter as ordinary cluster events; polling remains source reconciliation, with its own authoritative state.

| Event | Emitted when | Payload | Idempotency key | Consumer |
| --- | --- | --- | --- | --- |
| `runner.upstream_changed` | A watched upstream ref changes | Project, remote, ref, observed revision | Project, remote, ref, revision | `sdlc/procedure/upstream-watch` |
| `runner.ci_completed` | The integrate candidate's CI result completes, or the project has no CI | Project, candidate SHA, CI result (`success`, `failure` or `no_ci`) | Project, candidate SHA, CI result | Workflow's subscribed integrator |
| `runner.dependency_failing` | A check run on a current configured dependency-update head fails | Project, branch, head SHA, check run ID/name/conclusion | Project, branch, head, run ID and conclusion | Workflow's subscribed integrator, assessing and deduplicating bug work under its procedure |
| `runner.main_failing` | The CI of remote canonical main's head commit completes red; a red result on a commit that is no longer the head is not observed, as the head's own CI covers the combined state | Project, failing head commit | Project, head commit: a commit's red result is reported once, and a new head is observed afresh | Workflow's subscribed integrator, woken to restore the canonical branch (revert first, repair as separate work); the event authorizes nothing by itself |

`runner.review_needed` is a state: a newer request for the same subject takes
the place of an older one that delivery has not selected for handover. Its subject is
the pull request, or the integration branch for a candidate; the commit stays
in data.

| Event | Emitted when | Payload | Idempotency key | Consumer |
| --- | --- | --- | --- | --- |
| `runner.review_needed` | On a project that opted in to automatic review, a commit is owed a review and has no live request: it is the current head of an eligible pull request with no recognised verdict, or workflow finds that the active integration candidate owes one | Project, the exact commit, the pinned base commit its review starts from, and the request's number for that commit; for a pull request also its number, head branch and base branch | Project, commit, request number | The procedure subscribed to it under its own published declaration; the runner guarantees the subject, the data and the placement (`project/instrukt/spec/feature/runner/pull-request-review`) |

`runner.proposal_approved` is a state: a newer head of the same pull request
takes the place of an older one that delivery has not selected for handover.
Its subject is the pull request, `pull_request:<project>/<number>`; the head
commit stays in data.

| Event | Emitted when | Payload | Idempotency key | Consumer |
| --- | --- | --- | --- | --- |
| `runner.proposal_approved` | In a company home admitted for automatic review, an open pull request's current head is an admitted proposal, holds an approving verdict work answers is current and covers the canonical branch, and is not contained in the remote-tracking canonical branch | Project, the pull request's number, head branch and base branch, the exact head commit and the pinned base its verdict was admitted against | Project, head commit | Workflow's subscribed integrator, which composes that exact head into a candidate; the event authorizes nothing by itself |

Fleet attention's conditions are occurrences with a todo subject in their
project origin; which facts count, and from what point, is
`project/instrukt/spec/feature/runner/fleet-attention`'s.

| Event | Emitted when | Payload | Idempotency key | Consumer |
| --- | --- | --- | --- | --- |
| `runner.attention.execution_stalled` | The runner records that the session holding a todo's execution claim stopped, failed or is gone and its recovery did not move the work, or a start for the todo found no model in the order available | Project, todo, session when one existed, the workflow state observed, and whether the session stopped, failed, was gone or never started | Project, todo, session or the start's model order, workflow state | The procedure subscribed to fleet attention in the project |
| `runner.attention.input_waiting` | The runner observes the session holding a todo's execution claim begin a wait for input while work does not hold the todo waiting on a person | Project, todo, session, workflow state | Project, todo, session and that session's wait number on the todo | The procedure subscribed to fleet attention in the project |
| `runner.attention.hold_placed` | A change made on this computer places a dispatch hold on a todo, from any source | Project, todo, the hold's source, reason and time | Project, todo, the hold's placement time and source | The procedure subscribed to fleet attention in the project |
| `runner.attention.hold_released` | A change made on this computer removes a todo's dispatch hold, by release, replacement, group joining or label | Project, todo, the released hold's source and placement time | Project, todo, the released hold's placement time and source | The procedure subscribed to fleet attention in the project |
| `runner.attention.attempts_failing` | A role's attempt on a todo fails where an earlier attempt of that role on that todo already failed | Project, todo, the todo's identity, role, attempt, failure reason | Project, todo, the todo's identity, role, attempt | The procedure subscribed to fleet attention in the project |
| `runner.attention.fleet_quiet` | While the project's attention episode is open, its last active todo that is unheld and not a structural holder leaves active work or is held | Project, the todo whose change closed the episode | Project, the event that opened the episode | The procedure subscribed to fleet attention in the project |
| `runner.attention.raised` | An agent raises a condition about active todos of the project it works in | Project, the todos named, the obligation at stake; the raising session as the caller reference | One identity per raise | The procedure subscribed to fleet attention in the project |

The hold, repeated-failure and episode-end types are appended through the
fleet-attention contract in the transaction of work's record change. A stopped
execution's type is appended in the transaction that records the unrecovered
stop, a wait's in the transaction that records the wait, and an exhausted
start's in a transaction of its own; `runner.attention.raised` when the raise
is admitted.


### Event-platform facts emitted

For `instrukt.system.event.dead_lettered`, subject is the failing stream entry,
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
semantic states; they do not expand the first-build event vocabulary or replace
record-backed completion.

## Event subjects and origins

The source events above concern a watched ref, a main revision or an
integrate candidate in their project origin. The event declarations define
their typed subjects (`project/instrukt/spec/feature/runner/event-envelope`).
`instrukt.system.event.dead_lettered` concerns the failing stream entry and
has no origin, as declared in Canonical fields.

The `runner.attention.*` events concern a todo in its project origin; a
condition between todos names the others, and a raise its raising session, as
related references.

## Use cases

- **T3 dependency compatibility** — [project/instrukt/spec/feature/runner/t3-contract](../feature/runner/t3-contract.md) — deterministic real-release checks, test-double agreement and nightly warning.

- **History** — [project/instrukt/spec/feature/runner/history](../feature/runner/history.md) — local extraction from T3 and merged search across computers.
- **Work dispatch** — `project/instrukt/spec/feature/runner/work-dispatch`
  — choose automatic or deliberate pickup of eligible work.
- **Session continuity** — `project/instrukt/spec/feature/runner/session-continuity`
  — session reuse, five-state observation, working on T3 Code once it is
  chosen, refusing a feature the chosen runtime cannot carry, and native
  delegation with record-backed attempts. Archive search and agent-owned
  session ending are retired goals.
- **Conversation links** — `project/instrukt/spec/feature/runner/conversation-links`
  — hold a three-way conversation across two agent threads and close it for
  everyone; includes the runner-owned identity-envelope vocabulary.
- **Event activation** — `project/instrukt/spec/feature/runner/event-activation`
  — learn about upstream changes before deciding to reconcile a fork.
- **Task timing** — `project/instrukt/spec/feature/runner/task-timing`
  — arrange future work and recover missed recurring occurrences.
- **Anomaly notification** — `project/instrukt/spec/feature/runner/anomaly-notification`
  — notice unattended failures while keeping questions in their threads.
- **MCP access** — [project/instrukt/spec/feature/runner/mcp-access](../feature/runner/mcp-access.md)
  — use a declared server's tools under the right login and load them only when needed; sign in once, in your own browser, and be answered in the same turn.
- **Company containers** — [project/instrukt/spec/feature/runner/company-containers](../feature/runner/company-containers.md)
  — run each company apart from the others and from the host, keep its logins and conversations through a replaced container, update its tools without a new image and export and import it.

- **Pull-request review** — [project/instrukt/spec/feature/runner/pull-request-review](../feature/runner/pull-request-review.md)
  — have trusted authors' pull-request heads reviewed independently without anyone asking.


- **Requested review** — [project/instrukt/spec/feature/runner/requested-review](../feature/runner/requested-review.md)
  — have an exact committed worktree head reviewed independently on an assignment instrukt rendered from its coordinates.


- **Event facts and producer declarations** — `project/instrukt/spec/feature/runner/event-envelope`.
- **Discover declared event shapes** — `project/instrukt/spec/feature/runner/event-discovery`.
- **Scoped transport and publication recovery** — `project/instrukt/spec/feature/runner/event-transport`.
- **Independent retained reactions** — `project/instrukt/spec/feature/runner/event-consumers`.
- **Signed project and company sources** — `project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework`.
- **Single cluster intake** — `project/instrukt/spec/feature/runner/single-ingress-webhook-termination`.
- **GitHub provider translation** — `project/instrukt/spec/feature/runner/github-webhook-normalization`.
- **Enablement-driven infrastructure** — `project/instrukt/spec/feature/runner/module-installation`.
- **Continuing procedure responsibility** — `project/instrukt/spec/feature/runner/procedure-standing-agent`.
- **Eligible durable event handoff** — `project/instrukt/spec/feature/runner/procedure-event-delivery`.
- **Published subscriptions and durable activation acceptance** — `project/instrukt/spec/feature/runner/procedure-subscription-routing`.

- **Fleet attention** — `project/instrukt/spec/feature/runner/fleet-attention`
  — have stuck, converging or conflicting work across a project's todos attended to without watching its sessions.

- **Worktree reclamation** — [project/instrukt/spec/feature/runner/worktree-reclamation](../feature/runner/worktree-reclamation.md)
  — get the disk idle and delivered todo worktrees hold back without anyone running a deletion command, and reclaim one's own worktree during work.

- **Thread retention** — [project/instrukt/spec/feature/runner/thread-retention](../feature/runner/thread-retention.md)
  — have long-quiet finished threads leave the thread list without anyone archiving them.

## Governing designs and decisions

- `project/instrukt/design/architecture/overview` — module boundaries
  and the CLI's execution shape.

## Context map position

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| configuration → runner | Customer / Supplier | Runtime mapping, module switches, runner settings, and company homes with their settings and secrets |
| workflow → runner | Customer / Supplier; Published language | Computed state, transitions and finalize-ready event used for dispatch |
| work → runner | Customer / Supplier | Work reads and command-contract writes: claims and reflection flushes, last-fired stamps, automatic holds, expired-record purge and per-role session bindings |
| runner → work | Customer / Supplier | Event-log append used when the finalize-ready stamp records `workflow.finalized` |
| docs → runner | Customer / Supplier | Procedures and their event subscriptions |
| publication → runner | Customer / Supplier | Project checkout paths recorded in publication state |
| work → runner | Customer / Supplier | The state database's file, writer lock and migrations, holding the runner's tables |
| company → runner | Customer / Supplier | Conversation hosting in `instrukt run` and the conversation assignment lookup, declared by runner and implemented by company (`project/instrukt/spec/domain/company`); a connector's direct message to one person, which MCP sign-in sends its links through |
| runner → company | Customer / Supplier | Runtime adapter, native conversation source and turn relay, keyed start, identity envelopes and the state database handle |
| runner → workflow | Published language | `runner.ci_completed` wakes the integrator to assess candidates; `runner.main_failing` wakes it to restore a failing canonical branch; `runner.dependency_failing` wakes it to assess failed dependency updates |

Carrying a company's knowledge proposals adds these crossings:

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| company → runner | Customer / Supplier | The proposal lookups runner declares and company implements: whether a head is an admitted proposal, the unestablished entries on remote-tracking branches and the store's condition |
| work → runner | Customer / Supplier | Through its GitHub boundary, the proposal pull-request credential's readiness (met, unmet with a finding, or GitHub cannot be asked), whether any pull request has a commit as its head, and opening a pull request for a branch under the proposal pull-request credential |
| runner → workflow | Published language | `runner.proposal_approved` wakes the company's integrator to integrate an approved admitted proposal's exact head |

Fleet attention adds these crossings
(`project/instrukt/spec/feature/runner/fleet-attention`):

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| runner → work | Customer / Supplier | The fleet-attention contract, called inside work's transaction for each change it makes on this computer to a todo's status, hold or attempts, with that change's facts |
| work → runner | Customer / Supplier | A project's active todos with their status and holds, read when a raise is admitted and when a runner-observed condition opens an episode |
| workflow → runner | Customer / Supplier | Whether a claimed todo waits on a person, carried by the existing assignment and read when a claimed session's input wait is observed |

For automatic review the runner asks its suppliers and derives none of their
answers: work says whether a pull request is a todo's, whether a commit carries
a verdict and whether both identities are ready, and workflow says whether the
active candidate's content is covered. Through work's command contract the
runner records or withdraws a derived approval and has outstanding verdicts published,
handing over the eligible pull requests it observed at each head. It supplies
work with a commit's review requests, each with its number, standing, the
session it reached and pinned base, with the input that opened the turn a
session has in progress when it offers a revision, and with a named session's
conversation
(`project/instrukt/spec/domain/context-map`).


For a requested review the runner asks work whether the head's verdict stands
here, with the session that recorded its current version, or was handed off,
with the export that handed it off, and for the pinned base work admits its
verdict against; it reads the commit's review requests from its own delivery
records.
It starts the requested reviewer through the runtime adapter under its own
start key; work admits that reviewer's verdict like any other
(`project/instrukt/spec/feature/runner/requested-review`).


The runtime adapter and the event-source boundary form an
**Anti-corruption layer (ACL)** translating session operations and external
observations. These are
external boundaries, not additional bounded contexts. The runtime side is a Rust
trait and code implementations; T3 is the first, over authenticated native keyed-launch RPC, native-ID CLI reads and
read-scoped `subscribeThread` stream. The explicit
configuration name selects the implementation
(`project/instrukt/spec/feature/runner/session-continuity`).

Event activation carries the exact home and optional subject through this seam.
The adapter realizes ASAP or only-at-entry intent with steering, queued input
or idle entry under procedure-event-delivery. Runtime lifecycle and child-ending parent attention remain
runtime-owned. Instrukt delivers its own events/approvals under session-continuity;
stamps remain work-record evidence. Native instruction prose is resolved from
configuration through `@config`; it is never an adapter command template.

Todo dispatch reads work and workflow through runner-declared interfaces,
implemented by the todos crate and wired by the CLI. Work’s command contract
covers every runner mutation: recording execution claims, flushing pending
reflection and reading people's controls on ticks, writing last-fired stamps,
setting automatic holds, purging expired retained records, and reading and
writing per-role session bindings. Work owns validation and persistence of
those writes, and what a tick's reflection publishes.

For worktree reclamation the runner asks its suppliers and derives none of
their answers: configuration supplies the project's admitted declaration of
regenerable output, workflow each todo worktree's standing and what a
delivered todo still owes, and work the section of its record in which no
role attempt opens, together with the release of a delivered todo's worktree
and work branch (`project/instrukt/spec/domain/context-map`).

For thread retention the runner asks its suppliers and derives none of their
answers: configuration supplies the computer's admitted retention window, and
work the native sessions bound to, or holding the execution claim of, a
project's todos that have not reached a terminal outcome. The runtime adapter lists finished threads and archives them
(`project/instrukt/spec/domain/context-map`).

## Known caveats

- The target delivery contract requires durable acceptance, bounded batch
  selection, procedure-first briefing and untrusted activation provenance. Its
  realization uses the selected runtime's supported input boundary. Missing
  steering degrades ASAP attention to queued input or idle entry without a
  validation finding. Runner automation never settles a standing T3 thread; a
  standing procedure may self-settle when parked dormant
  (`general/procedure/event-handling/overview`), and a settled one stays
  reachable (`project/instrukt/spec/feature/runner/procedure-standing-agent`).
- A people registry, person homes, voice and memory, tmux/TUI/chiptunes/person chips,
  one-time layout migrations, experiment matching, dsh upgrades,
  harness-install management and session/work-record migration are outside
  instrukt. Moving a todo between computers is not supported: the runner picks
  up only the todos of its own computer's work record, and GitHub carries no
  copy of a todo for another computer to take up.
- There is no installation inspector or incident-alert subsystem. Module
  infrastructure checks retain their existing contract; anomaly notification
  and the log bug hunter cover unattended failures.
- Event reactions activate agents under their declared roles. Script-run
  event actions and event-bound maintenance are outside scope. Path-based
  post-commit follow-ups use git hooks, including existing sync hooks.
- A project's fleet coordinator is the standing agent of its subscribed
  procedure in that project, one per computer that works the project.
  Runtime lifecycle remains runtime-owned.


- UC-HR1 is retired: machine maintenance is outside instrukt; its id is not reused.
- UC-HR2 is retired: machine maintenance is outside instrukt; its id is not reused.
- UC-HR3 is retired: machine maintenance is outside instrukt; its id is not reused.
- UC-HR4 is retired: machine maintenance is outside instrukt; its id is not reused.
- Other caveats are recorded in the feature specs.
