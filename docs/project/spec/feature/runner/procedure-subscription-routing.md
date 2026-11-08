---
description: "Published subscriptions deterministically accept one activation per event, procedure and exact home on the emitting computer."
delivered_by: [instrukt-first-build]
---
# Procedure Subscription Routing — Spec

## What it is

One independent procedure-activation consumer per computer reads known-origin
events that computer emitted. It matches the current published subscription set
and durably accepts each matched responsibility in the local SQLite store.
Routing chooses no session and performs no runtime start, wake or domain effect.

## Canonical fields

### Declaration and ownership

Docs validates and resolves these procedure-only fields, and publication carries
them into its existing generated index. Reading an inline supporting procedure
changes no session. Event enablement gates activation, not document publication.

| Field | Meaning and validation |
| --- | --- |
| `subscriptions` | Nonempty list of shared validated type patterns; mutually exclusive with `activation_trigger`; requires `role`. |
| `subscription_home` | Required with subscriptions; exactly `project`, `platform`, `company` or `person`. |
| `subscription_identity` | `home` (default) or `subject`; selects one agent per home or per subject in that home. |
| `subscription_attendance` | `attended` (default) or `unattended`; runtime audience statement, not event reach or trust. |
| `interruptible` | Boolean, default true (ASAP); false requests only-at-entry. Missing steering is no finding. |

Home/identity/attendance/delivery fields without subscriptions are findings.
Unknown values, malformed patterns and incompatible publication reach are
findings and cannot publish an active subscription. Pattern validation and
routing share the consumer matcher; no second glob implementation is admitted.
Handling instructions come from the procedure's authored Required reads, resolved
through ordinary closure. Metadata injects no documentation dependencies.

General and domain positions answer every origin of their declared kind.
Project-position procedures must declare project home and answer only that
project's origin. This is instrukt's existing project-local reach, an explicit
adaptation of TeleClaude's global/company/person tiers. A domain's owner project
never silently restricts or broadens domain publication reach. Company/person
vocabulary is retained at general/domain positions; resolving those homes remains
an unsupported execution capability with a visible activation failure.

### Published set and matching

Publication is registration: no separate handler registration is needed. Pending
new snippets are indexed as pending and excluded from the active set; a current
snippet with pending changes uses its current declaration. Subscribed procedures
are listed as configured responsibilities with the patterns they answer, not
as self-service triggered procedures. Publication does not activate them while
events are off.

Before each event the consumer checks publication-index identity and reloads on
change. Additions, edits and removals take effect at the next event without a
restart. An unreadable or invalid index clears the cached set, surfaces a read
failure and never keeps stale subscriptions. An unchanged unreadable generation
need not emit repeated error reports; a repaired generation is read afresh.
Clearing the set is not proof that a formerly matching responsibility executed.
The contract supplies no historical replay of events matched against an empty set.

Interest is every type, own emitting computer only and known origin. `unknown`
activates nothing. `none` matches platform home only. A named origin matches its
exact kind and identity within publication reach; a project is never converted
to its owner and a company event never fans out to projects. Matching reads no
data. No match is a normal outcome with no activation.

External facts have ingress source, so only ingress activates procedures for
them. A project missing there or an unsupported company/person home fails visibly
at the accepted activation's execution boundary; it is never rerouted. Patterns
are membership only: overlapping patterns select one procedure once, while two
procedures remain two responsibilities. Pattern edits never create a new standing
identity.

### Durable acceptance and partial retry

An activation key derives from `(event ID, canonical procedure ID, exact home)`.
The exact home includes kind and slug, or platform; same-slug homes of different
kinds are distinct. The record retains the immutable event, procedure, selected
home and accepted identity, attendance and delivery intent. One event has
one subject, so subject need not be added to the activation key; subject-bound
resolution still receives the original subject.

All matched activations are submitted even if another submission fails. Repeated
submission of an existing key finds its durable accepted record. A failed
submission fails that consumer's event handling; bounded retry resubmits only
missing acceptance, and persistent failure follows normal consumer dead-lettering.
Durable keyed acceptance is established before the consumer's handled mark,
settled local cursor and Redis ACK. Accepted records exist before the successful local consumer-outcome transaction; newly accepted rows may join that transaction. No runtime call is held
inside that write section. Successful acceptance does not claim a started agent,
delivered event or completed domain mutation.

### Runtime delivery boundary

Acceptance precedes runtime input. Assignment-first briefing, untrusted data,
bounded batch selection and fixed session settings are owned by
`project/instrukt/spec/feature/runner/procedure-event-delivery`. Routing carries
the resolved intent unchanged; the adapter realizes ASAP with steering, queued
input or idle entry according to capability. Only-at-entry work waits for idle
entry. Missing steering creates no validation finding. The acceptance record
retains its declaration; the continuing session uses its initialized settings.
Runtime execution and continuation remain separate from subscription routing.

