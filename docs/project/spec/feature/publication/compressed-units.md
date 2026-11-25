---
description: Compressed units — every repository that owns inline documentation commits a compressed counterpart for each source an entrypoint rendering holds in full, named by the source digest it was written from; each entrypoint rendering holds a current unit in place of its source and the source's prose for each stale or missing one alone; an offline inspection names what is stale, missing or orphaned; publication records each unit's lineage; instrukt never writes or judges the compressed text.
delivered_by: [compressed-global-entrypoints]
---

# Compressed Units — Spec

## What it is

A compressed unit is a source's dense counterpart, written by an agent and
committed in the repository that holds the source. An entrypoint rendering
holds a source's unit in place of its prose while the unit is current, so an
agent receives every rule of the selection in less space, and the full prose
of each rule stays one read away.

A unit belongs to one source and is current only while that source is
unchanged. A change to one source makes that source's unit alone stale; the
rendering then holds that one source in prose until its unit is renewed, and
every other source stays compressed. A unit is never served for a source it
no longer stands for, and a change to the prose reaches agents without
waiting on compression.

instrukt treats units mechanically. It reads them, decides whether each is
current, holds them to the structural rules of the rendering they enter,
renders with them and reports their state. It never calls a model, writes a
unit's text or judges whether the text means what its source means
(`general/principle/intelligence-boundaries`). Producing a unit is an agent's
work: an authoring agent in its change, or the post-merge automation a
repository composes from its shared workflows
(`project/instrukt/policy/shared-workflows`).

## Canonical fields

### Which sources have a unit

A source needs a unit when an entrypoint rendering holds it in full:

- each `expanded` id of `docs/general/index.yaml` and each id of its `agents`
  lists, with their required-read closures;
- each `expanded` id of a project's `docs/project/index.yaml`, with its
  closure, for the snippets the project's root `AGENTS.md` holds in full;
- each `expanded` document of a company's `docs/index.yaml`, with its
  required reads.

A domain's own `index.yaml` selects nothing compressed: the domain read, a
`listed` snippet, a triggered procedure and a domain line always reach agents
in prose (`project/instrukt/spec/feature/docs/progressive-retrieval`).

A source's unit lives in the repository that holds the source: the
repository that owns its position for a snippet, the company's repository
for a company document. Every doc-owning repository keeps its units the same
way, in one place outside every documentation position, each at the source's
own path relative to `docs/`. A checkout anywhere, a fresh clone, a worktree,
a company container or a CI runner, therefore renders with the same units
offline. A unit lies outside every documentation position, so it is no
snippet (`project/instrukt/spec/domain/docs`), and it is no company document
(`project/instrukt/spec/feature/publication/agent-entrypoints`).

An index that selects sources carries only the selection. No digest, model
or other fact about a unit is written into it.

### What a unit holds

- **Its source digest.** The SHA-256 of the source's delivered prose when the
  unit was written, declared in the unit's frontmatter. It is the unit's
  identity and its causal link to its source.
- **Its model.** The model that wrote the unit, declared beside the source
  digest.
- **Its body.** The source compressed into plain ASCII that agents of
  different model families read alike, keeping every behavioral invariant of
  the source with a one-clause reason. Configured-value directives stand in
  the body exactly as in the source.

### Current, stale, missing, orphaned

| State | When |
| --- | --- |
| current | the unit's source digest equals the digest of its source's delivered prose |
| stale | the unit exists and its source digest differs |
| missing | a source of this repository that an entrypoint index of this repository inlines has no unit |
| orphaned | the unit's source no longer exists; or its source is a `general`, project or company source that no entrypoint index of the repository inlines |

A unit of a domain snippet is orphaned only when the snippet no longer
exists: another repository's `docs/general/index.yaml` may inline it. The
state depends on the repository's committed content alone, so it is the same
on every computer and for every runtime that renders it.

### Rendering with units

The entrypoint renderings hold sources this way: the global agent files, a
project's root `AGENTS.md`, a company home's `AGENTS.md`
(`project/instrukt/spec/feature/publication/agent-entrypoints`), and the main
file of a company plugin's skill
(`project/instrukt/spec/feature/publication/company-plugin`).

- **A current unit replaces its source.** The rendering holds the unit's body
  where it would hold the source, without the unit's frontmatter, under the
  source's canonical id or company document path.
- **Stale and missing fall back, one source at a time.** A source whose unit
  is stale or missing is held in its full prose. That decision is taken for
  that source alone, changes how no other source is rendered, is no finding,
  and never stops a sync, a commit, a rendering or a check. An orphaned unit
  is never rendered.
- **The legend once.** A rendering that holds at least one unit carries one
  fixed legend of the notation once, before the first unit, in text every
  runtime loads, including a runtime that strips HTML comments. A rendering
  that holds no unit carries no legend.
- **Deterministic.** The same content rendered by the same instrukt release
  yields the same bytes on every computer, with units as without them.

