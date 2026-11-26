---
description: One continuing procedure responsibility per exact home and optional subject, resolved through the keyed runtime adapter on the activating computer.
visibility: public
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

Only the activating computer resolves the activation: the emitting computer,
or for an external fact the reacting computer
(`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`). A project home uses that
computer's registered project checkout; a company home uses that computer's
`~/.instrukt/companies/<name>/`
(`project/instrukt/spec/feature/configuration/company-homes`); platform uses its
registered instrukt checkout. Platform and company-home enablement use the
machine's global events setting; the execution checkout's project override
governs that project's events, rather than platform or company responsibilities.
A general or domain procedure declaring company home has one responsibility per
company home that receives its events. Neither the invoking working directory
nor a domain publisher's checkout substitutes for that home. An absent home is a
visible activation failure, never a request to forward work. A person reference
remains representable; executing a person home is unsupported and fails visibly.
Unknown origin never means platform.

The assignment is system-origin: no initiating session becomes its dispatcher,
and no starter's human lineage is inherited. Event origin remains the producer's
fact. A company-home assignment names its company as its home. Project and
platform assignments acquire no company context from location or ownership. Standing confers no integrator, coordinator or job-runner authority;
the declared procedure role, or `event-handler` when omitted, supplies the
assignment. The role's class and grants follow
`project/instrukt/spec/feature/configuration/roles-and-capabilities`; standing
responsibility identity alone supplies no extra command grant. A human-lineage
conversation is not adopted merely because it reads the same procedure in the same checkout.

### Serialized adapter resolution

The runner durably accepts the activation and its pending event before any
runtime call. Resolution is serialized for a procedure and exact home, covering
all subjects in that home; the selected runtime title key also takes A3's
`runtime_key_locks`. These locks cover manual entry and automatic activation.
Database transactions are short and never span runtime I/O.

`start_or_wake` receives the exact responsibility, checkout and canonical
assignment facts. The durable native-ID binding, fixed-key launch replay and
project/placement verification follow
`project/instrukt/spec/feature/runner/session-continuity`. A failed send or
observation retains pending work and the same target. Titles never select an
actor or repair lost identity. The opening loader directs
`instrukt docs get <procedure-id> --closure` before handling events; continuation
retains its initialized role, home, subject, attendance and delivery facts.

The thread title names its actor (`sdlc/principle/domain-driven-design`): the
assignment's effective role alone for home identity, or `{role} · {subject}`
with the complete typed subject for subject identity. An assignment without an
effective role is titled by the last segment of its procedure ID in the same
shape. Automatic activation, manual entry reservation and successors title
their generation this way.

### Continuation and successors

Standing T3 threads are settled, never closed. Runner automation never settles a
standing agent (`UC-PSA27`). A standing procedure may self-settle when parked
dormant (`general/procedure/event-handling/overview`), or a human may settle it;
either way the thread remains findable and reachable, wakes in place on a
message and un-settles. Later activations reuse the same keyed assignment for home and
subject identity, attended and unattended alike. Settling neither retires a
responsibility nor authorizes a successor. There is no closed T3 context to
revive or non-resumability proof to obtain.

A confirmed native archive or deletion with pending/new work admits one
successor through the shared runner binding owner. Failed observation/send and
settled/idle state do not. Each responsibility retains generation history and
one current generation. Under the responsibility/key guard, retire the old
binding, increment generation once, retire its waiting input, and return only
unconsumed activations to pending. Compose the successor from the current
published assignment and pending events. Native launch uses a new generation key
and thread ID; retry reuses them. Native provider detachment remains the runtime's
responsibility; receipt deduplication proves one native thread per launch key,
not physical process exit across generations.

Manual reservations keep their record-only binding/no-start rules. Their
initialized assignment and observed native identity remain evidence; no listing
or title repairs a lost automatic binding.

### Fixed assignment and attendance

Role, attendance and delivery intent are fixed at session
initialization and survive continuation, including a human settling the thread.
Accepted activations retain their own
declaration as evidence. A new successor uses the current published role,
attendance and delivery settings. Required reads are retrieved from current
publication; settings persistence creates no historical documentation snapshot.

`attended` is the default. `unattended` declares an audience, never reach,
authorization or event trust: it changes neither whether an agent is started nor
which events it receives. Standing T3 threads stay visible and unsettled
under automation regardless of attendance. Work an unattended agent dispatches
has its own attendance; it does not inherit unattended status.