## Use cases

### UC-PSR1: Make a responsibility active by publishing its subscription

```gherkin
Given a developer publishes a delivered subscribed procedure and enables events
When a matching local event arrives
Then its keyed activation is accepted without separate handler registration
```

### UC-PSR2: Select the declared event vocabulary

```gherkin
Given a developer has procedures with exact, whole-segment and all-type patterns
When an event reaches routing
Then only procedures selected by the shared matcher receive activations
And a type matching none produces no activation and no routing failure
```

### UC-PSR3: Keep unrelated home kinds separate

```gherkin
Given a developer has project and platform responsibilities
When events have project, none or unknown origins
Then project matches only project and platform matches only none
And unknown activates nothing
```

### UC-PSR4: Answer only where the fact was emitted

```gherkin
Given a developer holds the same published subscriptions on two computers
When one computer emits an event both read
Then only the emitting computer accepts its procedure activations
```

### UC-PSR5: Keep every matched responsibility distinct

```gherkin
Given a developer publishes two procedures and one has overlapping matching patterns
When one local event matches both
Then one activation is accepted for each procedure and overlapping patterns do not duplicate either
```

### UC-PSR7: Avoid a second activation during retry

```gherkin
Given a developer's event handling is interrupted after some matched activations are durably accepted
When routing retries the event
Then existing event-procedure-home keys are reused and only missing activations are accepted
And persistent submission failure is retried then parked visibly
```

### UC-PSR11: Confine a scoped responsibility to its published reach

```gherkin
Given a developer has a project-position responsibility for project demo and a domain responsibility for project origins
When another project emits a matching event
Then only the domain responsibility can match it and the local one stays confined to demo
```

### UC-PSR12: Preserve the exact origin instead of inferring ownership

```gherkin
Given a developer receives a project event with company or person ownership and another origin with the same slug
When subscriptions are matched
Then ownership never converts the project into another kind and home keys keep the kinds distinct
```

### UC-PSR13: See an unavailable execution home as a failure

```gherkin
Given a developer has an accepted activation for a missing ingress project checkout or unsupported company or person home
When the execution boundary resolves that home
Then it records a visible failure and never substitutes another home or computer
```

### UC-PSR14: Carry subject-bound responsibility without changing the fact

```gherkin
Given a developer publishes subscription_identity subject
When an event matches that procedure
Then acceptance retains the subject and declared identity unchanged for runtime resolution
And the activation key remains event, procedure and exact home
```

### UC-PSR15: Make subscription edits effective on the next event

```gherkin
Given a developer edits or removes a delivered subscription and publishes the changed index
When the next local event arrives
Then routing uses the new publication without restart and a removed procedure receives no new activation
```

### UC-PSR16: Avoid using stale subscriptions after an index failure

```gherkin
Given a developer's active publication index becomes unreadable or invalid
When the next local event arrives
Then the cached subscriptions are cleared and a visible read failure is reported
And a repaired index generation is read before a later match
```

### UC-PSR17: Keep late attention separate from a business effect

```gherkin
Given a developer receives a distinct late or superseded domain event
When its subscription matches
Then its activation is accepted as attention due
And routing does not assert that a domain mutation is necessary or complete
```

### UC-PSR18: Carry the procedure audience declaration

```gherkin
Given a developer declares attended or unattended subscription attendance
When an event activates that procedure
Then durable acceptance carries the resolved declaration unchanged
And attendance changes neither selection reach nor event trust
```

### UC-PSR19: Carry delivery policy without reinterpreting it in routing

```gherkin
Given a developer publishes an interruptibility setting for bounded batch delivery
When an activation is accepted
Then its record carries the resolved settings and effective procedure-read references
And runtime delivery remains a separate boundary using the continuing session's fixed settings
```

### UC-PSR20: Keep a project-local subscription responsive within its project

```gherkin
Given a developer publishes a subscribed procedure in project demo
When that computer emits a matching event with demo project origin
Then the project-local procedure receives one accepted activation
And events for another project or none origin cannot activate it
```

## Known caveats

- Runtime acceptance is not entry consumption or domain completion. Delivery uses the adapter's supported input boundary under procedure-event-delivery, including queue-only T3; standing T3 threads stay reachable after human settling.
- Company/person homes are representable but not executable in this release; resolution fails visibly.
- Scheduled-only readers react at their next wake within stream retention. No other computer substitutes after missed retention.
- Replay protection is bounded by consumer marks and retained activation/delivery records, not exactly-once business effects.
- Source headings PSR1–PSR5, PSR7 and PSR11–PSR19 survive; absent PSR6 and PSR8–PSR10 are not recreated.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/event-consumers](event-consumers.md)
- [project/instrukt/spec/feature/docs/snippet-validation](../docs/snippet-validation.md)
- [project/instrukt/spec/feature/publication/project-publication](../publication/project-publication.md)
