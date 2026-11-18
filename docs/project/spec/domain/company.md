---
description: The company bounded context — a company's people talk to its agent in the channels they already use; connectors, the conversations they carry, who is answered, the answers posted back, and the speaker of each turn.
---

# Company — Spec

## What it is

The company context lets a company's people talk to the company's agent where
they already talk. It owns the connectors that carry addressed messages in and
answers out, the conversations those messages belong to, each bound to one agent
session in the company's home, the decision of who is answered, and the answers
posted back. It keeps that state in the conversation tables of the state
database. It does not own the company home and its settings (configuration), the
running of agent sessions and the relay of finished turns (runner), or the
state database's file and migrations (work).

## Canonical fields

### Aggregates

#### Conversation binding

- **Root entity:** One conversation's binding to its agent session.
- **Contains:** Company, connector and the connector's conversation identity;
  the session's runtime identity; accepted messages awaiting delivery with
  their senders; the speaker of the current turn; answers awaiting posting.
- **Invariants:** One conversation has one session and one session serves one
  conversation. Each addressed message is one turn. A message activates no
  subscribed procedure and emits no event. Only the answer to an addressed
  message is posted.
- **Identifier:** Company, connector and conversation identity.
- **Lifecycle:** The first addressed message from a member starts the session
  by key; later ones wake it. The binding outlives a settled thread and ends
  with nothing instrukt does. Its contract is
  `project/instrukt/spec/feature/company/conversations`.

#### Connector

- **Root entity:** One company's declared connector of one kind.
- **Contains:** Its kind, its declared settings, and its connection state.
- **Invariants:** A connector carries messages for the one company home that
  declares it. What it supplies for each addressed message, and what it does
  with an answer, is the connector contract of
  `project/instrukt/spec/feature/company/conversations`; which messages count as
  addressed, who counts as the company's own and how an answer is rendered are
  its kind's own contract.
- **Identifier:** Company and kind.
- **Lifecycle:** Declared in the company home under `connectors.<kind>`; hosted
  while `instrukt run` runs; ends when the declaration is removed.

## Ubiquitous language

**Connector**
: The code for one channel provider. It carries messages addressed to a
  company's agent in and the agent's answers out, and implements the connector
  interface this context defines.

**Connector kind**
: The channel provider a connector speaks to, named by its key under
  `connectors:` in a company home. Each kind declares the settings its
  declaration takes.

**Conversation**
: A person talking to a company's agent in one place of a connected channel,
  bound to exactly one agent session in the company's home.

**Addressed message**
: A message its connector's contract counts as meant for the agent; it starts
  one turn.

**Speaker**
: The company member whose addressed message started a conversation session's
  current turn.

## Ownership boundary

- **Owns:** connectors and their hosting; the connector interface; conversation
  bindings, per-message turn delivery with its speaker, who is answered, the
  fixed reply to someone who is not, and posting a conversation's answers back;
  the speaker of a conversation session's turn; the meaning and the writes of
  the conversation tables of `project/instrukt/spec/feature/work/state-database`.
- **Does not own:** company homes, members, secrets and the `connectors:` map's
  validation, owned by configuration; starting and waking agent sessions, the
  runtime adapter, turn-end relay and assignment resolution, owned by runner;
  which MCP declarations a caller sees and the login a call carries, owned by
  runner (`project/instrukt/spec/feature/runner/mcp-access`); the state
  database's file, migrations and writer rules, owned by work.
- **Published contracts:**
  - The connector interface: every connector implements it, and the
    conversation core reaches a channel only through it. What it carries is the
    connector contract of `project/instrukt/spec/feature/company/conversations`.
  - Each connector kind's declaration: the fields its `connectors.<kind>` entry
    takes, published as data, against which configuration validates a company
    home (`project/instrukt/spec/domain/configuration`).
  - The hosting the runner declares and this context implements: opening and
    supervising the connections, the relay pass over finished turns, whether
    retained work is outstanding, the relay's state, and the conversations
    section of `instrukt events status`.
  - The conversation assignment lookup, which the runner's assignment
    resolution takes beside work's own.
  - The speaker lookup: a function of this context that the CLI calls for
    `instrukt mcp` and hands to MCP access.
  Other contexts obtain conversation facts only through these; none reads the
  conversation tables.
- **Allowed dependencies:** runner supplies the runtime adapter, the native
  conversation source, keyed start, the identity envelopes and the state
  database handle the conversation core writes through; configuration supplies
  company homes with their settings, members and resolved secrets, and the
  declared connectors; work supplies the state database's file, writer lock and
  migrations. These relationships are specified in the context map position
  below.
- **Implementation roots:** `instrukt-company` (the conversation core and the
  connector host), `instrukt-connector` (the connector interface and the
  declaration data), `instrukt-connector-slack` (the Slack connector). Their
  place among the crates is
  `project/instrukt/design/architecture/overview`.

## Event subjects and origins

The company context emits no events.

## Use cases

- **Conversations** — [project/instrukt/spec/feature/company/conversations](../feature/company/conversations.md)
  — ask a company's agent where the team talks, be answered there by the one session that knows the conversation.
- **Slack connector** — [project/instrukt/spec/feature/company/slack-connector](../feature/company/slack-connector.md)
  — connect a company's Slack workspace without a public address; only a tag reaches the agent.
- **MCP access** — [project/instrukt/spec/feature/runner/mcp-access](../feature/runner/mcp-access.md)
  — a call carries the login of the person being answered; this context supplies who that person is.

## Governing designs and decisions

The company context has no governing design or decision record of its own.

## Context map position

- **Runner** — customer / supplier, both ways: runner supplies the runtime
  adapter, the conversation source and turn relay, keyed start, identity
  envelopes and the state database handle; this context supplies the hosting
  and the conversation assignment lookup that runner declares, the speaker
  that MCP access uses, and the direct messages to one person that MCP
  sign-in sends through a connector.
- **Configuration** — customer / supplier: configuration supplies company
  homes, their members, resolved secrets and declared connectors. Published
  language: this context's connector declarations, which configuration
  validates against.
- **Work** — customer / supplier: work supplies the state database's file,
  writer lock and migrations.
- **Channel providers** — anti-corruption layer: each connector translates its
  provider's messages into conversation turns and answers back, at the
  connector interface.

## Known caveats

- Slack is the one connector. Other channels and help desks stay in TeleClaude.
