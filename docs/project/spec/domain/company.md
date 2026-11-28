---
description: The company bounded context — a company's people talk to its agent in the channels they already use; connectors, the conversations they carry, who is answered, the answers posted back, and the speaker of each turn.
---

# Company — Spec

## What it is

<!-- planned-change:thin-live-runtime-boundary -->
The company context lets a company's people talk to the company's agent where
they already talk. It owns the connectors that carry addressed messages in and
answers out, the conversations those messages belong to, each bound to one agent
session in the company's home, the decision of who is answered, and the answers
posted back. It keeps that state in the conversation tables of the state
database. It does not own the company home and its settings (configuration), the
running of agent sessions and the relay of finished turns (runner), or the
state database's file and migrations (work).
<!-- change:thin-live-runtime-boundary -->
The company context lets a company's people talk to the company's agent where
they already talk and hosts each company in isolation on a host. It owns the
connectors that carry addressed messages in and explicit replies out, curated
working status (`instrukt company status`), the conversations those messages
belong to (each bound to one agent session in the company's home), the decision
of who is answered, fail-closed active-turn speaker resolution and live turn
reconciliation over the runtime adapter's `read_state`, and the two-tier
company hosting ladder (including company containers and their export/import).
It keeps its authoritative binding and delivery state in the conversation
tables of the state database, its agent-waited explicit reply outbox in
`state/company-replies.db`, and its ephemeral curated turn status under
`state/company-turn-status/`. It does not own the company home and its settings
(configuration), the runtime adapter boundary (runtime), keyed start
orchestration and `instrukt run` process hosting (runner), or the state
database's file and migrations (work).
<!-- /planned-change:thin-live-runtime-boundary -->

It also records each knowledge proposal: a commit a conversation session
makes in its company's repository, in a turn whose speaker the connector
recorded as an administrator of the company. From those records and the
repository it answers whether a head is an admitted proposal, so an
administrator's change to the company's knowledge is carried to review and
integration with no person moving it, while every other change waits for a
person.

## Canonical fields

### Aggregates

<!-- planned-change:thin-live-runtime-boundary -->
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
<!-- change:thin-live-runtime-boundary -->
#### Conversation binding

- **Root entity:** One conversation's or company-scoped connector's binding to
  its agent session.
- **Contains:** Company, connector name and conversation binding key (the
  connector's conversation identity under `session_scope: conversation`, or the
  shared company binding under `session_scope: company`); the connector's
  assigned role and session scope; the session's runtime identity; any pending
  one-time rollover flag; accepted messages awaiting delivery or completion
  with their senders and live turn correlation; explicit reply outbox records
  and connector posting receipts; and optional curated working status for the
  active addressed turn.
- **Invariants:**
  - Under `session_scope: conversation`, one conversation has one session and
    one session serves one conversation; under `session_scope: company`, one
    shared standing session serves all admitted conversations of
    `(company, connector)` serially, tagging each incoming turn with its
    conversation identity, place context, thread and speaker.
  - Explicit frontstage is the sole reply contract: raw model output and
    runtime transcripts are never scraped or posted; only explicit replies
    queued via `instrukt company reply` and curated working status set via
    `instrukt company status` reach members. No reply starts a conversation.
  - Both opening launches and follow-up sends persist their deterministic input
    command identity before runtime execution so every turn, Turn 1 included,
    correlates identically against live runtime observations without clearing
    active-turn correlation when an explicit reply is posted mid-turn.
  - Active-turn speaker resolution is fail-closed with three outcomes over
    `runtime`'s `read_state` (active turn identity plus neutral closed opening
    input) and a lock-free WAL snapshot of the state database
    (`project/instrukt/spec/feature/company/conversations`,
    `project/instrukt/spec/feature/runner/mcp-access`):
    1. *Verified active addressed turn:* present, active session whose live
       turn input identity matches an in-flight addressed message on this
       active binding returns that message's recorded member speaker, surviving
       restarts and remaining intact after explicit replies.
    2. *Fail-closed refusal:* any observation or store error, any non-present
       or non-running state, absent active turn or input identity, unavailable
       opening input, mismatched addressed turn, unknown input whose opening
       text carries a company member envelope, or any retired conversation
       generation (`runtime_generations`) refuses with a company-owned
       `SpeakerError` rather than falling back to a service login.
    3. *Confirmed direct operator / technical-owner turn:* confirmed running
       turn on the active generation whose input identity matches no
       company-delivered message and whose available opening text carries no
       company member envelope yields no addressed member speaker (never
       inheriting any prior member's identity, admin privilege, or credentials,
       and refused by `instrukt company status`).
  - Live turn completion and failure reconcile against `runtime`'s bounded
    recent turn history using neutral input identity so an overtaken turn is
    not lost between polls, keeping pre-start preparing/queued turns in flight
    until their own run reaches terminal completion or failure. Every relay
    pass ingests the explicit reply outbox before evaluating turn completion,
    failing closed on outbox read errors while allowing already-ingested
    answers to post.
  - Bounded failed-turn retries and terminal completion without an explicit
    reply atomically settle the message with one standard member-facing error
    reply, retain the real failure cause on company's status surface, and
    raise one per-message operator anomaly through
    `project/instrukt/spec/feature/runner/anomaly-notification`
    (`project/instrukt/spec/feature/company/conversations`).
  - `instrukt company status` is admitted under the `company-reply` capability
    action (`project/instrukt/spec/feature/configuration/roles-and-capabilities`),
    requires Outcome 1 (refusing Outcome 2 and Outcome 3), defaults to and
    restricts to the bound conversation under `session_scope: conversation`,
    and under `session_scope: company` requires an admitted `--conversation`
    matching the currently active addressed turn's conversation.
  - Pre-upgrade bindings flagged for explicit-frontstage rollover roll over on
    demand when unanswered or newly arrived work is present, via runner's
    present-idle rollover contract, once no turn is actively running, adopting
    a fresh generation with the explicit-frontstage opening brief and leaving
    the prior thread as history (`project/instrukt/spec/feature/work/state-database`).
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
  its kind-specific settings, and its connection state.
