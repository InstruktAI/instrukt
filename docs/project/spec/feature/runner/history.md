---
description: Per-computer searchable history extracted from T3 local state, a one-time TeleClaude mirror import and merged search across computers.
delivered_by: [instrukt-first-build]
---
# History — Spec

## What it is

instrukt makes past conversations searchable through a store on each computer.
An extract-and-load step reads T3's local state into instrukt's own history;
TeleClaude's existing mirrors are imported once. A search across computers
merges results from their local stores. History needs no central service or
Postgres deployment and does not replace the runtime's live conversations.

## Canonical fields

| Surface | Contract |
| --- | --- |
| T3 local state | The source of conversations for the extracting computer; instrukt loads its own searchable representation. |
| Local history | A computer's own extracted/imported store, independently searchable. |
| TeleClaude mirrors | A one-time legacy import into history, not a continuing runtime or mirror-reader dependency. |
| Merged search | Searches participating computers' local stores and combines results while retaining their computer/source identity. |

Extraction and search are distinct from the session adapter's keyed startup,
recent-conversation recovery and native current-thread discovery. No central
archive is a prerequisite for those operations. The work record still reflects
artifacts and stamps to GitHub, not conversational back-and-forth. Moving work
between computers requires no conversation-transfer mechanism.

## Use cases

### UC-HI1: Search conversations extracted on one computer

```gherkin
Given a developer has conversations in T3's local state on a computer
When instrukt extracts and loads that state
Then the conversations are searchable in that computer's instrukt history
And no central service is required
```

### UC-HI2: Keep the TeleClaude mirrors through one import

```gherkin
Given a developer has existing TeleClaude conversation mirrors
When the one-time history import completes
Then those conversations are searchable in instrukt's local history
And later extraction uses T3 local state rather than ongoing TeleClaude mirroring
And rerunning the completed import does not duplicate those conversations
```

### UC-HI3: Search history held by multiple computers

```gherkin
Given a developer's participating computers each hold local instrukt history
When they search across those computers
Then the results merge matches from those stores with their source computer identified
And no central database is introduced
```

## Known caveats

- This contract settles the source, per-computer storage, one-time import and
  merged search. It does not activate the held central-service design or its
  Postgres dependency.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/work/github-record](../work/github-record.md)
