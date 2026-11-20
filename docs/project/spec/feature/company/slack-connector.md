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
`project/instrukt/spec/feature/company/conversations`; it is a connector of the
company context and implements the connector interface that context defines
(`project/instrukt/spec/domain/company`).

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

`instrukt config schema --company <name> connectors.slack` shows this
declaration and the setup below, the manifest in full
(`project/instrukt/spec/feature/configuration/configuration-editing`).

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
      - channels:read
      - groups:history
      - groups:read
      - im:history
      - im:read
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
connector; other connectors, event readers and local work continue. Where
Slack limits a method and names a wait, the connector does not call that
method for that workspace again before the wait is over, for as long as the
runner runs; after a restart it learns the wait again from Slack's answer.
The runner admits declared connectors when it starts and at every wake: a
connector declared, changed or removed while it runs is connected, reconnected or
disconnected at the wake that admits the change
(`project/instrukt/spec/feature/runner/work-dispatch`: the runner works from its
admitted configuration). At admission the connector reads the scopes Slack names
for its bot token; one the manifest requires that is missing withholds the
connector with a finding naming it
(`project/instrukt/spec/feature/configuration/company-homes`). A token Slack
refuses later is retried and reported like a lost connection.

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

Only a message a person newly wrote starts a turn, with its text, whatever
else it carried. This rule decides which messages start a turn; it does not
filter what accompanies an addressed message. A direct message starts a turn
when Slack sends it with no subtype or with one of `file_share`, `me_message`
(written with `/me`) and `thread_broadcast`. A message that carries a file
starts a turn whether Slack sends it with no subtype or as `file_share`;
Slack's pages disagree on which of the two it sends. An edit
(`message_changed`), a deletion (`message_deleted`) and every other record
that is not a newly written message start nothing. In a channel the tag
addresses a message whether or not it carried a file, and an edit starts
nothing. A file is never carried: a message that carried only a file starts a
turn with empty text.

### Who is answered

For each addressed message the connector reads Slack's record of the sender
after the message was written. The sender is one of the company's own only
when that record shows all of the following; a record that leaves one of them
undecided admits no one. When Slack answers that it shows the app no such
user (`user_not_found`, `user_not_visible`), the sender is not one of the
company's own. When Slack gives no answer about the sender, because the
request fails, is limited or Slack refuses the app's own token, nothing is
decided and the message waits; Slack is asked again until it answers
(`project/instrukt/policy/checks-and-guards`: an outside failure is retried).

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

From the admission of an addressed message until it is complete — its answer
posted, or its turn completed with a bare `NOOP` or a successful blank reply —
the connector shows Slack's status line for it, with the text the message's state gives
(`project/instrukt/spec/feature/company/conversations`):

| The message's state | Text |
| --- | --- |
| its no-work interval is open, the relay-wide failure included (the interval the notice reads) | `is temporarily unavailable — your message is saved` |
| its turn runs, or ended with its answer being posted, and the agent has written in that turn | the newest message the agent has written in that turn, as written |
| otherwise, from its admission | `is working on your request` |

Slack puts the app's name in front of it. Each status pass sets the text that
matches the state as it then stands, so a change of state shows at the next
pass. Where one status serves several unfinished messages, the unavailable text
shows while every one of them has its interval open; otherwise the newest
message the agent has written in its turn, or the working text before there is
one.

| Where | The status sits on | It stays while |
| --- | --- | --- |
| A channel | the conversation's thread | any admitted message of that thread is unfinished |
| A direct message | the addressed message itself | that one message is unfinished |

A posted answer, a bare `NOOP`, or a successful blank reply completes the message.
A failed turn awaiting redelivery or an answer awaiting posting leaves it
unfinished.

