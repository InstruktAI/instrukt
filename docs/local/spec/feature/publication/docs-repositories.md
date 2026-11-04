---
description: Docs repositories — instrukt ships no doctrine; an adopted docs repository publishes it, instrukt enforces its format and never depends on its wording, and anyone may fork the shipped one or bring their own.
delivered_by: [instrukt-first-build]
---
# Docs Repositories — Spec

## What it is

instrukt ships no documentation of its own to adopters. Doctrine — the
principles, policies, procedures, briefs, workflow definitions and the eval
report procedure that agents follow — comes from a docs repository: an
ordinary project, adopted with `instrukt init`, that publishes domains. Our
docs repository publishes `general` and `sdlc`; anyone may fork it, change
it, or bring a repository of their own. instrukt publishes whichever docs
repositories a computer has adopted and validates their format; its code never
depends on their wording.

## Canonical fields

### A docs repository

- A docs repository is an adopted project whose `publishes` setting lists the
  domains it owns. Their content is authored under `docs/global/<domain>/`; a
  domain that defines a workflow carries it in `docs/global/<domain>/workflow/`
  (`local/projects/instrukt/spec/feature/workflow/workflow-definition`).
- A computer obtains a docs repository by cloning it and adopting it. instrukt
  bundles, downloads and fetches no doctrine; the repository's own sync
  publishes its domains into `~/.instrukt/docs/`, triggered like any project's.
- Each domain has one owner on a computer. A second repository claiming a
  published domain is refused and the refusal names the owner
  (`local/projects/instrukt/spec/feature/publication/project-publication`).
- The repository that publishes `general` writes the global agent entrypoints
  (`local/projects/instrukt/spec/feature/publication/agent-entrypoints`).

### What a project takes from a docs repository

| Need | Comes from |
| --- | --- |
| The baseline in its compiled instructions | a published domain's bundle, named in its master's required reads (`@~/.instrukt/docs/<domain>/bundle.md`) |
| Its workflow, when `todos` is on | the published domain named by `workflow.domain` (default `sdlc`) |
| The roles its procedures and dispatch use | that domain's `workflow/actions.yaml` `roles:` |
| Retrieval during work | `instrukt docs index` and `instrukt docs get` over everything published on the computer |

A project's own `docs/local/` stays in the project; it may reference
published domains but never publishes into them.

### The boundary

- instrukt validates what a docs repository publishes: the snippet format, the
  workflow format and its roles. A repository that does not fit — including
  one written for a newer instrukt — is refused at sync with its findings, and
  the previous publication stays.
- Docs name instrukt's commands; an agent reads a command's contract from the
  installed binary with `instrukt <command> --help`. instrukt reads a docs
  repository only through its declared formats — frontmatter, required reads,
  configured-value directives and workflow files — never through its prose.
- Published documentation runs nothing on the computer that reads or publishes
  it (`local/projects/instrukt/spec/feature/docs/progressive-retrieval`).
- A project that needs a domain no adopted repository publishes on the computer
  — a required read into it, or a `workflow.domain` naming it — is told which
  domain is missing and that a docs repository publishing it must be adopted.

### Bringing your own

A developer replaces a domain by forking the repository that publishes it, or
by authoring their own. To switch owners on a computer, the current owner stops
publishing the domain (its retraction removes it) and the new repository then
publishes it. A docs repository may also publish domains under names of its
own; a project selects such a workflow with `workflow.domain`.

### instrukt's own repository

instrukt's repository is a consumer like any adopting project: it carries
`docs/local/` and takes its doctrine from whichever docs repository publishes
it on the developer's computer.

### Use cases

#### UC-DOR1: A developer gives their computer its doctrine

```gherkin
Given a developer has instrukt and a docs repository that publishes doctrine domains
When they clone that docs repository and adopt it
Then its domains are published on their computer
And every supported agent on the computer starts with the doctrine's global baseline
And agents in the developer's other projects read that doctrine
```

#### UC-DOR2: A developer brings their own doctrine

```gherkin
Given a developer's computer publishes a domain from one docs repository
When they adopt their own docs repository that publishes the same domain
Then its publication is refused and names the current owner
When the first repository stops publishing that domain and their own repository publishes it
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
  repository is that docs repository and declares `publishes` for `general`
  and `sdlc`.

## See Also

- local/projects/instrukt/spec/domain/publication
- local/projects/instrukt/spec/feature/publication/project-publication
- local/projects/instrukt/spec/feature/publication/agent-entrypoints
- local/projects/instrukt/spec/feature/workflow/workflow-definition
