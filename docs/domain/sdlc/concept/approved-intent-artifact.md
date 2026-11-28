---
description: "The input artifact carries authoritative intent for every todo kind; configured human-or-peer gates approve derived outcomes."
visibility: 'public'
---

# Approved Intent Artifact — Concept

## What

The **approved intent artifact** is the `input` artifact for every todo kind:
`feature`, `change`, `bug` and `task`. It records the outcome requested in the
originating conversation, its author and version. A seeded plan does not
replace `input`; product requirements and plans derive from it.

The mechanical author of an `input` version — such as a splitting agent or an
agent that relayed work — is distinct from the human's originating conversation
(`general/policy/human-contract`). For a split child, the originating
conversation is the human conversation that authorized the parent's intent, not
the splitting session.

For human-lineage work, entry comparison and when a revised `input` triggers a
fresh comparison are governed by `sdlc/policy/todo-creation-discipline`; once
accepted, `input` bounds every derived artifact and later recipients carry its
intent unchanged.

Approval moments use the configured human-or-peer gate. Approval of derived
requirements or a task plan binds that evidence; it does not replace `input`
as the authoritative intent record.

The artifact bounds the work's scope. A product outcome beyond what a `change`
or `bug` input authorizes enters the feature intent path rather than reaching
the plan.

## Why

One intent artifact for every kind lets each recipient ground in the same
record. Kind selects preparation, while entry comparison guards fidelity on
human-lineage work and configured approval gates authorize the reviewed outcome.
