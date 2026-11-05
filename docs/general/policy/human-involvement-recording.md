---
description: 'Audit-only record of whether a human was in the loop: which durable facts carry it, how the caller declares it, and why it never refuses the act it annotates.'
visibility: 'public'
---

# Human Involvement Recording — Policy

## Rules

### The three tests

Every commit through `instrukt repo commit` writes `Human-Involved: true` or `false`. The three tests decide what a reader may infer from that record, never whether the trailer is written.

- **Commitment** — the fact binds the human or the business to something beyond the process: money or legal state, speech carried in the human's name, or knowledge that downstream agents act on without verifying.
- **Discretion** — the fact turned on a fork the human owns. A fact a specialist produces by following the process, identically every time, carries no discretion and records nothing.
- **Opacity** — the human's involvement is not already legible from the fact itself. A record whose entire content _is_ the human's decision states its own provenance and records nothing further.

The tests are conjunctive by design. Each one alone admits a wide class of facts; together they admit few.

### The record is declared, never adjudicated

- The caller declares involvement from the conversation it is in, with no recency window. It is audit-only and authorizes nothing.
- Involvement is **declared by the caller** of the fact-producing act, once, at the moment the fact is created. On a commit the caller declares it with `instrukt repo commit --human-involved true|false`, which writes the `Human-Involved:` trailer.
- A human approval is a record whose entire content is the human's decision. An approval given on GitHub is **attributed to the GitHub identity GitHub reports for that action**. An approval stated in a session is recorded by that session's agent and attributed to the session (runtime and id), never to a guessed identity. It carries no involvement declaration.
- Involvement **reports the human's presence at the fact, not the agent's use of their direction.** A human's direction stays live in an agent's context for the rest of the session, so an agent's sense of being shaped by the human never decays and cannot bound the declaration.
- Involvement is **never inherited or propagated**. A record on a session, todo, artifact, commit, parent session, or earlier operation supplies no value for any later fact.
- Involvement is **never inferred from identity**. The author of a commit, the account that opened a todo, or the git user's own identity names the authority the work ran under and never records involvement (`general/concept/lineage-vs-authorship`).
- The record is **orthogonal to authorization**. It records whether a human was present at the fact; it is never consulted to permit an operation. It widens no brief, suspends no ownership, and unfreezes no contract — authorization reads the acting agent and the object alone, and every caller is an agent (`general/principle/trust-and-guardrails`).
- The record **never refuses the act it annotates**. A malformed invocation — a declaration that is neither `true` nor `false` — is refused as argument validation, which judges the call and never the human.

### The record fails toward audit

Absence records `false`. The fact is then filed as agent-initiated and surfaced for audit; no default hides an autonomous act. Forgetting is therefore never the profitable move.

The record's integrity rests on **detection, not prevention**. The record makes every claim about the human falsifiable against the session's own transcript and traceable to the session that produced it.

### Granularity

Involvement is stamped **on the fact, once, and is immutable thereafter** — alongside the session that produced it. It is not stamped on operations. Almost every operation is determined by the process, so an operation-level flag would read `false` nearly everywhere and `true` wherever an agent chose, and a flag that is cheap to add is a flag that means nothing.

### Facts that qualify

Speech carried to third parties in the human's name. Records that commit money or legal standing. Knowledge published into the layer other agents read as contract.

## Rationale

Attribution matters exactly where a future reader forms a belief about the human. Nobody reads a set of review findings and concludes something about the human who owns the project; everyone reads an invoice or a published message and concludes the human stood behind it. The tests select for that reading.

Commitment alone over-selects: outward-facing work performed inside a brief — a support reply, an escalation — is expected, bounded, and speaks for a role rather than for the human. Discretion is what excludes it, and discretion is also what excludes the whole machinery of the process, where the human holds no intent and trusts the specialist. Opacity excludes the records that already say what they are, whose provenance is their own content.

A record that authorizes nothing must not veto anything. Holding an act hostage to an annotation gives the annotation the force the rest of this policy denies it, and the refusal lands on the party least able to satisfy it: the agent cannot make the human return. Recording `false` and proceeding is the same outcome omission produces, which is what the fail-toward-audit rule already requires.

Rarity is the record's value. It exists so the human can ask one question — which commitments were made in my name while I was not there — and receive a short, honest answer. Every fact that carries the record without earning it lengthens that answer and cheapens the attention it was built to direct.

## Scope

All agents, all projects. Every durable fact the platform creates or mutates on a human's behalf.

## Enforcement

- A fact that passes the three tests and carries no involvement record is a defect in the surface that creates it.
- For commits, the trailer is always retained; the three tests bound reader inference.
- A surface that refuses, aborts, or degrades a fact-producing act because of its involvement declaration is a defect. Only a malformed invocation is refused.
- Deriving involvement from a commit's author, an account, or any other identity on a record is a defect.

## Exceptions

- None.
