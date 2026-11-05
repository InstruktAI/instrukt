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
| Optional agent/model preference | Dispatch consults it before procedure dispatch defaults and global `agents.order`. |

The task record holds its schedule. Instrukt applies the same due-time and
missed-run logic on every machine; the external scheduler's own missed-run
behavior does not select task occurrences. A dependency such as "after todo
X is delivered" is an event, not a clock schedule.

Cron uses five fields in the machine's local time zone with the OS's
daylight-saving rules. `at` is ISO-8601 with an offset. An occurrence is
missed when it fell before this wake-up and after the last one and did not
fire. Last-fired accounting is a stamp on the task record. The runner lock
serializes wake-ups; an in-flight task is not fired again, and a failed
dispatch counts as not fired.

### Wake-up installation and visibility

`instrukt install` registers the generic wake-up with the available OS
scheduler, using the machine's native scheduler or its fallback. The wake-up
calls `instrukt tick` every minute. There is no per-task
registration with an external scheduler and no tick-adapter layer.

`ticks: off` opts out of installation of the wake-up. A developer supplying
their own scheduler calls `instrukt tick` every minute. The configured runtime
supplies session execution; instrukt owns the task calendar.

`instrukt ticks status` shows the last wake-up and warns when no wake-up has
arrived for five minutes. It reports "no wake-up ever recorded" when none
has been recorded. The last wake-up is a row of the state database
(`project/instrukt/spec/feature/work/state-database`). A wake-up is evidence that the runner was invoked,
not evidence that any task finished. Completion follows the record contract
in `project/instrukt/spec/feature/runner/session-continuity`.

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
Given a developer has authorized scheduled work for their project
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

## Known caveats

None.
