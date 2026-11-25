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

Fleet attention decides which facts about a project's active work are
attention conditions, and the point at which a fact counts as one. Each
underlying fact is established by its owner; fleet attention reads it through
that owner's contract and decides none of it again
(`sdlc/policy/domain-ownership`).

| Condition | Underlying facts and their owner |
| --- | --- |
| The session holding a todo's execution claim stopped or failed, or no session could be started for the todo, and the recovery its contract provides did not move the work. | The runner's observation of the executions it owns and of its own starts (`project/instrukt/spec/feature/runner/session-continuity`). |
| The session holding a todo's execution claim waits for input while the todo does not wait on a person. | The runner's observation of that execution (`project/instrukt/spec/feature/runner/session-continuity`) and the person wait work decides (`project/instrukt/spec/feature/work/github-record`). |
| A dispatch hold is placed on a todo. | Work, for every source of a hold (`project/instrukt/spec/feature/work/dependencies-and-holds`). |
| A role's attempts on one todo fail repeatedly: an attempt fails where an earlier attempt of that role on that todo already failed. | Work's record of each failed attempt (`project/instrukt/spec/feature/work/todos`). |
| A worker waits for input its orchestrator cannot supply. | That orchestrator, by raising it. |
| Two live todos of the project collide. | The agent that meets the collision, by raising it. |
| An agent meets a decision owned by a person and has no conversation with that person to ask it in. | That agent, by raising it. |

The runner observes the session holding a todo's execution claim and no worker
that session delegated: a worker's failure, wait for input or absence reaches
its orchestrator through the runtime
(`project/instrukt/spec/feature/runner/session-continuity`). A failure the
orchestrator records is work's attempt evidence
(`project/instrukt/spec/feature/work/todos`); a wait it cannot answer is the
raised condition above.

### Raised conditions

A condition that only an agent's judgment can establish is raised by that agent
through instrukt (`general/principle/intelligence-boundaries`). A raise names
the todos it concerns and the obligation at stake. It is recorded with the
session that raised it. Raising needs no knowledge of which session, if any,
answers the project's fleet attention. Which roles hold the raise, and what a
role without it does, follow
`project/instrukt/spec/feature/configuration/roles-and-capabilities`.

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
  waiting on a person's approval as work decides that wait
  (`project/instrukt/spec/feature/work/github-record`), and an execution whose
  claim was released or transferred.
- A project with no active todo. Every condition concerns an active todo, so a
  quiet project starts no standing agent for fleet attention
  (`project/instrukt/spec/feature/runner/procedure-standing-agent`).

A standing agent answering fleet attention is woken by these events alone. It
inspects no sessions or records on a timer to find out whether anything changed
(`general/principle/session-lifecycle`).

### Boundaries

- Routing an attention event to a subscribed procedure, and the one continuing
  agent that answers it per project, follow
  `project/instrukt/spec/feature/runner/procedure-subscription-routing` and
  `project/instrukt/spec/feature/runner/procedure-standing-agent`.
- What is an anomaly, and how the operator is told of one, belongs to
  `project/instrukt/spec/feature/runner/anomaly-notification`.
- Fleet attention follows the events switch and is independent of todo pickup
  mode (`project/instrukt/spec/feature/runner/work-dispatch`).

## Use cases

### UC-FA1: A developer's stuck work is attended to without their watching

```gherkin
Given a developer's project has active todos and event responses switched on
And a standing procedure of that project subscribes to fleet attention
When one of those todos meets an attention condition
Then that procedure's standing agent receives the condition once, with the todo and the session it concerns
But while every todo progresses, waits on a person's approval or has released its execution
Then no attention event is produced
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
