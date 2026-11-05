---
description: Runtime-independent procedure subscriptions, external event sources and the fork-upstream response example.
visibility: public
delivered_by: [instrukt-first-build]
---
# Event Activation — Spec

## What it is

The events module makes a project's standing agents responsive to changes,
mostly from outside instrukt. A developer adopting instrukt can learn what
changed upstream without a CI pipeline driving the response.

## Canonical fields

### Subscription contract

Procedure frontmatter carries subscriptions. They travel with the
documentation even when events are disabled. The runner reads an event,
resolves matching subscribed procedures and asks the sessions facade to start
or wake the responsible agent.

`modules.events` is the sole events switch, available in both layers (project overrides global). `events.watch` is only the project watch list. Off leaves existing subscriptions inert.
Todo pickup settings do not govern these activations.

Procedures subscribe to instrukt's vocabulary. Event-source adapters translate
native deliveries; procedures do not subscribe directly to one runtime's
native vocabulary. A runtime integration pairs an event source with the
session operations that receive the resulting assignment.

### Sources and reactions

| Origin | Source | Runner-facing meaning |
| --- | --- | --- |
| Workflow | `workflow.finalized` | Finalize-ready work wakes the subscribed standing integrator. |
| Repositories | `git ls-remote` for configured watched upstream refs on each tick | A changed upstream produces `runner.upstream_changed`. |
| Local repository | Compare local main with remote main using local git on each tick; no API call | When local main is ahead and no candidate is pending, emit `runner.main_ahead` to wake the integrator. |
| Candidate CI | Conditional requests for the active integrate candidate's CI status per SHA, respecting the source poll interval | A completed result produces `runner.ci_completed` and wakes the integrator. When nothing has reported for the candidate and the canonical head carries no status and no check run, the result is `no_ci`. |
| Canonical CI | Conditional requests for the CI status of remote canonical main's head commit, respecting the source poll interval | When that commit's CI completes red, produce `runner.main_failing` once for that commit and wake the integrator. |

The active candidate SHA is the fetched `origin/integrate` head not yet
contained in remote canonical main, under the integration-gate contract.
An absent ref or an already promoted head means no candidate is pending.

The project layer selects watched refs with `events.watch: [{remote, ref}]`.
The first build uses this state-reading mechanism for every repository; it
needs no public address or receiver. Each tick considers watched upstream refs, CI
status per active candidate SHA and the CI status of remote canonical main's
head commit, subject to the source poll interval. It does not depend on delivery guarantees: a missed
tick is caught by the next tick reading current state.

A runtime's webhook receipt is a later addition, only as a wake-up to tick
now. The tick still reads state; losing that wake-up loses no state, because
the next scheduled tick reads it. Agents and standing responsibilities react
to the resulting events regardless of what prompted the state read.

Polling checks only active subjects: a waiting integrate candidate, remote
canonical main's head commit, local items on this computer and configured
watched upstreams. API requests reuse
the previous ETag; an unchanged `304` does not consume GitHub rate quota.
Upstream refs use `git ls-remote`, consuming no API quota. Where GitHub sends
`X-Poll-Interval`, the runner never polls that source faster.

State watching is source acquisition. Standing agents still
wake on the resulting event; they do not repeatedly inspect repositories to
discover whether anything changed. Time-based task due logic belongs to
`project/instrukt/spec/feature/runner/task-timing`.

### Fork-upstream response

The shipped `general/procedure/upstream-watch` subscribes a standing agent to
`runner.upstream_changed`. The
agent reports the upstream changes and offers reconciliation that retains the
fork's local modifications. A person's affirmative response turns that offer
into a todo with review through work and workflow. The
event itself does not authorize reconciliation of the fork.

Human questions stay in the responding agent's thread, as governed by
`project/instrukt/spec/feature/runner/anomaly-notification`.

### Delivery and restart boundary

An event names work to reconsider; the current work record remains the source
of workflow progress. A turn-completion event cannot stand in for a step's
artifact and stamp. Session observation follows
`project/instrukt/spec/feature/runner/session-continuity`.

Events persist in the event log of the state database
(`project/instrukt/spec/feature/work/state-database`, Runner tables),
with per-consumer cursors, per-consumer handled marks and idempotency keys.
An event is appended with its idempotency key in one transaction, so it is
logged once. A delivery's handled marks, one per event it hands over, and
the consumer's cursor commit in one transaction with the delivery's outcome
once the runtime has answered; an event carrying a mark is never handed to
that consumer again. A send interrupted before that commit is repeated once
at the next delivery, which the handler absorbs by reconciling its subject.
Restart resumes from the cursors; handlers reconcile
their subject against current state, making replay idempotent. Event payloads
and keys are defined in `project/instrukt/spec/domain/runner`.
A standing agent's session is recorded per (subscribing procedure, project).

Conditional-request validators are kept in the state database per project
and request, so a later invocation asks only for what changed, beside each
watched ref's last observed revision.

A start is recorded before the runtime is asked, and its handled marks
commit with the session it binds. A start whose result never
arrived (the invocation ended after the runtime may have created the session)
is never repeated, so no second standing session is created for the same
agent: it is reported once as an anomaly naming the exact command line that
resolves it, `instrukt sessions resolve-start <procedure>@<project> --session
<runtime:id>` or `--none`, and the standing agent waits until it is resolved.

### Use cases

#### UC-EA1: Learn about an upstream change before reconciling a fork

```gherkin
  Given a developer has adopted instrukt for a fork with local modifications
  And the developer has enabled event responses for its upstream
  When the upstream changes
  Then a standing agent reports the changes
  And offers reconciliation that retains the fork's local modifications
```
