---
description: "Project initialization — `instrukt init` adopts a git repository in one idempotent step: project layer with its slug, `docs/project/` with its `index.yaml` and, when sdlc is published, a copy of sdlc's `schema.yaml`, editor schemas with modelines, and instrukt's hook sections; it reports the claims a first sync will make and never overwrites what exists."
---
# Project Initialization — Spec

## What it is

A developer adopts instrukt in an existing git repository with one command.
`instrukt init` writes what instrukt needs and nothing else, never overwrites
a file that exists — with one exception: it inserts or brings up to date the
schema modeline in the first line of instrukt's YAML files
(`project/instrukt/spec/feature/publication/editor-schemas`), leaving
every other line as the developer wrote it — and can be run again at any time to complete a partial
adoption. It shows what the repository will claim on the computer before
anything is published. It commits nothing: the developer reviews and commits
the result, and that first commit to main starts publication through the hook
sections.

## Canonical fields

<!-- planned-change:unify-index-yaml-entrypoints -->
Adoption replaces whole instruction files and never injects into existing
ones. A project's root `AGENTS.md` is taken over by adoption: when a
hand-written root `AGENTS.md` exists without a master, init writes the
purpose-only master as it does for any repository and keeps the file's
content, byte for byte, at `.instrukt/adopted/AGENTS.md`, so nothing the
developer wrote is lost while it finds its place. The master never takes that
content: what a project's `AGENTS.md` includes comes only from
`docs/project/index.yaml`.
<!-- change:unify-index-yaml-entrypoints -->
Adoption replaces whole instruction files and never injects into existing
ones. A project's root `AGENTS.md` is taken over by adoption: when a root
`AGENTS.md` exists that is not instrukt's rendering and no copy of it has
been kept yet, init keeps the file's content, byte for byte, at
`.instrukt/adopted/AGENTS.md`, whether or not the repository already holds a
project index, so nothing the developer wrote is lost while it finds its
place. What a project's `AGENTS.md` includes comes only
from `docs/project/index.yaml`.
<!-- /planned-change:unify-index-yaml-entrypoints -->

<!-- planned-change:unify-index-yaml-entrypoints -->
Placing it is migration the adopting agent guides
(`project/instrukt/spec/feature/distribution/agent-skill`): the file's purpose
into the master's `## Purpose`, each instruction into a project snippet of its
type under `docs/project/` with its entry in the index, and the kept copy
removed once nothing in it is left unplaced.
<!-- change:unify-index-yaml-entrypoints -->
Placing it is migration the adopting agent guides
(`project/instrukt/spec/feature/distribution/agent-skill`): the file's purpose
into a project snippet named first in the index's `expanded`, each
instruction into a project snippet of its type under `docs/project/` with its
entry in the index, and the kept copy removed once nothing in it is left
unplaced.
<!-- /planned-change:unify-index-yaml-entrypoints -->

The root `AGENTS.md` is generated from then on, and it stays committed: the
commit that adopts the project carries its rendering
(`project/instrukt/spec/feature/publication/agent-entrypoints`). Init reports
the kept copy and that the file is now generated; the next commit through the
hook writes the rendering over the hand-written file. Init never deletes or
edits the file itself: the kept copy is what holds the developer's words
until they are placed. The global agent files are instrukt's own, and
the sync of the project that owns `docs/general/` writes them whole
(`project/instrukt/spec/feature/publication/agent-entrypoints`).

### What init establishes

