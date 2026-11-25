---
description: Per-computer searchable history extracted from T3 local state, a one-time TeleClaude mirror import and merged search across computers.
---
# History — Spec

## What it is

History is a monorepo module controlled by `modules.history`, on by default.
When off, every history command reports the module is off before accessing its
data or source; `instrukt run` starts neither history extraction nor the history
query responder. Existing stored history and source positions survive disable.

<!-- planned-change:history-text-only-messages -->
instrukt makes past conversations searchable through a store on each computer.
An extract-and-load step reads T3's local state into instrukt's own history;
TeleClaude's existing mirrors are imported once, in bounded batches: an import
that stops continues after the last batch it committed, and a completed import
is never read again. A search across computers
merges results from their local stores. History needs no central service or
Postgres deployment and does not replace the runtime's live conversations.
<!-- change:history-text-only-messages -->
instrukt makes past conversations searchable through a store on each computer.
History is a text-only mirror of what people and agents said: for each
conversation it retains the conversation's identity, title, parent, archive
aliases and the source's own record of the conversation, and the conversation's
messages with their roles, times and text. It retains no tool call, command
output, file change, reasoning trace, run or subagent record, no report a
runtime writes of a background command it ran, and none of the terminal lines,
diff lines or page HTML a message's attached context captured
(`project/instrukt/spec/feature/runner/session-continuity` defines both); those
stay with the runtime that produced them. An extract-and-load step reads T3's local state
into instrukt's own history;
TeleClaude's existing mirrors are imported once, in bounded batches: an import
that stops continues after the last batch it committed, and a completed import
is never read again. A search across computers
merges results from their local stores. History needs no central service or
Postgres deployment and does not replace the runtime's live conversations.
<!-- /planned-change:history-text-only-messages -->

## Canonical fields

<!-- planned-change:history-text-only-messages -->
| Surface | Contract |
| --- | --- |
| T3 local state | The source of conversations for the extracting computer; instrukt loads its own searchable representation. |
| Local history | A computer's own extracted/imported store, independently searchable. |
| TeleClaude mirrors | A one-time legacy import into history, not a continuing runtime or mirror-reader dependency. |
| Merged search | Searches participating computers' local stores and combines results while retaining their computer/source identity. |
<!-- change:history-text-only-messages -->
| Surface | Contract |
| --- | --- |
| T3 local state | The source of conversations for the extracting computer; instrukt loads each conversation's own record and its messages, and nothing else the runtime holds of it. |
| Local history | A computer's own extracted/imported store of conversations and their messages, independently searchable by conversation title and message text. |
| TeleClaude mirrors | A one-time legacy import into history, not a continuing runtime or mirror-reader dependency. |
| Merged search | Searches participating computers' local stores and combines results while retaining their computer/source identity. |
<!-- /planned-change:history-text-only-messages -->

Extraction and search are distinct from the session adapter's keyed startup,
recent-conversation recovery and native current-thread discovery. No central
archive is a prerequisite for those operations. The work record still reflects
artifacts and stamps to GitHub, not conversational back-and-forth. Moving work
between computers requires no conversation-transfer mechanism.

### Operations and adapter boundary

<!-- planned-change:history-text-only-messages -->
History exposes explicit extraction, one-time selected-source TeleClaude import,
local or merged search, and retained-document lookup by native identity or archive
alias. The runtime adapter owns its optional durable conversation-source
capability. History consumes normalized conversation/message changes with native
ids, parent links, roles, available timestamps and completion evidence; it stores
the adapter's source position opaquely. A runtime without that capability reports
extraction as unsupported. This capability also admits other conversation
consumers; history does not implement conversation relay or runtime lifecycle.
<!-- change:history-text-only-messages -->
History exposes explicit extraction, one-time selected-source TeleClaude import,
local or merged search, and retained-document lookup by native identity or archive
alias. A retained document is the conversation's document as its source gave
it, rebuilt from what the store holds. A runtime conversation's document holds
the source's record of the conversation as `thread` and the conversation's
messages as `messages`, each the source's record of the message with its text
as `text`. An imported conversation's document is the document its import
retained, with every text back in its place.

