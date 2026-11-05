---
description: Use-case traceability through the tests — from a use case to the tests binding it and the code they reach in the latest graph, and from one or more files back to the use cases at stake, grouped by the folder of their spec; evidence, not proof, and no use-case ids in production code.
delivered_by: [instrukt-first-build]
---
# Use-Case Trace — Spec

## What it is

`instrukt specs trace` connects what a project promises to the code that keeps
the promise, without writing use-case ids into production code. The connection
runs through the tests: a use
case is bound by tests, and the latest structural graph says what those tests
reach. Read the other way, the files of a change are reached by tests, and
their bindings name the use cases at stake when those files change, grouped by
the folder of the spec that holds them. Trace is available only when the
blueprint module is on.

## Canonical fields

### Inputs

- **Bindings and use cases** — the specs context's published inventory and binding set
  (`local/projects/instrukt/spec/feature/specs/use-case-coverage`); the
  inventory carries each use case's spec file.
- **Reach** — the latest complete graph of the project and its traversal over
  dependency links, supplied by blueprint
  (`local/projects/instrukt/spec/domain/blueprint`).

### Directions

| From | Answer |
| --- | --- |
| a use case (`<spec_id> <UC-id>`) | the tests binding it, and the code those tests reach: graph nodes outside test files, reached by following dependency links forward from the tests' nodes |
| one or more files | the use cases at stake: every use case bound by a test from which a node of any given file is reachable over dependency links, grouped by the folder of its spec |

- Reverse trace is invoked with `instrukt specs trace --file <path> [--file <path> …]`.
  Each path is relative to the root of the repository or worktree the caller
  stands in; an absolute path inside that root is made relative. A file given
  twice counts once. `--file` and a use case are never given together.
- **Grouping.** One group per spec folder: the repository-relative folder that
  holds the spec file, such as `docs/project/spec/feature/blueprint/`. Groups
  are ordered by folder. Under its group, each use case at stake appears once,
  as `<spec_id> <UC-id>`, with the tests binding it that reach any of the given
  files; use cases are ordered by spec id and use-case id. The folder is the
  only thing instrukt groups by; it attaches no meaning to it (under sdlc's
  convention the folder is the bounded context).
- **Files without use cases.** The given files that no binding test reaches
  are listed after the groups, so the caller knows the trace says nothing
  about them. Files the graph does not know are among them.
- A test is located by its own node when the graph has it, otherwise by its file.
- Every answer names the revision (SHA) of the graph it was computed from.
  The caller never checks freshness (`local/projects/instrukt/spec/feature/blueprint/blast-radius`).
- A use case that no binding names answers with no tests and says so.
- An unknown spec id or use-case id is refused as unknown.
- With no graph for the project yet, a use-case trace says so and lists the
  binding tests only; a file trace says the blueprint is unavailable and that
  the caller explores by hand. With the blueprint module off, the command says
  the module is off.

### Use cases

#### UC-ST1: An agent changing a use case for a person finds the code its tests reach

```gherkin
Given a project whose tests bind a use case and whose structure has been mapped
When an agent working for a person traces that use case
Then it sees the tests that bind the use case and the code they reach
And it sees which revision the answer reflects
```

#### UC-ST2: An agent changing files for a person finds the use cases at stake

```gherkin
Given a project whose tests bind use cases and whose structure has been mapped
When an agent working for a person traces the files it is about to change
Then it sees each use case those files put at stake once, with the tests that reach the files
And it sees the use cases grouped by the folder of the spec that holds them
And it sees which of its files no test binding a use case reaches, so the trace offers no use-case evidence for them
And it sees which revision the answer reflects
```

## Known caveats

- A trace is evidence, not proof: a test also reaches helpers and shared code,
  and reach over the graph is only as complete as the engine's extraction.
- File fallback includes reach from other tests in that file.

## See Also

- docs/local/spec/domain/specs.md — the specs context.
- docs/local/spec/domain/blueprint.md — the graph and its traversals.
- docs/local/spec/feature/blueprint/blast-radius.md — the structural dependents of the same changed files.