<!-- planned-change:unify-index-yaml-entrypoints -->
| Part | Where | Written when |
| --- | --- | --- |
| Project layer | `instrukt.yaml` at the repository root, holding `schema_version` and `project` (the slug, from the primary worktree's folder name, normalized) with defaults | absent; an existing one is brought to the current schema version instead (`project/instrukt/spec/domain/configuration`) |
| Project docs | `docs/project/` | absent |
| Project index | `docs/project/index.yaml`, with empty `expanded` and `listed` | absent |
| Project schema | `docs/project/schema.yaml`, a copy of the published `sdlc` domain's `schema.yaml` | absent and `sdlc` is published on the computer with a `schema.yaml` |
| Master | `AGENTS.master.md` at the repository root, holding only `## Purpose` with the project name | absent |
| Kept instructions | `.instrukt/adopted/AGENTS.md`, the hand-written root `AGENTS.md` byte for byte | a root `AGENTS.md` exists that does not open with instrukt's generated line, and there is no master |
| Editor schemas and modelines | `.instrukt/schemas/` and the first line of each file they describe | as `project/instrukt/spec/feature/publication/editor-schemas` |
| Ignores | `.gitignore`: `.instrukt/` | the line is missing; it is appended, every other line kept |
| Hook sections | the hooks directory | as `instrukt hooks install` (`project/instrukt/spec/feature/repository/hook-sections`) |
<!-- change:unify-index-yaml-entrypoints -->
| Part | Where | Written when |
| --- | --- | --- |
| Project layer | `instrukt.yaml` at the repository root, holding `schema_version` and `project` (the slug, from the primary worktree's folder name, normalized) with defaults | absent; an existing one is brought to the current schema version instead (`project/instrukt/spec/domain/configuration`) |
| Project docs | `docs/project/` | absent |
| Project index | `docs/project/index.yaml`, with empty `expanded` and `listed` | absent |
| Project schema | `docs/project/schema.yaml`, a copy of the published `sdlc` domain's `schema.yaml` | absent and `sdlc` is published on the computer with a `schema.yaml` |
| Kept instructions | `.instrukt/adopted/AGENTS.md`, the hand-written root `AGENTS.md` byte for byte | a root `AGENTS.md` exists that is not instrukt's rendering, and no kept copy exists |
| Editor schemas and modelines | `.instrukt/schemas/` and the first line of each file they describe | as `project/instrukt/spec/feature/publication/editor-schemas` |
| Ignores | `.gitignore`: `.instrukt/` | the line is missing; it is appended, every other line kept |
| Hook sections | the hooks directory | as `instrukt hooks install` (`project/instrukt/spec/feature/repository/hook-sections`) |
<!-- /planned-change:unify-index-yaml-entrypoints -->

- Refused outside a git repository's working tree.
- Reports each part as created, completed or already present.
- Run again on an adopted repository, it changes nothing.
- Init installs no git filter and writes no `.gitattributes` line;
  documentation references are snippet ids that the CLI resolves when read.

### The project schema

- Created with the copy, init reports "given the sdlc schema; change it as you
  see fit".
- When `sdlc` is not published on the computer, init writes no
  `docs/project/schema.yaml` and says so; the project follows the root rules
  only (`project/instrukt/spec/domain/docs`).
- Once it exists, the file is the project's own: init and sync never rewrite
  it, apart from its modeline.

### The claims report

In a repository whose working tree holds `docs/general/` or any
`docs/domain/<x>/`, init reports, before anything is published:

- each position the repository's first sync will claim — `general` and each
  domain by name — and, for each, whether it is free, already owned by this
  repository, or owned by another project on this computer (naming that
  owner, since the sync would be refused);
- each `docs/domain/general/` or `docs/domain/project/`, as refused because
  the names are reserved;
- whether the sync will write the global agent files and, for the agent
  runtimes the selected runtime reports as configured, each file it will
  write, whatever that file holds now
  (`project/instrukt/spec/feature/publication/agent-entrypoints`).

The report claims nothing and writes nothing; ownership is taken by the sync
(`project/instrukt/spec/feature/publication/project-publication`).

## Use cases

<!-- planned-change:unify-index-yaml-entrypoints -->
### UC-IN1: A developer adopts instrukt in their repository in one step

```gherkin
Given a developer's existing git repository with no instrukt configuration
When the developer initializes instrukt in it
Then the repository has a project configuration named after its primary checkout's folder
And it has a project documentation folder with an empty index and a master stating the project's purpose
And instrukt's hooks are installed beside any hooks the repository had
And nothing has been committed
```
<!-- change:unify-index-yaml-entrypoints -->
### UC-IN1: A developer adopts instrukt in their repository in one step

```gherkin
Given a developer's existing git repository with no instrukt configuration
When the developer initializes instrukt in it
Then the repository has a project configuration named after its primary checkout's folder
And it has a project documentation folder with an index that selects nothing yet
And instrukt's hooks are installed beside any hooks the repository had
And nothing has been committed
```
<!-- /planned-change:unify-index-yaml-entrypoints -->

<!-- planned-change:unify-index-yaml-entrypoints -->
### UC-IN5: A developer adopting a repository with its own agent instructions keeps what they wrote

```gherkin
Given a developer's repository has a hand-written root agent instruction file and no instrukt configuration
When the developer initializes instrukt in it
Then everything the file said is kept aside for them, and the project's master holds only the purpose
And the developer is told that the instruction file is generated from now on
And the file itself is left as it was
When the developer commits
Then the commit carries the rendered instruction file in its place, and the kept copy still holds what they wrote
When their agent goes through the kept instructions with them
Then each lands in the project's documentation and index, and the published instruction file carries them
```
<!-- change:unify-index-yaml-entrypoints -->
### UC-IN5: A developer adopting a repository with its own agent instructions keeps what they wrote

```gherkin
Given a developer's repository has a hand-written root agent instruction file and no instrukt configuration
When the developer initializes instrukt in it
Then everything the file said is kept aside for them, and the project's index selects nothing yet
And the developer is told that the instruction file is generated from now on
And the file itself is left as it was
When the developer commits
Then the commit carries the rendered instruction file in its place, and the kept copy still holds what they wrote
When their agent goes through the kept instructions with them
Then each lands in the project's documentation and index, and the published instruction file carries them
Given another repository that already holds its own project index beside a hand-written root agent instruction file never kept aside
When the developer initializes instrukt in it
Then everything that file said is kept aside for them as well
```
<!-- /planned-change:unify-index-yaml-entrypoints -->

<!-- planned-change:unify-index-yaml-entrypoints -->
### UC-IN2: Initializing again completes an adoption without touching the developer's files

```gherkin
Given a developer's repository that already has its own master, project index and ignore file
And instrukt's hooks are missing
When the developer initializes instrukt in it
Then the developer's master and ignore lines are unchanged, and the project index is unchanged apart from its schema modeline
And the missing hooks are installed
When the developer initializes instrukt once more
Then nothing in the repository changes
```
<!-- change:unify-index-yaml-entrypoints -->
### UC-IN2: Initializing again completes an adoption without touching the developer's files

```gherkin
Given a developer's repository that already has its own project index, project documents and ignore file
And instrukt's hooks are missing
When the developer initializes instrukt in it
Then the developer's documents and ignore lines are unchanged, and the project index is unchanged apart from its schema modeline
And the missing hooks are installed
When the developer initializes instrukt once more
Then nothing in the repository changes
```
<!-- /planned-change:unify-index-yaml-entrypoints -->

### UC-IN3: A developer adopting a docs repository sees what it will claim before it is published

```gherkin
Given a developer's computer where another project already owns one domain
And a repository holding always-on doctrine, a free domain and that owned domain
When the developer initializes instrukt in the repository
Then they are told it will claim the always-on doctrine and the free domain
And that the owned domain belongs to the other project and its publication would be refused
And which global agent instruction files publication will write, for which configured agent runtimes
And nothing has been published
```

### UC-IN4: A developer's new project starts with the shape of the computer's sdlc doctrine

```gherkin
Given a developer's computer publishes the sdlc domain with its snippet schema
When the developer initializes instrukt in a new repository
Then the project's documentation has its own copy of that schema
And the developer is told it was given the sdlc schema to change as they see fit
When the published sdlc schema changes and the developer initializes again
Then the project's own schema is unchanged
Given a computer that does not publish sdlc
When the developer initializes instrukt in a new repository
Then no project schema is written and the developer is told why
```

## Known caveats

- The slug comes from the primary worktree's folder name, normalized to a
  slug. `instrukt init --slug <slug>` supplies an explicit usable slug instead
  of the folder-derived value. An empty result without that override is
  refused and names `--slug <slug>`.
- The claims report reflects the computer at the moment init runs; the sync
  decides against the ownership record it holds the docs lock on.

## See Also

- [project/instrukt/spec/domain/repository](../../domain/repository.md) — the repository context.
- [project/instrukt/spec/feature/repository/hook-sections](hook-sections.md) — the hook sections init installs.
- [project/instrukt/spec/feature/publication/editor-schemas](../publication/editor-schemas.md) — the editor schemas and modelines init writes.