- **Invariants:** A connector carries messages for the one company home that
  declares it; one company home may declare multiple named connectors of the
  same kind with isolated identities and provider rate limits. Explicit
  frontstage is unconditional across all connectors. What it supplies for each
  addressed message, and what it does with an answer or curated working status,
  is the connector contract of
  `project/instrukt/spec/feature/company/conversations`; which messages count
  as addressed, who counts as the company's own and how an answer is rendered
  are its kind's own contract.
- **Identifier:** Company and declared connector name.
- **Lifecycle:** Declared in the company home under `connectors.<name>`; hosted
  while `instrukt run` runs; ends when the declaration is removed.

#### Company host

- **Root entity:** One company's hosting model on a host under the two-tier
  hosting ladder (`project/instrukt/spec/feature/company/company-containers`).
- **Contains:**
  - *Tier 1 (shared host):* a company home folder on the operator's host with
    folder separation only under one OS user (no OS or runtime confinement
    between companies), per-company BYOS accounts via the selected runtime's
    optional multi-account provider isolation capability bound by `profiles`,
    and interaction strictly through business chat connectors.
  - *Tier 2 (isolated Linux container):* one long-running Linux container per
    company with hard OS and filesystem confinement, per-container endpoint
    authentication and egress isolation blocking host-bound and
    sibling-container runtime endpoints, self-contained host-folder state with
    consistent online export/import portability, and dual access (operator
    visibility plus optional direct steering and mobile access for a technical
    company owner under their own client account).
- **Invariants:** Company hosting is owned by the company context and stays
  independent of any specific runtime's internal storage format; Tier 2
  preserves irreplaceable logins, conversations, and company knowledge in the
  company's host folder across container replacement.
- **Identifier:** Company name on the host.
- **Lifecycle:** Starts at Tier 1 when a company home is added; advances to
  Tier 2 isolation and portable single-company export/import under
  `project/instrukt/spec/feature/company/company-containers`.
<!-- /planned-change:thin-live-runtime-boundary -->

#### Knowledge proposal

- **Root entity:** One commit a company's conversation session made in that
  company's repository, in a turn whose speaker the connector recorded as an
  administrator of the company, or whose speaker could not be established.
