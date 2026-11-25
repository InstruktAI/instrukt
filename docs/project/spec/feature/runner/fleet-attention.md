---
description: The conditions of a project's active work that need attention from outside one todo, made events in that project's origin so a subscribed standing procedure answers them without watching.
visibility: public
delivered_by:
  - standing-sdlc-coordinator
---
# Fleet Attention — Spec

## What it is

A project's active todos and the sessions working them are its fleet. Each todo
has an owner that sees its own work and nothing of the others. Fleet attention
is the set of conditions that owner cannot see or cannot resolve from inside
its todo. Each condition becomes an event in the project's origin, so the
standing procedure subscribed to it is started or woken for that condition. A
developer running several todos has stuck or colliding work attended to without
watching its sessions.

## Canonical fields

### Attention conditions

Each condition is established by the party whose contract already decides the
underlying fact. Fleet attention publishes that fact and decides none of it.

| Condition | Established by |
| --- | --- |
| A todo's session stopped, failed or could not be started, and the recovery its contract provides did not move the work. | The runner's observation of the executions it owns and of its own starts (`project/instrukt/spec/feature/runner/session-continuity`). |
| A todo's session waits for input while the todo does not wait on a person. | The runner's observation of the session, with the person wait work decides (`project/instrukt/spec/feature/work/github-record`). |
| A dispatch hold is placed on a todo. | Work, for every source of a hold (`project/instrukt/spec/feature/work/dependencies-and-holds`). |
| A role's attempts on one todo fail repeatedly. | Work's attempt evidence (`project/instrukt/spec/feature/work/todos`). |
| Two live todos of the project collide. | The agent that meets the collision, by raising it. |
| An agent meets a decision owned by a person and has no conversation with that person to ask it in. | That agent, by raising it. |

The threshold at which an observed fact counts as a condition belongs to the
contract that establishes it.

### Raised conditions

A condition that only an agent's judgment can establish is raised by that agent
through instrukt, from any session working in the project. A raise names the
todos it concerns and the obligation at stake. It is recorded with the session
that raised it. Raising needs no knowledge of which session, if any, answers
the project's fleet attention.

### What an attention event carries

- Its origin is the project the condition arose in, and its subject is the
  todo it concerns. A collision names every todo involved.
- It carries addresses: the todo, the session, the role and attempt, the hold
  or the raise. Conversation content stays in its session
  (`general/principle/session-lifecycle`).
- One occurrence of a condition is published once. Observing the same
  occurrence again publishes nothing. A new hold, a new failure or a new raise
  is a new occurrence.
- It names work to reconsider and authorizes nothing by itself. The work
  record remains the source of a todo's state
  (`project/instrukt/spec/feature/runner/event-activation`).

### What produces no attention event

- A running session, a turn that ended with its evidence recorded, a todo
  waiting at a configured approval gate, and an execution whose claim was
  released or transferred.
- A project with no active todo. Every condition concerns an active todo, so a
  quiet project starts no standing agent for fleet attention
  (`project/instrukt/spec/feature/runner/procedure-standing-agent`).

A standing agent answering fleet attention is woken by these events alone. It
inspects no sessions or records on a timer to find out whether anything changed.

### Boundaries

- Routing an attention event to a subscribed procedure, and the one continuing
  agent that answers it per project, follow
  `project/instrukt/spec/feature/runner/procedure-subscription-routing` and
  `project/instrukt/spec/feature/runner/procedure-standing-agent`.
- What is an anomaly, and how the operator is told of one, is unchanged
  (`project/instrukt/spec/feature/runner/anomaly-notification`).
- Fleet attention follows the events switch and is independent of todo pickup
  mode (`project/instrukt/spec/feature/runner/work-dispatch`).

## Use cases

### UC-FA1: A developer's stuck work is attended to without their watching

```gherkin
Given a developer's project has active todos and event responses switched on
And a standing procedure of that project subscribes to fleet attention
When one of those todos meets an attention condition
Then that procedure's standing agent receives the condition once, with the todo and the session it concerns
And a second condition in that project reaches the same agent
But while every todo progresses, waits at an approval gate or has released its execution
Then no attention event is produced
And a project with no active todo has no standing agent started for fleet attention
```

### UC-FA2: An agent hands a collision to the project's fleet attention

```gherkin
Given a developer's agent, working one todo, meets a collision with another live todo of that project
When it raises the collision, naming both todos and the obligation at stake
Then the project's fleet attention carries that collision once, attributed to the raising session
And the raise succeeds whether or not a standing agent for that project exists yet
```

## Known caveats

- A project worked from more than one computer has one answering agent on each
  computer for the conditions that computer emits; no agent sees the fleet
  across computers
  (`project/instrukt/spec/feature/runner/procedure-subscription-routing`).
- An attention event reports a condition as it was established. The condition
  can have cleared before the event is answered; the answering agent reads the
  current record before acting.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/event-activation](event-activation.md)
- [project/instrukt/spec/feature/runner/procedure-standing-agent](procedure-standing-agent.md)
- [project/instrukt/spec/feature/runner/anomaly-notification](anomaly-notification.md)
- [project/instrukt/spec/feature/work/dependencies-and-holds](../work/dependencies-and-holds.md)