Manual agent entry first records `instrukt events bind <procedure> --reserve`.
This record-only command takes the responsibility guard, resolves the procedure's
declared home (the caller's project, the platform, or the company named with
`--company <name>`), and reserves an attended home-identity generation
only when no standing generation or unresolved reservation exists. It returns a
reservation token, exact title/key and initialized assignment brief. An existing
generation refuses naming its session, before any native call. A repeated caller
reservation returns the same facts without permitting another native start.

The agent uses `@config: runtime.start_thread`, then records `instrukt events bind <procedure> --reservation <token>
--session <runtime:id> --evidence <file|->`. The agent supplies the native
identity and observed title/initialized-assignment evidence. Binding checks those
facts against the reserved home, role, attendance and delivery settings, records
the generation under that same guard. An exact replay succeeds. A replay for a
different native ID refuses before inspecting that thread and names the recorded
ID and the status command. Reserving and binding start no thread. An interrupted
native result retains the reservation for native inspection and binding.

`events bind <procedure> --reservation <token> --not-started --reason <text>`
permits retry of the same assignment when the native listing finds no thread
for the key. A previously observed thread also needs confirmation of absence
by its exact ID. With `--stopped <id>`, confirmed absence of the previously
observed ID can settle the reservation when the listing is unavailable.
Any remaining same-key thread found by either check blocks retry. Instrukt retains its identity until the existing reservation is bound or its
no-start requirements are satisfied. Automatic activation waits for the reservation's
resolution before it can start a generation.

Runner-started standing agents use automatic native receipt replay and
terminal succession from `project/instrukt/spec/feature/runner/session-continuity`.
No operator start-resolution/replacement command is exposed.

Subject-bound manual entry remains unavailable; module and runtime gates apply
before reservation. Both automatic activation and manual entry reservation use
the declared role or, for an otherwise valid subscription that omits it,
`event-handler`. Omission alone never refuses entry. Malformed entry metadata
that cannot supply a usable effective role refuses before session creation or
reservation. No history lookup is introduced.

### Activation outcome

The activation identity remains `(event ID, procedure, exact home)`. Repeating
an accepted or delivered activation reuses its record while retained. Distinct
events remain distinct even when their payloads match. Acceptance, session
resolution, runtime input acceptance, entry consumption and domain completion are
different outcomes. Missing home, missing subject, unusable procedure, failed
continuation and delivery failure stay observable with the activation and
responsibility identity. No failure silently disposes of retained work.

## Use cases

### UC-PSA1: Establish an agent for a new responsibility

```gherkin
Given a developer has enabled a published project responsibility with no standing agent
When its first matching event arrives on the activating computer
Then one agent starts in that project's registered checkout with its procedure assignment
And its event was durably retained before startup
```

### UC-PSA2: Continue an established responsibility

```gherkin
Given a developer's responsibility already has an open standing agent
When another matching event arrives
Then it is queued for that agent at the procedure's eligible delivery boundary
And no second agent starts
```

### UC-PSA3: Every declared event reaches the one standing agent

```gherkin
Given a developer's procedure answers several event types
When distinct events of those types activate the same home
Then each event reaches that home's same procedure agent
```

### UC-PSA4: Edit event membership without changing the agent

```gherkin
Given a developer has a standing agent for a procedure and home
When the developer publishes changed subscription patterns and a newly matching event arrives
Then the existing agent answers it without a new standing identity
```

### UC-PSA5: Keep overlapping responsibilities distinct

```gherkin
Given a developer publishes two procedures with overlapping event patterns
When an event matches both in one home
Then each procedure has its own agent and receives one activation
```

### UC-PSA6: Keep procedure and home identities distinct

```gherkin
Given a developer has two procedures answering two registered projects
When each procedure receives events in both projects
Then each procedure and exact home pair has its own continuing agent
```

### UC-PSA7: Avoid duplicate agents during concurrent activation

```gherkin
Given two instrukt invocations encounter first events for one procedure and home
When both attempt activation
Then serialized resolution establishes only one standing session
And both events remain accounted for
```

### UC-PSA9: See why a responsibility could not be activated

```gherkin
Given a developer has accepted work whose procedure entry or runtime continuation cannot be fulfilled
When activation is attempted
Then its failure and cause are visible beside the activation identity
And undelivered work is distinguishable from delivered work
```

### UC-PSA10: Reuse a qualifying system-origin assignment

```gherkin
Given a standing responsibility retains the native ID and initialized facts of its system-origin agent
And its current runtime binding is missing
When a matching event arrives
Then instrukt restores that binding from the retained owner identity without creating another
```

### UC-PSA11: Start a home's agent deliberately without duplicating it