History consumes the text read of the runtime adapter's optional durable
conversation-source capability, and no other read of it; the text read and
what each of its items carries are defined by
`project/instrukt/spec/feature/runner/session-continuity`. History retains each
item's conversation from that conversation's own record when the item carries
one, and each of the item's messages as the item gives it. It stores the
adapter's source position opaquely. A runtime without the
capability reports extraction as unsupported. History does not implement
conversation relay or runtime lifecycle.
<!-- /planned-change:history-text-only-messages -->

The T3 adapter reads the durable projection database located by
`T3Home::database`, discovered from the machine user home through the shared T3
location capability. Instrukt's development publication-output home never selects
the source. It opens the database strictly read-only and WAL-aware, never writes
or migrates it, and does not authenticate to T3 or use its WebSocket API for
history. Retained older messages and archived threads are projection rows and
are included independently of event-log retention or a recent UI window.

<!-- planned-change:history-text-only-messages -->
The adapter owns source discovery and conversation change accounting. Catch-up
recognizes newly durable identities and in-place changes independently of native
timestamps, including later hydration of older records and equal-time updates.
Each scoped read obtains presence, fingerprints and all changed records in one
finite read-only SQLite transaction. An all-conversation pass discovers its
inventory in one finite transaction, then reads each selected conversation in a
fresh transaction; the returned records determine its loaded fingerprint.
Streaming content remains deferred until complete. A conversation's loaded-source
fingerprint and its retained normalized records, searchable body and index commit
atomically;
failed reads do not mark that source state as loaded. Snapshot consistency
covers committed SQL state; it does not establish semantic turn completion.
Source reads never retain a read transaction across local history writes.
<!-- change:history-text-only-messages -->
The adapter owns source discovery and conversation change accounting. Catch-up
recognizes newly durable identities and in-place changes independently of native
timestamps, including later hydration of older records and equal-time updates.
The text read discovers its inventory of every conversation in one finite
read-only SQLite transaction, then reads each selected conversation in a fresh
transaction; the returned records determine its loaded fingerprint. That
fingerprint covers the conversation's own record and its messages only
(`project/instrukt/spec/feature/runner/session-continuity`), so a change to
anything else the source holds of a conversation is not a change history is
told of. A stored position whose fingerprint of a conversation differs from
the text read's has that conversation read again.
Streaming content remains deferred until complete. A conversation's loaded-source
fingerprint and its retained conversation, messages and index entry commit
atomically;
failed reads do not mark that source state as loaded. Snapshot consistency
covers committed SQL state; it does not establish semantic turn completion.
Source reads never retain a read transaction across local history writes.
<!-- /planned-change:history-text-only-messages -->

History stores the source position opaquely. The T3 adapter is the authoritative
owner of vendor-table selection, fingerprint inputs and read mechanics; its
verified vendor contract is T3's projection writer (see See Also).

### Freshness and participation

The existing long-lived `instrukt run` catches up at startup and on native
notifications of database or WAL changes. A native notification can arrive
before the change it announces is readable. While both T3's database and its
WAL are watched, the adapter also asks its readers to read again: a first time
shortly after the subscription, an established watch or a native notification,
then after a delay that doubles up to a longest delay, for as long as both
stay watched. The next native notification starts the delays from the shortest
again. The T3 notification producer owns the shortest and the longest delay.
Such a request becomes due in the producer; a reader receives it when it next
waits on the source, and then does its own read. History extraction, whose
read covers the whole source, follows the first request after a native change
and after that a request only once the time since that change has doubled.
While T3's database or its WAL does not exist or cannot be watched, the runner
tries every two seconds to watch it, reading nothing, and catches up once when
the watch is established: a source that is absent or unwatchable is recovered
from, not failed on (`sdlc/principle/design-fundamentals`). Only the repeated
requests pause while either file is unwatched; the catch-up at startup, at an
established watch and on a native notification is unchanged. Explicit
extraction and every history search also catch up before reading the local
index. Each remote participant performs its own catch-up before answering.
There is no tick-driven extraction, and outside these repeated reads nothing
reads T3's state on a schedule.

