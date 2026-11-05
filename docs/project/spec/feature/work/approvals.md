---
description: Evidence-bound human or peer approval and a position-based check of recorded intent.
visibility: public
delivered_by: [instrukt-first-build]
---
# Approvals — Spec

## What it is

A developer retains control of approval moments while allowing a trusted peer
to judge eligible work on their behalf. Recorded intent is independently
checked at its first handoff and then travels unchanged until the intent
itself changes.

## Canonical fields

### Approval records

Work records approval stamps and their evidence; gate requirements, escalation,
task-authorization checks and preparation routing belong to
`project/instrukt/spec/feature/workflow/preparation`.

| Approval source | Attribution retained |
| --- | --- |
| Human action on GitHub | The identity GitHub reports for that action and the approved evidence. |
| Human statement in a session | The session (runtime and id), recorded by that session’s agent, and the approved evidence; no guessed GitHub identity. |
| Peer approval | The peer session, reasoning and approved evidence, explicitly marked as peer approval. |

Approval binds the current artifact; changing it leaves the old approval stale.
The evidence and provenance envelope belongs to
`project/instrukt/spec/feature/work/stamps`.

Every recorded approval is eligible for carry only on a current independent
reviewer's conclusion, under
`project/instrukt/spec/feature/workflow/preparation`. Carry retains the
earlier source, identity and evidence and binds the current target. A carried
peer approval remains a peer approval. A carry is recorded with
`instrukt todo stamp <slug> --prd-approved --carry-from <stamp-id>` (or
`--task-approved`), accepted only when a current review verdict `approve` binds
the new digest; the new stamp records `carried_from`, so the original approver
stays attributed. Without that verdict the carry is refused, naming the missing
verdict.

### Intent check by position

The intent record of every kind is its `input` artifact: the outcome the
agent in the conversation recorded. It identifies its current version, author
and the conversation in which that version was authored. The first recipient who is not that
author reads that conversation and checks the record against it. They confirm
it, clarify its expression without changing its meaning, or return a mistaken
interpretation to the person for the missing intent. The check binds to the
`input` version it checked; a new `input` version invalidates it, and
rewriting a derived artifact (requirements, plans) does not.

The confirming recipient leaves an intent-checked stamp naming who checked,
which conversation and which intent version. Later recipients accept a current
stamp and carry that intent unchanged. A rewrite invalidates the prior check;
the next non-author checks the replacement version. The checker's position,
rather than a fixed workflow role, determines who performs the check. For work
committed directly from a conversation, commit review is the first recipient.

The check runs on the computer where the originating conversation is readable.
That computer’s runner dispatches the checker; GitHub carries no copy of the
conversation.

An intent check establishes fidelity of the record. An approval authorizes a
particular reviewed outcome. One never substitutes for the other.

If the originating conversation is permanently unavailable, the checker
returns the intent to the human. A new `input` version recorded from that
conversation becomes the checkable basis.

### Command boundary

Approvals and intent checks are recorded through `instrukt todo stamp` and
are visible through `instrukt todo show`. They are stamps of the work record:
rows of the state database's `stamps` table with `fact` `approval` or
`intent_checked`
(`project/instrukt/spec/feature/work/state-database`). Approval stamps distinguish human decisions from peer judgments and retain
the source attribution above even when an agent records the decision.

Approval commands are `instrukt todo stamp <slug> --prd-approved --by human|peer
[--session <runtime:id>]` and `instrukt todo stamp <slug> --task-approved
--by human|peer [--session <runtime:id>]`. A peer approval states its reason
with `--reason <text>`, the CLI's one justification flag; it requires the peer
session and is refused without either. Session statements retain the
observed source session; GitHub approvals arrive by pull with GitHub attribution.
Carry retains the earlier attribution and evidence under the current independent
reviewer's conclusion, recorded with `--carry-from <stamp-id>` as above.

### Use cases

#### UC-AP1: Authorize a reviewed outcome

```gherkin
  Given a developer's todo is waiting at a human approval moment
  And the required review is complete
  When the developer approves the current outcome
  Then the record attributes that approval to its observed source
  And binds the approval to the evidence they approved
```

#### UC-AP2: Hand off intent without repeating the request

```gherkin
  Given a developer's request has been recorded by its author
  When another agent first receives that version and checks it against the originating conversation
  Then the record identifies that version as independently checked
  And subsequent recipients can accept it without asking the developer to repeat it
```

#### UC-AP3: An earlier approval carries to a revised outcome only under a current review

```gherkin
  Given a developer approved an outcome that was later revised
  When an agent carries that approval to the revised outcome
  Then the carry is accepted only when a current review approves the revised outcome
  And the approval remains attributed to the developer who gave it
  But when no current review approves the revised outcome
  Then the carry is refused and names the missing review
```

#### UC-AP4: A peer approves where the project allows it, with its reason on record

```gherkin
  Given a developer's project lets a peer approve a gate
  When an agent in a peer session approves that gate and states its reason
  Then the approval is recorded as the peer's, with that session and reason
  But when the approval comes from no peer session or states no reason
  Then it is refused
  And when the project requires the developer for that gate
  Then a peer's approval does not satisfy it
```

## Known caveats

None.