Admitting a message wakes the connector's status pass, so the person normally
sees at once that the request was taken on; that first setting can wait behind
status calls already under way. The status is a courtesy, kept on a best-effort
basis; no timing of it is promised. Slack drops a status after two minutes and
when the app posts, so
while a status stays the connector sets it again about every half minute, and
after a post of the app's own it can be missing until the next setting. When a
status no longer stays the connector clears it once; where that does not
happen, Slack drops it within two minutes. After a restart the connector sets
the status again for every message still unfinished and clears nothing it set
before. A status call falls under Slack's named wait like any other call and
is skipped while the wait lasts; nothing skipped is made up for. Setting a
status never delays a message, and a status Slack refuses or the connector
skips does not fail the conversation.

### What accompanies an addressed message

The connector reads the earlier messages in the Slack thread, or in the
channel when the addressed message starts a thread. The conversation keeps
a durable record of which Slack messages, identified by timestamp, have
reached the agent as their own turn or as accompanying context.

For each delivery, first attempt or retry, the accompanying context is
the earlier messages in that range absent from the record. Messages still
pending as their own turn are excluded so they can take that turn. A
message that was refused can still accompany an admitted message if the
agent has never received it. The record is updated in the same transaction
as the delivery receipt and follows the retention in the state-database
spec. An attempt with uncertain acceptance keeps its exact framed input
until reconciled. Once the runtime confirms non-acceptance,
the next attempt selects context again against the current record.

An addressed message is accompanied by at most `context_messages` messages.
When its range holds more, the most recent ones accompany it and the
conversation context says that earlier messages were left out. Slack context is
read from newest towards oldest, stopping once the eligible selection and
whether earlier messages were left out are known. The read retains every page
it already fetched for use if eligibility changes before delivery.

Each accompanying message carries its speaker and time; the agent's own posts
are marked as its own. A speaker is named as a sender is, and in every message
the connector supplies, the addressed one included, a mention is written as
`@` and that person's name; nothing else in a text changes. A name Slack
cannot give at that moment is written as the Slack id and holds no message
back. Slack's own notices in a conversation accompany no message and do not
count toward `context_messages`. A notice is what Slack itself writes about a
conversation, and Slack marks it with one of these subtypes: the `channel_`
and `group_` forms of `join`, `leave`, `topic`, `purpose`, `name`, `archive`
and `unarchive`, `channel_convert_to_private`, `channel_convert_to_public`,
`channel_posting_permissions`, `pinned_item`, `unpinned_item` and
`reminder_add`. Everything else is a message, in whatever form Slack records
it: what a person or an integration wrote, and a form the connector does not
know.

The context bound is declared by `CONTEXT_BOUND` in
`crates/instrukt-company/src/conversations.rs`. It starts when the addressed
message is retained and uses the runner's clock. The sender must still be
admitted. Context expiry and handoff run independently of sender lookups and
other provider calls. At expiry the admitted question proceeds, even during an active read.
The context keeps usable messages already read and says that earlier
messages could not be read only when part or all of the needed range
remained unread. If the messages were
read in time, the agent receives them. Name enrichment gets at most the
next intake retry interval and never extends the context bound. Names
still unresolved at handoff use Slack IDs. Later results
cannot change the stored delivery. The context note and provider errors
stay out of Slack.

### Posting

An answer is posted with the bot token into the conversation it answers: in
the thread for a channel, in the direct-message channel otherwise. It is
posted as the agent wrote it, as Markdown in the `markdown_text` argument of
`chat.postMessage`, and Slack renders it: bold and italic, links, lists,
headers, inline code, code blocks, quotes, dividers, tables and task lists.
The connector changes nothing in the agent's text.

An answer longer than 12,000 characters, Slack's limit for `markdown_text`, is
split, and each part is posted as it is, in order. While more than 12,000
characters remain to be posted, a part takes the first 12,000 of them and ends
after the last line break among them when that line break lies after the
first 6,000, and after the 12,000th character otherwise. Once at most 12,000
characters remain, that remainder is posted unchanged as the last part and
splitting ends, so an answer that is an exact multiple of the limit is
followed by no empty part. Nothing is inserted, so no part is longer than
12,000 characters.

