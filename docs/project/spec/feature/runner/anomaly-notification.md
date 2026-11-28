---
description: Runner reporting for anomalies outside waiting threads, with human decisions kept in agent conversations.
visibility: public
---
# Anomaly Notification — Spec

## What it is

Anomaly notification makes otherwise unattended failures visible. Questions
requiring a person's intent or approval are asked by an agent in its session
thread, where the runtime shows that input is needed.

## Canonical fields

### Routing contract

| Situation | Surface |
| --- | --- |
| An agent needs approval or new intent | The agent asks in its thread; the runtime exposes `needs-input`. |
| A failed task waits in no thread | An anomaly eligible for runner notification. |
| The dispatch agent order is exhausted | An anomaly eligible for runner notification. |
| Main is red and the integrator cannot repair it, with no waiting thread | An anomaly eligible for runner notification. |

Notification is for anomalies only. It does not duplicate ordinary questions
from a waiting thread or supply a separate approval channel.

### Configuration ownership

A notification carries only a reference to the session holding the anomaly,
preferably a deep link that opens it. The anomaly content stays in that session;
nothing is copied into logs, status output or the notification. The command
receives the session reference and a deep link when the runtime adapter supplies
one.

`runner.notify` is a command in global configuration; `runner.notify: off`
disables notification. It controls whether and how the runner reports anomalies.
When unset, the runner defaults to the notification the runtime adapter declares
supported, including mobile notification where that capability is available.

The adapter's optional `notify` capability declares support; the runner setting
chooses anomaly-reporting policy. T3's published host CLI has no notification
operation, so its default is unavailable; an explicit `runner.notify` command
can provide it. The project layer does not supply this machine-level
notification command.

The runner also raises an anomaly, once per running process, for what automation
cannot resolve in a conversation or a start: an unresolved start, a live key
conflict or an archived bound thread that persists past the runner's operator
bound, a message no turn has taken past that bound, and a message whose turn keeps
failing (`project/instrukt/spec/feature/runner/session-continuity`,
`project/instrukt/spec/feature/company/conversations`). Each is also listed in
`instrukt events status` with its door, whether or not a notification command is
configured, so the operator sees what automation cannot resolve even when the
runtime offers no notification.

For a stopped execution, `instrukt events status` lists its work item,
session, workflow state and whether a notification command is configured.
A recovery input that ends unconfirmed raises the stopped-execution anomaly,
except while its todo waits on a person
(`project/instrukt/spec/feature/runner/session-continuity`).
The finding persists until the workflow advances or the execution is released.

## Use cases

### UC-AN1: Notice a failed task that is not waiting in a conversation

```gherkin
  Given a developer has adopted instrukt with an available runtime notification facility
  And has left anomaly reporting at its default
  When a task fails with no conversation waiting for the developer
  Then the developer receives a reference to the session holding the failure
  But when a stopped execution has no notification command
  Then events status lists it as stopped with notification unconfigured until the work advances
```

## Known caveats

- When a notification command is unavailable or delivery fails, the anomaly
  remains visible in status. Its content stays where it was recorded.
- When no session could start (the dispatch order is exhausted), the runner
  releases its own claim and sets no hold; the notification references the
  work item instead of a session.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
