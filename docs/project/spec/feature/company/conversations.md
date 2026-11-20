---
description: Conversations — a person addresses a company's agent in a connected channel; each conversation is bound to one agent session in the company's home, started from the company-assistance procedure, every addressed message is its own turn naming its sender, and the session's answer is posted back, never through event subscriptions.
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

A company home declares its connectors under `connectors:`
(`project/instrukt/spec/feature/configuration/layered-configuration`).
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
provider knows the sender as one of the company's own people, the sender's
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

- A conversation is identified by company, connector and the connector's
  conversation identity. It is bound to exactly one agent session, started or
  woken by key through the runtime adapter. Key discovery, ambiguous starts and
  settled threads follow session-continuity, case by case: a failed listing,
  start or send leaves the message pending for that same conversation and is
  retried. A lost acknowledgement or a live key conflict goes to the operator.
- When work is waiting, the relay checks the bound session. A rename preserves
  delivery to that session. An archived session is left in place and the operator
  is told to unarchive or delete it. Deletion is not evidence that the old
  provider stopped. Once the operator knows it has stopped, the finding directs
  them to `instrukt sessions replace-deleted <key> --stopped <runtime:id>`
  (`project/instrukt/spec/feature/runner/session-continuity`). Neither an archive
  nor a deletion starts a successor automatically. While work is blocked the member
  sees the fixed unavailable status and can receive the usual notice, with the
  cause and recovery command visible only to the operator.
- An operator-confirmed successor starts with the assignment alone. The relay
  binds it to the same conversation and hands over the questions still awaiting
  turns in written order, including work handed to the stopped session whose
  answer has not yet been retained. Retained answers keep their posting state.
- The session works in the company's home
  (`project/instrukt/spec/feature/configuration/company-homes`), where it
  starts with the company's own knowledge
  (`project/instrukt/spec/feature/publication/agent-entrypoints`). It answers
  no event subscription.
- It starts from the company-assistance procedure: the published procedure that
  declares the role `company-agent`. One procedure on a computer declares a
  role, so every company's conversations start from the same one. It comes
  from the docs repository that publishes it
  (`project/instrukt/spec/feature/publication/docs-repositories`); instrukt's
  own publishes it as `general/procedure/company-assistance`. The procedure is
  instrukt's and the knowledge is the company's; neither repeats the other.
- The session's opening input is the runner-authored assignment envelope,
  followed by the first addressed message's input. The assignment is the
  loader of `project/instrukt/spec/feature/runner/procedure-event-delivery`:
  it names the procedure, the role with its resolved grants, the company, the
  connector and the conversation, and directs
  `instrukt docs get <procedure-id> --closure` before the message is handled.
  The procedure is read from current publication when the session starts; a
  woken session keeps the assignment it started with.
- The conversation's agent works through the company's declared MCP servers
  (`project/instrukt/spec/feature/runner/mcp-access`) and on the company's
  own documents in its home, which it may change and commit
  (`project/instrukt/spec/feature/publication/agent-entrypoints`), and
  declines anything else. Its procedure instructs it so.
- Its role is `company-agent`; model selection, class and grants follow that
  role's entry in `agents.yaml`
  (`project/instrukt/spec/feature/configuration/agent-selection`).
- Resolution is serialized per conversation. A delivery pass that cannot take
  one conversation's lock within its wait skips that conversation, whose messages
  stay pending for the next pass, and goes on with the others; this is apart from
  the runtime-dispatch lock, whose timeout skips a whole wake
  (`project/instrukt/spec/feature/runner/work-dispatch`). The company context keeps the
  binding, each accepted message and each answer, so a message is not lost
  before it is delivered and an answer is not lost before it is posted. What is stored and
  kept is `project/instrukt/spec/feature/work/state-database`.

### One addressed message, one turn

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
  connector supplies it, with connector, conversation, member and time in its
  opening.
  Both carry the delivery's boundary token, so a line inside them that looks
  like an opening or a closing stays content.
- The speaker of a turn is the person who wrote its addressed message. Their
  connector identity, name and email address, as the provider gave them when
  that message was admitted, are stored with the message and do not change
  afterwards; `project/instrukt/spec/feature/runner/mcp-access` reads them.

### Who is answered