The one platform notice of `project/instrukt/spec/feature/company/conversations`
is posted the same way, worded
`<@member-id> The assistant is temporarily unavailable. Your message is saved and you'll get the answer here when it's back — no need to send it again.`,
with the waiting person's Slack id. Each notice and answer piece carries
`chat.postMessage` metadata with event type `instrukt_post` and a single
opaque id in `event_payload.id`. The connector uses
`include_all_metadata=true` with `conversations.replies` for a thread or
`conversations.history` for a direct message. It matches the id on a post
by its own bot and returns the posting timestamp. It reads notices from the
current epoch's anchor and answers from the first addressed message.
For an answer attempted before ids were recorded, the single best-effort
read matches an untagged post from the same bot and conversation by text
and requires a provider timestamp after the recorded attempt.

These Web API calls use the existing bot scopes `chat:write` and the
conversation's history scope. The metadata Events API scope is needed for
metadata event subscriptions. The connector uses Web API read-back.

### Reading back after a connection

After every connection is established, the connector reads back what was written
while it was not connected:

- For each conversation the app is in (`users.conversations`, types
  `public_channel`, `private_channel` and `im`), it reads every parent message
  Slack serves (`conversations.history`, no time filter on parents), the replies
  after the read window's start of each parent whose `latest_reply` is after it
  (`conversations.replies`), and the top-level and direct messages after it.
- The window starts where the last completed read-back started, less a margin;
  with none completed, at the earlier of the time the runner first saw the
  connector declared and the answering bound back
  (`project/instrukt/spec/feature/company/conversations`). Only a read-back that completes moves it;
  a failed, partial or interrupted one reads the same window again.
- Every message read goes through the rules of "What reaches the agent" and "Who
  is answered". One already retained is skipped by its identity; an addressed one
  written within the answering bound is admitted like a live one; an older one is
  not admitted and is named to the operator
  (`project/instrukt/spec/feature/company/conversations`).
- Slack's retention, the app's access and Slack's rate limits are its only
  limits; in a large workspace it runs long, in the background.

### Direct messages from instrukt

When instrukt has something for one person alone, such as a sign-in link
(`project/instrukt/spec/feature/runner/mcp-access`), the connector posts it
with `chat.postMessage` and the bot token to that person's Slack id, which
Slack delivers in the person's direct messages with the agent. It is never
posted in a channel or a thread, whichever conversation the person was in.
Slack's `ratelimited`, a server error and a failed connection are retried;
every other error Slack answers for that call fails the delivery, and the
connector never posts the message anywhere else. Such a message is no answer:
it starts no turn and is not delivered to the agent.

