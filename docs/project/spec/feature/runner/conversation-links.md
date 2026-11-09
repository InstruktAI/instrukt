---
description: Three-way conversations across two linked T3 threads, with attributed turn-end relay in the long-lived runner and durable restart positions.
visibility: public
delivered_by: [instrukt-first-build]
---
# Conversation Links — Spec

## What it is

A conversation link joins two agent threads into one conversation in which a
person participates by typing in either thread. Each finished turn's human
words and agent reply, as T3 recorded them, reach the other thread with their
original speakers identified. The runner owns the link, its delivery positions
and its identity-envelope vocabulary. T3 owns the source conversation and the
target's native input boundary.

## Canonical fields

### Opening, participation and closure

Opening identifies two distinct runtime-scoped native threads and records one
active link between them. Subsequent completed turns are shared automatically;
participants continue by speaking in either thread. Positions start at link
creation, so earlier conversation is not replayed into the link.

Either member can close the link for everyone. Closure ends relay in both
directions and retains the link's delivery evidence. It neither settles a T3
thread nor changes a dispatch relationship. A closed link admits no further
handover; input already accepted by T3 remains T3's input.

Link admission uses recorded assignments and the effective role class under
`project/instrukt/spec/feature/configuration/roles-and-capabilities`:

- A focused role is not linked. The refusal names its own output and artifacts
  as the reporting path.
- A worker is not linked to the orchestrator that dispatched it. The refusal
  names native child-result delivery as its upward reporting path.
- A thread already linked to a peer refuses a third peer, identifies its active
  peer and preserves the existing conversation. Opening an already-linked pair
  creates neither a second link nor a duplicate opening contribution.

### Finished-turn relay

Turn completion triggers deterministic relay of that turn's recorded human
input and conversational agent output, in their source order. Incomplete
streaming output waits for its turn to finish. Tool calls, tool results,
thinking, control input and assignment/runtime instructions are excluded.
Nothing in this path depends on an agent invoking a relay tool or remembering
to forward a message.

A native human entry is segmented into complete identity/control envelopes and
ordinary spans using the vocabulary below. An inherited identity envelope keeps
its original source, author and occurrence time. Ordinary spans are the human
in the source thread; the source agent's reply is that agent's contribution.
An assistant's quotation of a delimiter remains assistant content. Classification
does not inspect an envelope's body or reinterpret its author from account
lineage (`general/concept/lineage-vs-authorship`).

A contribution is omitted when its original source is the destination thread;
it is never echoed back to its author. Other eligible spans retain their order.
An agent's bare `NOOP` contributes no peer text under
`general/policy/no-output-turn`; human input in the same completed turn remains
eligible. A turn with no eligible content still advances its delivery position.

### Identity-envelope vocabulary

The runner owns one registry of envelope kinds, openings and closure rules.
Writers and readers consume that registry. Recognition covers the whole native
entry, accepting only complete registered envelopes; every remaining span is
ordinary input from this thread's human. An unrecognized or incomplete shape
neither blocks neighboring content nor establishes platform provenance.

| Opening | Kind | Closure | Meaning |
| --- | --- | --- | --- |
| `[Peer (source-thread · author-token · utc-minute)]` | identity | paired, `[/Peer (source-thread · author-token · utc-minute)]` | A contribution authored in another thread. |
| `[Instrukt Assignment]` | control | paired, `[/Instrukt Assignment]` | Runner-authored procedure/assignment input. |
| `[Instrukt Notification]` | control | paired, `[/Instrukt Notification]` | Runner-authored event, approval or recovery delivery to the agent's loop. |

`source-thread` is the runtime-scoped native thread reference, including its
endpoint scope. `author-token` distinguishes human from agent and carries the
source's recorded display identity or available provider/model identity. A
missing display identity remains explicitly human or agent; it is never filled
from a person/company record or the account's authority. `utc-minute` is the
contribution's recorded occurrence time in UTC, not its later relay time. An
unavailable occurrence time stays explicitly unknown. The paired close derives
from the exact opening fields. Identity resolution is fixed for the contribution,
not recomputed from a thread's later configuration.

