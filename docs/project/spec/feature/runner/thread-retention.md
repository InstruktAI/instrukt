---
description: The runner archives the finished agent-runtime threads of registered projects once they have been quiet past a retention window a person chooses, and never archives pinned threads, standing responsibilities' threads or threads of open work.
delivered_by:
  - archive-finished-threads
---
# Thread Retention — Spec

## What it is

Thread retention keeps a computer's agent-runtime thread list from filling with
long-finished threads. Workflows finish a completed thread into the runtime's
quiet, readable and wakeable state; the runner later archives it once it has
been quiet for the retention window the computer's owner chose. Archiving is a
runner decision carried out through the selected runtime's adapter: the runner
decides which thread is archived and when, and the runtime performs the archive
and owns everything else in the thread's lifecycle
(`project/instrukt/spec/domain/runner`).

## Canonical fields

### Retention window

The window is the global runner setting `runner.archive_finished_after`,
whose admitted values belong to
`project/instrukt/spec/feature/configuration/layered-configuration`. It is
either a duration or off. While it is off, as it is until a person sets a
window, the runner archives no thread. The window has the same meaning on every
runtime.

### Eligible threads

On each computer, the runner considers the threads its selected runtime lists
for each registered project there. A thread the runtime does not list for a
project is not considered.

A thread is eligible when the runtime reports it finished and quiet — not
running, queued, snoozed, or waiting on a person's input or approval — and both
its finishing and its latest activity are at least the window old. Activity
after finishing, such as a person reading or writing to the thread, restarts
its quiet time. A thread with no known finishing time or activity time is not
eligible.

Eligibility does not depend on what the thread was for: a finished thread of a
delivered or withdrawn todo and a finished ad-hoc thread, including one a
person started themselves, are archived under the same rule.

### Threads that are kept

Whatever its age, a thread is never archived when:

- the person pinned it in the runtime;
- it is the thread of a standing responsibility the runner keeps a binding
  for, which automation never finishes or archives
  (`project/instrukt/spec/feature/runner/procedure-standing-agent`); or
- it is bound to a todo that has not reached a terminal outcome, delivered or
  withdrawn, as the work record decides (`project/instrukt/spec/domain/work`).

The runner archives only threads that are already finished. It never finishes
or deletes a thread. Archived threads remain within history extraction
(`project/instrukt/spec/feature/runner/history`).

### Pace

Each runner pass archives at most a bounded number of threads and leaves the
rest eligible for later passes. A backlog of thousands of
eligible threads therefore drains over successive passes, and the runtime keeps
serving people and instrukt's other work while it does.

### Runtime capability

Archiving a thread is a capability a runtime adapter states in instrukt's own
terms, and a feature needing it is available only on a runtime that states it
(`project/instrukt/spec/feature/runner/session-continuity`). Archiving a thread that is already archived
counts as done.

A window set on a runtime whose adapter cannot archive is a configuration
finding naming the runtime and the missing capability, wherever configuration
enters (`project/instrukt/policy/checks-and-guards`). The runner's dispatch,
event work and other maintenance do not depend on it. A runtime that is
unreachable during a pass leaves its eligible threads for a later pass.

### Mechanical work

The sweep runs on runner invocations beside the runner's other mechanical
maintenance, after dispatch and event work, so no pickup, event reaction or
other command waits on it; an `instrukt tick` returns once its own sweep is
done. It starts no agent session, sends no agent a message and
emits no event. It keeps no record of its own: which threads are eligible is
read again on every pass from the runtime, the work record and the runner's
standing bindings (`sdlc/policy/state-storage`). The runner's diagnostics show
what each pass archived and each archive that failed, with its cause.

## Use cases

### UC-TR1: Long-quiet finished threads leave the thread list on their own

```gherkin
Given a developer has set a retention window on their computer
And a registered project has a finished thread that has been quiet for longer than that window
When the runner next runs its maintenance there
Then that thread is archived without anyone asking for it
But a thread the developer pinned stays in the thread list
And a standing responsibility's thread stays in the thread list
And a thread bound to a todo that is not yet delivered or withdrawn stays in the thread list
And a thread that was active more recently than the window stays in the thread list
```

## Known caveats

- Native child threads that the runtime omits from a project's listing are not
  archived.
- The occurrence threads of a recurring task stay bound to their task while it
  stays active, so they are kept for as long as the task recurs.
- Company homes are not registered projects, and their conversation sessions
  are outside the sweep.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
