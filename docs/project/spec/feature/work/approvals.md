---
description: Evidence-bound human or peer approval and intent checks bound to a version and observed conversation.
visibility: public
---
# Approvals — Spec

## What it is

The work record stores approval decisions and intent checks with their
attribution and target digests.

## Canonical fields

### Approval records

Work records approval stamps and their evidence. Configured approval checks
are specified in
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

### Intent check

`instrukt todo stamp <slug> --intent-checked [--conversation <runtime:id>]`
binds the current `input` digest to the checking actor and the supplied
conversation, if any. The command stores the supplied session reference
without reading the conversation. An omitted conversation stays absent.
Changing `input` changes the digest that a current check must identify.

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
observed source session; GitHub approvals arrive through the runner's
reflection pass with GitHub attribution.
Carry retains the earlier attribution and evidence under the current independent
reviewer's conclusion, recorded with `--carry-from <stamp-id>` as above.

## Use cases

### UC-AP1: Authorize a reviewed outcome

```gherkin
  Given a developer's todo is waiting at a human approval moment
  And the required review is complete
  When the developer approves the current outcome
  Then the record attributes that approval to its observed source
  And binds the approval to the evidence they approved
```

### UC-AP2: Inspect the evidence of an intent check

```gherkin
Given a todo has a current input version
When a caller records an intent check with an explicit originating conversation
Then the record identifies the checked digest, the checking actor and the supplied conversation
```

### UC-AP3: An earlier approval carries to a revised outcome only under a current review

```gherkin
  Given a developer approved an outcome that was later revised
  When an agent carries that approval to the revised outcome
  Then the carry is accepted only when a current review approves the revised outcome
  And the approval remains attributed to the developer who gave it
  But when no current review approves the revised outcome
  Then the carry is refused and names the missing review
```

### UC-AP4: A peer approves where the project allows it, with its reason on record

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

## See Also
- [project/instrukt/spec/domain/work](../../domain/work.md)
