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

<!-- planned:company-home-integration -->
It also records each knowledge proposal: a commit a conversation session
makes in its company's repository, with the speaker of the turn it was made
in. From those records it answers whether every commit of a head was asked
for by an administrator of the company, so an administrator's knowledge
change is carried to review and integration with no person moving it, while
anyone else's waits for a person.
<!-- /planned:company-home-integration -->

## Canonical fields

### Aggregates

#### Conversation binding

- **Root entity:** One conversation's or company-scoped connector's binding to
  its agent session.
- **Contains:** Company, connector name and conversation binding key (the
  connector's conversation identity under `session_scope: conversation`, or the
  shared company binding under `session_scope: company`); the connector's
  assigned role and session scope; the session's runtime identity; accepted
  messages awaiting delivery with their senders; the speaker of the current
  turn; answers awaiting posting.
- **Invariants:** Under `session_scope: conversation`, one conversation has one
  session and one session serves one conversation; under
  `session_scope: company`, one shared standing session serves all admitted
  conversations of `(company, connector)` serially, tagging each incoming turn
  with its conversation identity, place context, thread and speaker. Each
  addressed message is one turn. A message activates no subscribed procedure
  and emits no event. Under `reply_mode: turn_output`, turn output is posted only as
  the answer to an addressed message, and an explicit reply queued through
  `instrukt company reply` is posted from any turn; under
  `reply_mode: explicit`, raw turn output is suppressed and only explicit
  replies are posted. No reply starts a conversation.
- **Identifier:** Company, connector name and conversation identity (or shared
  company binding).
- **Lifecycle:** The first addressed message from an admitted sender starts the
  session by key; later ones wake it. The binding outlives a settled thread and
  ends with nothing instrukt does. Its contract is
  `project/instrukt/spec/feature/company/conversations`.

#### Connector

- **Root entity:** One company's named connector declaration of a registered
  kind.
- **Contains:** Its declared name, its registered kind (defaulting to the name
  when omitted), its role (default `company-agent`), its admission policy
  (`members` or `admins_only`), its session scope (`conversation` or `company`),
  its reply mode (`turn_output` or `explicit`), its kind-specific settings, and
  its connection state.
- **Invariants:** A connector carries messages for the one company home that
  declares it; one company home may declare multiple named connectors of the
  same kind with isolated identities and provider rate limits. What it supplies
  for each addressed message, and what it does with an answer, is the connector
  contract of `project/instrukt/spec/feature/company/conversations`; which
  messages count as addressed, who counts as the company's own and how an answer
  is rendered are its kind's own contract.
- **Identifier:** Company and declared connector name.
- **Lifecycle:** Declared in the company home under `connectors.<name>`; hosted
  while `instrukt run` runs; ends when the declaration is removed.

<!-- planned:company-home-integration -->
#### Knowledge proposal

- **Root entity:** One commit a company's conversation session made in that
  company's repository.
- **Contains:** The commit; the branch it was made on; the connector, the
  conversation and the session; the speaker of the turn it was made in, by
  the provider identity and the name the connector supplied; whether the
  connector recorded that speaker as an administrator of the company with the
  addressed message that opened the turn; and when it was recorded.
