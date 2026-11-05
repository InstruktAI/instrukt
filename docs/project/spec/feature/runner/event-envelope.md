---
description: "Typed event facts, explicit origin and subject, stable occurrence identity and producer-owned declarations."
delivered_by: [instrukt-first-build]
---
# Event Envelope — Spec

## What it is

An event states what happened, the computer and origin where it happened, the
one thing it concerns and the other things it involves. Producers own meaning;
the runner owns envelope shape and the generated catalog. Consumers own reactions.

## Canonical fields

### Attributes and construction

| Attribute | Contract |
| --- | --- |
| `id` | Nonempty occurrence identity; a retryable occurrence keeps its ID and distinct occurrences have distinct IDs. Neither SQLite seq nor Redis entry ID is this identity. |
| `type` | Dotted owner-qualified name. New platform types use `instrukt`; producing domains own their vocabulary. Existing `runner.*` and `workflow.*` remain declared vocabulary. External types use the producer's domain and name. |
| `source` | Stable emitting computer identity. Ingress is the source of relayed requests. |
| `subject` | One nonempty typed reference naming the thing concerned; every current producer states it. |
| `time` | When the occurrence happened, represented as an RFC 3339 timestamp. |
| `data` | Type-specific facts; opaque JSON object for relayed events. |
| `home` | Wire name of origin: registered `project:<slug>`, `person:<slug>`, `company:<slug>`, `none` or `unknown`. It is no consumer destination. |
| `home_owner` | Project ownership as known at emission: company/person reference, `ownerless` or `unresolved`; `none` when not applicable. Unresolved never means ownerless. |
| `related` | Typed references to other involved things, including an actual caller session when present. Automation invents none. |
| `causationid` | Optional ID of the causing event; distinct from this occurrence's ID. |

The producer supplies subject, explicit origin and type-specific facts. The
emission boundary adds the source, time, actual caller-session reference and
resolved project owner, and generates an ID only if the producer has supplied
none. Missing optional attributes state nothing; they do not mean unknown.
An origin of none claims no registered home; unknown makes no origin claim.
Unknown ownership is recorded unresolved even where company/person management
is unavailable. No lookup failure fabricates ownerlessness or an origin.

All attributes travel unchanged to every receiving computer, including references
it cannot resolve. An event carries no action, audience, notification rule,
consumer home or transport scope. Emission performs no consumer authorization
or selection from these facts. Producer construction still checks its own
subject, origin and data against its declaration.

### Reference vocabulary

A reference is a nonempty kind and opaque nonempty identity. Its token splits
at the first colon only: `<kind>:<identity>`. Current producers use typed kinds;
readers retain unfamiliar kinds and identities unchanged.

| Kind | Identity |
| --- | --- |
| `todo` | Todo slug within the event origin |
| `session` | Runtime session identity |
| `operation` | Persisted operation identity |
| `job` | Project-qualified job key |
| `event_type` | Full type name |
| `computer` | Stable computer identity |
| `configuration` | Machine/global configuration or project slug |
| `stream_entry` | Stream and entry identity; diagnostic reference, not occurrence ID |
| `captured_item` | Inbox entry or content identity |
| `inbound_source` | Protocol and declaring home, e.g. `native:project:demo` |
| `signal_item`, `signal_cluster`, `signal_synthesis` | Opaque item, cluster or synthesis identity |
| `deployment` | Deployed project across its deployments |
| `schema_proposal` | Proposal identity |
| `escalation` | Escalation identity |
| `memory_scope` | Identity key, optionally desk-qualified |
| `ci_run` | Repository/run/attempt identity |
| `commit` | Repository and commit SHA |
| `install` | Computer and installation number |
| `release` | Repository and release version |

Vocabulary support creates no subsystem for signals, companies, people,
operations, channels or memory. A relayed subject names its declaring source,
including protocol, even when its type resembles another protocol's types.

### Typed declarations and evolution

The catalog is generated from the event types producers construct, never from
a separate handwritten list. Each declaration supplies name, version,
description, subject kind (or any reference kind), allowed origin kinds (or no
registered home), data field names/value kinds/nullability, related kinds and
optional lifecycle state. JSON kinds are string, integer, number, boolean,
array, object or any. Producers construct only declaration-valid events.
Relayed names need no catalog registration; their data meaning belongs to the
external producer's domain.

Within a version, changes are additive: readers tolerate unknown fields and
absent optional fields. Nonadditive change creates a new version; retained
local records migrate forward under instrukt's versioned migrations. Different
instrukt versions preserve occurrence identity and unknown facts at the codec
boundary. No TeleClaude-specific legacy fields enter the canonical envelope.