- **Contains:** The commit; the branch it was made on; the connector, the
  conversation and the session; the speaker, by the provider identity and the
  name the connector supplied, and whether the turn was in a direct message;
  when it was recorded. An unestablished entry holds the cause in place of the
  speaker.
- **Invariants:**
  - A proposal is recorded only by this context's recording, once the commit
    is made, from the speaker of the session's turn in progress, through the
    speaker lookup every speaker decision of this context uses. No commit
    message, procedure text or anything the agent writes makes a commit a
    proposal.
  - An administrator is admitted in a direct message and in a channel alike.
  - A commit in a turn whose speaker is no administrator, or in a session
    bound to no conversation of the company, is not recorded and is no
    proposal; the proposal store is not opened for it.
  - When the speaker lookup answers with an error because the runtime or the
    state database does not answer, it is asked again for up to five seconds;
    an error that remains records the commit as unestablished, with the error
    as its cause. An unestablished entry admits nothing.
  - When the proposal store cannot be opened or written, the recording writes
    its cause to the commit's standard error and records nothing; the commit
    is unrecorded and waits for a person. A failure of this local store is a
    defect of the part, not an outage.
  - A head is an admitted proposal only when every commit it adds to the
    company's canonical branch is a proposal, and the paths it changes are
    only the company's knowledge as its owners define it (the company's
    documents and its index,
    `project/instrukt/spec/feature/configuration/company-homes`, The home;
    a company document as
    `project/instrukt/spec/feature/publication/agent-entrypoints` defines one)
    and the root `AGENTS.md` rendered from them. One commit or one path
    outside that leaves the whole head waiting for a person.
- **Identifier:** Company and commit.
- **Lifecycle:** Recorded by this context's recording operation, which the
  repository's `post-commit` hook run invokes
  (`project/instrukt/spec/domain/repository`). Retired by a later recording
  once its commit is contained in the remote-tracking canonical branch, or no
  local branch, remote-tracking branch or worktree's `HEAD` holds it. A commit
  not yet pushed therefore stays current for as long as a branch or worktree
  carries it.

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

<!-- planned:thin-live-runtime-boundary -->
**Curated turn status**
: An optional short, agent-authored progress line set via
  `instrukt company status` for the currently active addressed turn, replacing
  the default working indicator until overwritten or the turn ends.

**Company host**
: One company's hosting model under the two-tier hosting ladder: Tier 1
  shared-host BYOS with folder separation only, or Tier 2 isolated Linux
  container (`project/instrukt/spec/feature/company/company-containers`).
<!-- /planned:thin-live-runtime-boundary -->

**Knowledge proposal**
: A commit a company's conversation session made in its company's
  repository, in a turn whose speaker the connector recorded as an
  administrator of the company.

**Admitted proposal**
: A head every commit of which, added to the company's canonical branch, is a
  knowledge proposal, and whose changed paths are only the company's
  knowledge as its owners define it and the root `AGENTS.md` rendered from
  it. Only an admitted proposal is carried to review and integration with no
  person moving it.

## Ownership boundary

<!-- planned-change:thin-live-runtime-boundary -->
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
<!-- change:thin-live-runtime-boundary -->
- **Owns:** connectors and their hosting; the connector interface; conversation
  bindings, per-message turn delivery and live turn-completion reconciliation
  over `runtime`'s `read_state`, who is answered, the fixed reply to someone
  who is not, bounded failed-turn retry exhaustion, explicit reply intake
  (`instrukt company reply`) and connector posting, curated working status
  (`instrukt company status`), fail-closed active-turn speaker resolution and
  its `SpeakerError` type, two-tier company hosting and containers
  (`project/instrukt/spec/feature/company/company-containers`), the meaning and
  writes of the conversation tables of
  `project/instrukt/spec/feature/work/state-database`, the explicit reply
  outbox (`state/company-replies.db`), and curated turn status files
  (`state/company-turn-status/`).