The agent answers the company's own people, as the connector's provider knows
them; nobody is listed for it. Whether a sender is one of them is decided for
each addressed message, from what the provider says about the sender after
that message was written. An earlier answer about the same person admits
nothing. A message is delivered only once that is decided; while the provider
cannot be asked, the message waits, nothing is delivered and nothing is
replied, and the cause shows in `instrukt events status`. A message that was
admitted is delivered, and a turn that has started runs to its end.

A sender who is not one of the company's own gets one fixed reply in that
conversation, worded by the connector; no session is started or woken and
nothing is delivered. The reply is sent once per sender and conversation. Such
a person still appears in a context envelope as a speaker, by the name the
connector gives.

### The answer back

- When a turn started by an addressed message finishes, the agent's
  conversational output of that turn is posted to the conversation as the
  agent wrote it, in parts where the connector's message size limit requires,
  in order. Tool calls, tool results,
  thinking and control input are not posted. A bare `NOOP` posts nothing.
- The turn relay `instrukt run` already hosts does this: the same durable
  conversation source, completed-turn evidence and retained positions as
  conversation links. A position advances only after the connector accepted
  the post; a failed post stays pending and is retried.
- A person receives each part of an answer once. Each part is recorded as
  being posted before its post is sent. A part the connector accepted is never
  posted again: when recording that acceptance fails, only the record is
  retried. A part left being posted with no acceptance recorded, because its
  post's response was lost or the runner stopped in between, is first looked
  for among the connector's own posts in the conversation since its first
  addressed message. The connector matches a durable id reserved for the
  piece before the post. For a retained attempt that predates those ids, the
  relay makes one best-effort read within the existing settle window. A
  matching bot post after the attempt supplies the receipt. If the window
  ends without a match, the connector's notice pass settles the piece with
  delivery unknown using the retained record. Expiry proceeds during a
  source or provider outage. The piece produces neither a retry nor a
  notice, and later work proceeds. Status keeps the cause with the
  conversation, turn and piece. A matching receipt from a read started
  within the window still confirms delivery if it arrives after expiry.
  It replaces the unknown result. Expiry leaves a confirmed delivery intact.
  A successful post response confirms delivery even when the provider omits
  its timestamp. A lost or failed response that leaves delivery uncertain
  is read back by metadata. Another attempt waits for a read begun after the
  timeout window that finds no post. When receipt reads keep failing through
  that window, the piece settles as delivery unknown and later work proceeds.
  Status retains the cause, and the piece produces neither a retry nor a notice.
- Completion comes only from the conversation source. A turn running however
  long is left to run. A turn that ended failed has its message delivered again
  to the session after a growing delay, at the end of the conversation's queue,
  so later messages are not held behind it; what the failed turn already did is
  the agent's to reconcile (`general/procedure/company-assistance`). A completed
  turn whose content the source cannot yet read whole is read again every pass;
  nothing partial and nothing of another message is posted meanwhile. When the
  source's identity changes, the conversation's thread is read whole from the new
  source and matched by each message's boundary; turns already posted are
  recognised by their turn identity and not posted again; a message whose turn
  cannot be found stays sent and goes to the operator, never delivered again.
- A turn someone starts directly in the session's thread is not posted. The
  agent posts only as the answer to an addressed message.
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
| The agent has been unable to work on it for one minute | One message, worded by the connector, beginning with the person's mention: "The assistant is temporarily unavailable. Your message is saved and you'll get the answer here when it's back — no need to send it again." Once per person and conversation until an answer to that person has been posted. |
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

**The one notice** is once per person per epoch. An epoch records the durable
admission positions of the person's messages that made the notice due.

- The decision is recorded before the post (owed, posting with the attempt's time,
  posted); the record is never taken as proof of delivery.
- Each answer piece carries the admission position of the message it answers.
  An epoch closes when every covered position is settled. A turn that
  completes successfully with a bare `NOOP` or blank output settles its
  position. A multipart answer settles its position once every piece is
  confirmed or resolved as delivery unknown. A failed turn with no reply
  keeps its interval and notice epoch open until a later-admitted message
  from the same member in the same conversation settles. That settlement
  supersedes every earlier failed message from that member, closes those
  intervals, and settles those positions for the notice. Provider timestamps
  and the runner clock do not order answers, notices, or supersession.
