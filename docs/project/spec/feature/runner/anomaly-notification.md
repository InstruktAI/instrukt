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

<!-- planned-change:thin-live-runtime-boundary -->
| Situation | Surface |
| --- | --- |
| An agent needs approval or new intent | The agent asks in its thread; the runtime exposes `needs-input`. |
| A failed task waits in no thread | An anomaly eligible for runner notification. |
| The dispatch agent order is exhausted | An anomaly eligible for runner notification. |
| Main is red and the integrator cannot repair it, with no waiting thread | An anomaly eligible for runner notification. |
| A home backup fails or becomes stale | An anomaly eligible for runner notification (failure notified directly by backup run; staleness notified on runner wakes and surfaced on local inspection). |
<!-- change:thin-live-runtime-boundary -->
| Situation | Surface |
| --- | --- |
| An agent needs approval or new intent | The agent asks in its thread; the runtime exposes `needs-input`. |
| A failed task waits in no thread | An anomaly eligible for runner notification. |
| The dispatch agent order is exhausted | An anomaly eligible for runner notification. |
| Main is red and the integrator cannot repair it, with no waiting thread | An anomaly eligible for runner notification. |
| A home backup fails or becomes stale | An anomaly eligible for runner notification (failure notified directly by backup run; staleness notified on runner wakes and surfaced on local inspection). |
| A company conversation turn exhausts bounded failed-turn retries or finishes idle without an explicit reply | An anomaly eligible for runner notification (deduplicated per unresolved message and surfaced on `instrukt events status`, `project/instrukt/spec/feature/company/conversations`). |
<!-- /planned-change:thin-live-runtime-boundary -->

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

<!-- planned-change:thin-live-runtime-boundary -->
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
<!-- change:thin-live-runtime-boundary -->
The adapter's optional `notify` capability
(`project/instrukt/spec/feature/runtime/adapter`) declares support; the runner
setting chooses anomaly-reporting policy. When the selected adapter does not
declare `notify`, the default is unavailable; an explicit `runner.notify`
command can provide it. The project layer does not supply this machine-level
notification command.

The runner also raises an anomaly for what automation cannot resolve in a
conversation or a start: an unresolved start, a live key conflict or an
archived bound thread that persists past the runner's operator bound, a message
no turn has taken past that bound, a message whose turn exhausts bounded
failed-turn retries without an explicit reply, and a message whose turn
completes idle without an explicit reply
(`project/instrukt/spec/feature/runner/session-continuity`,
`project/instrukt/spec/feature/company/conversations`). Conversation-message
anomalies are deduplicated per unresolved message identity
`(company, connector, conversation, message)` within the running process rather
than per conversation binding key, so later unrecovered messages in the same
conversation or shared company session still alert the operator. Each is also
listed in `instrukt events status` with its real cause and door, whether or not
a notification command is configured, so the operator sees what automation
cannot resolve even when the runtime offers no notification.
<!-- /planned-change:thin-live-runtime-boundary -->

For a stopped execution, `instrukt events status` lists its work item,
session, workflow state and whether a notification command is configured.
A recovery input that ends unconfirmed raises the stopped-execution anomaly,
except while its todo waits on a person
(`project/instrukt/spec/feature/runner/session-continuity`).
The finding persists until the workflow advances or the execution is released.

When a home backup run fails (`project/instrukt/spec/feature/runner/home-backup`),
`instrukt backup run` invokes `runner.notify` directly on every host (including
backup-only hosts with neither `instrukt tick` nor `instrukt run`). When home
backup is stale past runner's staleness threshold, active runner invocations
(`instrukt tick` or `instrukt run`) invoke `runner.notify` once per staleness
episode until a subsequent backup succeeds; on a backup-only host with neither
`tick` nor `run`, staleness surfaces on local inspection (`instrukt status` and
`instrukt events status`) rather than via push notification. For a failed or
stale home backup, `runner.notify` receives a non-session home-backup anomaly
reference identifying this computer's condition and never secrets or sensitive
payload (`sdlc/policy/security-awareness`). Every active backup failure or
staleness condition is listed in `instrukt events status` with its repair door,
whether or not a notification command is configured, until a subsequent backup
succeeds.

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

- For a failed or stale home backup, no agent session or work item exists; the
  notification passes the non-session home-backup anomaly reference instead of
  a session reference, once per failed run and once per staleness episode until
  a subsequent backup succeeds.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
