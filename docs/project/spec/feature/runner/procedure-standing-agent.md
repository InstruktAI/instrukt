---
description: One continuing procedure responsibility per exact home and optional subject, resolved through the keyed runtime adapter on the emitting computer.
visibility: public
delivered_by: [instrukt-first-build]
---
# Procedure Standing Agent — Spec

## What it is

A subscribed procedure has one continuing agent for each home it answers, or
one per event subject within that home when it declares subject identity.
The first activation starts the agent; later activations reach that responsibility
through durable procedure-event delivery. Subscription patterns select events;
they are never part of the agent's identity.

## Canonical fields

### Responsibility and execution home

The responsibility key is `(canonical procedure ID, exact home, subject or none)`.
Home includes its kind and slug, or platform. Subject identity uses the complete
typed event reference, including its qualifiers; a subjectless event cannot
activate a subject-bound agent. Home identity deliberately ignores subject.
Overlapping patterns never merge procedures; edits to patterns never fork an
agent. A change of identity mode selects a different responsibility, without
moving already accepted work out of its original responsibility.

Only the emitting computer resolves the activation. A project home uses that
computer's registered project checkout; platform uses its registered instrukt
checkout. Neither the invoking working directory nor a domain publisher's
checkout substitutes for that home. An absent home is a visible activation
failure, never a request to forward work. Company and person references remain
representable; executing those homes is unsupported in this release. Unknown
origin never means platform.

The assignment is system-origin: no initiating session becomes its dispatcher,
and no starter's human lineage is inherited. Event origin remains the producer's
fact. Project and platform assignments acquire no company context from location
or ownership. Standing confers no integrator, coordinator or job-runner authority;
the declared procedure role supplies the assignment. A human-lineage conversation
is not adopted merely because it reads the same procedure in the same checkout.

### Serialized adapter resolution

The runner durably accepts the activation and its pending event before any
runtime call. Resolution is serialized for a procedure and exact home, covering
all subjects in that home; the selected runtime title key also takes A3's
`runtime_key_locks`. These locks cover manual entry and automatic activation.
Database transactions are short and never span runtime I/O.

`start_or_wake` receives the exact responsibility, execution checkout and
canonical assignment facts. The adapter discovers the title key in the runtime
before considering a start. Its binding is a cache. Only a successful complete
listing with no matching thread and no unresolved start or fixed delivery target
permits a start. Multiple matches, failed discovery, an ambiguous start or failed
send keep work pending under the same key and report their cause. A selected
target commits before sending; a failed send never starts or selects another
thread. Interrupted-start reconciliation follows session-continuity, including
explicit `resolve-start` when absence cannot be established safely.

An existing keyed assignment is reusable only for the same procedure, home,
subject and system-origin assignment. Missing provenance or contradictory
assignment evidence is a visible conflict, not permission to adopt it. Exact
title discovery repairs a lost cache without silently adopting unrelated work.
The opening loader identifies the procedure, role and scope and directs
`instrukt docs get <procedure-id> --closure` before event handling. Bound inputs
and entry facts persist across continuation. Reading a supporting procedure
inline changes no assignment.

### Continuation and successors

An open subject-bound session receives later events about that subject.
An attended closed session resumes its own context through the runtime before
receiving them. The runtime owns revival and transcript retention. Proven
inability to resume permits a new successor for the same responsibility;
failed observation or a failed revival attempt is a visible failure and never
proves non-resumability. A purged runtime record loses earlier context only
after that absence and the prior assignment's terminal disposition are resolved.

For home identity, a confirmed closed agent no longer stands; a later activation
may initialize a successor after retained work is accounted for. Subject identity
instead preserves the attended subject's resumable conversation as described
above. An unattended session is usable while open, but is never revived after closure.
Its next eligible activation resolves a new successor. Explicit session close
suspends pending work without itself starting a successor. A later eligible
continuation reconciles retained older events before newer ones. Failed sends
and ambiguous starts retain their original targets; confirmed closure is a
separate continuation decision, not send-failure recovery.