- **Does not own:** company homes, members, secrets and the `connectors:` map's
  validation, owned by configuration; the vendor-neutral runtime adapter
  boundary and concrete runtime adapters, owned by runtime
  (`project/instrukt/spec/domain/runtime`); keyed start and present-idle
  rollover orchestration, `instrukt run` process hosting, and assignment
  resolution, owned by runner; which MCP declarations a caller sees and the
  login a call carries, owned by runner
  (`project/instrukt/spec/feature/runner/mcp-access`); the state database's
  file, migrations and writer rules, owned by work.
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
    supervising the connections, running the company relay pass over connector
    intake, explicit reply outbox ingestion, and live `read_state` polling of
    in-flight addressed turns, reporting whether retained work (including
    un-ingested explicit replies) is outstanding, the relay's state, and the
    conversations section of `instrukt events status`.
  - The conversation assignment lookup, which the runner's assignment
    resolution takes beside work's own, returning the connector's recorded role
    and company home. It recognises a conversation's session from the
    runner's binding of the conversation's key, which the runner records with
    the session's fixed native thread before the session starts, so a
    conversation session is recognised from its first command.
  - The explicit reply queueing operation (`instrukt company reply`), which
    verifies an active company conversation binding via lock-free WAL read,
    checks that the target conversation is admitted on that
    `(company, connector)`, and writes ordered reply pieces with active-turn
    correlation into `state/company-replies.db` under
    `state/company-replies.lock` without acquiring `state/writer.lock`.
  - The curated turn status operation (`instrukt company status`), admitted
    under `company-reply`, which verifies an active addressed turn (Outcome 1)
    and target conversation binding (under `session_scope: company`, matching
    the active addressed turn's conversation), and atomically writes the
    validated progress line to `state/company-turn-status/<binding-digest>.json`
    without acquiring any database or writer lock.
  - The fail-closed speaker lookup: a function of this context returning a
    company-owned `SpeakerError` (mapped by each consumer seam, such as MCP
    access), called by the CLI for `instrukt mcp`, admin-DM `instrukt config`,
    `post-commit` knowledge proposals, and `instrukt company status`, reading
    `state/instrukt.db` lock-free via WAL snapshot and correlating against
    `runtime`'s live `read_state`.
  - Two-tier company hosting and container contracts
    (`project/instrukt/spec/feature/company/company-containers`).
  Other contexts obtain conversation facts only through these; none reads the
  conversation tables or reply/status stores directly.
- **Allowed dependencies:** runtime supplies the vendor-neutral
  `RuntimeAdapter` trait (live `read_state` with structured active/recent turn
  correlation and neutral opening-input data, plus idempotent start and send
  primitives) through `instrukt-runtime`, without importing any concrete
  adapter crate; runner supplies keyed start and present-idle rollover
  orchestration, identity envelopes, and the state database handle the
  conversation core writes through; configuration supplies company homes with
  their settings, members and resolved secrets, and the declared connectors;
  work supplies the state database's file, writer lock and migrations. These
  relationships are specified in the context map position below.
- **Implementation roots:** `instrukt-company` (the conversation core, explicit
  reply outbox, curated status, and connector host), `instrukt-connector` (the
  connector interface and the declaration data), `instrukt-connector-slack`
  (the Slack connector). Their place among the crates is
  `project/instrukt/design/architecture/overview`.
<!-- /planned-change:thin-live-runtime-boundary -->

Knowledge proposals extend this boundary:

- **Owns:** which commits are knowledge proposals, and the unestablished
  entries; the admission rule; the recording, with its retry bound, its
  failure behaviour and its retirement; the proposal store with its bound and
  its condition.
- **Does not own:** what the company's knowledge is (configuration and
  publication, as cited in the invariants); the hook run at a commit
  (repository); reporting to the operator, opening a proposal's pull request
  and handing an approved one to the integrator
  (`project/instrukt/spec/domain/runner`); the review and its evidence
  (runner and work); the candidate (workflow and its integrator).
- **Published contracts:**
  - The proposal recording operation, which the `post-commit` hook run
    invokes through an interface core declares and the CLI wires. It never
    refuses or fails the commit. A recording that records first computes,
    outside any transaction, which standing entries the retirement rule
    retires, then deletes them and inserts its entry in one short
    transaction.
  - The proposal lookups, declared by runner, implemented by this context and
    wired by the CLI, as the conversation assignment lookup is:
    - whether a head is an admitted proposal;
    - the unestablished entries whose commit a remote-tracking branch holds,
      with their causes;
    - the store's condition: usable, unreadable, of an older or unknown
      shape, or unwritable.
    They read a snapshot of the store and never migrate or write it.
