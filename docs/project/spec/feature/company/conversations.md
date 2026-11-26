---
description: Conversations — a person addresses a company's agent in a connected channel; each conversation is bound to an agent session in the company's home, started from the procedure declaring the connector's role (default `company-assistance`), every addressed message is its own turn naming its sender, and the session's answer is posted back, never through event subscriptions.
---
# Conversations — Spec

## What it is

A conversation is a person talking to a company's agent in a connected
channel. A connector carries each message addressed to the agent into the one
agent session bound to that conversation, and that session's answer back to
where it was asked. Conversations are a path of their own: no event is
emitted for a message and no subscribed procedure is activated by one. Events
stay the path for facts
(`project/instrukt/spec/feature/runner/procedure-subscription-routing`).

## Canonical fields

### Enablement and hosting

A company home declares its named connectors under `connectors.<name>`
(`project/instrukt/spec/feature/configuration/layered-configuration`). Each
entry selects a registered provider kind via optional `kind` (defaulting to the
map key `<name>` when omitted), so one company home may declare multiple named
connectors of the same kind (for example, multiple Slack apps). Each entry also
accepts four generic conversation settings:

| Field | Allowed values | Default | Contract |
| --- | --- | --- | --- |
| `kind` | Registered connector kind name | The map key `<name>` | Selects the channel provider implementation. |
| `role` | Configured or published role name | `company-agent` | Role recorded on the connector's session binding(s), governing procedure selection, model order/tier, and capabilities. |
| `admit` | `members` \| `admins_only` | `members` | Who is admitted to address the connector. |
| `session_scope` | `conversation` \| `company` | `conversation` | Whether each conversation gets its own session or all admitted conversations of `(company, connector)` share one standing session. |
| `reply_mode` | `turn_output` \| `explicit` | `turn_output` | Whether finished turn output is posted automatically or suppressed in favor of explicit `instrukt company reply` calls. |

Declaring a connector is what gives a company conversations. The long-lived
`instrukt run` hosts every declared connector of every company home on the
computer; a scheduled tick hosts none. While a connector is declared,
`instrukt install` keeps that run running through an owned service
(`project/instrukt/spec/feature/runner/module-installation`). Conversations need a
selected runtime with the capabilities
`project/instrukt/spec/feature/runner/session-continuity` lists for them.

### What a connector supplies

A connector is the code for one channel provider. For each message addressed
to the agent it supplies the company home that declares it, a stable
conversation identity, the sender's identity on that connector, whether its
provider knows the sender as one of the company's own people, whether its
provider knows the sender as a workspace administrator (`is_admin`), whether
the conversation is a direct message (`is_direct`), the sender's
name and, where the provider gives one, email address, the time the message
was written, its text, the conversation's earlier messages its provider
contract says accompany it, and whether that contract left earlier messages
out. It posts the agent's answer to a conversation it carries, rendered as its
provider contract says, and shows there the state of the person's message
(working on it, or temporarily unavailable with the message saved) where its
provider has a way to. It also sends a person it carries messages from a
direct message that instrukt has for them alone, such as a sign-in link
(`project/instrukt/spec/feature/runner/mcp-access`); that message goes
nowhere else, starts no turn and accompanies no later message. Which messages
count as
addressed, who counts as the company's own, and what accompanies a message is
each connector's own contract
(`project/instrukt/spec/feature/company/slack-connector`). Everything else on this
path is the same for every connector, whichever channel it speaks to
(`project/instrukt/spec/domain/company`).

### The session bound to a conversation

- A conversation is identified by company, connector and provider conversation
  identity. Its durable binding follows the native launch/replay and automatic
  succession rules in `project/instrukt/spec/feature/runner/session-continuity`.
  Rename and settlement preserve its native ID. Runtime failures retain work
  and report the actual cause to the operator; the member sees only the normal
  temporarily unavailable status/notice.
- Saved or new unanswered work after confirmed native archive/deletion starts
  one successor with the assignment alone. A pending successor replays its
  persisted native launch identity on later relay passes until accepted. A failed
  successor launch opens the normal unavailable interval without resetting an
  existing timestamp; resumed delivery clears it normally. Harvest
  readable completed old answers
  before adoption. Reset only unanswered work for ordered delivery, including
  questions whose old turn has no retained answer. Retained answers keep their
  posting state and identities; posting continues during a runtime outage.
