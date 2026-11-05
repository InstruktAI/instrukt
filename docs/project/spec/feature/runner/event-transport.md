---
description: "Scoped Redis publication and bounded replay, with retryable local publication and separate SQLite accounting."
delivered_by: [instrukt-first-build]
---
# Event Transport — Spec

## What it is

The runner publishes domain facts to the Redis of their scope. Local operational
records remain in A1's SQLite database; Redis owns stream acceptance, ordering,
pending deliveries and transport progress. Publication does not complete a reaction.

## Canonical fields

### Scope selection and connection ownership

Scope is producer transport policy, outside event facts. Shipped declarations
have one code-owned scope classification: local by default; cluster when a
cross-computer consumer requires it. The five retained runner/workflow source
facts are local. Relayed native/GitHub events always use cluster, even when
their names resemble a local platform type. Retired platform names remain
local; undeclared external names are relayed types.

A local event uses configured local Redis. Only absence of local configuration
uses cluster Redis, visibly reported at reader startup. A configured local Redis
becoming unreachable preserves its pending positions; it never causes silent
rerouting. Cluster configuration is required for enabled event transport; its
unreachability does not stop unaffected local readers or unrelated local work.

A scope selects one logical endpoint/database. Connections to the same logical
Redis are shared and each source is read once per logical consumer. Separate
blocking-read connections do not hold unrelated writers. A timeout shorter than
the configured blocking-read interval is refused at configuration load. Each
Redis has one reachability record, logging loss and return once per transition;
its readers resume after recovery without discarding positions.

### Coordinates, order and reader lifetime

Transport coordinates identify logical Redis endpoint/database, stream and entry
ID. A SQLite local `seq`, an occurrence `id` and a Redis entry ID are three
different identities. Positions across independent Redis instances are never
compared. Each consumer keeps separate scoped progress; its handler is serialized
across its streams, with no total arrival order invented between scopes.

`instrukt:events` is the platform stream. Its approximate maximum length is
10,000 entries. Retained replay depends on remaining payloads, not elapsed days.
A new retained group starts at the current tail; resumed groups keep their own
pending and new positions. All new group positions are prepared before admitting
relevant local producers. A Redis unavailable at startup is prepared when it
returns: a preexisting group resumes; a new one starts then, omitting prior
entries. No local log backfills a new group. Retrying a still-unconfirmed local publication is source emission, not replay of published history.

Long-lived `instrukt run` hosts independent consumer readers. A scheduled
`instrukt tick` performs bounded pending-then-new drains and exits, retaining
groups. Scheduled-only computers react on their next wake within retention;
no consumer daemon or runtime-owned task scheduler is introduced.

When a retained consumer changes from cluster to local scope, it drains and
settles its outstanding former cluster group in order before retiring that
group and reading the local scope. The new local position is captured before
producers run. A connection outage never masquerades as this planned handover.

### Local committed facts and publication recovery

A finalize-ready stamp and its local event row commit in one SQLite transaction.
A subsequent Redis append is separate. The existing event row retains its stable
occurrence ID, target scope, publication state and confirmed coordinate. An
unconfirmed local publication remains retryable and visibly pending or failed;
a later tick/run republishes the same occurrence, records acceptance and does
not fabricate a rollback of the source fact. An uncertain append response can
produce two stream entries with the same occurrence ID; consumer duplicate
protection absorbs the repeat while retained.

The local event log also materializes received envelopes with their coordinates
idempotently, preserving opaque data and nullable/typed origin. Locally produced
pending publications are distinguished from received records: observing an event
does not publish it again. Local cursor accounting records only settled progress,
partitioned by logical consumer and source. It creates no Redis group position.
No network operation or reaction holds the SQLite writer lock.

### Three distinct retention contracts

The local event log and settled accounting are retained as operational state.
Redis replay ends when the bounded stream trims a payload. SQLite occurrence
handled marks for `(logical consumer, event ID)` expire after seven days. A
retained local row extends neither of the latter two windows. Lost Redis groups
and missing payloads remain visible recovery limits, never successful effects.
Ingress claim retention is a separate three-day source-delivery window.

### Use cases

#### UC-ET1: Keep a computer-local occurrence local

```gherkin
Given a developer has working separate local and cluster Redis endpoints
When a local runner fact is produced and consumed on that computer
Then it is transported only through local Redis
```

#### UC-ET2: Make an external occurrence available across the cluster

```gherkin
Given a developer receives a signed native event named runner.upstream_changed
When ingress publishes it
Then it uses cluster Redis despite resembling a local type
```

#### UC-ET3: Use visible fallback only when local Redis is unconfigured

```gherkin
Given a developer enables events without local Redis configuration
When event readers start
Then local events use cluster Redis and the fallback is reported
But a configured local endpoint outage does not move its positions to cluster
```

#### UC-ET4: Avoid duplicate reads when both scopes name one Redis

```gherkin
Given a developer uses one logical Redis for both scopes
When a consumer selects both local and cluster types
Then it reads that source once and handles one occurrence once while its duplicate record is retained
```

#### UC-ET5: Recover publication of a committed local fact

```gherkin
Given a developer finalizes a todo and the stamp and local event commit
When Redis publication fails or its response is lost and instrukt restarts
Then the committed work remains intact and the unconfirmed event is retried with the same ID
And confirmed stream acceptance is recorded separately from reaction completion
```

#### UC-ET6: Keep unrelated local work moving during a Redis outage

```gherkin
Given a developer has local readers and a cluster reader on separate Redis endpoints
When cluster Redis stops answering
Then local reactions and unrelated work continue
And cluster readers retain their positions and resume pending work after its return
```

#### UC-ET7: Recover each scope from its own progress

```gherkin
Given a developer consumes events from independent local and cluster streams
When readers resume after interruption
Then each uses its own Redis coordinates and settled local accounting
And no local sequence or entry ID from the other scope advances it
```

## Known caveats

- Redis and SQLite share no transaction. A side effect before a handled commit can be repeated and requires owner idempotency.
- The local publication handover covers committed local facts only. Ingress cannot accept a failed cluster append into a local outage buffer.
- Stream trimming, a new or lost group, or a repeat after handled-mark expiry prevents a permanent exactly-once guarantee.
- This contract creates no live UI consumer. Live delivery, where an actual owner admits it, captures a current position, uses bounded memory and source recovery rather than groups or historical replay.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
- [project/instrukt/spec/feature/runner/event-envelope](event-envelope.md)
- [project/instrukt/spec/feature/runner/event-consumers](event-consumers.md)
