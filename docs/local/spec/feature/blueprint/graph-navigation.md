---
description: Navigating the latest graph — the path from one part to another over the contract's links under any engine, and free query and explain offered only where the configured engine supports them.
delivered_by: [instrukt-first-build]
---
# Graph Navigation — Spec

## What it is

Beyond blast radius, an agent sometimes needs to know how one part of a
project reaches another, or to ask the map a looser question. Path works on
the graph contract under any engine. Query and explain need what an engine
knows beyond the contract, so they are offered only when the configured
engine's adapter supports them. Every answer comes from the latest graph and
names its SHA.

## Canonical fields

### Commands

| Command | Works on | Answer |
| --- | --- | --- |
| `instrukt blueprint path <A> <B>` | the contract, any engine | the shortest chain of links leading from A to B, following link direction, each step with its relation; or that B is not reachable from A |
| `instrukt blueprint query` | the engine, where offered | the engine's answer to a free question, from the latest graph |
| `instrukt blueprint explain` | the engine, where offered | the engine's explanation of a node and its neighbours, from the latest graph |

- Path endpoints are a node id or a file path.
- Arguments to `query` and `explain` pass through to the adapter unchanged.
- Every answer names the SHA of the graph it came from.
- An endpoint of `path` that names no node is refused as unknown.
- Under an engine that does not offer `query` or `explain`, the command says
  the configured engine does not offer it. This is an answer, not an error.
- No graph yet, an unreadable graph and the module off are answered as for
  blast radius (`local/projects/instrukt/spec/feature/blueprint/blast-radius`).

### Use cases

#### UC-BN1: An agent working for a person finds how one part reaches another

```gherkin
Given a project whose latest pushed revision has been mapped
When an agent working for a person asks how one part of the project reaches another
Then it receives the chain of relations leading from the first to the second, or that there is none
And it sees which revision the answer reflects
```

## See Also

- docs/local/spec/domain/blueprint.md — the blueprint context.
- docs/local/spec/feature/blueprint/graph-engine.md — what an engine offers.
