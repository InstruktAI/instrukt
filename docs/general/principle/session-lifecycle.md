---
description: 'Sessions are ephemeral; artifacts carry continuity. Whoever dispatches work ends its session once the outcome is recorded, retaining context for pending human conversations or later rounds.'
visibility: 'public'
---

# Session Lifecycle — Principle

## Principle

Sessions are ephemeral expressions; artifacts carry continuity. A session
exists for an outcome, and the work record carries that outcome beyond the
conversation. Once the outcome is recorded, whoever dispatched the work ends
the session with `instrukt sessions end <id>`. Retain it while later review
rounds or a pending human conversation still needs its context.

Completion of a workflow assignment, release of its claim, end of an execution
and loss of a session are separate facts. A step is done when the record carries
its required artifact and stamp, never because a reply says it is done.

## Rationale

A workflow can establish completion of one assignment; the agent holds the
current conversation and its remaining obligations. Treating one assignment
as the whole session can cut off a forming conversation or discard context
needed by a later review round.

A claim determines who drives the work next, while a session determines where
the conversation continues. Holding the claim after handing off the work stalls
its next owner. Ending a turn, by itself, proves neither handoff nor a completed
record write. Durable evidence lets future agents continue without pretending
to remember.

A session kept alive past its purpose is noise. It hides the sessions that
genuinely need attention and blurs what is working and what is done. A clean
session list tells a true story. Ending belongs to the one with the wider
view: whoever dispatched the work knows when the mission is complete.

## Implications

- **Workflow completion closes that assignment.** Assess other obligations,
  including unfinished human conversation, before departure.
- **An obligation keeps the session, never the claim.** Confirm release at
  handoff even if this session stays for another obligation.
- **Artifacts are the evidence trail.** Write outcomes that stand alone for
  the next reader. Conversations are read through
  `instrukt sessions conversation` when their evidence is needed; only artifacts
  and stamps are reflected into the work store.
- **Reuse follows recorded identity.** Preserve the session per todo and role;
  observe it with `instrukt sessions alive`, continue it through `send`, and
  start through `run` when a new session is required. Similar briefs establish
  no session identity. An ended turn may leave a reusable conversation.
- **Observation and completion differ.** `running`, `needs-input`, `ended ok`,
  `ended failed` and `gone` describe runtime execution. A failed observation
  establishes none of them. Artifact and stamp requirements still govern completion.
- **Incomplete evidence remains incomplete.** The runner sends once to the
  same session after an ended run without its artifact or stamp, or a gone
  session; continued absence is an anomaly. Silence alone authorizes no duplicate.
- **A scoped worker never ends itself.** A scoped worker records its outcome
  and waits for its dispatcher, which accounts for later rounds and pending
  human conversation before ending it with `instrukt sessions end <id>`.
  Starting a peer alone confers no authority over that peer's other obligations.
  A runtime without the end template reports the missing capability.
- **Standing roles persist under their mandate.** The standing integrator
  parks after current work settles and is woken by its subscribed events.
  Task recurrence is governed by its record and runner, not an improvised timer.
- **Human questions stay in their thread.** Ask for approval or missing intent
  there and leave it waiting as `needs-input`. Resolve agent-owned technical
  work with its owner; disagreement alone does not transfer it to the human.
- **Anomalies are referenced, not broadcast.** The runner may notify an
  unattended failure with a reference to its session, or its work-record hold
  when no session could start. The content remains at that source.

## Tensions

- **Signal vs. noise.** Observability invites keeping sessions alive; a true
  session list invites ending them. End after the recorded outcome, retaining
  context only for a pending human conversation or later rounds.

- **Ephemerality vs. continuity.** Finished execution can leave a useful
  conversation available. Retain that context without retaining a released claim.
- **Observation vs. evidence.** A successful run is useful evidence of execution;
  only the required record writes establish completion.
- **Autonomy vs. lifecycle control.** A worker owns its contribution while the
  governing role owns the wider handoff. Neither supplies runtime capabilities
  the mapping does not declare.
- **Iteration cost vs. human attention.** Agents resolve what they can; the
  human supplies only decisions and intent that are theirs.