Each such message is posted with Slack message metadata whose `event_type` is
`instrukt_private`, and the connector reads conversation history with that
metadata (`include_all_metadata`). A message the app posted that carries it
accompanies no addressed message, also when its post was accepted but the
answer never reached the connector or a retry posted it twice: it is left out
of every range the connector reads, the first of a conversation, a later one
and one read again after a restart, before what accompanies a message is
captured and framed. The connector also keeps the channel and the time Slack
gives each such message it posted.

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
When someone then tags the agent in that thread in a message that carries a file
Then the agent receives that message's text with the most recent messages written there since the previous message that tagged it
```

### UC-SL3: Bring the agent into a conversation that is already under way

```gherkin
Given people have been talking in a channel without the agent
When someone tags the agent for the first time there
Then the agent receives that message with the most recent messages before it
When the first context read of a reply fails and its late root reaches the agent before the retry
Then the retry includes intervening context and does not send the delivered root again
```

### UC-SL4: Talk to the agent directly without tagging it

```gherkin
Given a company member opens a direct message with the agent
When they write to it, one message with `/me` and one with a file among them
Then each of their messages reaches the agent with its text without a tag
When they edit or delete one of those messages
Then nothing more reaches the agent
```

### UC-SL5: Read the agent's answer as the agent formatted it

```gherkin
Given the agent answers a company member in Slack with bold text, a list, a link and a code block
When the answer is posted
Then the member sees them rendered, not the Markdown that writes them
And an answer longer than Slack takes in one message arrives in parts, in order, with nothing added to what the agent wrote
```

### UC-SL6: Be answered when the earlier messages cannot be read

```gherkin
Given a member addresses the agent in a thread whose earlier messages Slack cannot hand over
Then the message stays saved while context is still being read within the context bound
When the context bound passes
Then the agent receives the member's question and identity with a note that earlier messages could not be read
And the member receives the answer without a technical message about the read
When context becomes readable on retry
Then previously unseen messages can accompany the question even if a refused tag or a later delivery followed them
And queued tags still take their own turns
And a confirmed send refusal causes the next attempt to exclude messages delivered in the meantime
And a stalled earlier page never drops context already read
And messages already delivered out of written order or pruned within the receipt window are not replayed
```

## Known caveats

- A message written while the connection is down is read back after it returns,
  as far as Slack still serves it; one older than the answering bound is named to the operator,
  not answered.
- The agent reads no more of a conversation than what accompanies an addressed
  message.
- Files, images and reactions are not carried; a message's text is. The
  agent is not told that a message carried a file.
- An image the agent writes in an answer shows as a link with its text, not as
  an image.
- Formatting of anything that crosses a split can be lost: a code block, a
  table, a list, a quote, and inline formatting such as bold text, inline code
  or a link. Nothing is repaired.
- How Slack renders HTML, and its own mention syntax, inside an answer is not
  documented; the connector passes them on unchanged.
- The agent's earlier answers come back among the accompanying messages as
  Slack returns their text, which can differ from what the agent wrote.
- One Slack workspace serves one company home.
- Two people with the same name look alike to the agent.
- Slack does not promise the order in which it sends messages. A message that
  reaches instrukt after a later-written one of its conversation keeps its own
  turn: before the later one when that has not been delivered yet, after it
  otherwise. What accompanies the later one was read when it arrived and is
  not read again, so it can hold the earlier message, which the agent then
  sees there and as its own turn.
- An upgraded database can identify previously delivered addressed messages.
  Accompanying messages delivered before their Slack timestamps were kept can
  reappear in context. A Slack message can also reappear after its receipt's
  retention window expires.

## See Also

- [project/instrukt/spec/feature/company/conversations](conversations.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- https://docs.slack.dev/apis/events-api/using-socket-mode/ — Slack's Socket Mode: the app opens the connection and acknowledges each envelope.
- https://docs.slack.dev/reference/events/app_mention/ — the mention event the app subscribes to.
- https://docs.slack.dev/reference/events/message.im/ — the direct-message event the app subscribes to.
- https://docs.slack.dev/reference/events/message/ — the message subtypes, among them `me_message`, `thread_broadcast`, `message_changed` and `message_deleted`.
- https://docs.slack.dev/reference/events/message/file_share/ — says the subtype is no longer served, and shows a direct message that carries a file with `files` and its text and no subtype.
- https://docs.slack.dev/changelog/2018-05-file-threads-soon-tread/ — says the Events API keeps sending the subtype `file_share`, with the files in a `files` array.
- https://docs.slack.dev/reference/methods/chat.postMessage/ — `markdown_text`: Markdown Slack renders, at most 12,000 characters, never together with `text` or `blocks`; `thread_ts` for a reply in a thread.
- https://docs.slack.dev/reference/block-kit/blocks/markdown-block/ — the Markdown Slack renders, and that an image shows as link text.
- https://docs.slack.dev/reference/objects/user-object/ — the fields that tell a full member from a bot, a guest and a member of another workspace.
- https://docs.slack.dev/messaging/message-metadata/ — message metadata: an `event_type` of letters, digits, underscores and hyphens, attached with the `metadata` argument, needing no registration; `conversations.history` returns it with `include_all_metadata`.
- https://docs.slack.dev/reference/methods/conversations.replies/ — thread read-back with `include_all_metadata` and the applicable history scope.
- https://docs.slack.dev/reference/methods/chat.postMessage/#post-to-a-direct-message-channel — a bot posts in its direct messages with a person by passing the person's user id as the channel, with `chat:write`.
- https://docs.slack.dev/reference/methods/assistant.threads.setStatus/ — the status line: its text, its two-minute limit and its removal when the app replies.
