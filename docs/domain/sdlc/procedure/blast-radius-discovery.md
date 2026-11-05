---
activation_trigger: Use when you are about to edit, refactor, or explore existing code beyond a trivial one-line change — before making the edit or concluding no dependents exist.
description: Derive the blast radius of an intended code change via the codebase Blueprint before editing existing code or drawing conclusions from manual exploration.
visibility: public
---

# Blast Radius Discovery — Procedure

## Required reads

- [sdlc/spec/codebase-blueprint](../spec/codebase-blueprint.md)

## Goal

Derive the dependents of an intended change from the latest Blueprint before
editing existing code or concluding that a symbol or file has no dependents.
What the graph is and which revision it reflects belong to
`sdlc/spec/codebase-blueprint`.

## Preconditions

- The change touches existing code (not a brand-new, standalone file with no callers).
- The `blueprint` module is on for the project (see the codebase-blueprint spec).

## Steps

1. Run `instrukt blueprint affected <changed-files>` for every file the
   change touches. It resolves the files' nodes and names the revision its
   answer reflects.
2. Use the answer as given: it comes from the latest graph, and you never check
   its freshness (`sdlc/spec/codebase-blueprint`). When it reports that no graph
   exists yet, state the gap and explore the affected files manually.
3. Treat the returned dependents as candidates, not an oracle. Verify each
   against the source and decide relevance; bias to over-inclusion — a missed
   dependent is a build-time surprise, an extra candidate is a discarded read.
4. Check whether a Blind spot (per the codebase-blueprint spec) applies to the
   area you're editing — dynamic dispatch, registry/string-keyed wiring,
   config-driven behavior, cross-language boundaries, reflection. Name the
   categories that apply and read those areas directly. An empty or thin
   TypeScript result is low-confidence, not evidence of no dependents.
5. For reachability beyond direct dependents, use
   `instrukt blueprint path <A> <B>`; for seed discovery,
   `instrukt blueprint query "<terms>"` or `instrukt blueprint explain "<symbol>"`
   where the configured adapter offers them. When the intended change starts
   from a use case rather than a source file, run
   `instrukt specs trace <spec_id> <UC-id>` for the tests binding it and the
   code they reach. To see the use cases the change puts at stake, run
   `instrukt specs trace --file <a> [--file <b> …]` with every file the change
   touches; it groups the use cases by the folder of their spec. Trace links
   are evidence, not proof that every reached symbol implements the behavior.
6. Ground your change surface — or your plan, or your understanding of the
   codebase — in the verified candidate set from steps 3–4.

## Outputs

- A verified dependent set, or an explicit stated gap when the Blueprint is
  unavailable or unsupported for the area — grounding the change surface.

## Recovery

- If the CLI reports the Blueprint unavailable, do not attempt to
  regenerate it. State the gap and fall back to manual `grep`/exploration,
  noting the fallback explicitly rather than silently proceeding as if no
  dependents exist.

## Discipline

Blast-radius discovery gets skipped for two reasons: the agent doesn't know
this procedure exists, or the agent treats a thin/empty result as proof of no
dependents rather than a blind-spot signal. Both produce the same failure — a
change lands, breaks an unseen caller, and nobody grepped for it. Run this
before concluding a file is unreferenced or safe to change; an empty result
from a Blind-spot area is not evidence of safety.
