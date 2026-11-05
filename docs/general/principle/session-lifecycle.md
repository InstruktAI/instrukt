---
description: 'Artifacts carry continuity; dispatchers record outcomes and handoffs, retain needed conversation context and leave session lifecycle to the runtime.'
visibility: 'public'
---

# Session Lifecycle — Principle

## Principle

Sessions are ephemeral expressions; artifacts carry continuity. A session exists
for an outcome, and the work record carries that outcome beyond the
conversation. Whoever dispatches work records its outcome and handoff, retaining
context needed by later review rounds or a pending human conversation. The
runtime owns session lifecycle, ending and cleanup.

Completion of a workflow assignment, release of its claim, end of an execution
and loss of a session are separate facts. A step is done when the record carries
its required artifact and stamp, never because a reply says it is done.

## Rationale

A workflow can establish completion of one assignment; the agent holds the
current conversation and its remaining obligations. Treating one assignment as
the whole session can cut off a forming conversation or discard context needed
by a later review round. instrukt records that assignment's handoff; the runtime
manages the conversation's lifecycle.

A claim determines who drives the work next, while a session determines where
the conversation continues. Holding the claim after handing off the work stalls
its next owner. Ending a turn, by itself, proves neither handoff nor a completed
record write. Durable evidence lets future agents continue without pretending
to remember.

A recorded handoff makes clear what is done and who acts next. Runtime execution
state alone cannot show whether an assignment's evidence is complete or a human
conversation still has an open question.

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
- **Workers record their contribution.** The dispatcher accounts for later
  rounds and pending human conversation before recording the handoff.
  Starting a peer confers no authority over that peer's other obligations.
- **A lane releases its claim before handoff.** Confirm `claim.held` is false
  after `instrukt todo update <slug> --unclaim`, then record the assignment's
  outcome and next owner. Session ending and cleanup belong to the runtime.
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

- **Signal vs. noise.** Keep recorded assignments and runtime observations
  distinct, so a reusable conversation does not appear to hold a released claim.

- **Ephemerality vs. continuity.** Finished execution can leave a useful
  conversation available. Retain that context without retaining a released claim.
- **Observation vs. evidence.** A successful run is useful evidence of execution;
  only the required record writes establish completion.
- **Autonomy vs. lifecycle control.** A worker owns its contribution while the
  governing role owns the wider handoff. Neither supplies runtime capabilities
  the adapter does not support.
- **Iteration cost vs. human attention.** Agents resolve what they can; the
  human supplies only decisions and intent that are theirs.
