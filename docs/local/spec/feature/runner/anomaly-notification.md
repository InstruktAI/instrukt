---
description: Runner reporting for anomalies outside waiting threads, with human decisions kept in agent conversations.
visibility: public
delivered_by: [instrukt-first-build]
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
preferably a deep link that opens it. The anomaly content stays in that
session; nothing is copied into logs, status output or the notification.
The command receives the session reference and a deep link when the runtime
mapping can build one.

`runner.notify` is a command in global configuration; `runner.notify: off`
disables notification. It controls whether and
how the runner reports anomalies. When unset, the runner defaults to the
notification the runtime mapping says that runtime offers, including mobile
notification where that capability is available.

The runtime entry's optional `notify` command template declares capability; the runner setting chooses anomaly
reporting policy. The project layer does not supply this machine-level
notification command.

### Use cases

#### UC-AN1: Notice a failed task that is not waiting in a conversation

```gherkin
  Given a developer has adopted instrukt with an available runtime notification facility
  And has left anomaly reporting at its default
  When a task fails with no conversation waiting for the developer
  Then the developer receives a reference to the session holding the failure
```

## Known caveats

- When the runtime offers no notification, or delivering one fails, nothing
  else happens: the anomaly stays where it lives, and nothing is copied or
  retried elsewhere.
- When no session could start (the dispatch order is exhausted), the runner
  releases its own claim and sets no hold; the notification references the
  work item instead of a session.