- Optional connector display facts supply a channel name or direct-message
  counterpart. Company captures the readable title during message capture and
  freezes it into the generation's request. Slack titles use company plus channel
  and a shortened first-text preview, company plus DM counterpart, or company
  plus `Slack thread` when enrichment is absent. Enrichment never controls identity
  or delivery and requires no new mandatory provider scope.


- The session works in the company's home
  (`project/instrukt/spec/feature/configuration/company-homes`), where it
  starts with the company's own knowledge
  (`project/instrukt/spec/feature/publication/agent-entrypoints`). It answers
  no event subscription.
- It starts from the delivered published procedure declaring the connector's
  configured `role` (default `company-agent`). When a connector configures a
  custom role present in `agents.yaml` that has no dedicated published procedure
  of its own, it falls back to the delivered published procedure declaring
  `company-agent` while retaining the connector's recorded role on the session
  binding. The procedure comes from the docs repository that publishes it
  (`project/instrukt/spec/feature/publication/docs-repositories`); instrukt's
  own publishes `company-agent`'s as `general/procedure/company-assistance`. The
  procedure is instrukt's and the knowledge is the company's; neither repeats
  the other.
- The session's opening input is the runner-authored assignment envelope,
  followed by the first addressed message's input. The assignment is the
  loader of `project/instrukt/spec/feature/runner/procedure-event-delivery`:
  it names the procedure, the connector's role with its resolved grants, the
  company, the connector and the conversation (or company session scope, and
  explicit reply instructions when `reply_mode: explicit`), and directs
  `instrukt docs get <procedure-id> --closure` before the message is handled.
  The procedure is read from current publication when the session starts; a
  woken session keeps the assignment it started with.
- Under `session_scope: conversation` (the default), each `(company, connector,
  conversation)` has its own session keyed by
  `instrukt:conversation:<company>:<connector>:<conversation>`. Under
  `session_scope: company`, one shared standing session keyed by
  `instrukt:conversation:<company>:<connector>` serves all admitted direct
  messages and channel threads of that `(company, connector)` serially. Every
  incoming turn delivered to a company-scoped session is preceded by a sanitized
  control notification header naming the connector, opaque conversation ID,
  direct-message flag, connector-supplied thread identifier (when threaded) and
  display label (when available), and authoritative speaker metadata (provider
  ID, email, administrator flag, and sanitized display name on separate fields),
  alongside the conversation-context and member envelopes so the shared agent
  maintains cross-user awareness and knows which conversation ID to address when
  replying.
- **Binding-pinned policy and scope transitions:** a connector's declared
  `role`, `session_scope`, and `reply_mode` are recorded on
  `connected_conversations` when a binding is created and remain fixed for that
  binding's lifetime (so an already-running session never disagrees with its
  opening assignment). Each binding owns a disjoint set of conversations for
  `(company, connector)`: a `conversation`-scoped binding owns only its own
  conversation ID, and a `company`-scoped binding owns only conversations that
  have no `conversation`-scoped binding. If an operator changes `session_scope`
  on a connector that already has live bindings, previously bound conversations
  stay exclusively with their existing bindings and new conversations bind under
  the newly declared scope, with no overlap or double delivery.
- Under the default `company-agent` procedure
  (`general/procedure/company-assistance`), the conversation's agent works
  through the company's declared MCP servers
  (`project/instrukt/spec/feature/runner/mcp-access`) and on the company's
  own documents in its home, which it may change and commit
  (`project/instrukt/spec/feature/publication/agent-entrypoints`), and
  when an authenticated workspace administrator addresses it in a direct
  message (`is_admin = true` and `is_direct = true`), may inspect, discover
  and manage that company's `mcp` service declarations under the company-agent
  scope of `project/instrukt/spec/feature/configuration/configuration-editing`;
  it declines anything else, as its procedure instructs. A connector declaring a
  role with its own published procedure follows that procedure's duties (for
  example, `technical-coordinator` opens `general/procedure/technical-coordinator`,
  which includes filing and coordinating project todos) while keeping the
  connector's admission, company, speaker-envelope and administrator-DM
  boundaries intact.
