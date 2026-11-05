---
description: Durable activation data reaches a procedure-first assignment through eligible runtime hooks, with fixed session settings and untrusted provenance.
visibility: public
delivered_by: [instrukt-first-build]
---
# Procedure Event Delivery — Spec

## What it is

A subscribed agent receives its assignment through the canonical procedure
loader and its event data through durable hook delivery. The runtime adapter
carries this boundary; runtime lifecycle and provider hooks remain runtime-owned.
Assignment, activation acceptance, wake acceptance, event handoff and domain
completion remain distinguishable outcomes.

## Canonical fields

### Assignment before data

The runner retains the initial event under its activation and responsibility
before bootstrap can offer an initial hook. The canonical brief establishes the
procedure ID, role, home, optional subject, bound inputs and fixed delivery
settings. It directs the agent to load `instrukt docs get <procedure-id> --closure`
before handling event data. The hook associated with that actual procedure entry
supplies the initial event alongside the loader. A generic session-start hook
that precedes the loader is not an activation entry. Repeated hooks for the same
entry select no additional non-interruptible work.

Event bodies never occupy the composer, ordinary conversational prompts or the
assignment's instructions. Platform-authored assignment and delivery metadata
have separate provenance. Event facts remain untrusted data regardless of
signature verification, encoding or hook placement. The native transcript keeps
the procedure assignment distinguishable from event data. An event cannot claim
human authorship or platform instruction authority through text it contains.

The hook states the session's fixed settings and directs an agent missing its
assignment to that procedure's instructions. Shared handling duties come only
from the procedure's declared Required reads, resolved through ordinary closure;
delivery metadata injects neither a second discipline nor new dependencies.

### Eligible boundaries and selection

| Fixed declaration | Eligible delivery |
| --- | --- |
| `interruptible: false` (default) | Initial procedure entry and explicit wake entry only. |
| `interruptible: true` | Those entries and runtime-supported in-turn hook boundaries. |
| `event_delivery: sequential` (default) | Oldest pending event at an eligible boundary. |
| `event_delivery: batched` | Ordered snapshot pending at that boundary. |

Cardinality and timing are independent. Sequential non-interruptible delivery
supplies one event per run. Already pending unselected events and events arriving
after a boundary's selection remain pending. Batching adds no collection delay,
coalescing or parallel agent execution. Eligibility is checked at delivery:
a queued wake cannot inject events into a run that began before that wake entered.

For a non-interruptible run, later events remain pending through tool hooks,
checkpoint continuations and the stop hook. Once the run actually ends, pending
work triggers a content-free wake and reaches the next eligible entry without
unrelated input. Interruptible runs may receive data at the next supported
in-turn boundary; an in-flight tool itself is not preempted. Ordinary idle cleanup
accounts for pending work so completion cannot strand deferred events.

### Durable handoff and continuation

Pending events belong to the responsibility, not to the lifetime of one session.
Activation acceptance records the immutable event, accepted declaration and an
ordered enqueue position before runtime resolution. Procedure-event rows retain
pending/selected/handoff/failed evidence and the chosen session generation.
Only a successful eligible hook handoff records delivered context. A successful
start or content-free wake alone consumes no event. Failed handoff is visible
with its retained outcome; retryable pre-handoff failures leave work pending.
An uncertain handoff retains evidence for reconciliation and never becomes a
claim of successful domain execution.

Runtime boundaries for a session are ordered. Selection and handoff accounting
use activation identities; a later boundary skips recorded completed handoffs.
The successful handoff and its local outcome are not a transaction with the
runtime: death after handoff but before local confirmation can repeat an event.
The procedure reconciles its domain effects idempotently.

Explicit session close suspends delivery without causing revival by itself.
A later eligible continuation, ordinary revival or confirmed successor resolves
the retained older work first under the existing home, subject and attendance
rules. Session transport cleanup occurs only after the responsibility retains
that work or a visible terminal failure accounts for it. Pending work is not
silently deleted or marked failed merely because continuation has not occurred.
An established failed send keeps its fixed target; only separately confirmed
terminal continuation can transfer retained work to a successor.

### Initialization-fixed settings

Initialization records role, attendance, cardinality and interruptibility as
session entry facts. All events delivered by that session use those settings,
including activations accepted under a later declaration. Revival preserves
them; a newly initialized successor adopts the current publication. Each
activation retains its accepted declaration as evidence. The loader reconstructs
its fixed entry facts while loading current procedure instructions; persistence
does not retain historical document generations. Shared handling instructions
apply duties to the actual invocation and fixed settings, including when the
current publication differs or the procedure is read inline.