Extraction and the query responder each retry inside themselves when their
source or transport fails after they started, and report their state in
`instrukt events status`; their failure ends neither the other nor the runner.
An adopted project whose configuration does not validate is not admitted when
deciding whether history is on (`project/instrukt/spec/feature/runner/work-dispatch`:
the runner works from its admitted configuration).

The conversation source can read a scoped thread from no position for an initial
read. Company bindings refuse a changed source identity under
`project/instrukt/spec/feature/company/conversations`. History extraction remains
independently keyed by source identity and keeps its retained archives.

Local extraction, import, lookup and search require no Redis. Installation
counts enabled history independently as demand for cluster Redis for merged
search, without local event consumers, minute wake-up or receiver demand.
History never requires `modules.events`.

Merged search reuses the existing configured cluster transport and explicitly
named instrukt computer identities. A remote answer requires a running,
history-enabled participant. The computer hosting a history store remains distinct
from original source-computer provenance. Redis transports bounded queries and
answers and holds no central durable archive.

Failed catch-up and unavailable participants remain explicit in the result.
Successful retained matches still return, with unsuccessful completion status
when any part is incomplete. Public errors provide safe operation diagnostics;
detailed internal errors remain in the responding computer's logs. Each committed
extraction batch emits a DEBUG operation summary identifying its source, loaded
count, committed position and duration, without conversation content.

### History store

A computer's history lives in its own store, `state/history.db` in the
canonical state directory: one SQLite database in WAL journal mode, readable
and writable by its owner only, with its own schema version and migrations. A
committed write survives a process crash and a power loss.

A store whose schema is newer than the running instrukt knows is refused, and
so is a symbolic link at the store or at one of its sidecars. The refusal a
caller receives names its recovery — the command that installs the current
instrukt, or linking the instrukt home or its `state/` directory instead — and
carries no path or version. The local command and this computer's failure in a
merged search carry the same text; the detailed error stays in this computer's
log.

<!-- planned-change:history-text-only-messages -->
The history lock `state/history.lock` is the store's write-isolation unit
(`sdlc/policy/state-storage`). Every history write runs while its process holds
it: a catch-up section, one import batch, or moving earlier history in. Readers
take no lock, and work's writes never wait on the history lock. Once the store
records that earlier history has been moved in, no history read or write opens
the state database or takes its writer lock. Moving earlier history in is the
only history operation that does, as described below.
<!-- change:history-text-only-messages -->
The history lock `state/history.lock` is the store's write-isolation unit
(`sdlc/policy/state-storage`). Every history write runs while its process holds
it: a catch-up section, one import batch, bringing a store of an earlier
schema version forward, reclaiming the space that freed, or moving earlier
history in. A reader of a store that is at the current schema version and
records the move complete takes no lock. A reader that finds an earlier schema
version, or the move not recorded complete, takes the write path, and with it
the lock, before it reads. Work's writes never wait on the history lock. Once
the store records that earlier history has been moved in, no history read or
write opens the state database or takes its writer lock. Moving earlier
history in is the only history operation that does, as described below.
<!-- /planned-change:history-text-only-messages -->

<!-- planned-change:history-text-only-messages -->
| Table | Holds | Nature and loss cost |
| --- | --- | --- |
| `history_conversations` | completed conversation views with their archive aliases and original source documents | authoritative; a conversation no source still holds cannot be extracted again |
| `history_projection_records` | normalized native records by source, kind and native id, including records delivered by earlier batches | authoritative for the same reason |
| `history_fts` | the full-text index over each retained conversation's title and text | derived from `history_conversations` and written only by that table's triggers; rebuilt on this computer from the retained rows, without any source |
| `history_source_positions` | each runtime source's opaque committed position; history does not interpret adapter cursor fields | external bookkeeping; losing it reads the whole source again |
| `history_sources` | for each TeleClaude source an import selected, whether the import completed and its position: the id of the last mirror it committed, absent until a first batch commits. Rows moved from the state database keep other values, described below | external bookkeeping; losing it reads the mirror store again |