- **Invariants:** A proposal is recorded only by instrukt, while the commit is
  made, from the speaker of the session's turn in progress, through the same
  speaker lookup every speaker decision of this context uses (the
  administrator check of a company agent's own MCP configuration among them).
  No commit message, procedure text or anything the agent writes makes a
  commit a proposal or makes its speaker an administrator. A commit made
  outside a turn with a speaker, made where instrukt's hook section did not
  run, or whose speaker cannot be established is no proposal. A head is an
  admitted proposal only when every commit it adds to the company's canonical
  branch is a proposal whose speaker was an administrator: one commit that is
  not leaves the whole head waiting for a person.
- **Identifier:** Company and commit.
- **Lifecycle:** Recorded when a company's conversation session commits in its
  company's repository, by the repository's `post-commit` hook section through
  this context's recording operation
  (`project/instrukt/spec/domain/repository`). Retired at a later recording in
  that repository once its commit is contained in the canonical branch or no
  ref of the repository holds it.
<!-- /planned:company-home-integration -->

## Ubiquitous language

**Connector**
: One company home's named channel connection, declared under
  `connectors.<name>`. It uses a registered connector kind to carry addressed
  messages in and answers out, and implements the connector interface this
  context defines.

**Connector kind**
: A registered channel provider (such as `slack`), selected by a connector's
  `kind` field or by its map key under `connectors:` when `kind` is omitted.
  Each kind declares the provider settings its entry takes.

**Conversation**
: A person talking to a company's agent in one place of a connected channel,
  bound to exactly one agent session in the company's home.

**Addressed message**
: A message its connector's contract counts as meant for the agent; it starts
  one turn.

**Speaker**
: The company member whose addressed message started a conversation session's
  current turn.

<!-- planned:company-home-integration -->
**Knowledge proposal**
: A commit a company's conversation session made in its company's
  repository, recorded with the speaker of the turn it was made in.

**Admitted proposal**
: A head every commit of which, added to the company's canonical branch, is a
  knowledge proposal whose speaker the connector recorded as an administrator
  of the company. Only an admitted proposal is carried to review and
  integration with no person moving it.
<!-- /planned:company-home-integration -->

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
    home (`project/instrukt/spec/domain/configuration`), together with the kind's
    setup at its provider, which `instrukt config schema` shows
    (`project/instrukt/spec/feature/configuration/configuration-editing`).
  - The hosting the runner declares and this context implements: opening and
    supervising the connections, the relay pass over finished turns, whether
    retained work is outstanding, the relay's state, and the conversations
    section of `instrukt events status`.
  - The conversation assignment lookup, which the runner's assignment
    resolution takes beside work's own, returning the connector's recorded role
    and company home. It recognises a conversation's session from the
    runner's binding of the conversation's key, which the runner records with
    the session's fixed native thread before the session starts, so a
    conversation session is recognised from its first command.
  - The explicit reply queueing operation (`instrukt company reply`), which
    verifies an active company conversation binding, checks that the target
    conversation is admitted on that `(company, connector)`, and writes the
    reply exactly as the agent wrote it, in ordered pieces, into
    `conversation_answers` for connector delivery.
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

<!-- planned:company-home-integration -->
Knowledge proposals extend this boundary:

- **Owns:** which commits are knowledge proposals and their record; the rule
  that admits a proposal, administrators only, as the connector recorded the
  speaker; the answer, for a range of commits, whether every one is a
  proposal of an administrator; and which recorded proposals stand on a
  branch not yet integrated.
- **Does not own:** the hook section that runs at a commit and when it runs
  (repository); whether a head is reviewed, its verdict and the review
  evidence (runner and work); opening a proposal's pull request and handing an
  approved one to the integrator (runner, through work's GitHub boundary,
  `project/instrukt/spec/feature/runner/pull-request-review`); composing,
  rendering and promoting the candidate (workflow and the integrator).
- **Published contracts:**
  - The proposal recording operation, which the `post-commit` section invokes
    through an interface core declares and the CLI wires. It records nothing
    for a session bound to no conversation of the company whose repository it
    commits in, and refuses nothing: an unrecorded commit is no proposal.
  - The proposal lookups the runner takes, wired by the CLI: the recorded
    proposals of a company that stand on a branch, with that branch; and, for
    the commits a head adds to the canonical branch, whether every one is a
    proposal whose speaker was an administrator.
- **State:** the company's proposal record, one state file in the home's
  local state, `.instrukt/knowledge-proposals.yaml`, kept out of the
  repository by the home's ignore lines.

  | Nature | Owner | Loss cost | Medium and isolation | Scope and lifecycle |
  | --- | --- | --- | --- | --- |
  | Authoritative: who asked for each commit, learned only while the turn is in progress and never recoverable afterwards. | This context; written only by its recording operation. | Lost intent, failing closed: an administrator's pending proposal is no longer admitted and waits for a person, as a non-administrator's does. | A state file: it holds the proposals of branches not yet integrated, is read whole or by commit and is never queried across. Written whole beside the previous version and renamed over it, under `.instrukt/knowledge-proposals.lock`, by recordings that take turns; readers take no lock. No background writer and no wait of a caller but the commit being made shares that lock. | The company's home: it moves with the home, its container's exported state included. Bounded by the proposals not yet integrated; each recording first retires the entries its repository no longer needs. |
- **Allowed dependencies:** runner supplies the session's turn in progress, as
  for the speaker lookup; the repository's own git answers containment and
  refs.
<!-- /planned:company-home-integration -->
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
<!-- planned:company-home-integration -->
- **Repository** — customer / supplier: this context supplies the proposal
  recording operation that repository's `post-commit` hook section invokes.
- **Runner** (knowledge proposals) — customer / supplier: this context
  supplies the proposals standing on a branch and whether a head is an
  admitted proposal; the runner opens the proposal's pull request and hands
  an approved admitted proposal to the integrator.
<!-- /planned:company-home-integration -->
- **Channel providers** — anti-corruption layer: each connector translates its
  provider's messages into conversation turns and answers back, at the
  connector interface.

## Known caveats

- Slack is the one connector. Other channels and help desks are outside instrukt.
