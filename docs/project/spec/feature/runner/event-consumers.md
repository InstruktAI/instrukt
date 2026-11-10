---
description: "Independent retained reactions with attribute-only selection, occurrence protection, ordered retries and dead letters."
---
# Event Consumers — Spec

## What it is

Each logical reaction on each computer has its own stream progress. Failure or
lag in one consumer neither hides an event from another nor blocks its handler.
The shared consumer runtime owns selection, replay protection, retries and parking.

## Canonical fields

### Interest and one matcher

Interest consists only of type patterns, an own-emitting-computer filter, and
optional subject/origin kinds. Selection never inspects data. Each declaration
uses the same validated matcher: an exact type; a whole-segment prefix followed
by `.*`, selecting at least one further segment; or `*`. Thus `erp.invoice.*`
selects `erp.invoice.paid` and `erp.invoice.overdue.reminded`, but neither
`erp.invoice` nor `erp.invoices.paid`. Embedded/partial wildcards, empty segments
and other forms are refused at declaration with the invalid pattern named.

### Retained progress and completion

A consumer is a logical reaction with a stable unique name across this computer's
handled store. It uses a separate `consumer-<name>-<computer>` Redis group on each
distinct scoped endpoint/database and a stable reader identity for pending
recovery. Two consumers are separate groups, not competing readers in one group.
The procedure-activation matcher is one consumer, not one group per procedure.

New groups start at the current tail, prepared before relevant producers.
Existing groups resume their own pending entries in stream order before reading
new ones, reclaiming stale pending entries as needed. Each handler is serialized
across its bound streams; consumers run independently. A bounded tick gives each
reader an opportunity and retains unresolved work across invocations.

Before calling an interested reaction the runtime checks `(logical consumer,
event ID)` in SQLite. On successful acceptance it commits local outcome, handled
mark and contiguous settled cursor together, then ACKs only that group's entry.
A duplicate or uninterested entry settles without calling the reaction. A
committed handled mark survives a crash before ACK: redelivery skips the effect
and retries ACK. A crash before the handled commit retries handling, including
an external side effect whose response was lost. Handlers own idempotent effects.
For activation, success is durable acceptance of keyed activations, not a
session's start, delivery, turn completion or domain mutation.

Handled occurrence marks expire after seven days. After expiry the same ID may
be handled again. Existing groups catch up only while source payloads remain;
new consumers do not scan the SQLite history or replay old stream entries.

### Ordered failure and dead letters

The reliability defaults are three handling attempts, 30,000 ms stale-pending
threshold and a 5,000 ms recovery sweep while running. Ordered retries stay on
the failing entry; later work in that consumer waits until it is handled or
parked. Invocation shutdown retains pending obligations for the next drain.
Malformed envelopes enter this failure path with original raw evidence.

At the attempt ceiling the consumer persists a dead-letter record on its failing
Redis, announces `instrukt.system.event.dead_lettered`, then ACKs the original.
The record names consumer and source coordinate, retains original envelope or raw
entry, and records attempts, first/last attempt and failure details. The
announcement names the consumer and parked entry, uses a stream-entry subject
and none origin, and is a declared stateless fact. It never states successful
business handling. Parking advances settled accounting only after these stages
succeed. A failed persist, announcement or ACK remains unsettled and retries
unfinished parking before later entries; already completed stages remain
recognizable after interruption. Other groups keep their own progress.

### Lifetime distinction

This port's procedure reactions are retained occurrences. Live notifications
have no pending groups or handled ledger, capture a current position and use a
bounded backlog; current-state owners recover their authoritative source before
resuming after gaps, while transient presentation intentionally omits missed
frames. That distinction creates no UI, correlation or notification consumer.

## Use cases

### UC-EC1: Keep one responsibility responsive while another fails

```gherkin
Given a developer has two independent consumers interested in an occurrence
When one reaction fails or remains slow
Then the other receives and settles it without waiting for the first
```

### UC-EC2: Avoid acting twice on a retained occurrence

```gherkin
Given a developer has a successful reaction recorded for an event ID
When that ID returns at the same or another stream entry within seven days
Then the reaction is skipped and transport acknowledgment is retried
But after the handled record expires the repeat may be handled again
```

### UC-EC3: Resume handling that was interrupted

```gherkin
Given a developer has a pending reaction when instrukt stops
When it resumes before stream retention ends
Then work without a committed handled record is offered again
And a handled commit made before a lost ACK prevents another effect
```

### UC-EC4: Act only where a procedure event was emitted

```gherkin
Given a developer runs an own-computer consumer on two computers
When one emits a matching occurrence
Then only that computer is handed it for reaction
```

### UC-EC5: Select a vocabulary branch precisely

```gherkin
Given a developer subscribes to erp.invoice.*
When invoice.paid and invoice.overdue.reminded events arrive in the erp domain
Then both are selected
And erp.invoice and erp.invoices.paid are not selected
```

### UC-EC6: Correct an invalid event selection before publication

```gherkin
Given a developer declares a partial or embedded wildcard or an empty-segment type pattern
When the declaration is validated
Then it is refused naming the invalid pattern and cannot become an active subscription
```

### UC-EC7: Route the same facts independently of opaque payloads

```gherkin
Given a developer receives events with the same selection attributes but different opaque data
When consumer interest is evaluated
Then selection is identical and only the selected reaction interprets data
```

### UC-EC8: Inspect a persistent failure while later work proceeds

```gherkin
Given a developer has an event that fails through the attempt ceiling, including a malformed entry
When parking succeeds
Then its original evidence and failure facts are inspectable as that consumer's dead letter and it is announced before ACK
And later entries proceed
But any failed parking stage leaves it unsettled and resumes the unfinished stages after interruption
```

### UC-EC9: Catch up an existing responsibility after downtime

```gherkin
Given a developer stops a retained consumer while events arrive
When it runs again
Then its pending entries precede new entries within the retained stream
And missing trimmed payloads remain a visible recovery limit rather than successful handling
```

### UC-EC10: Start a newly registered reaction from the present

```gherkin
Given a developer registers a new retained consumer after older events exist
When its group is prepared and new producers are admitted
Then it receives subsequent occurrences without replaying stream or local-log history
```

## Known caveats

- Duplicate protection lasts seven days and replay lasts only while Redis retains payloads. This is no permanent exactly-once business-effect contract.
- UC-EC11 and UC-EC12's TeleClaude notification/correlation goals have no admitted instrukt consumers in this port and are not imported or repurposed.
- A dead-letter announcement failure can hold later entries of that consumer; it never blocks another group. Failure reporting remains bounded rather than recursively reporting failures of reporting.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/event-transport](event-transport.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