A conversation's completed view, archive aliases and original source document
commit, with the index entries the triggers write for them, in the transaction
that commits the source position that delivered them. The store also records
how far moving earlier history in has progressed.
<!-- change:history-text-only-messages -->
| Table | Holds | Nature and loss cost |
| --- | --- | --- |
| `history_conversations` | one row per retained conversation: its source and native identity, the computer it came from, its title, parent, archive aliases and limitations, and its document without the texts its messages hold | authoritative; a conversation no source still holds cannot be extracted again |
| `history_messages` | one row per retained text of a conversation: its native id, role, text, creation and update times, its place in the conversation's document and, for a runtime message, the source's record of the message without that text | authoritative for the same reason |
| `history_fts` | the full-text index over each retained conversation's title and the text of its messages, one entry per conversation | derived from `history_conversations` and `history_messages` and written with them; rebuilt on this computer from the retained rows, without any source |
| `history_source_positions` | each runtime source's opaque committed position; history does not interpret adapter cursor fields | external bookkeeping; losing it reads the whole source again |
| `history_sources` | for each TeleClaude source an import selected, whether the import completed and its position: the id of the last mirror it committed, absent until a first batch commits. Rows moved from the state database keep other values, described below | external bookkeeping; losing it reads the mirror store again |

Nothing is stored twice: a text held in a row of `history_messages` is not in
the conversation's document and no other table repeats it. The
full-text index stores terms and reads the text it shows from the retained
rows. A row already retained stays when a later read of its conversation no
longer returns it, so messages a source has dropped stay in history.

A conversation's messages are ordered by creation time and then by native id.
A runtime message's creation time is stored in one fixed form whose text order
is its time order. A text of an imported conversation has no creation time of
its own in its row and a native id that encodes its position in the import's
document, so its native id orders it; the time its source gave it stays in its
document.

A retained document is rebuilt by putting each text back at its place. A text
with a record is that record with the text as `text`, appended to the list at
its place; a text without a record is the string at its place; a text without
a place belongs to no document.

A conversation's row, its messages and its index entry commit in the
transaction that commits the source position that delivered them. A message
whose conversation the store has not retained and whose batch does not carry
that conversation's own record fails its batch, which commits nothing. The
store also records how far moving earlier history in has progressed and whether
space a migration freed is still to be reclaimed.

A store at an earlier schema version is brought to the current one in place
when it is opened for writing, in one transaction under the history lock,
before anything else is written. Every retained conversation keeps its key,
its source and native identity, its computer, title, parent, archive aliases
and limitations. Its document and its messages come from what the earlier
store held of it, by the kind of its source:

- **A conversation of a runtime source**, one whose source identity begins
  `t3:`, takes the native record of its retained conversation record as
  `thread` of its document and one message for each of its retained message
  records but a report of a background command, which the text read leaves
  out too, placed in the document's `messages`. A message takes its record's
  native id and role, the times its record holds in the fixed form, and the
  text of the native record where the native record has one, otherwise the
  record's own text; its record is the native record without that text and
  without the members of its attached context the text read leaves out
  (`project/instrukt/spec/feature/runner/session-continuity`).
  Everything else the earlier store held of the conversation is removed: its
  run, subagent and turn-item records, its earlier document and its earlier
  searchable text. A runtime conversation without a retained conversation
  record fails the migration.
- **Any other conversation whose earlier document is a mirror document**, an
  object holding a mirror and its sources as the TeleClaude import writes it,
  keeps that document. Its texts are the texts it is searched by: the mirror's
  transcript text where the mirror has one, and otherwise each source's
  archival text and the text of each of that source's mirror messages, in the
  document's order, a message's text under that message's role. Each of these
  that is a non-empty string is taken out of the document and placed where
  the document held it, without a record. Nothing else leaves the document:
  every other member stays as it is, and every mirror message entry stays in
  its list at its position, an entry whose text is empty or absent unchanged.
