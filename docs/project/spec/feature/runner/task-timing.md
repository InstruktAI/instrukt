---
description: Authorized task due times, missed-run policy and OS-supplied runner wake-ups.
visibility: public
delivered_by: [instrukt-first-build]
---
# Task Timing — Spec

## What it is

Task timing lets a developer arrange prepared work for a future time or a
recurring schedule. Instrukt decides what is due and how to handle missed
occurrences; the operating system supplies a generic wake-up.

## Canonical fields

### Task and schedule

A task is a prepared one-off or recurring instruction, routed through the
prepare flow as a task. Once authorized and due, the runner dispatches the
procedure that declares the `task-runner` role in the selected flavor (the
sdlc flavor's `run-task`), found as it finds the lane orchestrators, under the
todo pickup policy. A claim covers one occurrence's run: it is released when
that run's completion or failure is recorded, and the next occurrence claims
afresh. The run's opening message carries `occurrence: <RFC 3339>`, and its
started and outcome stamps pass that value as `--occurrence`; an outcome whose
occurrence is not the current one is refused as stale. A task does not introduce a
workflow engine of its own. The work context owns task
records, artifacts and stamps; workflow owns preparation and authorization; this feature owns time
eligibility and missed-run decisions.

| Task-record field | Meaning |
| --- | --- |
| `at: <ISO-8601>` | One-off scheduled time. |
| `cron: "<expr>"` | Recurring schedule. |
| `catch_up: once` | Default: missed occurrences result in one catch-up run rather than a replay of every missed occurrence. |
| `catch_up: skip` | Missed occurrences are skipped. |
| Optional agent/model preference | Dispatch consults it before the role mapping in `~/.instrukt/agents.yaml`. |

The task record holds its schedule. Instrukt applies the same due-time and
missed-run logic on every machine; the external scheduler's own missed-run
behavior does not select task occurrences. A dependency such as "after todo
X is delivered" is an event, not a clock schedule.

Cron uses five fields in the machine's local time zone with the OS's
daylight-saving rules. Changing the computer's timezone moves recurring
schedules with it; no task declares a timezone. `at` is ISO-8601 with an offset.
An occurrence is missed when it fell before this wake-up and after the last one and did not
fire. Last-fired accounting is a stamp on the task record. The runner lock
serializes wake-ups; an in-flight task is not fired again, and a failed
dispatch proven rejected before creation counts as not fired. An ambiguous
start remains in flight until reconciled, even without a task-started stamp.

### Occurrence identity and interrupted dispatch

Each occurrence has one title key:
`instrukt:task:<encoded-project>/<encoded-slug>:<encoded-occurrence>` under
`project/instrukt/spec/feature/runner/session-continuity`. The occurrence is
its scheduled instant in canonical RFC 3339 UTC with `Z`; a catch-up selects one
missed instant and persists it before dispatch. A later wake-up retries that
same occurrence, rather than selecting another while its start is unresolved.

Commit its `runtime_starts` marker before invoking the runtime. After restart,
keyed discovery recovers the existing thread before any new start for that
occurrence. A successful empty listing leaves it unresolved and exposes
`instrukt sessions resolve-start <key>`; it does not make the task eligible for
another firing. A failed send retains its recorded target. Acceptance binds
the thread and last-fired evidence to that occurrence; completion or error
still requires its matching outcome stamp. Only after the occurrence resolves
may the recurring schedule select the next due occurrence under `catch_up`.

### Wake-up installation and visibility

`instrukt install` converges a single shared minute wake-up only when an
adopted project's effective todos or events module needs it and `ticks: on`.
Core-only adoption installs none. The native scheduler or tick-only cron
fallback calls `instrukt tick` every minute; no task has its own external
registration. Installation, checks, repair and shared-resource withdrawal follow
`project/instrukt/spec/feature/runner/module-installation`.

`ticks: off` leaves scheduling to the operator calling `instrukt tick` each
minute and withdraws an unneeded owned registration on explicit convergence.
Task selection and catch-up remain instrukt's, with the configured runtime
supplying execution. An event receiver's socket service is independent of ticks.

`instrukt ticks status` identifies no module demand, native/fallback registration,
missing or broken registration, or operator scheduling. It independently reports
the last wake-up, including "no wake-up ever recorded"; enabled scheduled work
with no wake-up for five minutes gets a stale warning. Inactivity with no enabled
work is not missing required infrastructure. Last wake is a state-database row.
A wake proves invocation, not event delivery or task completion; completion
follows `project/instrukt/spec/feature/runner/session-continuity`.

Task execution records its result with
`instrukt todo stamp <slug> --task-outcome success|error [--reason <text>]`,
bound to the particular task execution; the reason records failure evidence.

## Use cases

### UC-TT1: Arrange a prepared task for a future time

```gherkin
Scenario: Execute an authorized instruction when its time arrives
  Given a developer has prepared and authorized a task for a future time
  And the project permits automatic pickup
  When a scheduled wake-up arrives at that time
  Then the task is dispatched for execution
```

### UC-TT2: Recover recurring work according to its missed-run policy

```gherkin
Scenario Outline: Resume after several scheduled occurrences were missed
  Given a developer has an authorized recurring task with <policy> missed-run behavior
  And the project permits automatic pickup
  And several occurrences passed while the computer was unavailable
  When the next wake-up arrives between scheduled occurrences
  Then <outcome>

  Examples:
    | policy        | outcome                                           |
    | one catch-up  | one catch-up execution is dispatched              |
    | skip          | no execution is dispatched for missed occurrences |
```



### UC-TT3: A developer enables unattended scheduled work

```gherkin
Given a developer has enabled todos and authorized scheduled work for their project
When the developer installs the periodic wake-up
Then due work can run without an interactive runner
And missed-run decisions remain governed by each task’s policy
```

### UC-TT4: Recurring work continues after each run

```gherkin
Given a developer has an authorized recurring task
And the project permits automatic pickup
When an occurrence's run succeeds and its outcome is recorded
Then the next occurrence is dispatched when its time arrives
But when a run reports an error that needs the developer's judgment
Then the task is held with that reason and the developer is notified
And no further occurrence runs until the developer resolves it
```

### UC-TT5: Recover scheduled work after an interrupted start

```gherkin
Given a developer's scheduled task was started but instrukt lost the reply
When a later wake-up finds that occurrence's keyed thread
Then the task continues in that thread without firing the occurrence again
But when listing finds no matching thread
Then the occurrence remains unresolved and the developer sees its resolve-start command
And later occurrences wait until the interrupted occurrence is resolved
```

### UC-TT6: Follow the computer's timezone

```gherkin
Given a developer's authorized recurring task has a cron schedule
When the computer's timezone changes
Then later recurring occurrences follow that computer's new local timezone and daylight-saving rules
And the task supplies no timezone override
```

## Known caveats

- A keyed thread prevents duplicate starts during recovery, but unconfirmed
  input can be delivered again. Task operations reconcile their external result;
  this is not an exactly-once side-effect guarantee.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
- [project/instrukt/spec/feature/runner/module-installation](module-installation.md)
