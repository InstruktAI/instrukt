---
description: The position index — one `index.yaml` per position choosing which snippets are rendered in full (`expanded`), which as one line (`listed`), a domain's description, and `general`'s per-agent snippets (`agents`), with the findings and notices validation reports on it.
delivered_by: [instrukt-first-build]
---

# Position Index — Spec

## What it is

Each position chooses what agents read from it with one `index.yaml` at its
root: `docs/general/index.yaml`, `docs/domain/<x>/index.yaml` and
`docs/project/index.yaml`. The index names snippets by id; it says what is
rendered in full and what is listed as one line, and a domain's index says in
one line what the domain is about. It replaces baseline manifests, bundles and
the domain roster. What publication renders from these indexes into the global
agent files and the project root `AGENTS.md` is publication's contract
(`project/instrukt/spec/feature/publication/agent-entrypoints`); what
`instrukt docs get --domain` renders is retrieval's
(`project/instrukt/spec/feature/docs/progressive-retrieval`). This spec
states what an index may say and what validation reports about it.

## Canonical fields

### Keys per position

| Position | Allowed keys |
| --- | --- |
| `docs/general/index.yaml` | `expanded`, `listed`, `agents` |
| `docs/domain/<x>/index.yaml` | `description`, `expanded`, `listed` |
| `docs/project/index.yaml` | `expanded`, `listed` |

- **`expanded`** — snippet ids rendered in full, each with its closure: its
  transitive required reads, each once.
- **`listed`** — snippet ids rendered as one listing line each: id,
  description and the command that reads it.
- **`description`** — one line saying what the domain is about; the line that
  stands for the domain in every agent's global file.
- **`agents`** — a map from a supported agent CLI (`claude`, `codex`,
  `gemini`, `deepseek`) to snippet ids. Those snippets are rendered into that
  one CLI's global file only, after everything else.

`general`'s `expanded` and `listed` may name ids from any domain: always-on
content from several domains is chosen in this one place. A domain's index and
the project's index name only snippets of their own position.

An index is the only one in its position. Depth comes from required reads — a
listed overview's closure — never from nested indexes.

### Findings

Reported by `instrukt docs validate` and, through validation, by the sync that
publishes the position, which refuses on them
(`project/instrukt/spec/feature/publication/project-publication`). Each
names the `index.yaml`, the key and the offending value.

| A finding when |
| --- |
| a key the position does not allow appears, `agents` outside `docs/general/index.yaml` among them |
| a value has the wrong shape: `expanded`, `listed` or an `agents` entry that is not a list of ids, a `description` that is not one non-empty line |
| a domain's `index.yaml` has no `description` (the line the global agent files show for the domain) |
| an id names no snippet, or names a pending snippet |
| a domain's or the project's index names an id of another position |
| an `agents` key is not a supported agent CLI |
| a `listed` id is already in the closure of `expanded`; the finding names the `expanded` id whose closure carries it |
| an `index.yaml` sits anywhere but a position's root |

### Notices

A notice is reported with its file and code and never fails the run.

- **A domain without `index.yaml`.** The domain is absent from the global
  agent files and found only through `instrukt docs index --domain <x>`.
- **An unreferenced snippet** is reported by snippet validation
  (`project/instrukt/spec/feature/docs/snippet-validation`).

## Use cases

### UC-PI1: A publisher learns what their index cannot say before publishing

```gherkin
Given a developer has written the index of a position they publish
When they validate the project's documentation
Then every key the position does not allow, every id that names no delivered snippet, and every agent name instrukt does not deploy to is reported with the index and the offending value
And a domain's index without a description is reported
And a listed snippet that the expanded snippets already bring in is reported, naming the expanded snippet that brings it
And the run reports failure while any of these remains
```

### UC-PI2: A publisher is told a domain has no index

```gherkin
Given a developer publishes a domain that has no index
When they validate the project's documentation
Then they are told the domain has no index and is reachable only by listing its snippets
And the run does not fail on that alone
```

## Known caveats

- An `agents` snippet is not checked for type: by convention it is a `policy` snippet.

## See Also

- [project/instrukt/spec/domain/docs](../../domain/docs.md)
- [project/instrukt/spec/feature/docs/progressive-retrieval](progressive-retrieval.md)
- [project/instrukt/spec/feature/docs/snippet-validation](snippet-validation.md)
- [project/instrukt/spec/feature/publication/agent-entrypoints](../publication/agent-entrypoints.md)
