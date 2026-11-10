---
description: Reservation of use cases before the build — a todo's planned bindings are checked against the pending markers that already claim use cases for other todos, so two todos never bind the same use case.
---
# Binding Reservation — Spec

## What it is

Before a build binds a use case, its todo reserves it in the preparation worktree. Integration publishes that ownership on main, preserving it independently of pending visibility, so a concurrent todo sees the claim and reuses or
authors a distinct use case instead of adding a second binding
(`sdlc/policy/testing`). `instrukt specs reserve` is the check: given the
requesting todo's slug and the bindings its plan declares, it reports every
binding another todo already holds. The command reads; the claim itself is the
marker the todo authors.

## Canonical fields

### The claim

- An explicit `binding_reservations` entry claims its use case; otherwise a use case is claimed by the slug of the pending marker that encloses its
  definition line: a `planned:<slug>` block (a newly authored use case) or a
  `planned-change:<slug>` block (an existing use case restated unchanged, or
  changed). Preparation folding activates documentation while preserving each unbound use case's ownership in frontmatter `binding_reservations: {UC-id: todo-slug}`. This metadata is independent of pending visibility. Work-delivery folding clears an owned reservation only when an attached test binding exists in the candidate. The marker grammar is owned by docs
  (`project/instrukt/spec/domain/docs`).
- Whole-file `delivered_by` claims all use cases only when it names exactly
  one delivering slug. With several slugs, claims come from in-body pending
  markers; a use case enclosed by no marker is unclaimed. Markers never nest.
- A whole file with several delivering slugs and no in-body markers is an
  ambiguous claim: reservation answers it as such with a non-zero exit, and
  coverage's `--check` fails on it.
- The inventory, spec ids and use-case ids are those of
  `project/instrukt/spec/feature/specs/use-case-coverage`.

### The check

`instrukt specs reserve --slug <slug> --binding <spec_id>:<UC-id>…`

| Requested binding | Answer |
| --- | --- |
| names no use case in the inventory | refused as unknown, before any claim is checked |
| unclaimed | clear |
| claimed by the requesting slug | clear |
| claimed by another slug | reported as reserved, naming the holding slug and the requester |
| in a whole file pending for several slugs with no in-body markers | reported as an ambiguous claim, naming the delivering slugs; non-zero exit |

- Unknown bindings are refused first, so a mistyped id never reads as clear.
- Exit status: zero when every requested binding is clear; one distinct
  non-zero status when any is reserved by another slug; another when any is
  unknown.
- A `--binding` value without the `:` separator is refused as malformed.
- The slug is checked for form only; whether it names an existing todo is
  checked only when the `todos` module is on.

## Use cases

### UC-SR1: An agent preparing work for a person confirms its planned use cases are free

```gherkin
Given an agent preparing a todo for a person, whose plan will bind several use cases
And none of them is held by another todo
When the agent checks its planned bindings
Then the check passes
```

Extensions of UC-SR1: if another todo holds a planned use case, the check
fails and names the use case and holding todo. If a planned binding names no
existing use case, it is refused as unknown and is never reported as free.

## Known caveats

- The check does not serialize two todos; it reports what is already on main.
  Two todos that check before either claims both pass.

## See Also

- [project/instrukt/spec/domain/specs](../../domain/specs.md) — the specs context.
- [sdlc/policy/testing](../../../../domain/sdlc/policy/testing.md) — reservation at prepare, on main, before the build binds.