<!-- planned-change:thin-live-runtime-boundary -->
- **State:**

  | Store | Nature | Owner | Loss cost | Medium and write isolation | Scope, bound and retirement |
  | --- | --- | --- | --- | --- | --- |
  | `.instrukt/knowledge-proposals.db` in the company's home, kept out of the repository by the home's ignore lines | Authoritative: who asked for each recorded commit, learned only while the turn is in progress. | This context; written only by its recording operation. | Lost intent, failing closed: an administrator's pending proposal is no longer admitted and waits for a person. | A database of its own: several recording processes write it at once, one per committing conversation session, and it is read by commit. Its file is its write-isolation unit. Its only writers are recordings a commit waits on, each holding one short transaction; no background writer shares it. | The company's home, moving with it, its container's exported state included. Bounded by the recorded commits a local branch, remote-tracking branch or worktree's `HEAD` still holds and the remote-tracking canonical branch does not contain, retired under the Lifecycle rule above. A store of an older shape is migrated by the next recording, never discarded; an unreadable one is left as it is, since nothing rebuilds it. |

- **Allowed dependencies:** runner supplies the session's turn in progress, as
  for the speaker lookup; the repository's own git answers refs and
  containment.
<!-- change:thin-live-runtime-boundary -->
- **State:**

  | Store | Nature | Owner | Loss cost | Medium and write isolation | Scope, bound and retirement |
  | --- | --- | --- | --- | --- | --- |
  | `state/company-replies.db` with `state/company-replies.lock` | Authoritative explicit reply outbox queued by `instrukt company reply` before relay intake (`project/instrukt/spec/feature/work/state-database`). | This context; written exclusively by synchronous agent-waited `instrukt company reply` commands. | Lost explicit reply pieces not yet copied into the state database's answer table. | Dedicated SQLite WAL store with its own advisory lock, isolated from `state/writer.lock`. Background company relay reads lock-free via WAL snapshot and copies rows idempotently into the state database; synchronous `instrukt company reply` never acquires `state/writer.lock`. | Per home (`INSTRUKT_HOME`), covered by home backup (`project/instrukt/spec/feature/runner/home-backup`) and carried by single-company export/import via consistent filtered backup or pre-snapshot drain (`project/instrukt/spec/feature/company/company-containers`). Opportunistically pruned of aged confirmed-posted rows by `instrukt company reply` alone so no background writer shares the lock (`project/instrukt/spec/feature/work/state-database`). |
  | `state/company-turn-status/<binding-digest>.json` | Authoritative latest curated working status for one active addressed turn (`project/instrukt/spec/feature/work/state-database`). | This context; single writer is `instrukt company status`. | Low: working indicator degrades to the default calm line. | Bounded single-record atomic JSON file written whole with symlink refusal and zero database or writer lock. Background company relay reads lock-free, never mutates or unlinks the file, ignores entries whose session or turn input identity does not match the active turn, and warns-and-ignores malformed files. | Per home (`INSTRUKT_HOME`), excluded from home backup (`project/instrukt/spec/feature/runner/home-backup`). Bounded to at most one small file per conversation binding, overwritten in place by later status updates. |
  | Conversation tables in `state/instrukt.db` and connector read-back store (`state/connector-read-back.sqlite`) | Authoritative conversation bindings, addressed-message speaker and live turn correlation, connector posting receipts, and read-back windows (`project/instrukt/spec/feature/work/state-database`). | This context (meaning and writes); work owns `state/instrukt.db`'s file, `state/writer.lock`, and migrations. | Lost bindings, in-flight turn tracking, and posting receipts; must be preserved. | Shared SQLite WAL state database written by the background company relay under `state/writer.lock` (with read-back state in its own SQLite store). Every synchronous agent-waited caller (`instrukt mcp`, admin-DM `instrukt config`, `post-commit` proposals, `instrukt company reply`, `instrukt company status`) reads lock-free via WAL snapshot and never acquires `state/writer.lock`. | Per home (`INSTRUKT_HOME`), covered by home backup and single-company export/import (`project/instrukt/spec/feature/company/company-containers`). |
  | `.instrukt/knowledge-proposals.db` in the company's home, kept out of the repository by the home's ignore lines | Authoritative: who asked for each recorded commit, learned only while the turn is in progress. | This context; written only by its recording operation. | Lost intent, failing closed: an administrator's pending proposal is no longer admitted and waits for a person. | A database of its own: several recording processes write it at once, one per committing conversation session, and it is read by commit. Its file is its write-isolation unit. Its only writers are recordings a commit waits on, each holding one short transaction; no background writer shares it. | The company's home, moving with it, its container's exported state included. Bounded by the recorded commits a local branch, remote-tracking branch or worktree's `HEAD` still holds and the remote-tracking canonical branch does not contain, retired under the Lifecycle rule above. A store of an older shape is migrated by the next recording, never discarded; an unreadable one is left as it is, since nothing rebuilds it. |

