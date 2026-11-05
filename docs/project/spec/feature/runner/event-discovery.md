---
description: "List and inspect producer-generated event declarations, optionally narrowed by vocabulary owner."
delivered_by: [instrukt-first-build]
---
# Event Discovery — Spec

## What it is

Developers and their agents discover the types instrukt declares before writing
consumers. Discovery describes producer facts, without requiring a receiver,
Redis connection or running consumer.

## Canonical fields

### Commands

`instrukt events list [--domain <owner>]` lists every declared type with name,
version, description and what it is about, and points to `instrukt events schema
<type>`. The owner filter selects the first segment, including `instrukt`,
`runner` and `workflow` as applicable to shipped producers.

`instrukt events schema <type>` shows the full generated declaration: name,
version, description, subject kind, allowed origin kinds or none, data fields
with JSON kind and nullability, related kinds, and any family/state/standing.
An unknown type fails visibly naming the requested type; an empty owner list
is an empty result. Both views derive from the exact producer declarations.
Neither shows notification rules, consumer actions, audiences or routing scope.
External vocabularies remain discoverable from their producing domain's docs;
relaying an undeclared external name needs no new catalog entry.

### Use cases

#### UC-ED1: Find the events available to consume

```gherkin
Given a developer wants to make a procedure responsive to instrukt events
When their agent lists the event catalog
Then every declared type is listed with version, description and subject information and a way to inspect its schema
```

#### UC-ED2: Understand a type before consuming it

```gherkin
Given a developer needs the shape of a listed type
When their agent inspects its schema
Then the full producer declaration identifies subject, origins, data kinds and nullability, related kinds and any lifecycle state
```

#### UC-ED3: Find the events published by one owner

```gherkin
Given a developer needs events of one vocabulary owner
When their agent lists the catalog for that owner
Then only that owner's declared types are listed
```

## Known caveats

- Discovery declares available shapes, not evidence of delivered events or installed transport.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/event-envelope](event-envelope.md)
