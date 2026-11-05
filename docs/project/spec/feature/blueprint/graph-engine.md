---
description: The engine behind the blueprint — the minimal node-link graph contract, the one shipped adapter, and a developer's own engine as a command configured in the global layer emitting the contract JSON; output that breaks the contract is never served.
delivered_by: [instrukt-first-build]
---
# Graph Engine — Spec

## What it is

The blueprint does not extract structure itself; an engine does. Any engine
whose output satisfies a small node-link contract can serve, so a developer is
not tied to the one instrukt ships. instrukt ships one adapter and accepts a
configured command in its place; agents never see which is in use.

## Canonical fields

### The graph contract

A graph is one JSON object:

| Field | Shape | Required members |
| --- | --- | --- |
| `nodes` | list of objects | `id` (string), `source_file` (string, repository-relative) |
| `links` | list of objects | `source` (node id), `target` (node id), `relation` (string) |

- Members beyond these are allowed and kept; answers instrukt computes itself
  use only the required members. A node may carry `label` and
  `source_location` (`L<line>` or `L<line>-<line>`); answers show them when
  present.
- `relation` values that answers traverse are the dependency relations
  (`project/instrukt/spec/domain/blueprint`). Links with other
  relations are kept and not traversed for blast radius.
- A graph is refused when it is not an object, when `nodes` or `links` is
  missing or not a list, or when any `source_file` is not a string, is
  absolute, contains a `..` segment or contains a backslash. A refused graph is
  a failed generation (`project/instrukt/spec/feature/blueprint/graph-generation`).
- Nodes or links that are not objects, or that lack a required member, are
  skipped by answers rather than refusing the graph.

### Engine selection

`blueprint.engine` selects the engine
(`project/instrukt/spec/domain/configuration`). Either layer may name a shipped
adapter. The command form is permitted only in the global layer: a command in a
project layer is a validation finding and is ignored, as project-layer
`runner.runtime` is.

| Value | Engine |
| --- | --- |
| `graphify` | the shipped adapter; it offers `query` and `explain` |
| a command (global layer only) | the developer's own engine; it offers only what the contract supports (`affected`, `path`) |

- The engine runs against the committed tree of the revision being mapped,
  never the caller's working tree.
- A second shipped adapter is added only when an engine proves widely used.

- The configured command receives the path of a checkout of the revision as its argument and writes contract JSON to stdout. A non-zero exit is a failed generation.

### Use cases

#### UC-BE1: A developer maps their project with an engine of their own

```gherkin
Given a developer whose own tool describes their project's structure in the node-link form instrukt reads
And they configured that tool as the engine
When a revision of their project is pushed to main
Then agents' structural answers for the project come from that tool's output
And agents ask the same questions they would ask of the shipped engine
```

Extension of UC-BE1: invalid engine output is never served as the project
map; agents continue receiving answers from the previous complete map.

## Known caveats

- The engine's extraction quality bounds every answer; instrukt validates
  shape, not completeness.

## See Also

- [project/instrukt/spec/domain/blueprint](../../domain/blueprint.md) — the blueprint context.
