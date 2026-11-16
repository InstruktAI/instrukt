---
description: Conversations — a person addresses a company's agent in a connected channel; each conversation is bound to one plain agent session in the company's home, every addressed message is its own turn naming its sender, and the session's answer is posted back, never through event subscriptions.
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

`modules.conversations` is a global-only switch, off by default. A company home
declares its connectors under `connectors:`
(`project/instrukt/spec/feature/configuration/layered-configuration`); a
declared connector with the switch off is a validation finding naming the
switch. The long-lived `instrukt run` hosts every declared connector of every
company home on the computer; a scheduled tick hosts none. Conversations need a
selected runtime with the capabilities
`project/instrukt/spec/feature/runner/session-continuity` lists for them.

### What a connector supplies

A connector is the code for one channel provider. For each message addressed
to the agent it supplies the company home that declares it, a stable
conversation identity, the sender's identity on that connector, the time the
message was written, its text, and the conversation's earlier messages its
provider contract says accompany it. It posts text to a conversation it
carries. Which messages count as addressed, and what accompanies them, is each
connector's own contract
(`project/instrukt/spec/feature/runner/slack-connector`).

### The session bound to a conversation

- A conversation is identified by company, connector and the connector's
  conversation identity. It is bound to exactly one agent session, started or
  woken by key through the runtime adapter. Key discovery, ambiguous starts and
  settled threads follow session-continuity: a failed listing, start or send
  leaves the message pending for that same conversation and never creates a
  second session.
- The session works in the company's home
  (`project/instrukt/spec/feature/configuration/company-homes`). It is a plain
  agent: it has no procedure assignment and answers no event subscription. Its
  opening input is a runner-authored assignment envelope naming the company,
  the connector and the conversation, and saying that its answers are posted
  back.
- Its role is `conversation`; model selection, class and grants follow that
  role's entry in `agents.yaml`
  (`project/instrukt/spec/feature/configuration/agent-selection`).
- Resolution is serialized per conversation. The runner retains the binding,
  each accepted message until it is delivered, and each answer until it is
  posted.

### One addressed message, one turn

- Each addressed message is delivered as its own input and starts its own
  turn. Messages of one conversation are delivered in the order they were
  written; a later one waits until the session's current turn has finished.
- The input is platform-framed with the registered envelopes of
  `project/instrukt/spec/feature/runner/conversation-links`: a conversation
  context envelope carrying the accompanying earlier messages as data, each
  with its speaker and time and with the agent's own earlier posts marked as
  its own, followed by a member envelope carrying the addressed message's text
  unchanged, with connector, conversation, member and time in its opening.
- The speaker of a turn is the member named by its member envelope. The runner
  records it for the turn; `project/instrukt/spec/feature/runner/mcp-access`
  reads it.

### Members and unknown senders

The connector's sender identity is resolved to a member through the company's
member list. A sender no member lists gets one fixed reply in that
conversation, saying they are not listed for this company's agent; no session
is started or woken and nothing is delivered. The reply is sent once per sender
and conversation. A person who is not a member still appears in a context
envelope as a speaker, by the name the connector gives.

### The answer back

- When a turn started by an addressed message finishes, the agent's
  conversational output of that turn is posted to the conversation, in order,
  split to the connector's message size limit. Tool calls, tool results,
  thinking and control input are not posted. A bare `NOOP` posts nothing.
- The turn relay `instrukt run` already hosts does this: the same durable
  conversation source, completed-turn evidence and retained positions as
  conversation links. A position advances only after the connector accepted
  the post; a failed post stays pending and is retried.
- A turn someone starts directly in the session's thread is not posted. The
  agent posts only as the answer to an addressed message.
- `instrukt events status` shows each connector's connection state, the
  conversations with pending messages or answers, and failures with their
  cause, without conversation text or credentials.

## Use cases

### UC-CV1: Ask a company's agent where the team talks and be answered there

```gherkin
Given a company member can address the company's agent in a connected channel
When they address it with a question
Then the agent's answer appears in that same conversation
And the answer appears nowhere else
```

### UC-CV2: Continue a conversation with the agent that already knows it

```gherkin
Given a conversation already has its agent session
When a member addresses the agent there again
Then the same session answers with what it already knows of the conversation
And no second session exists for that conversation
```

### UC-CV3: Know who is asking in a shared conversation

```gherkin
Given two members address the agent in one conversation, one after the other
When their messages reach the agent
Then each message is its own turn naming the member who wrote it
And the second turn starts only after the first has finished
```

### UC-CV4: Turn away a person the company has not listed

```gherkin
Given a person who is not among the company's members addresses the agent
When their message arrives
Then they receive one fixed reply saying they are not listed
And no agent session starts and nothing is delivered to one
```

### UC-CV5: Keep conversations apart from event procedures

```gherkin
Given a developer has published a procedure subscribed to every event
When a member addresses the company's agent in a connected channel
Then the conversation's own session answers
And no subscribed procedure is activated by that message
```

## Known caveats

- Connectors are served only while `instrukt run` is running. instrukt installs
  nothing that keeps it running, and a message written while it is stopped is
  received only when the provider still offers it on reconnection.
- The agent cannot start a conversation or post anything that is not the
  answer to an addressed message.
- A post whose acceptance was lost can be sent twice.
- Everyone the connector lets address the agent and whom the company lists as
  a member is served alike; a member entry grants nothing and restricts
  nothing.
- A conversation session reads what its home and the computer let it read. A
  connector open to people outside the company needs an agent that is confined;
  no such confinement exists here.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/slack-connector](slack-connector.md)
- [project/instrukt/spec/feature/runner/conversation-links](conversation-links.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/runner/mcp-access](mcp-access.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