- **Any other conversation** keeps its earlier document unchanged and its
  earlier searchable text, when it has any, as one text without role, record
  or place.

An earlier document or record that is not the JSON its writer produced fails
the migration. A failed migration changes nothing and leaves the store at its
earlier version. The TeleClaude import retains each mirror by the second rule.

The same transaction rebuilds the full-text index from the retained titles and
texts, leaves `history_source_positions` and `history_sources` as they
are, and records that the freed space is to be reclaimed. Every retained
conversation survives with its messages and, for an imported conversation,
its document and the texts it is searched by; the migration removes only what
the rules above take out of the store, which history no longer retains.
Reclaiming follows the
migration under the same hold of the history lock: the rewrite and checkpoint
`project/instrukt/spec/feature/work/state-database` describes for the state
database, applied to this store, and recorded done once the rewrite has
committed. A store opened for writing while reclaiming is still recorded
pending reclaims before anything else is written.
<!-- /planned-change:history-text-only-messages -->

Earlier builds kept history in the state database. Opening the history store
for writing completes the move of that history before anything else is
written, and a reader that finds the move not recorded complete takes that path
before it reads. The move holds the history lock throughout and proceeds by
what the state database holds:

- **No state database exists.** It is created at its current schema, which
  holds no history tables and which earlier builds refuse to open, and the move
  is recorded complete.
- **The state database is at its current schema and holds no history tables.**
  The move is recorded complete without taking the writer lock.
- **The state database holds history tables.** An older database is first
  brought to its current schema under the writer lock, which earlier builds
  then refuse to open. The retained rows are copied and then removed.

<!-- planned-change:history-text-only-messages -->
Copying reads the state database without the writer lock. Rows of
`history_conversations` and `history_projection_records` are written
unchanged, with their keys, in bounded batches, each committed with its
progress; a copy that stops continues after its last committed batch. Rows of
`history_source_positions` and `history_sources` are written unchanged in the
transaction that records the copy finished and the removal pending. The
full-text index is not copied: the store's triggers build it from the copied
conversations.
<!-- change:history-text-only-messages -->
Copying reads the state database without the writer lock. Each retained
conversation is written with its key in the store's current shape, by the rule
the store's own migration applies to a conversation of an earlier schema
version, in batches bounded in conversations and in retained text, each
committed with its progress; a copy that stops continues after its last
committed batch. Run, subagent and turn-item records are not copied. Rows of
`history_source_positions` and `history_sources` are written unchanged in the
transaction that records the copy finished and the removal pending. The
full-text index is not copied: each batch writes the index entries of its
conversations. A store whose copy an earlier schema version left unfinished
starts the copy again from the first conversation: its migration removes the
conversations copied so far, every one of which the state database still
holds.
<!-- /planned-change:history-text-only-messages -->

Rows of `history_sources` moved from the state database keep the values they
held there, with the meanings they had. A TeleClaude source whose row comes
from there is complete, and its position is the number of conversations its
import wrote, not a mirror id. A source whose identity begins `t3:` holds a
position the runtime source kept in this table before
`history_source_positions` held it.
Neither value is read as a mirror id: an import looks up only its own source,
a completed import ends before its position is read, opens no mirror store and
reports no position, and nothing reads a `t3:` row.