- Its role is the connector's `role` (default `company-agent`), stored on the
  session's `connected_conversations` binding so caller assignment lookup
  grants that role's class, capabilities, and model selection from `agents.yaml`
  (`project/instrukt/spec/feature/configuration/agent-selection`,
  `project/instrukt/spec/feature/configuration/roles-and-capabilities`).
- Resolution is serialized per conversation binding (per conversation under
  `session_scope: conversation`, or per `(company, connector)` under
  `session_scope: company`). A delivery pass that cannot take
  one binding's lock within its wait skips that binding, whose messages
  stay pending for the next pass, and goes on with the others; this is apart from
  the runtime-dispatch lock, whose timeout skips a whole wake
  (`project/instrukt/spec/feature/runner/work-dispatch`). The company context keeps the
  binding, each accepted message and each answer, so a message is not lost
  before it is delivered and an answer is not lost before it is posted. What is stored and
  kept is `project/instrukt/spec/feature/work/state-database`.

### One addressed message, one turn

Each conversation keeps a durable receipt for every provider message the
agent has received, identified in Slack by its timestamp. A receipt covers
a message delivered as its own turn or as context for another message.
For each delivery, including a retry, select only earlier messages in the
provider's thread or channel range that lack a receipt. Exclude messages
waiting for their own turns. Apply the context limit to that selection and
include the omission note when the limit trims it. If the connector cannot read the required context, the question stays saved and its unavailable interval opens until delivery resumes. Record the selected
messages and addressed message in the same transaction that records the
delivery. A send or start whose acceptance is uncertain retains the exact
input for reconciliation and keeps later work queued until it is resolved.
The relay sends through T3's authenticated WebSocket command endpoint. It
records a command ID derived from the message and its attempt boundary before
dispatch. An uncertain retry keeps that ID and framed input. T3's durable
command receipt prevents a second turn across connections and restarts.
A confirmed refusal allows the next attempt to select context again and use a
fresh boundary. Presence and its recovery doors still apply.
Failures before any byte of the request frame is written leave the message
waiting with its command ID and an operator-visible cause. The relay retries
these outages without a delivery-unknown deadline. Once any request byte has
been written, delivery is uncertain and the same ID is used to reconcile or
retry it.
If the runtime has other work or cannot confirm delivery through
`SEND_RECONCILE_BOUND` (five minutes on the operator clock), retain the input,
record delivery unknown for the operator, and release its place in the queue.
An older CLI attempt without a command ID cannot be retried automatically.
A later match to either attempt confirms delivery and lets the answer proceed. Receipts follow message retention as stated in
the state-database spec.


- Each addressed message is delivered as its own input and starts its own
  turn. Messages of one conversation are delivered in the order they were
  written, as far as they have arrived; a later one waits until the session's
  current turn has finished.
- The input is platform-framed with the registered envelopes of
  `project/instrukt/spec/feature/runner/conversation-links`: a conversation
  context envelope carrying the accompanying earlier messages as data, each
  with its speaker and time and with the agent's own earlier posts marked as
  its own, and saying so when the connector left earlier messages out,
  followed by a member envelope carrying the addressed message's text as the
  connector supplies it, with connector, conversation, member, `admin`,
  `direct`, and time in its opening.
  Both carry the delivery's boundary token, so a line inside them that looks
  like an opening or a closing stays content.
- The speaker of a turn is the person who wrote its addressed message. Their
  connector identity, name, email address, administrator status (`is_admin`)
  and conversation direct-message flag (`is_direct`), as the provider gave
  them when that message was admitted, are stored with the message and do not
  change afterwards; `project/instrukt/spec/feature/runner/mcp-access` and
  `project/instrukt/spec/feature/configuration/configuration-editing` read them.

### Who is answered

