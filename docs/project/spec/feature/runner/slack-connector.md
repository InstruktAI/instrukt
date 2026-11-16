---
description: Slack connector — a company's Slack workspace reaches the company's agent over a connection instrukt opens outward, with no public address; in a channel only a message that tags the agent reaches it, together with what was said before.
---
# Slack Connector — Spec

## What it is

The Slack connector joins one Slack workspace to one company's agent. instrukt
opens the connection to Slack itself, so the computer needs no public address
and no webhook. In a channel the agent hears only what is addressed to it: a
message that tags it, with the conversation around that message. The
conversation contract it serves is
`project/instrukt/spec/feature/runner/conversations`.

## Canonical fields

### Declaration

```yaml
connectors:
  slack:
    app_token: ${SLACK_APP_TOKEN}
    bot_token: ${SLACK_BOT_TOKEN}
    context_messages: 30
```

| Field | Contract |
| --- | --- |
| `connectors.slack` | Company home only; one Slack workspace per company home. |
| `app_token` | Reference to the Slack app's connection token in the home's secrets file. Required. |
| `bot_token` | Reference to the Slack app's bot token in the home's secrets file, used to read a conversation and to post. Required. |
| `context_messages` | How many earlier messages accompany the first tag in a conversation. A positive whole number; 30 when omitted. |

A literal token, a missing token and an unknown field are validation findings
naming the company and the field. A member's Slack identity is
`members.<member>.slack`, the person's member id in that workspace
(`project/instrukt/spec/feature/configuration/company-homes`).

### Connection

`instrukt run` opens one Socket Mode connection per declared Slack connector,
outward, with the app token. It acknowledges what it receives as Slack
requires and reconnects after a drop. A connection that cannot be established,
or whose tokens Slack refuses, is a visible failure for that company's
connector; other connectors, event readers and local work continue.

The Slack app is subscribed to exactly two kinds of message: mentions of the
agent and direct messages to it. Slack sends instrukt nothing else from the
workspace.

### What reaches the agent

| Where | A message is addressed when | The conversation is |
| --- | --- | --- |
| A channel | it tags the agent | the thread the tag is in; a tag outside a thread starts the thread under itself |
| A direct message to the agent | always | that direct-message channel |

A message that does not tag the agent in a channel never reaches instrukt,
whether or not the agent already takes part in that thread.

### What accompanies an addressed message

The connector reads the conversation with the bot token at the moment the
addressed message arrives.

| Addressed message | Accompanied by |
| --- | --- |
| The first in a conversation | The most recent `context_messages` messages before it: of the thread, or of the channel when the tag starts the thread. |
| A later one | Every message written in that conversation since the agent's last post there. |

Each accompanying message carries its speaker and time; the agent's own posts
are marked as its own. Messages the agent was sent before stay in its session
and are not sent again.

### Posting

An answer is posted with the bot token into the conversation it answers: in
the thread for a channel, in the direct-message channel otherwise. A long
answer is split at Slack's message size limit and posted in order.

## Use cases

### UC-SL1: Connect a company's Slack without exposing the computer

```gherkin
Given a developer has declared a company's Slack connector and switched conversations on
When the runner is running
Then messages addressed to the agent in that workspace reach the company's agent
And the computer exposes no public address for them
But when Slack refuses the declared tokens
Then that connector's failure is visible and the company's other work continues
```

### UC-SL2: Reach the agent in a channel only by tagging it

```gherkin
Given people talk in a channel thread where the agent has answered before
When they write messages that do not tag the agent
Then none of them reaches the agent
When someone then tags the agent in that thread
Then the agent receives that message with everything said there since its last post
```

### UC-SL3: Bring the agent into a conversation that is already under way

```gherkin
Given people have been talking in a channel without the agent
When someone tags the agent for the first time there
Then the agent receives that message with the most recent messages before it
```

### UC-SL4: Talk to the agent directly without tagging it

```gherkin
Given a company member opens a direct message with the agent
When they write to it
Then each of their messages reaches the agent without a tag
```

## Known caveats

- A message written while the connection is down reaches the agent only when
  Slack still delivers it after reconnection.
- The agent reads no more of a conversation than what accompanies an addressed
  message.
- Files, images and reactions are not carried; a message's text is.
- One Slack workspace serves one company home.

## See Also

- [project/instrukt/spec/feature/runner/conversations](conversations.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- https://docs.slack.dev/apis/events-api/using-socket-mode/ — Slack's Socket Mode: the app opens the connection and acknowledges each envelope.
- https://docs.slack.dev/reference/events/app_mention/ — the mention event the app subscribes to.
- https://docs.slack.dev/reference/events/message.im/ — the direct-message event the app subscribes to.