<!-- planned-change:history-text-only-messages -->
Removal starts only after that record is committed. In one hold of the writer
lock, one state-database transaction drops the history tables still present
and the state database then reclaims the freed space
(`project/instrukt/spec/feature/work/state-database`). The move is recorded
complete once that database has been rewritten without the freed pages. A move
that stops after the copy finished continues with removal and reclamation
whether or not the tables are already gone, so space freed by a drop is
reclaimed even when the process stopped between the two. No retained
row is discarded (`project/instrukt/policy/checks-and-guards`). While history
is off nothing moves, and the rows stay in the state database until history is
switched on.
<!-- change:history-text-only-messages -->
Removal starts only after that record is committed. In one hold of the writer
lock, one state-database transaction drops the history tables still present
and the state database then reclaims the freed space
(`project/instrukt/spec/feature/work/state-database`). The move is recorded
complete once that database has been rewritten without the freed pages. A move
that stops after the copy finished continues with removal and reclamation
whether or not the tables are already gone, so space freed by a drop is
reclaimed even when the process stopped between the two. Every conversation
the state database retained reaches the store with its messages and, for an
imported conversation, its document and the texts it is searched by; the move
leaves behind only what the store's migration rules take out, which history
no longer retains. While history
is off nothing moves, and the rows stay in the state database until history is
switched on.
<!-- /planned-change:history-text-only-messages -->

The TeleClaude import reads the selected mirror store strictly read-only and
never writes or migrates it. It imports mirrors in ascending id order, starting
at the lowest id the store holds whatever its sign, in batches bounded both in
mirror count and in retained text, at least one mirror each. A batch is read in
its own read-only source transaction, which ends before the batch is written;
the batch's conversations and the id of its last mirror commit in one
transaction under the history lock, which is released between batches. The
import is marked complete in the transaction that commits the last mirror the
store holds, or at once when it holds none past the last committed mirror. A
rerun continues after the last committed mirror and reads no mirror at or
before it; a completed import never opens the mirror store again.

## Use cases

### UC-HI1: Search conversations extracted on one computer

```gherkin
Given a developer has conversations in T3's local state on a computer
When instrukt extracts and loads that state
Then the conversations are searchable in that computer's instrukt history
And no central service or Redis is required
But when the developer disables history
Then every history data operation reports the module is off without touching its data or source
And stored history survives disabling and can be searched after re-enabling
```

### UC-HI2: Keep the TeleClaude mirrors through one import

```gherkin
Given a developer has existing TeleClaude conversation mirrors
When the one-time history import completes
Then those conversations are searchable in instrukt's local history
And the mirror store is unchanged
And later extraction uses T3 local state rather than ongoing TeleClaude mirroring
And rerunning the completed import does not duplicate those conversations
But when the import stops before it completes
Then the conversations it already imported are searchable
And rerunning the import adds the remaining conversations without duplicating any
```

### UC-HI3: Search history held by multiple computers

```gherkin
Given a developer's participating computers each hold local instrukt history
And history is enabled independently of events
When they search across those computers
Then the results merge matches from those stores with their source computer identified
And no central database is introduced
```

### UC-HI4: A developer's history follows a T3 database that appears while the runner is running

```gherkin
Given a developer runs `instrukt run` on a computer where T3's database does not exist
When T3 creates its database and the developer completes a conversation there
Then that conversation is in the developer's instrukt history without a search or a tick
```

## Known caveats

- A conversation physically deleted from the runtime before any extraction cannot
  be recovered. Extracted history survives later source deletion. Archival is not
  deletion and does not exclude a retained source conversation.
- T3 owns event replay retention and snapshot recovery for its API consumers;
  history reads its durable projections and does not depend on that event log or
  the compatibility API's recent window.
<!-- planned-change:history-text-only-messages -->
- Reasoning is available only when a provider persisted it. The T3 database
  source retains stored message, reasoning, tool and diff content, including
  content that wire projections withhold. This states no guarantee that private
  model reasoning or material never stored by the runtime exists.
<!-- change:history-text-only-messages -->
- History holds message text only. What a conversation's tools ran and
  returned, the files it changed, its reasoning traces, its runs and its
  subagents are not retained, not searchable and not part of a retained
  document; the runtime's own state is where they are read while the runtime
  still holds them. A store brought forward from an earlier schema version has
  dropped what it held of them, and nothing restores it.