Each responsibility retains its session generations and one current generation.
The first uses its base standing key; a successor uses a recorded increasing
generation under session-continuity. Retiring an old generation and admitting
the next are one serialized durable decision, possible only after its terminal
state is established and unresolved input is accounted for. No concurrent live
generation is admitted. Discovery considers the responsibility's generation
family before admitting a start. If lost local evidence prevents establishing
which generation is current, resolution fails visibly; an older exact-title
match or an empty cache cannot select a retired generation or permit a duplicate. The runtime creates and resumes sessions; instrukt
records assignment bindings and the continuation decision only.

### Fixed assignment and attendance

Role, attendance, cardinality and interruptibility are fixed at session
initialization and survive revival. Accepted activations retain their own
declaration as evidence. A new successor uses the current published role,
attendance and delivery settings. Required reads are retrieved from current
publication; settings persistence creates no historical documentation snapshot.

`attended` is the default. `unattended` declares an audience, never reach,
authorization or event trust. The target runtime behavior omits unattended
sessions from ordinary human listings unless explicitly requested. This is a
runtime capability, with the T3 limitation below. Work an unattended agent
dispatches has its own attendance; it does not inherit unattended status.

Manual `instrukt sessions run --procedure <id>` for a subscribed home-identity
procedure takes the same guard and resolves the home of the declared kind from
the invocation's registered project, or platform. It starts an attended agent
only when none stands; otherwise it refuses naming the existing agent.
Subject-bound manual start is unavailable. A procedure with no usable declared
role creates no agent. Manual entry never bypasses module or runtime gates.

### Activation outcome

The activation identity remains `(event ID, procedure, exact home)`. Repeating
an accepted or delivered activation reuses its record while retained. Distinct
events remain distinct even when their payloads match. Acceptance, session
resolution, runtime wake acceptance, event handoff and domain completion are
different outcomes. Missing home, missing subject, unusable procedure, failed
continuation and delivery failure stay observable with the activation and
responsibility identity. No failure silently disposes of retained work.

### Use cases

#### UC-PSA1: Establish an agent for a new responsibility

```gherkin
Given a developer has enabled a published project responsibility with no standing agent
When its first matching event arrives on the emitting computer
Then one agent starts in that project's registered checkout with its procedure assignment
And its event was durably retained before startup
```

#### UC-PSA2: Continue an established responsibility

```gherkin
Given a developer's responsibility already has an open standing agent
When another matching event arrives
Then it is queued for that agent at the procedure's eligible delivery boundary
And no second agent starts
```

#### UC-PSA3: Every declared event reaches the one standing agent

```gherkin
Given a developer's procedure answers several event types
When distinct events of those types activate the same home
Then each event reaches that home's same procedure agent
```

#### UC-PSA4: Edit event membership without changing the agent

```gherkin
Given a developer has a standing agent for a procedure and home
When the developer publishes changed subscription patterns and a newly matching event arrives
Then the existing agent answers it without a new standing identity
```

#### UC-PSA5: Keep overlapping responsibilities distinct

```gherkin
Given a developer publishes two procedures with overlapping event patterns
When an event matches both in one home
Then each procedure has its own agent and receives one activation
```

#### UC-PSA6: Keep procedure and home identities distinct

```gherkin
Given a developer has two procedures answering two registered projects
When each procedure receives events in both projects
Then each procedure and exact home pair has its own continuing agent
```

#### UC-PSA7: Avoid duplicate agents during concurrent activation

```gherkin
Given two instrukt invocations encounter first events for one procedure and home
When both attempt activation
Then serialized resolution establishes only one standing session
And both events remain accounted for
```

#### UC-PSA9: See why a responsibility could not be activated

```gherkin
Given a developer has accepted work whose procedure entry or runtime continuation cannot be fulfilled
When activation is attempted
Then its failure and cause are visible beside the activation identity
And undelivered work is distinguishable from delivered work
```

#### UC-PSA10: Reuse a qualifying system-origin assignment

```gherkin
Given the runtime holds one keyed system-origin agent assigned the same procedure and home
And instrukt's binding cache is missing
When a matching event arrives
Then keyed discovery recovers that agent without creating another
```

#### UC-PSA11: Start a home's agent deliberately without duplicating it

```gherkin
Given a developer is in a registered project with an enabled home-identity subscribed procedure
When the developer starts that procedure deliberately
Then it uses the same standing identity as event activation
But when that agent already stands
Then the start is refused naming that agent
```

#### UC-PSA12: Keep a human conversation separate from standing work