The `Instrukt` prefix belongs to control text; identity contributions use the
peer shape. Control text drives the receiving agent's loop and is not authored
conversation content. Every instrukt injection through the native user-input
channel has registered framing, including the loader and delivery log; their
existing structured metadata and untrusted event bodies remain intact inside
the control envelope. Identity-envelope bodies remain opaque, including any
header-looking text they quote. Reading discipline is
`general/policy/message-identity-headers`.

### Stream reader and durable progress

The relay is one more independent reader hosted by the existing long-lived
`instrukt run`. Its T3 adapter reads the runtime's durable conversation source: the same
read-only T3 projection-database reader, per-conversation fingerprints and
native database change notifications used by history. Each link is an
independent consumer with its own opaque source position; it never borrows
history's extraction position. This reader needs no T3 session token. Native target delivery uses the adapter's supported input operation;
read scope alone grants no send authority. Redis is transport only and owns
neither turns, link membership nor relay decisions. There is no separate relay
process, agent relay tool, scheduled tick relay or periodic fallback.

Runner-owned link and direction positions live in the existing state database
under `project/instrukt/spec/feature/work/state-database`. Each direction
retains source stream position, completed-turn delivery position, fixed target
and any pending handover. Stream observation and accepted peer delivery are
separate progress boundaries: reading a completion does not prove its content
reached the peer. Progress is source-native identity/sequence, independent of
rendered timestamps. A pending range and its source identities survive restart;
a directional delivery position advances only after native acceptance, or a
recorded empty-content disposition.

Ordinary runner startup catches up active links from retained positions
and resumes unfinished ranges. Native projection changes and completed-run/message evidence are interpreted through
the adapter without treating an observed high-water position as proof of
completed delivery. Missing source content, an unsupported stream shape or an
unresolved handover is a visible failure; no fabricated transcript or successful
position substitutes for evidence. T3 protocol discovery and supported-version
checks remain at the runtime boundary.

`instrukt events status` exposes active links, their directional progress,
pending/failed handovers and stream failures. Logs carry the link, source,
target and native turn/position references with the failure cause, without
conversation bodies or credentials. A blocked relay does not block unrelated
runner readers. Native change notifications and runner restart continue the same
recorded work; no second timer-driven delivery path is introduced.

## Use cases

### UC-CL1: Bring two agents into a shared discussion

```gherkin
Given a developer wants two peer agents to discuss a problem with them
When they link the agents' conversations
Then each agent receives the other agent's finished replies under that agent's identity
And contributions are not echoed back to their original speaker
```

Sharing the developer's clarification is indexed by UC-SS1 in
`project/instrukt/spec/feature/runner/session-continuity`. Admission refusals
above are extensions of the shared-discussion goal; they do not create a
conversation or change an existing one.

### UC-CL2: End the shared conversation

```gherkin
Given a developer is participating in a linked conversation with two agents
When any participant closes the link
Then further contributions stop crossing between the conversations for everyone
And the developer can continue separately in either conversation
```

## Known caveats

- Target acceptance is not target consumption. An interrupted handover with no
  native acceptance evidence remains visibly uncertain; this contract does not
  claim exactly-once delivery across an ambiguous external send.
- Link opening and closure are defined at the runner API; the user/runtime entrypoint for invoking them is not specified. `runtime.discuss` must report that entrypoint gap rather than invent a native T3 linking operation.
- Continuous relay requires `instrukt run`. A scheduled-only computer's ticks
  do not substitute for the stream reader.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/configuration/roles-and-capabilities](../configuration/roles-and-capabilities.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
- [general/policy/message-identity-headers](../../../../general/policy/message-identity-headers.md)
- [general/policy/no-output-turn](../../../../general/policy/no-output-turn.md)
- T3 source: the projection-database reader and source-shape checks in the runtime adapter; native sends use the existing T3 host CLI adapter.