- Bringing a store forward from an earlier schema version holds the history
  lock from start to end, and reclaiming the space it freed follows in the
  same hold once the migration has committed. The history lock bounds only how
  long another process waits to acquire it; it bounds neither step, whose
  migration takes as long as removing the earlier records needs and whose
  reclamation grows with the data that remains. While the migration runs the
  store is still at its earlier version, so every history operation takes the
  write path and fails visibly when its wait ends, a search included. While
  reclamation runs the store is current: a document lookup and a search's own
  read take no lock and succeed, an extraction or an import fails visibly when
  its wait ends, and a search returns what the store retains with its
  catch-up reported failed. The runner's extraction records each failure and
  retries. Operations started after both complete succeed.
- The first catch-up after a store is brought forward reads again every
  conversation the source still holds whose stored fingerprint covered more
  than its own record and its messages, one conversation per committed batch,
  in one hold of the history lock.
<!-- /planned-change:history-text-only-messages -->
- The T3 source adapter validates every required table, column, type and consumed
  row shape. Migration levels and reported T3 releases are diagnostics, not
  equality gates. Unknown source shape fails visibly without committing its
  batch or position. Record bounds are owned by the source adapter; oversized
  records fail visibly rather than silently truncating history.
- The legacy importer supports the source revision owned by its importer and
  refuses other shapes without migrating the source. A completed selected-source
  import requires no continuing TeleClaude mirror or runtime dependency.
- Native notifications must be available on the source filesystem. A watch
  that cannot be established is reported in the runner's log when it starts
  failing and when it recovers, and the runner keeps trying; no polling
  backend or new daemon is introduced. Searches remain an explicit catch-up
  boundary.
- This contract introduces neither the held central-service design nor Postgres.
- The history store grows with every conversation its sources held. Nothing
  retires a retained conversation: outliving its source is the store's purpose.
- Moving earlier history out of the state database holds the history lock from
  its start until it is recorded complete. It holds the writer lock while it
  creates a missing state database or brings an older one to its current
  schema, and again, in one hold, while it drops the copied tables and the
  freed space is reclaimed. Each lock bounds only how long another process
  waits to acquire it, 30 seconds for the history lock and 60 for the writer
  lock; it does not bound the move, whose copying takes as long as the retained
  history needs, whose removal grows with the space freed and whose reclamation
  grows with the data that remains. Meanwhile an extraction, an import or a
  document lookup fails visibly when its wait ends, the runner's extraction
  records the failure and retries, and a work command that waits out the writer
  lock fails with its lock timeout. The move continues, and operations started
  after it completes succeed.
- A search waits for the history lock twice while another process moves earlier
  history, once for its catch-up and once for its read. It answers in full when
  the move ends during the first wait, answers with the retained matches and a
  catch-up failure when the move ends during the second, and otherwise fails
  when the second wait ends. A computer asked by a merged search answers in the
  same three ways, its failure travelling as its answer, and a computer whose
  own catch-up performs the move answers when the move ends. The requester
  shows the answer that arrives before its deadline, 10 seconds by default and
  at most 120, and reports a computer whose answer has not arrived by then as
  not having answered before the deadline.
- The move is complete once the state database has been rewritten without the
  freed pages. When the database file itself shortens is that database's rule
  (`project/instrukt/spec/feature/work/state-database`) and can come after the
  move is complete; the move does not return to it.
- A mirror is never split across import batches, so a single mirror larger than
  a batch's text bound is read and written whole.
- A mirror added to the mirror store below the last committed id while an
  import is incomplete is not imported.
- A turn whose commit becomes readable after the read its notification
  triggered is read at a later repeated read. The source asks for one no later
  than its longest delay after the previous request. When a reader acts on it
  depends on when that reader next waits on the source and on the work its
  read takes, which this contract does not bound.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/work/github-record](../work/github-record.md)
- https://github.com/pingdotgg/t3code/blob/6417151c093e54dbba0e6c824c69d03983793b9a/apps/server/src/orchestration-v2/ProjectionStore.ts — T3's projection writer, the vendor contract the T3 adapter's source reads are verified against.