```gherkin
Given a developer's human-lineage conversation reads the same procedure in its project
When an event needs the procedure's standing agent
Then that human conversation is not adopted as the standing assignment
```

#### UC-PSA13: Work in the stated project or platform home

```gherkin
Given a developer enables project and platform responsibilities
When events with project and none origins activate them
Then each agent works in its corresponding registered home on the emitting computer
And neither gains company context from its location
```

#### UC-PSA14: Report a missing home without moving the work

```gherkin
Given an external event's project checkout is absent on ingress
When its accepted activation resolves the home
Then the missing home fails visibly and no other checkout or computer substitutes
```

#### UC-PSA15: Avoid handing off a recognized activation twice

```gherkin
Given an activation's successful event handoff is retained
When the same event, procedure and home activation is submitted again
Then its existing outcome is returned and no second handoff is scheduled
```

#### UC-PSA16: Keep one agent for each subject

```gherkin
Given a developer's procedure declares subject identity
When events concern two different typed subjects in one home
Then each subject has its own procedure agent
```

#### UC-PSA17: Continue the open agent for the same subject

```gherkin
Given a subject-bound agent is open
When a later event concerns that same typed subject and home
Then its event reaches that same agent
```

#### UC-PSA18: Resume a subject's own conversation

```gherkin
Given an attended subject-bound agent has closed and the runtime can resume its context
When a later activation concerns that subject
Then the runtime resumes that same agent with its own context before delivering the event
```

#### UC-PSA19: Continue when the earlier context cannot be resumed

```gherkin
Given the runtime has confirmed a subject-bound agent closed and its context cannot resume
When a later activation needs that responsibility
Then one new successor is bound to the same subject and home
And retained older work is reconciled before newer work
But a failed observation or revival attempt never authorizes that successor
```

#### UC-PSA20: Avoid concurrent agents for one subject

```gherkin
Given concurrent activations concern one subject in one procedure home
When their responsibility is resolved
Then one current session answers that subject and every activation remains accounted for
```

#### UC-PSA21: Report a subject-bound event that names no subject

```gherkin
Given a developer's procedure declares subject identity
When an activation's event names no subject
Then the activation fails visibly and no agent starts or wakes for an invented subject
```

#### UC-PSA22: Answer every subject through one home agent

```gherkin
Given a developer's procedure declares home identity
When events about different subjects concern the same home
Then they reach one procedure agent for that home
```

#### UC-PSA25: Keep an open unattended responsibility responsive

```gherkin
Given an unattended procedure agent is still open
When another event concerns its responsibility
Then it receives that event under the same declared delivery rules as an attended agent
```

#### UC-PSA26: Attend an agent started deliberately

```gherkin
Given a developer deliberately starts a home-identity procedure declaring unattended event activation
When its standing session is initialized
Then that manually started session is attended
```

#### UC-PSA27: Continue unattended work without reviving a closed run

```gherkin
Given an unattended agent has closed
When a later activation needs its responsibility
Then the closed run is not revived and a new successor uses the current declaration
```

#### UC-PSA29: Refuse an assignment without a declared role

```gherkin
Given a procedure has no usable declared role
When an agent is requested for it
Then entry refuses before creating a session and identifies the missing role
```

## Known caveats

- Company/person management and execution are unavailable. Source UC-PSA8's
  company continuation is not imported; UC-PSA13 retains project/platform scope.
- T3's published start/send/list/show surface has not demonstrated subject
  revival, proven non-resumability, assignment provenance or attendance-aware
  visibility. These are build gates, not permission to narrow this target or
  rebuild lifecycle in instrukt. Source UC-PSA23/24's runtime UI goals are not
  claimed as supported instrukt use cases. The absent source UC-PSA28 stays absent.
- A runtime's idle or settled thread is not necessarily closed. Observation
  errors and duplicate key titles never authorize a replacement.
- Replay suppression lasts only while activation/delivery evidence is retained.
  Lost handoff acknowledgements can repeat data; procedures reconcile effects
  idempotently. Standing agents can compact and rely on domain-owned records.
- Subject-bound population grows with distinct subjects. Runtime record purge
  can lose resumable context; instrukt supplies no transcript archive.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/procedure-subscription-routing](procedure-subscription-routing.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](procedure-event-delivery.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