### Structural rules

A unit entering a rendering is held to every structural rule that rendering
applies to its source: the directives it forbids or resolves, references an
agent program would be left to resolve, and, in a company's knowledge, the
rule that it references only the company's own documents and the plugin's
visibility rules. A configured-value directive the unit carries and its
source does not, or one the source carries and the unit lacks, breaks that
rule. Each of these is a finding naming the unit, as is a unit whose
frontmatter cannot be read or lacks its source digest or model:

- every unit is read from the repository's place for units and nowhere else,
  and nothing is read through a symbolic link at or above that place or
  inside it; such a link is a finding naming it;
- a unit with a finding never stands in for its source. A rendering that
  stops on a finding stops on it as on a finding of its source
  (`project/instrukt/spec/feature/publication/agent-entrypoints`,
  `project/instrukt/spec/feature/publication/company-plugin`,
  `project/instrukt/spec/feature/publication/project-publication`); a
  rendering that is written despite a finding, as a global agent file is
  with a reference left for an agent program, holds the source's prose in
  that unit's place and reports the finding.

### Inspecting units

An agent or a pipeline asks instrukt, in a repository's checkout, which of
its sources need compressing. The inspection is the publication context's
command, like the renderings'. It reads that checkout's files alone: no home,
publication, database, configuration or network is needed, and a fresh clone
with the binary is enough. It writes nothing.

- It names each source whose unit is stale or missing, with the source's
  canonical id or company document path, the source's file, where its unit
  belongs, and the digest of its current delivered prose, which is the
  source digest a renewed unit declares.
- It names each orphaned unit, and each finding of the Structural rules above
  that it can establish from the checkout.
- When every source that needs a unit has a current one and nothing is
  orphaned or found, it says there is nothing to compress, and its outcome
  tells that apart from an outcome that names work.

### Lineage

Each sync that publishes records, for each snippet it publishes, in the
home's global index: the digest of the snippet's delivered prose and, when
the snippet has a unit, the source digest the unit declares and the digest
of the unit's body. They are computed once, by the sync that publishes the
commit, and read from there afterwards. The index holds these facts and no
verdict on whether a unit is current. A company's knowledge is not published
(`project/instrukt/spec/feature/configuration/company-homes`); its units carry
their own source digests in its repository.

Which units the eval fingerprint covers is
`project/instrukt/spec/feature/publication/eval-fingerprint`'s.

## Use cases

### UC-CU1: A developer's agents receive the always-on doctrine compressed

```gherkin
Given a developer's computer publishes always-on doctrine whose every source has a current compressed unit
When publication runs for the project that owns the always-on doctrine
Then each configured runtime's global instruction file holds every always-on source as its compressed unit, under the source's name
And it holds the notation's legend once, before the first unit
And none of those sources is held in prose
```

### UC-CU2: A maintainer's prose change reaches agents before its compression catches up

```gherkin
Given a maintainer's repository whose always-on sources each have a current compressed unit
When the maintainer changes one of those sources and merges the change without renewing its unit
And the repository's renderings are made from the merged content
Then that source reaches agents in its changed prose
And every other source reaches agents compressed
And the maintainer's checks, the commit and publication all succeed
When a renewed unit for that source is committed
Then that source reaches agents compressed again
```

### UC-CU3: A maintainer's compression pipeline finds exactly what needs compressing

```gherkin
Given a fresh clone of a maintainer's repository and the instrukt binary, with no home and no network
And one inlined source changed since its unit was written, another inlined source has no unit, and one unit's source was removed
When an agent acting for the maintainer asks which sources need compressing
Then it is told the two sources by name and file, where each unit belongs, and the digest each renewed unit declares
And it is told the unit whose source was removed
When every inlined source has a current unit and no unit is left over
Then it is told there is nothing to compress
```

## Known caveats

- instrukt decides whether a unit is current and structurally admissible,
  never whether its text keeps every rule of its source. A unit the
  post-merge automation commits reaches agents on the strength of the source
  review alone; its model, its lineage and the eval fingerprint are what an
  evaluation attributes a regression to.
- Between a merged prose change and its renewed unit, the rendering holds
  that source in prose, and a file close to its runtime's size limit can pass
  it for that while; the inspection names the source.
- A domain repository's inspection names no missing unit for a domain
  snippet that only another repository's always-on selection inlines; that
  snippet is held in prose in the global files until its unit is committed.
- A runtime's size limit is that runtime's; nothing measures a written file
  against it.

## See Also

- [project/instrukt/spec/feature/publication/agent-entrypoints](agent-entrypoints.md) — the renderings that hold units.
- [project/instrukt/spec/feature/publication/company-plugin](company-plugin.md) — the company plugin's skill.
- [project/instrukt/spec/feature/publication/eval-fingerprint](eval-fingerprint.md) — the fingerprint that covers units.
