---
description: Use-case traceability through the tests — from a use case to the tests binding it and the code they reach in the latest graph, and from a file back to the tests reaching it and the use cases they bind; evidence, not proof, and no use-case ids in production code.
delivered_by: [instrukt-first-build]
---
# Use-Case Trace — Spec

## What it is

`instrukt specs trace` connects what a project promises to the code that keeps
the promise, without writing use-case ids into production code. The connection
runs through the tests: a use
case is bound by tests, and the latest structural graph says what those tests
reach. Read the other way, a file is reached by tests, and their bindings name
the use cases at stake when the file changes. Trace is available only when the
blueprint module is on.

## Canonical fields

### Inputs

- **Bindings and use cases** — the specs context's published inventory and binding set
  (`local/projects/instrukt/spec/feature/specs/use-case-coverage`).
- **Reach** — the latest complete graph of the project and its traversal over
  dependency links, supplied by blueprint
  (`local/projects/instrukt/spec/domain/blueprint`).

### Directions

| From | Answer |
| --- | --- |
| a use case (`<spec_id> <UC-id>`) | the tests binding it, and the code those tests reach: graph nodes outside test files, reached by following dependency links forward from the tests' nodes |
| a file | the tests that reach it: test nodes from which the file's nodes are reachable over dependency links, with the use cases those tests bind |

- Reverse trace is invoked with `instrukt specs trace --file <path>`.
- A test is located by its own node when the graph has it, otherwise by its file.
- Every answer names the revision (SHA) of the graph it was computed from.
  The caller never checks freshness (`local/projects/instrukt/spec/feature/blueprint/blast-radius`).
- A use case that no binding names answers with no tests and says so.
- An unknown spec id or use-case id is refused as unknown.
- With no graph for the project yet, the answer says so and lists the binding
  tests only. With the blueprint module off, the command says the module is
  off.

### Use cases

#### UC-ST1: An agent changing a use case for a person finds the code its tests reach

```gherkin
Given a project whose tests bind a use case and whose structure has been mapped
When an agent working for a person traces that use case
Then it sees the tests that bind the use case and the code they reach
And it sees which revision the answer reflects
```

#### UC-ST2: An agent changing a file for a person finds the use cases at stake

```gherkin
Given a project whose tests bind use cases and whose structure has been mapped
When an agent working for a person traces a file it is about to change
Then it sees the tests that reach the file and the use cases they bind
And it sees which revision the answer reflects
```

## Known caveats

- A trace is evidence, not proof: a test also reaches helpers and shared code,
  and reach over the graph is only as complete as the engine's extraction.
- File fallback includes reach from other tests in that file.

## See Also

- docs/local/spec/domain/specs.md — the specs context.
- docs/local/spec/domain/blueprint.md — the graph and its traversals.
