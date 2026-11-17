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
| `context_messages` | The largest number of earlier messages that accompany an addressed message. A positive whole number; 30 when omitted. |

A literal token, a missing token and an unknown field are validation findings
naming the company and the field.

### The Slack app

Each company's workspace has its own Slack app, created from this manifest and
installed to the workspace. Its name is `agent`: Slack shows that name as the
sender of every answer and in front of the status line.

```yaml
display_information:
  name: agent
  description: The company's agent. Tag it in a channel or message it directly.
features:
  app_home:
    home_tab_enabled: false
    messages_tab_enabled: true
    messages_tab_read_only_enabled: false
  bot_user:
    display_name: agent
    always_online: true
oauth_config:
  scopes:
    bot:
      - app_mentions:read
      - chat:write
      - channels:history
      - groups:history
      - im:history
      - users:read
      - users:read.email
settings:
  event_subscriptions:
    bot_events:
      - app_mention
      - message.im
  interactivity:
    is_enabled: false
  org_deploy_enabled: false
  socket_mode_enabled: true
  token_rotation_enabled: false
```

The app token is generated in the app's settings as an app-level token with
the scope `connections:write`. The bot token is issued when the app is
installed. The agent hears a channel only once it has been invited to it.

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

### Who is answered

For each addressed message the connector reads Slack's record of the sender
after the message was written. The sender is one of the company's own only
when that record shows all of the following; a record that leaves one of them
undecided admits no one.

| Slack's record | Required |
| --- | --- |
| `deleted` | false: the account is not deactivated. |
| `is_bot` | false, and the sender is not Slackbot. |
| `is_restricted`, `is_ultra_restricted` | both false: the person is no guest. |
| Workspace | The workspace the app is installed in, which the connector learns from Slack for its own bot token. The record's `team_id` equals it, or, in an Enterprise organization, the workspaces the record lists for the person include it. |

The sender's name and email address come from that same record and are kept
with the message. The name is the person's display name, their real name when
that is empty, and their Slack id when Slack gives neither. Anyone who is not
one of the company's own gets the fixed reply
`I only answer members of this workspace.`, in a channel and in a direct
message alike.

### While the agent works

From the admission of an addressed message until its turn ends, the connector
keeps Slack's status line set for that message with the text
`is working on your request`. Slack puts the app's name in front of it. In a
channel the status is set on the conversation's thread; in a direct message it
is set on the addressed message. Slack drops a status after two minutes and
when the app posts, so the connector sets it again at least once a minute
while a message of that conversation waits or is being answered, and clears it
when the last such turn ends, whatever that turn posted. After a restart it
sets the status again for every message still waiting or being answered. A
status Slack refuses is logged and does not fail the conversation.

### What accompanies an addressed message

The connector reads the accompanying messages with the bot token when the
addressed message arrives and keeps them with that message until it is
delivered, so a message that waits for an earlier turn is delivered with
exactly its own range.

| Addressed message | Accompanied by |
| --- | --- |
| The first in a conversation | The messages before it: of the thread, or of the channel when the tag starts the thread. |
| A later one | The messages written in that conversation after the previous addressed message and before this one. |

An addressed message is accompanied by at most `context_messages` messages.
When its range holds more, the most recent ones accompany it and the
conversation context says that earlier messages were left out.

Each accompanying message carries its speaker and time; the agent's own posts
are marked as its own. A speaker is named as a sender is, and in every message
the connector supplies, the addressed one included, a mention is written as
`@` and that person's name; nothing else in a text changes. A name Slack
cannot give at that moment is written as the Slack id and holds no message
back. Slack's own notices in a conversation, such as a person joining a
channel, accompany no message and do not count toward `context_messages`. The range does not depend on whether the agent posted an
answer. Messages the agent was sent before stay in its session and are not
sent again.

### Posting

An answer is posted with the bot token into the conversation it answers: in
the thread for a channel, in the direct-message channel otherwise. A long
answer is split at Slack's message size limit and posted in order.

## Use cases

### UC-SL1: Connect a company's Slack without exposing the computer

```gherkin
Given a developer has declared a company's Slack connector
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
Then the agent receives that message with the most recent messages written there since the previous message that tagged it
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
- Two people with the same name look alike to the agent.

## See Also

- [project/instrukt/spec/feature/runner/conversations](conversations.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- https://docs.slack.dev/apis/events-api/using-socket-mode/ — Slack's Socket Mode: the app opens the connection and acknowledges each envelope.
- https://docs.slack.dev/reference/events/app_mention/ — the mention event the app subscribes to.
- https://docs.slack.dev/reference/events/message.im/ — the direct-message event the app subscribes to.
- https://docs.slack.dev/reference/objects/user-object/ — the fields that tell a full member from a bot, a guest and a member of another workspace.
- https://docs.slack.dev/reference/methods/assistant.threads.setStatus/ — the status line: its text, its two-minute limit and its removal when the app replies.