### Platform lifecycle states

For `instrukt` types that report lifecycle state, the last segment is one of
`failed`, `stalled`, `degraded`, `detected` (failing), or `succeeded`, `recovered`,
`subsided` (ok). States share a family prefix and subject kind. Failure and
recovery for one thing carry the same subject and origin. A failing family
also declares an ok state. Type alone carries state. Catalog generation refuses
a duplicate declaration, an undeclared state, an inconsistent family or a
stateless platform type ending in a reserved state segment. A dead letter is
an act, not a failing lifecycle. Domain vocabularies need not use these states.

## Use cases

### UC-EE1: Understand the core facts of an occurrence

```gherkin
Given a developer investigates an emitted event
When their agent reads its envelope
Then its ID, type, emitting computer, explicit origin, subject and time identify what happened
```

### UC-EE2: Recognize the owner of an event vocabulary

```gherkin
Given a developer consumes platform and domain events
When their agent reads each type
Then its first segment identifies its producing owner
```

### UC-EE3: Distinguish the thing concerned from where it happened

```gherkin
Given a developer follows a todo event in project demo
When their agent reads it
Then its subject identifies the todo and its origin identifies project demo
```

### UC-EE4: See project ownership as known where an event happened

```gherkin
Given a developer receives events for projects with declared, ownerless or unreadable ownership
When their agent reads the origins
Then declared owners, ownerless and unresolved are distinguishable
```

### UC-EE5: Avoid acting on an invented origin

```gherkin
Given a developer receives an event whose producer cannot establish its origin
When their agent inspects it
Then it states unknown and does not claim none or a guessed project
```

### UC-EE6: Consume facts without inheriting an instruction

```gherkin
Given a developer subscribes a procedure to an external fact
When the fact enters the platform
Then its envelope supplies no action, audience, notification rule or selected consumer home
```

### UC-EE7: Trace the actual emitting session and cause

```gherkin
Given a developer causes an event from a session while answering another event
When their agent reads the emitted event
Then the actual session is related and the causing occurrence is its causation ID
```

### UC-EE8: Publish independently of current consumers

```gherkin
Given a developer publishes a valid occurrence with no interested consumer or locally resolvable destination
When the producer emits it
Then its facts are published without selecting or authorizing a reaction
```

### UC-EE9: Retain facts another computer cannot resolve

```gherkin
Given a developer receives an event on a computer unfamiliar with its named home or reference kind
When their agent reads the received event
Then every original attribute and opaque reference identity is preserved
```

### UC-EE10: Discover exactly what typed producers emit

```gherkin
Given a developer needs the shape of a declared event
When their agent reads the generated catalog
Then it describes the same typed declaration the producer constructs
```

### UC-EE11: Keep consuming an additively extended type

```gherkin
Given a developer uses a consumer of one event version
When its producer adds optional facts within that version
Then the consumer keeps reading the known facts and tolerates the new fields
```

### UC-EE12: Exchange facts across instrukt versions

```gherkin
Given a developer runs compatible instrukt versions on two computers
When a newer producer includes an unfamiliar optional attribute or reference kind
Then the older reader preserves occurrence identity and the unfamiliar facts without rejecting the event
```

### UC-EE13: Recognize one occurrence published again

```gherkin
Given a developer retries publication after an uncertain response
When the same occurrence reaches a consumer at two transport positions
Then both envelopes carry the same occurrence ID and a distinct occurrence carries another ID
```

### UC-EE14: Recognize platform lifecycle state from its type

```gherkin
Given a developer consumes a declared instrukt lifecycle family
When their agent reads a state event
Then its final type segment identifies the declared state and its failing or ok standing without reading data
```

### UC-EE15: Observe a way back from a reported failure

```gherkin
Given a developer declares an instrukt family with a failing state
When its catalog is generated
Then the family also declares an ok state for the same subject kind
And events for one failure and recovery preserve that thing and origin
```

## Known caveats

- Company/person homes are vocabulary only for execution in this release; resolution fails visibly. Unavailable project-owner metadata is unresolved and supplies no new owner registry.
- Reference vocabulary does not install the subsystems it names. Signal emitters and correlation are outside this port.
- Cross-version compatibility is between compatible instrukt codecs; no TeleClaude fleet negotiation or pre-release legacy-wire compatibility is claimed.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/event-discovery](event-discovery.md)
