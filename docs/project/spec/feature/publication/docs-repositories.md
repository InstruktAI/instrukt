---
description: Docs repositories — instrukt ships no doctrine; an adopted docs repository publishes it through its `docs/general/` and `docs/domain/<x>/` positions, one owner per position per computer, instrukt enforces its format and never depends on its wording, and anyone may fork the shipped one or bring their own.
delivered_by: [instrukt-first-build]
---
# Docs Repositories — Spec

## What it is

instrukt ships no documentation of its own to adopters. Doctrine — the
principles, policies, procedures, briefs, workflow definitions and the eval
report procedure that agents follow — comes from a docs repository: an
ordinary project, adopted with `instrukt init`, that carries the published
positions `docs/general/` and `docs/domain/<x>/`. Our docs repository
publishes `general` and `sdlc`; anyone may fork it, change it, or bring a
repository of their own. instrukt publishes whichever docs repositories a
computer has adopted and validates their format; its code never depends on
their wording.

## Canonical fields

### A docs repository

- A docs repository is an adopted project whose committed main holds
  `docs/general/`, one or more `docs/domain/<x>/`, or both. Holding a position
  is what publishes it; no setting declares it. A domain that defines a
  workflow carries it in `docs/domain/<x>/workflow/`
  (`project/instrukt/spec/feature/workflow/workflow-definition`).
- A domain is a body of knowledge one publisher is the authority on,
  published as one unit — its own schema, its own index, optionally its own
  workflow — named after what it is about, never after a category it belongs
  to. `general` and `project` are reserved and never a domain's name, nor is
  `third-party`, which keeps its existing special case. Agents
  find a domain by its `description` and its procedures' activation
  triggers; there are no tags.
- A computer obtains a docs repository by cloning it and adopting it. instrukt
  bundles, downloads and fetches no doctrine; the repository's own sync
  publishes its positions into `~/.instrukt/docs/`, triggered like any
  project's.
- Each domain, and `docs/general/`, has one owner on a computer. A second
  repository claiming either is refused and the refusal names the current
  owner (`project/instrukt/spec/feature/publication/project-publication`).
  Ownership is recorded internally under `~/.instrukt/state/`; no repository
  authors it.
- The repository that owns `docs/general/` renders the global agent files
  (`project/instrukt/spec/feature/publication/agent-entrypoints`).

### What a project takes from a docs repository

| Need | Comes from |
| --- | --- |
| Always-on doctrine in every agent's global file | the `docs/general/` owner's `index.yaml` selection |
| A domain's core set in its own docs | a required read naming the domain by its bare name, meaning that domain's `expanded` set, or a required read naming a snippet id (`project/instrukt/spec/domain/docs`) |
| Its workflow, when `todos` is on | the published domain named by `workflow.domain` (default `sdlc`) |
| The roles its procedures and dispatch use | that domain's `workflow/actions.yaml` `roles:` |
| Its starting `docs/project/schema.yaml` | the published `sdlc` domain's `schema.yaml`, copied once by `instrukt init` (`project/instrukt/spec/feature/repository/project-initialization`) |
| Retrieval during work | `instrukt docs index`, `instrukt docs get <id>` and `instrukt docs get --domain <x>` over everything published on the computer |

A project's own `docs/project/` stays in the project; it may reference
published doctrine but never publishes into it.

### The boundary

- instrukt validates what a docs repository publishes: the snippet format,
  each position's `index.yaml` and `schema.yaml`, the workflow format and its
  roles. A repository that does not fit — including one written for a newer
  instrukt — is refused at sync with its findings, and the previous
  publication stays.
- Docs name instrukt's commands; an agent reads a command's contract from the
  installed binary with `instrukt <command> --help`. instrukt reads a docs
  repository only through its declared formats — frontmatter, required reads,
  `index.yaml`, `schema.yaml`, configured-value directives and workflow files
  — never through its prose.
- Published documentation runs nothing on the computer that reads or publishes
  it (`project/instrukt/spec/feature/docs/progressive-retrieval`).
- A project that needs a domain no adopted repository publishes on the computer
  — a required read into it or naming it, or a `workflow.domain` naming it —
  is told which domain is missing and that a docs repository publishing it
  must be adopted.

### Bringing your own

A developer replaces a domain by forking the repository that publishes it, or
by authoring their own. To switch owners on a computer, the current owner
removes the position from its main (its retraction releases the claim) and
the new repository then publishes it. A docs repository may also publish
domains under names of its own; a project selects such a workflow with
`workflow.domain`.

### instrukt's own repository

instrukt's repository is a consumer like any adopting project: it carries
`docs/project/` and takes its doctrine from whichever docs repository
publishes it on the developer's computer.

### Use cases

#### UC-DOR1: A developer gives their computer its doctrine

```gherkin
Given a developer has instrukt and a docs repository holding always-on doctrine and doctrine domains
When they clone that docs repository and adopt it
Then its doctrine is published on their computer
And every supported agent on the computer starts with the always-on doctrine and a line for each domain
And agents in the developer's other projects read that doctrine
```

#### UC-DOR2: A developer brings their own doctrine

```gherkin
Given a developer's computer publishes a domain from one docs repository
When they adopt their own docs repository holding the same domain
Then its publication is refused and names the current owner
When the first repository removes that domain and their own repository publishes it
Then their projects read their own repository's version of the domain
```

#### UC-DOR3: A project needing doctrine the computer lacks is told what to adopt

```gherkin
Given a developer's project needs a domain that no adopted docs repository publishes on their computer
When they validate or publish the project
Then they are told which domain is missing
And that a docs repository publishing it must be adopted
```

## Known caveats

- Until our docs repository is split out of instrukt's repository, instrukt's
  repository is that docs repository and carries `docs/general/` and
  `docs/domain/sdlc/` itself.

## See Also

- project/instrukt/spec/domain/publication
- project/instrukt/spec/feature/publication/project-publication
- project/instrukt/spec/feature/publication/agent-entrypoints
- project/instrukt/spec/feature/workflow/workflow-definition
