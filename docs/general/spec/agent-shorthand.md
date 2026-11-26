---
description: 'Vocabulary and protocol for agent-to-agent exchanges when compression is enabled, and the fixed notation compressed units are written in. Compression of an exchange is opt-in per `communication-compression`.'
visibility: 'public'
---

# Agent Shorthand — Spec

## Required reads

- [general/principle/communication-compression](../principle/communication-compression.md)

## What it is

Defines the vocabulary and protocol for compressed agent-to-agent exchanges _when compression is in use_. Per `communication-compression`, compression is opt-in — invoked by explicit human authorization or in agent-only channels that genuinely have no human observer. The default register for every exchange is `L1` prose. This spec is the reference for what compression looks like once enabled; it does not prescribe when to enable it.

It also defines the compressed-unit notation: the one fixed, plain-ASCII form in which a compressed unit — the dense counterpart an entrypoint rendering holds in place of a prose source — is written.

## Canonical fields

### Registers

A register names the type of exchange. Registers do not auto-prescribe a compression level — they describe the context in which an opt-in decision is made.

| Register            | Meaning                                                 |
| ------------------- | ------------------------------------------------------- |
| `human-present`     | A human is participating or actively observing          |
| `human-aware`       | A human opted into structured compression               |
| `agent-only`        | Agent-only channel with no human observer               |
| `structured-report` | Worker-style reporting to an orchestrator or supervisor |

### Compression levels

| Level | Name                | Use                                                |
| ----- | ------------------- | -------------------------------------------------- |
| `L1`  | prose               | human-native explanation and all durable artifacts |
| `L2`  | labeled assertions  | human-scannable structured exchange                |
| `L3`  | symbolic compressed | recoverable shorthand with disambiguation anchors  |
| `L4`  | semantic tokens     | minimal same-model exchange with tight context     |

`L1` is the default for every register. `L2`, `L3`, and `L4` require explicit human authorization or a verified human-absent channel before they may be used.

### Phase markers

| Marker      | Meaning                                 |
| ----------- | --------------------------------------- |
| `[inhale]`  | diverging; add options, do not converge |
| `[hold]`    | tensions visible; sit with them         |
| `[exhale]`  | converging on decisions or synthesis    |
| `[✓exhale]` | final artifact-writing phase            |

Phase markers govern level switching within an already-enabled compressed exchange. They do not authorize compression on their own.

### Negotiation

When compression has been enabled for an exchange, the initiator sends a negotiation line stating the protocol in use. Examples:

- `PROTOCOL: L3 all phases, artifacts in prose`
- `PROTOCOL: phase-locked L4 inhale/hold, L3 exhale, artifacts in prose`

Compressed equivalent (when both agents already share the contract):

- `⊢proto:phased L4↔L3 artifacts:prose`

If the receiving agent does not recognize or follow the proposed protocol, it responds in prose and the initiator adapts. Negotiation is one exchange, not a thread.

### Fallback rules

- If a compressed exchange produces correction churn or reconstruction drift, drop one level. From `L4` go to `L3`; from `L3` go to `L1`.
- Cross-model peer exchange must not exceed `L3`. `L4` assumes tight shared context that does not survive model boundaries.
- If a human enters the exchange — or asks "what's happening here?" — fall back to `L1` immediately and stay expanded for the remainder of the exchange.

### Artifact format

Durable artifacts are always prose. Requirements, reports, plans, commit messages, and doc snippets must remain readable without access to the working exchange.

A compressed unit is no authored artifact of its own: it is a derived rendering form of one prose source, committed beside the documentation and written in the compressed-unit notation below. Its source stays prose and stays the authority; every rendering that holds a unit names its source, whose full prose one read returns.

### Compressed-unit notation

A unit is read by agents of every model family, and by the people who open what agents were given, without shared working context. Its notation is therefore fixed, explained by one legend in every rendering that holds a unit, and limited to these elements:

- **Plain ASCII.** No typographic quotes, dashes, arrows or other symbols outside ASCII.
- **One flat list.** Each line `- ` and one line per rule; a heading only where the source has parts a reader must tell apart, such as a procedure's ordered steps, which stay numbered.
- **Rule lines** open with one keyword: `MUST` (required), `NEVER` (forbidden), `MAY` (permitted, not required), `PREFER` (the default, which yields to a stated reason). The rule follows in telegraphic words.
- **The reason** follows the rule after ` -- `, in one clause naming the invariant the rule protects or the failure it prevents.
- **Fact lines** state a term the rules rest on: `<term> = <meaning>`.
- **Connectives.** `->` reads "leads to" or "then"; `|` reads "or"; `vs` names two forces held in tension.
- **Verbatim.** Snippet ids, commands, paths, code spans and `@config` directives stand exactly as in the source, in backticks where the source has them.

Writing and renewing a unit follows `general/procedure/compressed-unit-authoring`.

### Output validity

Default emit state is false in agent-to-agent exchanges. Output is valid only when it does one of the following:

- answers a direct question
- reports a blocker, contradiction, scope change, or decision point
- provides new information that changes the receiver's next action
- delivers a command result, artifact, or computed answer

The following are invalid output:

- acknowledgment-only replies such as `idle`, `done`, `ack`, `noted`, `got it`
- echoing what the other side just said
- narration that announces work without delivering findings

Linked-output wrappers do not make a message actionable. The wrapped payload is what determines whether the output is valid.

When no valid form applies, the turn still has to close. It closes with the no-output sentinel under `general/policy/no-output-turn`; writing the silence out in prose produces one of the invalid forms above.

## Allowed values

- Registers: `human-present`, `human-aware`, `agent-only`, `structured-report`
- Levels: `L1`, `L2`, `L3`, `L4`
- Phase markers: `[inhale]`, `[hold]`, `[exhale]`, `[✓exhale]`
- Compressed-unit keywords: `MUST`, `NEVER`, `MAY`, `PREFER`
- Compressed-unit connectives: ` -- `, `=`, `->`, `|`, `vs`

## Known caveats

- `L1` is the default for every register. Higher levels require explicit human authorization or a verified human-absent channel.
- `L4` assumes tight shared context and is unsafe across model families.
- Phase markers govern level switching only within an already-enabled compressed exchange.
- If compressed exchange produces correction churn or ambiguity, drop to a less compressed level immediately.
- A compressed unit keeps its source's rules, not its reasoning beyond one clause; a case the rules do not settle is read in the source's full prose.