### Use cases

#### UC-PED1: Load the assignment before handling its first event

```gherkin
Given a developer's subscribed responsibility has no initialized agent
When its first activation starts the agent
Then the canonical brief identifies the procedure and scope and directs its complete contract load
And the initial event arrives through the hook associated with that actual entry
And the event body is absent from ordinary conversational input
```

#### UC-PED2: Finish a non-interruptible run before receiving later events

```gherkin
Given a non-interruptible procedure agent is working
When another event arrives during its run
Then that event remains pending through tool, checkpoint and completion hooks
And after the run ends a content-free wake delivers it through the next eligible entry
And no unrelated input is required
```

#### UC-PED3: Receive new attention during an interruptible run

```gherkin
Given an interruptible procedure agent is working under its loaded handling contract
When another event arrives
Then it receives the event at the next runtime-supported in-turn hook boundary
And the event body is absent from terminal or conversational input
```

#### UC-PED4: Handle the oldest pending event first

```gherkin
Given a developer's procedure uses sequential delivery with several pending events
When an eligible delivery boundary occurs
Then only the oldest pending event is selected
And the others remain pending for later eligible boundaries
And a non-interruptible run receives no additional event through repeated entry hooks
```

#### UC-PED5: Receive the pending batch without losing later arrivals

```gherkin
Given a developer's procedure uses batched delivery
When an eligible boundary selects its pending events
Then it receives that ordered snapshot with each activation identity preserved
And events arriving after selection remain pending without delaying the selected batch
```

#### UC-PED6: Keep external facts separate from instructions

```gherkin
Given an activation event contains text claiming to be a human or platform instruction
When it reaches the agent through a hook
Then it remains identifiable as untrusted event data
And the canonical procedure assignment retains its separate provenance in the transcript
```

#### UC-PED7: Recognize startup failure without consuming unbriefed work

```gherkin
Given an agent's canonical initial brief cannot be delivered
When a developer inspects the activation
Then startup failure is visible
And its retained event is distinguishable from delivered work
And no event was consumed by an agent lacking its assignment
```

#### UC-PED8: Retain a responsibility's work across session replacement

```gherkin
Given accepted events belong to a continuing procedure responsibility
When its session closes or is replaced before handoff
Then pending work remains durably accounted for under the same home and subject
And the next eligible continuation reconciles that work before newer events under its attendance rules
And terminal inability to continue is a visible failure
```

#### UC-PED9: Keep a queued wake out of an already running turn

```gherkin
Given a non-interruptible agent has a wake queued for pending events
And a run starts before that wake enters
When the queued wake reaches a boundary of that run
Then delivery checks eligibility again and does not inject those events into that run
And pending events receive a later eligible wake without unrelated input
```

#### UC-PED10: Preserve initialized delivery settings through publication edits

```gherkin
Scenario Outline: Continue with fixed settings
  Given an agent was initialized with one set of delivery settings
  And the procedure author publishes different settings
  When a later activation reaches <agent>
  Then its acceptance retains the declaration under which it was accepted
  And delivery uses <settings>

  Examples:
    | agent                     | settings                       |
    | the existing agent        | its original settings          |
    | the revived agent         | its original settings          |
    | a newly created successor | the current published settings |
```

#### UC-PED11: Apply handling duties to the session's actual entry facts

```gherkin
Given a continuing agent's fixed delivery settings differ from current publication
When its procedure instructions are loaded for event handling
Then the loader and hook state its fixed settings
And shared handling duties use those actual invocation facts
And reading the procedure inline changes no assignment or session settings
```

## Known caveats

- The T3 build must demonstrate actual initial-entry hook association, ordered
  eligible in-turn/wake delivery, content-free wake, native-transcript provenance,
  attendance and subject continuation through its published boundary. Its
  ordinary `thread send` acceptance and fake adapter tests prove none of these.
  Unsupported capabilities fail visibly; narrowing the delivery outcome needs
  owner direction. instrukt does not implement provider hooks or lifecycle.
- Hook handoff establishes context, never successful domain execution. Lost
  confirmation can repeat data; retained identities do not promise exactly-once
  effects or replay after retention expires.
- Non-interruptible work may defer events for its entire run. Explicit closure
  may suspend accepted work until the next eligible continuation. Other runtime
  control producers retain their own delivery contracts.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/procedure-standing-agent](procedure-standing-agent.md)
- [project/instrukt/spec/feature/runner/procedure-subscription-routing](procedure-subscription-routing.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
