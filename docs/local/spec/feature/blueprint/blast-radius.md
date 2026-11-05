---
description: Blast radius from the latest graph — changed files to every dependent reachable backwards over dependency links, naming the graph's SHA, listing changed files the graph does not know, and saying so when there is no graph yet.
delivered_by: [instrukt-first-build]
---
# Blast Radius — Spec

## What it is

Before changing code, an agent needs to know what depends on it.
`instrukt blueprint affected <files…>` answers from the latest graph: every
part of the project that depends, directly or through others, on what the
changed files define. The answer names the revision it reflects; the agent uses
it as it is and never checks freshness. Where the graph cannot answer, the
command says so and the agent explores by hand.

## Canonical fields

### Computation

1. **Changed files.** Each argument is a path relative to the root of the
   repository or worktree the caller stands in; an absolute path inside that
   root is made relative. At least one file is required.
2. **Seeds.** Every node whose `source_file` is a changed file.
3. **Traversal.** From the seeds, follow dependency links backwards (from a
   link's `target` to its `source`), without a depth limit, visiting each node
   once. Only the dependency relations count
   (`local/projects/instrukt/spec/domain/blueprint`).
4. **Dependents.** The reached nodes that are not seeds and whose
   `source_file` is not a changed file.

### Answer

| Part | Content |
| --- | --- |
| revision | the SHA of the latest graph |
| dependents | one line per dependent: its file, with line when the node carries a location, and its label (its id when it carries none); ordered by file, line, label |
| none found | a statement that no dependents were found for the given number of changed files |
| unknown files | the changed files that define no node in the graph, to be read by hand |

- **No graph yet** for the project: the answer says the blueprint is
  unavailable and that the agent explores by hand. This is an answer, not an
  error.
- **Unreadable graph** (missing or not satisfying the contract): the same
  unavailable answer.
- **Module off:** the command says the blueprint module is off.

### Use cases

#### UC-BA1: An agent changing files for a person learns what depends on them

```gherkin
Given a project whose latest pushed revision has been mapped
When an agent working for a person asks what depends on the files it is about to change
Then it receives the dependents represented in the map that are reachable from those files, directly or indirectly
And it sees which revision the answer reflects
And it sees which of its files the map does not know
```

Extension of UC-BA1: with no mapped revision, the agent is told no map is
available and to explore by hand; no dependents are presented as an answer.

## Known caveats

- The answer reflects the latest pushed main, not the caller's branch: code
  the caller added since is unknown to the graph and appears among the
  unknown files.
- Reach is over the engine's extracted links; dependencies the engine does not
  see (dynamic dispatch, configuration, generated code) are absent.

## See Also

- docs/local/spec/domain/blueprint.md — the blueprint context.
- docs/local/spec/feature/blueprint/use-case-trace.md — the use cases the same changed files put at stake.
- ~/.instrukt/docs/sdlc/procedure/blast-radius-discovery.md — how agents use the answer.