- **Allowed dependencies:** runtime supplies live `read_state` with structured
  active/recent turn correlation and neutral closed opening-input data for
  speaker lookup; the repository's own git answers refs and containment.
<!-- /planned-change:thin-live-runtime-boundary -->

## Event subjects and origins

The company context emits no events.

## Use cases

- **Conversations** — [project/instrukt/spec/feature/company/conversations](../feature/company/conversations.md)
  — ask a company's agent where the team talks, be answered there by the one session that knows the conversation.
- **Slack connector** — [project/instrukt/spec/feature/company/slack-connector](../feature/company/slack-connector.md)
  — connect a company's Slack workspace without a public address; only a tag reaches the agent.
- **MCP access** — [project/instrukt/spec/feature/runner/mcp-access](../feature/runner/mcp-access.md)
  — a call carries the login of the person being answered; this context supplies who that person is.
- **Company containers** — [project/instrukt/spec/feature/company/company-containers](../feature/company/company-containers.md)
  — run each company apart from the others and from the host, keep its logins and conversations through a replaced container, update its tools without a new image and export and import it.

## Governing designs and decisions

The company context has no governing design or decision record of its own.

## Context map position

<!-- planned-change:thin-live-runtime-boundary -->
- **Runner** — customer / supplier, both ways: runner supplies the runtime
  adapter, the conversation source and turn relay, keyed start, identity
  envelopes and the state database handle; this context supplies the hosting
  and the conversation assignment lookup that runner declares, the speaker
  that MCP access uses, and the direct messages to one person that MCP
  sign-in sends through a connector.
<!-- change:thin-live-runtime-boundary -->
- **Runtime** — customer / supplier: runtime supplies live `read_state` with
  structured active/recent turn correlation and neutral closed opening-input
  data for fail-closed active-speaker resolution and live turn tracking, plus
  idempotent start and send primitives (`project/instrukt/spec/domain/runtime`).
- **Runner** — customer / supplier, both ways: runner supplies keyed start and
  present-idle rollover orchestration, identity envelopes and the state
  database handle; this context supplies the hosting and the conversation
  assignment lookup that runner declares, the fail-closed speaker lookup that
  MCP access uses, and the direct messages to one person that MCP sign-in sends
  through a connector.
<!-- /planned-change:thin-live-runtime-boundary -->
- **Configuration** — customer / supplier: configuration supplies company
  homes, their members, resolved secrets and declared connectors. Published
  language: this context's connector declarations, which configuration
  validates against.
- **Work** — customer / supplier: work supplies the state database's file,
  writer lock and migrations.
- **Repository** — customer / supplier: this context supplies the proposal
  recording operation the `post-commit` hook run invokes.
- **Runner** (knowledge proposals) — customer / supplier: runner declares the
  proposal lookups and this context implements them; what reaches the
  operator and what is carried to review and integration are the runner's.
- **Channel providers** — anti-corruption layer: each connector translates its
  provider's messages into conversation turns and answers back, at the
  connector interface.

## Known caveats

- Slack is the one connector. Other channels and help desks are outside instrukt.
