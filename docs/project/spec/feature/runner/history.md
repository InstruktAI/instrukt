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

### Operations and adapter boundary

History exposes explicit extraction, one-time selected-source TeleClaude import,
local or merged search, and retained-document lookup by native identity or archive
alias. The runtime adapter owns its optional durable conversation-source
capability. History consumes normalized conversation/message changes with native
ids, parent links, roles, available timestamps and completion evidence; it stores
the adapter's source position opaquely. A runtime without that capability reports
extraction as unsupported. This capability also admits other conversation
consumers; history does not implement conversation relay or runtime lifecycle.

The T3 adapter reads the durable projection database located by
`T3Home::database`, discovered from the machine user home through the shared T3
location capability. Instrukt's development publication-output home never selects
the source. It opens the database strictly read-only and WAL-aware, never writes
or migrates it, and does not authenticate to T3 or use its WebSocket API for
history. Retained older messages and archived threads are projection rows and
are included independently of event-log retention or a recent UI window.

The adapter processes short, bounded reads in each projection's timestamp/native-id
order. T3 projections with `updated_at` use that value; runs have no such column
and use their retained completion/request timestamps. These per-projection
positions remain internal to T3's opaque source position. Rows still streaming
are deferred with durable pending identities and revisited on the next catch-up,
even if other rows have advanced the position. The adapter yields only completed
message text, normalized native records and the next position. History commits
those changes, its FTS index and the opaque position atomically; a failed batch
advances no progress. Source reads never retain a read transaction across local
history writes.

### Freshness and participation

The existing long-lived `instrukt run` catches up at startup and on native
notifications of database or WAL changes. Explicit extraction and every history
search also catch up before reading the local index. Each remote participant
performs its own catch-up before answering. There is no tick-driven extraction,
periodic polling loop or scheduled reconciliation fallback.

Merged search reuses the existing configured cluster transport and explicitly
named instrukt computer identities. A remote answer requires a running,
event-enabled participant. The computer hosting a history store remains distinct
from original source-computer provenance. Redis transports bounded queries and
answers and holds no central durable archive.

Failed catch-up and unavailable participants remain explicit in the result.
Successful retained matches still return, with unsuccessful completion status
when any part is incomplete. Public errors provide safe operation diagnostics;
detailed internal errors remain in the responding computer's logs. Each committed
extraction batch emits a DEBUG operation summary identifying its source, loaded
count, committed position and duration, without conversation content.

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

- A conversation physically deleted from the runtime before any extraction cannot
  be recovered. Extracted history survives later source deletion. Archival is not
  deletion and does not exclude a retained source conversation.
- T3 owns event replay retention and snapshot recovery for its API consumers;
  history reads its durable projections and does not depend on that event log or
  the compatibility API's recent window.
- Reasoning is available only when a provider persisted it. The T3 database
  source retains stored message, reasoning, tool and diff content, including
  content that wire projections withhold. This states no guarantee that private
  model reasoning or material never stored by the runtime exists.
- The T3 source adapter validates every required table, column, type and consumed
  row shape. Migration levels and reported T3 releases are diagnostics, not
  equality gates. Unknown source shape fails visibly without committing its
  batch or position. Record bounds are owned by the source adapter; oversized
  records fail visibly rather than silently truncating history.
- The legacy importer supports the source revision owned by its importer and
  refuses other shapes without migrating the source. A completed selected-source
  import requires no continuing TeleClaude mirror or runtime dependency.
- Native notifications must be available on the source filesystem. Notification
  failure is explicit; no polling backend or new daemon is introduced. Searches
  remain an explicit catch-up boundary.
- This contract introduces neither the held central-service design nor Postgres.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/work/github-record](../work/github-record.md)