Under `admit: members` (the default), the agent answers the company's own
people, as the connector's provider knows them; nobody is listed for it. Under
`admit: admins_only`, the agent answers only authenticated workspace
administrators (`is_admin = true` on a full member of the company's workspace).
Whether a sender is admitted is decided for each addressed message, from what
the provider says about the sender after that message was written. An earlier
answer about the same person admits nothing. A message is delivered only once
that is decided; while the provider cannot be asked, the message waits, nothing
is delivered and nothing is replied, and the cause shows in
`instrukt events status`. A message that was admitted is delivered, and a turn
that has started runs to its end.

Under `admit: members`, a sender who is not one of the company's own gets one
fixed `'not_own'` reply in that conversation, worded by the connector; under
`admit: admins_only`, a sender who is not an authenticated workspace
administrator (whether a non-admin member or not one of the company's own) gets
one fixed `'not_admin'` reply in that conversation, worded by the connector. In
both cases no session is started or woken and nothing is delivered. Each fixed
reply is sent once per sender, conversation and refusal reason. Such a person
still appears in a context envelope as a speaker, by the name the connector
gives.

### The answer back

- Under `reply_mode: turn_output` (the default), when a turn started by an
  addressed message finishes, the agent's conversational output of that turn is
  posted to the conversation exactly as the agent wrote it, in parts where the
  connector's message size limit requires, in order. No secret value is
  searched for or replaced in it. Tool calls, tool results, thinking and
  control input are not posted. A bare `NOOP` posts nothing.
- Under `reply_mode: explicit`, the runner suppresses automatic posting of raw
  turn output (and suppresses relaying raw turn text to the connector's status
  line, showing only the generic working or unavailable state) so internal
  monologue and status lines never leak to the channel. Replies are posted only
  when the agent explicitly queues them via
  `instrukt company reply [--conversation <id>] --text <text>` (with `--text -`
  reading the reply body from stdin).
- `instrukt company reply` is admitted through caller assignment lookup: the
  caller must be an active company conversation session bound in
  `connected_conversations` (and pass its role's `company-reply` capability
  check). Under `reply_mode: explicit`, it is the sole reply path; under
  `reply_mode: turn_output`, an active conversation session may also invoke it
  to queue an explicit reply through the same durable outbox in addition to
  final turn output. Under `session_scope: conversation`, `--conversation`
  defaults to the session's own bound conversation and refuses any other target;
  under `session_scope: company`, `--conversation <id>` is required and must
  name an admitted conversation (one with an admitted, non-refused message and
  no conflicting conversation-scoped binding) of that `(company, connector)`.
  The command takes the reply text exactly as the agent wrote it, searching for
  and replacing no secret value in it, splits it to the connector kind's
  declared message limit, and writes ordered pieces into
  `conversation_answers`, so the runner's existing answer-posting relay
  delivers them with full retry and recovery. Its own output is the queued
  reply's summary (company, connector, conversation, turn and piece count),
  never the reply text.
- The turn relay `instrukt run` already hosts does this: the same durable
  conversation source, completed-turn evidence and retained positions as
  conversation links. The relay advances its position in the transaction
  that retains the answers it reads. Posting settles each retained piece
  separately through the protocol below. Retained answers can post while
  the conversation source is unreadable.
- Answers are retained as ordered pieces until a post call confirms acceptance.
  The provider's returned message id, when present, is recorded with its posting
  time or the time the successful response was observed. Success without an id
  also confirms acceptance; no locally reserved id stands in for a provider id.
  Confirmed pieces are skipped. Failed or uncertain posts remain pending for the
  normal retry pass, without a provider receipt search or uncertainty expiry.
  Later pieces wait for the preceding piece; another conversation proceeds
  independently. While this process knows a post succeeded but could not record
  it, only that record is retried. A restart without durable success can repeat
  an already delivered piece. This accepted duplicate risk preserves delivery
  of every retained answer, including unconfirmed attempts from older versions.
- Completion comes only from the conversation source's whole-turn evidence.
  A turn running however long is left to run. A recent-message listing supplies
  neither completion nor missing answer content. If a completed turn's content
  is missing or unstable, the relay keeps the message outstanding, opens its
  no-work interval and reads again. The cause shows in status and raises an
  anomaly past `ANSWER_OPERATOR_BOUND`. Nothing partial and nothing of another
  message is posted.
- An unsuccessful terminal turn leaves its message failed. Text written
  during that turn is progress and supplies no answer.
  The relay retries it after a growing delay, behind new messages, until it
  receives an answer or a later-admitted settled message from the same member
  supersedes it. Each send has a fresh boundary recorded before dispatch.
  The opening request keeps its recorded boundary through reconciliation.
  The relay applies an acknowledgement only to the attempt it identifies
  while that attempt is outstanding. It discards stale acknowledgements and
  records them for the operator.
  It uses the bound session and the presence and operator recovery rules above.
  Repeated failed turns are visible to the operator. What the failed turn
  already did is the agent's to reconcile
  (`general/procedure/company-assistance`). The company relay declares
  `FAILED_RETRY_DELAY`, `FAILED_RETRY_LIMIT`, and `ANSWER_OPERATOR_BOUND`
  in `crates/instrukt-company/src/conversations.rs`.
- A bound conversation keeps its recorded native source identity. A different
  current identity, or a missing identity on an existing binding, is an
  operator-visible configuration error naming the conversation and recorded and
  current identities. Native reads, observation results, retries and dispatch
  stay blocked without changing its position or input disposition. Restore the
  recorded source to resume. Initial binding and existing confirmed-replacement
  recovery can establish an identity; a copied or moved source is never adopted
  automatically. Retained answer pieces can still post while native work is
  blocked. No implementation diagnostics are sent to the member.
- A turn someone starts directly in the session's thread, a person or another
  agent, posts no turn output. In such a turn the agent posts only an explicit
  reply queued through `instrukt company reply`, under the admission above.
- From the admission of an addressed message until it is complete — its
  answer posted, or its turn completed successfully with a bare `NOOP` or blank
  output — the connector shows the message's state where the provider supports
  it. The state shows that the assistant is working on the request from its
  admission, and that the assistant is temporarily unavailable and the message
  is saved while its no-work interval is open, which happens only once a failure
  shows the agent cannot work on it ("The minute", below). The indication and
  the notice read the same interval, the relay-wide failure included, so the
  indication never says working while the notice's minute runs. A slow but healthy
  start shows the working text. While the message's turn runs, the indication
  carries the newest message the agent has written in the running turn,
  relayed as the runtime records it, and keeps it until the turn's answer is
  posted; before the agent has written one it is the working text. The indication follows the message, not its turn: through a
  failed turn awaiting redelivery and through an answer complete but not yet
  posted, it stays, with the text its interval gives; it is a courtesy, and each connector states
  how closely it follows the state.
- `instrukt events status` shows each connector's connection state, the
  conversations with pending messages or answers, and failures with their
  cause, without conversation text or credentials. A connector that no running
  runner hosts is shown as stopped. What automation cannot resolve — a message no
  turn has taken past the operator bound, a turn that keeps failing, a start that
  stays unresolved, a key conflict, an archived thread, a message whose turn cannot
  be found — is listed there and raised once as an anomaly
  (`project/instrukt/spec/feature/runner/anomaly-notification`), never told to
  the person.

### What a person experiences

A person in a conversation hears about their request and their own world, never
about instrukt's internals (`project/instrukt/policy/checks-and-guards`: a person
is told only what belongs to their world):

| Moment | What the person sees |
| --- | --- |
| Their message is admitted | The connector's indication of its state (above). |
| The agent works on it, however long | The newest message the agent has written in the running turn, as the runtime records it; the indication that the assistant is working on the request until the agent has written one. |
| A failure shows the agent cannot work on it | The indication that the assistant is temporarily unavailable and the message is saved, from the next status pass after its no-work interval opens. |
| The agent has been unable to work on it for one minute | One message, worded by the connector, beginning with the person's mention: "The assistant is temporarily unavailable. Your message is saved and you'll get the answer here when it's back — no need to send it again." One best-effort attempt per person and conversation outage. |
| The work is done | The agent's answer; the person never repeats their message. |
| A service of theirs is unreachable or refuses, inside a turn | The agent says so in its answer (`general/procedure/company-assistance`). |
| They are not one of the company's own | The fixed reply above. |

Nothing else is posted: no part names, no causes inside instrukt or the runtime,
no error codes.

**The minute** is measured per waiting person. Each admitted message without an
answer has its own interval, which opens only on a confirmed failure that keeps
the agent from the work it waits on; admission and a wait that is merely slow
open nothing. It starts, if not already open:

- when a start, send or listing for it is refused or fails;
- when a wait for the dispatch lock or the conversation's lock times out while it
  waits;
- when it has been handed to a turn: at the first failed observation of that turn
  (session state unobservable, source unreadable) — loss of reachability for work
  already handed over;
- when its own turn is observed ended failed;
- when its own turn ended with an answer that cannot be posted: a post of it
  failed, or it is found unposted after a post of it was tried, a restart
  included. The interval then starts at that turn's end. A turn that ended and
  whose answer is being posted opens nothing: the unavailable indication shows
  only while a failure keeps the agent's answer from the person.

It ends, for that message only: when the runtime accepts its hand-over (a
redelivery included); when its own turn is observed running again; when a turn it
waits behind is observed running; when its answer is posted; or when its turn
completes successfully with a bare `NOOP` or blank output. An answer to another
person, a read showing an idle, archived or absent session, an empty read and a
refused attempt end nothing. A person's interval is the earliest among their
unanswered messages. While the relay cannot acquire the conversation source,
every unanswered message counts from the later of its admission and the start
of that failure, unless its own interval started earlier. Each connector
evaluates notice eligibility from the state
database and the clock alone at least once per intake interval while an admitted
message of its company has no posted answer. A provider read for one member
leaves the eligibility evaluation and other members' reads free to proceed.
A retained message without a provider sender still has status and cannot
produce an addressed notice.

**The one notice** is one best-effort attempt per person and conversation
outage, using that person's effective no-work interval above. Admission and a
healthy wait create no outage. Confirmed work or answer recovery clears the
attempt marker; a later failure can earn a new notice.

The decision transaction rechecks eligibility and records the attempt timestamp
before provider I/O. A failed post, a lost response or a crash after that record
never schedules another notice attempt in the same outage. A failure remains
visible to the operator. Notice delivery and ordering against an answer are best
effort: a post already in flight can arrive after recovery. No provider read-back
or shared notice/answer post lock establishes notice delivery.

The intake interval is declared with the conversation core. The read-back answering bound, reporting period and scan overlap are
declared in `crates/instrukt-company/src/conversations/read_back.rs`.

### After the agent was offline

A connector reads back what was written while it was not connected
(`project/instrukt/spec/feature/company/slack-connector`). A message written
within the answering bound that is addressed and not yet retained is admitted and
answered like a live one. An older one is not answered and is named to the
operator in `instrukt events status` for the reporting period, once per message
(`project/instrukt/policy/checks-and-guards`: what instrukt cannot resolve goes to
the operator).

Read-back runs alongside the connection and intake. A repeated read skips
messages by provider identity. An admitted offline message uses the same
delivery receipt rule for context as a live message: earlier messages absent
from the durable receipt record supply context. A send to a retained session
uses the same durable command identity and replay path as a live message.
The saved window decides whether a recovered message is answered. The
saved question waits until its required context can be read.
An interrupted scan resumes its saved window and provider page before
covering the interval since that window began. Scan failures and older
missed messages are visible to the operator without posting a diagnostic in
the conversation.

## Use cases

### UC-CV1: Ask a company's agent where the team talks and be answered there

```gherkin
Given a company member can address the company's agent in a connected channel
When they address it with a question
Then the agent's answer appears in that same conversation
And the answer appears nowhere else
```

### UC-CV2

### UC-CV3: Know who is asking in a shared conversation

```gherkin
Given two members address the agent in one conversation, one after the other
When their messages reach the agent
Then each message is its own turn naming the member who wrote it
And the second turn starts only after the first has finished
```

### UC-CV4: Turn away a person who is not one of the company's own

```gherkin
Given a person who is not one of the company's own people addresses the agent in a connected channel
When their message arrives
Then they receive one fixed reply saying the agent only answers the company's own people
And no agent session starts and nothing is delivered to one
```

### UC-CV5: Keep conversations apart from event procedures

```gherkin
Given a developer has published a procedure subscribed to every event
When a member addresses the company's agent in a connected channel
Then the conversation's own session answers
And no subscribed procedure is activated by that message
```

### UC-CV6

### UC-CV7: Hear once that the assistant is unavailable, then get the answer

```gherkin
Given a member addresses the agent and a failure shows the agent cannot work on it
When a minute passes from that failure without the agent working on the message
Then the assistant makes one best-effort attempt to notify the member, beginning with their mention, that it is temporarily unavailable, that their message is saved and that they need not send it again
And writing again during that outage brings no second attempt
And another member waiting in the same conversation can get their own notice
And recovery allows a later outage to earn a new notice
And the member's answer follows once the agent has done the work
And a healthy wait does not earn an unavailable notice
And a request completed successfully without a reply is complete without a post or an unavailable notice
```

### UC-CV19: Get an answer while another conversation is still opening

```gherkin
Given the assistant is waiting for its new thread to be verified in one conversation
When a member asks a question in another conversation
Then the assistant starts there and answers while the first verification is still waiting
And the first conversation gets its own answer once verification resumes
And if replacement later times out waiting for the stopped thread's input, the operator sees the conversation key and the command to retry
And a recovery command for a session with shell metacharacters can be retried in a shell with the original output format
```

### UC-CV8: Get the answer after the assistant's work failed

```gherkin
Scenario: The member receives the answer after a retry
  Given the assistant's work on a member's question ends in failure
  When the assistant can work on the question again
  Then the member receives the retained answer without sending the question again, allowing a repeat after an uncertain post
  And progress from the unsuccessful work is never posted as the answer
  And another member can be answered while the failed question waits
  And each outage that earns an unavailable notice gets at most one best-effort attempt
  And a send with a lost response and no runtime record is retried with the same command identity once the runtime is idle
  And if the original dispatch reaches T3 after a runner restart and its retry, only one turn takes the question
  And an outage before dispatch keeps a saved question retryable even past the reconciliation bound
  And later questions reach the assistant after the saved question without a duplicate delivery

```

### UC-CV11: Onboard a company while the runner runs

```gherkin
Given the runner is running and no company is set up
When an operator sets up a company with its connector and publishes its procedure
Then its members' questions are answered without restarting the runner
And a question written before the company was admitted is answered too
When the operator updates the connector's tokens
Then subsequent questions are answered through the renewed connection
When the operator removes the company home
Then its connector disconnects
```

### UC-CV12: Be answered while a project's event responses are not admitted

```gherkin
Given a computer serves a company and a project that switches on event responses without the event transport they need
When a member addresses the company's agent
Then the agent's answer appears in that conversation
```

Which project layers are admitted for events, and how status names one that is
not, is `project/instrukt/spec/feature/runner/work-dispatch`.

### UC-CV13: Be answered for a message written while the assistant was offline

```gherkin
Given the assistant was offline
When a member addressed it during that time, in a direct message or in a reply to an existing thread
Then each addressed message gets its retained answer when the assistant is back, with possible repeats after uncertain posting
And a message that did not address it gets nothing
But a message older than the answering bound is not answered and is named to the operator
```

### UC-CV14: Avoid repeating confirmed answers

```gherkin
Given a member's answer was accepted by the connector
When recording acceptance fails or the runner restarts
Then a recorded acceptance is not posted again
And a success still known by the running process retries only its record
And an acceptance lost across a restart may be posted again until delivery is recorded
```

### UC-CV15: Follow the status of a question while waiting

```gherkin
Given a member asks the assistant in a conversation that shows work status
When the question waits, work begins, service becomes unavailable, or work resumes
Then the member sees whether the assistant is working or temporarily unavailable
And the member sees the assistant's newest update while the turn is working
And the status clears once the answer arrives or the turn completes without a reply
```

### UC-CV16: Get a saved answer after its delivery is interrupted

```gherkin
Given the assistant has a retained answer for a member and its delivery is uncertain
When the assistant recovers from the interruption
Then it retries unconfirmed pieces until posting succeeds, even while the native source is unreadable
And a possibly delivered piece may appear again
And the member sees the unavailable status while delivery is blocked
And the member can continue asking questions after the answer is delivered
```

### UC-CV17: Keep serving conversations after an upgrade

```gherkin
Given an operator upgrades a runner that stopped while an answer was being delivered
When the operator resumes serving the conversation
Then confirmed pieces remain confirmed
And retained unconfirmed pieces are retried until posting succeeds, including attempts without a provider id
And members can continue asking questions and receiving answers
```

### UC-CV18: Keep asking questions after unsuccessful work

```gherkin
Given a member is still waiting after the assistant failed to answer an earlier question
When they ask another question and the assistant completes it
Then the later question is served even while the earlier one remains unserved
And its completion clears the earlier failure from the member's status
And a subsequent outage can earn its own notice
```

### UC-CV20: A member keeps their question after native terminal presence

```gherkin
Given a member has an unanswered saved question in a company conversation
When its bound native thread is archived or deleted and the runner resumes
Then one successor receives the assignment and outstanding questions in order
And the member receives their answer normally
And retained answers preserve their posting identities and state
```

### UC-CV21: Keep an answer through an interruption at its source

```gherkin
Scenario Outline: Receive the complete answer when it can be read
  Given a member has a question waiting for an answer
  When <interruption>
  Then the member receives the complete answer once it can be read, allowing a repeat after an uncertain post
  And no partial answer is posted
  And confirmed earlier answers are not repeated and the member does not need to ask again

  Examples:
    | interruption                                                  |
    | the source temporarily holds only part of the completed answer |
```

### UC-CV22: Restrict a named connector to workspace administrators

```gherkin
Given a company home declares a named connector with admit set to admins_only
When a non-admin member addresses that connector
Then they receive one fixed administrator-only refusal reply in that conversation
And no agent session starts or wakes
When a workspace administrator addresses that connector
Then their message is admitted and answered normally
```

### UC-CV23: Share one standing session across a named connector's conversations

```gherkin
Given a company home declares a named connector with session_scope set to company
When two admitted members address that connector from different conversations
Then both messages are delivered serially to the one shared standing session
And each turn's input tags its conversation ID, place context, thread and speaker identity
And each answer posts back to the conversation where its message was asked
```

### UC-CV24: Suppress raw turn output and deliver explicit replies

```gherkin
Given a company home declares a named connector with reply_mode set to explicit
When a member addresses that connector and the agent finishes its turn with raw output
Then the raw turn output is not posted to the conversation
When the active conversation session runs instrukt company reply for an admitted conversation
Then the command outputs only the queued reply summary without the reply text
And the connector posts the explicit reply to that conversation through the durable answer outbox exactly as the agent wrote it, without searching for or replacing any secret value
```

## Known caveats

- An old unmatched-input finding from an automatically adopted source cannot
  recover the original source identity. Migration preserves that finding and
  holds the binding with an unavailable identity. The operator restores the
  correct binding/source from retained evidence or uses existing confirmed
  replacement recovery when its presence conditions permit it; migration
  invents neither an identity nor a rebinding command.

- If a thread is deleted after the relay checks readiness and before it
  revalidates dispatch under lock, T3 can refuse the send, and the saved
  question remains saved for automatic native succession on the next pass.

- Read-back reads what the provider still serves; its retention, the app's
  access and rate limits are its only limits.
- A turn that failed may already have acted; its message is delivered again and
  the agent checks before acting again.
- The agent cannot start a conversation. Outside the answer to an addressed
  message it posts only an explicit reply, into a conversation its binding
  already admits. The direct messages instrukt sends about a person's sign-ins
  are instrukt's own, not the agent's.
- A conversation session loads its procedure, and its agent program reads
  the company's knowledge from the home's committed `AGENTS.md`, when the
  session starts. A later change to either
  reaches a session already running only as far as its agent program reads
  its instructions again.
- That the agent acts on the addressed message for its writer, reads the
  context as context and works only through the company's declared servers
  is an instruction to the agent in its procedure, not confinement, which
  this contract does not provide.
- Everyone the connector's provider knows as one of the company's own is
  served alike.
- A conversation session runs in its company's container and reads what that
  container holds, the home's secrets file among it, and nothing of another
  company or the host
  (`project/instrukt/spec/feature/runner/company-containers`). A company's
  container is not confinement enough for an agent that answers people outside
  the company, and no such agent exists here.

## See Also

- [project/instrukt/spec/domain/company](../../domain/company.md)
- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/company/slack-connector](slack-connector.md)
- [project/instrukt/spec/feature/runner/conversation-links](../runner/conversation-links.md)
- [project/instrukt/spec/feature/runner/session-continuity](../runner/session-continuity.md)
- [project/instrukt/spec/feature/runner/mcp-access](../runner/mcp-access.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