```gherkin
Given a developer is in a registered project with an enabled home-identity subscribed procedure
When their agent reserves manual entry before starting it natively and binding its native id
Then the same standing identity is recorded for later event activation
But when a generation or unresolved reservation already stands
Then entry is refused before native start and identifies that assignment
And a replay for a different native ID refuses naming the recorded ID
And a no-start confirmation with a remaining same-key thread retains that identity and refuses retry
```

### UC-PSA12: Keep a human conversation separate from standing work

```gherkin
Given a developer's human-lineage conversation reads the same procedure in its project
When an event needs the procedure's standing agent
Then that human conversation is not adopted as the standing assignment
```

### UC-PSA13: Work in the home the event names

```gherkin
Given a developer enables responsibilities answering project, company and platform homes
When events with project, company and none origins activate them
Then each agent works in the home its event names on the activating computer
And a project or platform agent gains no company context from its location
```

### UC-PSA14: Report a missing home without moving the work

```gherkin
Given an external event's project checkout is absent on the reacting computer
When its accepted activation resolves the home
Then the missing home fails visibly and no other checkout or computer substitutes
```

### UC-PSA15: Avoid handing off a recognized activation twice

```gherkin
Given an activation's successful event handoff is retained
When the same event, procedure and home activation is submitted again
Then its existing outcome is returned and no second handoff is scheduled
```

### UC-PSA16: Keep one agent for each subject

```gherkin
Given a developer's procedure declares subject identity
When events concern two different typed subjects in one home
Then each subject has its own procedure agent
```

### UC-PSA17: Continue the open agent for the same subject

```gherkin
Given a subject-bound agent is open
When a later event concerns that same typed subject and home
Then its event reaches that same agent
```

### UC-PSA18: Wake a human-settled subject conversation

```gherkin
Given a human has settled a subject-bound standing T3 thread
When a later activation concerns that subject
Then the message wakes the same reachable thread with its existing context
And settling does not create a new generation
```

### UC-PSA19: Preserve a standing assignment when observation fails

```gherkin
Given a subject-bound standing T3 thread has been settled by a human
When a later activation cannot observe or send to that thread
Then the original assignment and undelivered event stay pending
And no successor is authorized by that failure or settled state
```

### UC-PSA20: Avoid concurrent agents for one subject

```gherkin
Given concurrent activations concern one subject in one procedure home
When their responsibility is resolved
Then one current session answers that subject and every activation remains accounted for
```

### UC-PSA21: Report a subject-bound event that names no subject

```gherkin
Given a developer's procedure declares subject identity
When an activation's event names no subject
Then the activation fails visibly and no agent starts or wakes for an invented subject
```

### UC-PSA22: Answer every subject through one home agent

```gherkin
Given a developer's procedure declares home identity
When events about different subjects concern the same home
Then they reach one procedure agent for that home
```

### UC-PSA25: Keep an open unattended responsibility responsive

```gherkin
Given an unattended procedure agent is still open
When another event concerns its responsibility
Then it receives that event under the same declared delivery rules as an attended agent
```

### UC-PSA26: Attend an agent started deliberately

```gherkin
Given a developer deliberately starts a home-identity procedure declaring unattended event activation
When their agent reserves attended entry and binds its natively initialized standing session
Then that session retains its attended initialization for event delivery
```

### UC-PSA27: Keep unattended standing agents unsettled under automation

```gherkin
Given an unattended standing T3 agent finishes its current work
When automation records that outcome
Then its standing thread remains unsettled in the sidebar
And a human may settle it without preventing later messages from waking it
```

### UC-PSA29: Refuse an assignment with an unusable effective role

```gherkin
Given a developer's procedure has malformed entry metadata that cannot supply a usable effective role even after the subscribed-role default applies
When automatic activation or manual entry reservation is requested for it
Then entry refuses before creating a session and identifies the unusable effective role
```

### UC-PSA30: Keep a quiet responsibility's agent out of the active list

```gherkin
Given a developer's standing agent answers a responsibility whose procedure names when it rests
When that rest condition holds and the agent's current work is recorded
Then its thread leaves the active list and stays findable and reachable
And the next activation wakes that same thread without a new generation
```

## Known caveats

- Person homes and their execution are unavailable. Source UC-PSA8's company
  continuation is not imported.
- A standing T3 thread is never closed by settling. Observation
  errors and duplicate key titles never authorize a replacement.
- Steering is optional: ASAP uses queue-only or idle-entry delivery when that
  is the runtime's capability. Missing steering is no activation finding.
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
