---
description: Conversations — a person addresses a company's agent in a connected channel; each conversation is bound to one agent session in the company's home, started from the conversation procedure, every addressed message is its own turn naming its sender, and the session's answer is posted back, never through event subscriptions.
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
provider contract says, and shows there that the agent is working where its
provider has a way to. Which messages count as
addressed, who counts as the company's own, and what accompanies a message is
each connector's own contract
(`project/instrukt/spec/feature/runner/slack-connector`).

### The session bound to a conversation

- A conversation is identified by company, connector and the connector's
  conversation identity. It is bound to exactly one agent session, started or
  woken by key through the runtime adapter. Key discovery, ambiguous starts and
  settled threads follow session-continuity: a failed listing, start or send
  leaves the message pending for that same conversation and never creates a
  second session.
- The session works in the company's home
  (`project/instrukt/spec/feature/configuration/company-homes`), where it
  starts with the company's own knowledge
  (`project/instrukt/spec/feature/publication/agent-entrypoints`). It answers
  no event subscription.
- It starts from the conversation procedure: the published procedure that
  declares the role `conversation`. One procedure on a computer declares a
  role, so every company's conversations start from the same one. It comes
  from the docs repository that publishes it
  (`project/instrukt/spec/feature/publication/docs-repositories`); instrukt's
  own publishes it as `general/procedure/conversation`. The procedure is
  instrukt's and the knowledge is the company's; neither repeats the other.
- The session's opening input is the runner-authored assignment envelope,
  followed by the first addressed message's input. The assignment is the
  loader of `project/instrukt/spec/feature/runner/procedure-event-delivery`:
  it names the procedure, the role with its resolved grants, the company, the
  connector and the conversation, and directs
  `instrukt docs get <procedure-id> --closure` before the message is handled.
  The procedure is read from current publication when the session starts; a
  woken session keeps the assignment it started with.
- While no published procedure declares the role `conversation`, a
  conversation's first message waits: no session is started, nothing is
  delivered or replied, and the cause shows in `instrukt events status`.
- Its role is `conversation`; model selection, class and grants follow that
  role's entry in `agents.yaml`
  (`project/instrukt/spec/feature/configuration/agent-selection`).
- Resolution is serialized per conversation. The runner keeps the binding,
  each accepted message and each answer, so a message is not lost before it is
  delivered and an answer is not lost before it is posted. What is stored and
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
- A turn someone starts directly in the session's thread is not posted. The
  agent posts only as the answer to an addressed message.
- From the admission of an addressed message until its turn ends, the
  connector shows in that conversation that the agent is working, where its
  provider has a way to. The indication belongs to the turn, whether the turn
  posts an answer, posts nothing or fails; it is a courtesy, and each
  connector states how closely it follows the turn.
- `instrukt events status` shows each connector's connection state, the
  conversations with pending messages or answers, and failures with their
  cause, without conversation text or credentials. A connector that no running
  runner hosts is shown as stopped.

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

### UC-CV6: Be answered by an agent that knows its job in the conversation

```gherkin
Given the computer publishes the procedure that declares the conversation role
When a member first addresses the company's agent in a conversation
Then the conversation's session starts in the company's home with the role conversation
And its opening input names that procedure and directs loading it in full before the member's message
And the member's message follows as its own envelopes
But when no published procedure declares the conversation role
Then the message waits, no session starts and the cause is shown
```

## Known caveats

- A message written while a connector's connection is down is received only
  when the provider still offers it on reconnection.
- The agent cannot start a conversation or post anything that is not the
  answer to an addressed message.
- A post whose acceptance was lost can be sent twice.
- A conversation session keeps the procedure and the company knowledge it
  loaded when it started. A later change to either reaches only conversations
  whose session starts after it.
- That the agent acts on the addressed message for its writer, and reads the
  context as context, is what its procedure tells it. The agent is a language
  model and nothing beyond the procedure enforces that reading.
- Everyone the connector's provider knows as one of the company's own is
  served alike.
- A conversation session reads what its home and the computer let it read,
  the home's secrets file among it. A connector open to people outside the
  company needs an agent that is confined; no such confinement exists here.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/slack-connector](slack-connector.md)
- [project/instrukt/spec/feature/runner/conversation-links](conversation-links.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/runner/mcp-access](mcp-access.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