- When the delivery of an attempted answer is uncertain, the relay reconciles
  it before a notice can post to that person. A confirmed rejection leaves the
  person's notice eligible once the no-work interval reaches a minute and
  no earlier attempt is still uncertain. A read that finds no post clears
  earlier uncertainty only if it began after the latest unconfirmed request's
  timeout window.
- The notice and the conversation's answers are posted under one short
  per-conversation post lock, held for the decision and one post, never for a
  read at the provider or a runtime call. An answer receipt can retire a fully
  answered epoch in the same critical section once the notice's own delivery is
  known. An unconfirmed notice keeps its id until read-back finds it or
  establishes absence after the settle window.
- A notice whose post timed out or lost its response is not posted again before
  the settle bound after the attempt has passed. After it, the connector reads the
  conversation from the epoch's start for a post of its own with the notice's
  durable id. Finding one records it posted, and finding none posts again
  with the same id.

If a post is committed by the provider later than the settle bound after it was sent,
the person can see the sentence twice, or once after their answer: the provider's
methods promise neither a write deadline nor idempotent posts.

The intake interval, the settle bound, and read-back's answering bound and
reporting period below are declared with the conversation core in
`crates/instrukt-company/src/conversations.rs`.

### After the agent was offline

A connector reads back what was written while it was not connected
(`project/instrukt/spec/feature/company/slack-connector`). A message written
within the answering bound that is addressed and not yet retained is admitted and
answered like a live one. An older one is not answered and is named to the
operator in `instrukt events status` for the reporting period, once per message
(`project/instrukt/policy/checks-and-guards`: what instrukt cannot resolve goes to
the operator).

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
Then the member gets one message, beginning with their mention, that the assistant is temporarily unavailable, that their message is saved and that they need not send it again
And writing again before they are answered brings no second one
And another member waiting in the same conversation gets their own
And the member's answer follows once the agent has done the work
And a healthy wait or a turn completed without output does not earn an unavailable notice
```

### UC-CV8: Get the answer after the assistant's work failed

```gherkin
Scenario Outline: Recover the answer
  Given a member has a message waiting for an answer in a connected conversation
  When <failure>
  And the conversation recovers
  Then the member gets the answer to that message
  And nothing else is posted in the conversation, except the one notice that the assistant is temporarily unavailable when the recovery leaves the message without work for a minute

  Examples:
    | failure |
    | the agent's turn failed |
    | the turn's content was not yet readable |
    | the runtime moved its conversation store |
    | the conversation is deleted and the operator confirms the old session stopped before replacing it |
    | the conversation is archived and the operator unarchives it |
```

### UC-CV11: Onboard a company while the runner runs

```gherkin
Given the runner is running and no company is set up
When an operator sets up a company with its connector and publishes its procedure
Then its members' questions are answered without restarting the runner
And a question written before the company was admitted is answered too
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
Then each addressed message is answered once when the assistant is back
And a message that did not address it gets nothing
But a message older than the answering bound is not answered and is named to the operator
```

### UC-CV14: Receive each answer once

```gherkin
Scenario Outline: An answer is posted once
  Given a member's message reached the agent and the connector accepted the post of its answer
  When <interruption>
  Then the member gets that answer exactly once

  Examples:
    | interruption |
    | recording that the connector accepted the post fails |
    | the runner stops before it records that the connector accepted the post |
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
Given the assistant has an answer for a member and its delivery is uncertain
When the assistant recovers from the interruption
Then an answer that reached the member is neither repeated nor followed by an unavailable notice for that answer
And an answer confirmed absent can be sent again
And if delivery cannot be established, the operator sees that outcome and the member can still ask another question
```

### UC-CV17: Keep serving conversations after an upgrade

```gherkin
Given an operator upgrades a runner that stopped while an answer was being delivered
When the operator resumes serving the conversation
Then a delivered answer is recognized when the provider can confirm it
And if delivery cannot be determined, the operator sees that outcome without an extra reply or notice for that question
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

## Known caveats

- Read-back reads what the provider still serves; its retention, the app's
  access and rate limits are its only limits.
- A turn that failed may already have acted; its message is delivered again and
  the agent checks before acting again.
- The agent cannot start a conversation or post anything that is not the
  answer to an addressed message. The direct messages instrukt sends about a
  person's sign-ins are instrukt's own, not the agent's.
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
