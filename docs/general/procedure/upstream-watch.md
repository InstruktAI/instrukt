---
description: Assess watched upstream changes against project needs, admit and route warranted work, and report the outcome.
visibility: public
role: upstream-watcher
subscriptions:
  - runner.upstream_changed
subscription_home: project
interruptible: false
---
# Upstream Watch — Procedure

## Required reads

- [general/procedure/event-handling/overview](event-handling/overview.md)
- [sdlc/policy/todo-creation-discipline](../../domain/sdlc/policy/todo-creation-discipline.md)

## Goal

Upstream changes are assessed against the project's contracts and relevant
work reaches its owning delivery path, with the assessment and disposition
reported.

## Preconditions

- The runner delivered `runner.upstream_changed` for a project, remote, ref
  and observed revision.
- The project's checkout and the relevant upstream history are accessible.

## Steps

1. Read the event's project, remote, ref and revision. Read the project's fill
   of the extension point below when it exists, using its id from the project
   baseline. When none exists, include that fact in the report and ground the
   assessment in the project's available contracts and dependency evidence.
2. Inspect the upstream change against the project's affected contracts,
   including the language-specific contracts the change engages. Establish
   which consumed surfaces changed and what adaptation, if any, the project
   needs. For a fork reconciliation, identify the local modifications the work
   must preserve.
3. Apply admission, evidence and duplicate checks before filing or amending
   work (`sdlc/policy/todo-creation-discipline`). Reconcile repeated events
   with existing work. Record the observed revision, affected contracts and
   bounded outcome in the appropriate artifact. File warranted work through
   that policy's admitted route; an assessment requiring no change records
   that disposition in the report.
4. Route admitted work through its owning project's normal preparation,
   build, independent review and integration path. Carry any fork-preservation
   requirements into that work. Report the assessment, disposition, routed
   work addresses and upstream revision. Reach the human only for input or
   exposure reserved to them (`general/policy/human-contract`), continuing
   the settled work while that part remains open.

### Project extension point

`project/<slug>/spec/upstream-watch-focus` — the upstream dependencies,
consumed surfaces and project contracts to assess when this procedure runs.

## Outputs

- An upstream-change report naming the observed revision, project focus used
  or its absence, affected contracts, assessment and disposition.
- Addresses of existing or newly admitted work and its routing, including
  preservation requirements for a fork reconciliation.

## Recovery

- If history, the checkout or a declared focus fill cannot be read, report
  the missing evidence and the assessment that remains unfinished.
- If equivalent work already exists, use its admitted amendment or routing
  path and report its address.
- If a human-owned question remains, report that question and the affected
  portion of work under `general/policy/human-contract`.
