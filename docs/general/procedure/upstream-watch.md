---
description: Report watched upstream changes and turn an accepted reconciliation into reviewed work that preserves local modifications.
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

The developer sees what changed upstream and can accept a reviewed
reconciliation that preserves the fork's local modifications.

## Preconditions

- The runner delivered `runner.upstream_changed` for a project, remote, ref
  and observed revision.
- The project's checkout and upstream history are accessible.

## Steps

1. Read the event's project, remote, ref and revision. Inspect current upstream
   history and the fork's local changes. Reconcile a repeated event against
   existing work before proposing or filing anything.
2. Report the upstream changes and their likely effect on the fork. Identify
   local modifications the reconciliation must preserve and offer to reconcile
   through the project's reviewed todo workflow. The event supplies information;
   the human supplies the decision to reconcile.
3. On one affirmative answer, record that intent with the upstream revision and
   preservation of local modifications. Apply the creation policy's admission
   and duplicate checks, then file bounded work with
   `instrukt todo create <slug> <content> --kind change --approved`, passing
   content as one quoted argument. That answer supplies the reconciliation
   intent; carry it into the todo without asking for it again.
4. Route the item through `instrukt todo prepare <slug>` and the normal
   build, independent review and integration path. Keep the local modifications
   in the work's recorded scope; conflicts are reviewed as part of that work.
   Report the todo slug and the upstream revision it addresses.

## Outputs

- An upstream-change report naming its observed revision and local impact.
- On acceptance, one recorded reconciliation todo carrying the preservation
  requirement and the normal review obligations.

## Recovery

- If history or the checkout is unavailable, report the missing evidence before
  offering a concrete reconciliation.
- If an equivalent todo already exists, use that record and report its slug.
- If the human has not accepted, leave the offer in its conversation. A later
  upstream event is compared with current history and existing work.
