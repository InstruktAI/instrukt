---
description: "Project initialization — `instrukt init` adopts a git repository in one idempotent step: project layer with its slug, `docs/project/` with its `index.yaml` and, when sdlc is published, a copy of sdlc's `schema.yaml`, a purpose-only root master, editor schemas with modelines, and instrukt's hook sections; it reports the claims a first sync will make and never overwrites what exists."
delivered_by: [instrukt-first-build]
---
# Project Initialization — Spec

## What it is

A developer adopts instrukt in an existing git repository with one command.
`instrukt init` writes what instrukt needs and nothing else, never overwrites
a file that exists — with one exception: it inserts or brings up to date the
schema modeline in the first line of instrukt's YAML files
(`local/projects/instrukt/spec/feature/publication/editor-schemas`), leaving
every other line as the developer wrote it — and can be run again at any time to complete a partial
adoption. It shows what the repository will claim on the computer before
anything is published. It commits nothing: the developer reviews and commits
the result, and that first commit to main starts publication through the hook
sections.

## Canonical fields

Adoption replaces whole instruction files and never injects into existing
ones. A project's root `AGENTS.md` is taken over by adoption: when a
hand-written root `AGENTS.md` exists without a master, init seeds the master
with that file's content as its `## Purpose` (the file's own headings demoted
under it), so nothing the developer wrote is lost, and from the first sync
the root `AGENTS.md` is rendered by instrukt. Init reports that the file is now
generated and that its tracked copy is to be removed from version control in
the same commit as the master (`git rm --cached AGENTS.md`); it never deletes
or edits the file itself. User-scope agent files are the adopter's decision:
sync refuses one instrukt did not write and names it
(`local/projects/instrukt/spec/feature/publication/agent-entrypoints`).

### What init establishes

| Part | Where | Written when |
| --- | --- | --- |
| Project layer | `instrukt.yaml` at the repository root, holding `schema_version` and `project` (the slug, from the primary worktree's folder name, normalized) with defaults | absent; an existing one is brought to the current schema version instead (`local/projects/instrukt/spec/domain/configuration`) |
| Project docs | `docs/project/` | absent |
| Project index | `docs/project/index.yaml`, with empty `expanded` and `listed` | absent |
| Project schema | `docs/project/schema.yaml`, a copy of the published `sdlc` domain's `schema.yaml` | absent and `sdlc` is published on the computer with a `schema.yaml` |
| Master | `AGENTS.master.md` at the repository root, holding only `## Purpose` with the project name | absent |
| Editor schemas and modelines | `.instrukt/schemas/` and the first line of each file they describe | as `local/projects/instrukt/spec/feature/publication/editor-schemas` |
| Ignores | `.gitignore`: the generated `AGENTS.md` files and `.instrukt/` | a line is missing; it is appended, every other line kept |
| Hook sections | the hooks directory | as `instrukt hooks install` (`local/projects/instrukt/spec/feature/repository/hook-sections`) |

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
  only (`local/projects/instrukt/spec/domain/docs`).
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
- whether the sync will render the global agent files of the agent CLIs
  installed on the computer, and each existing global file instrukt did not
  write, which would stop that sync
  (`local/projects/instrukt/spec/feature/publication/agent-entrypoints`).

The report claims nothing and writes nothing; ownership is taken by the sync
(`local/projects/instrukt/spec/feature/publication/project-publication`).

### Use cases

#### UC-IN1: A developer adopts instrukt in their repository in one step

```gherkin
Given a developer's existing git repository with no instrukt configuration
When the developer initializes instrukt in it
Then the repository has a project configuration named after its primary checkout's folder
And it has a project documentation folder with an empty index and a master stating the project's purpose
And instrukt's hooks are installed beside any hooks the repository had
And nothing has been committed
```

#### UC-IN5: A developer adopting a repository with its own agent instructions keeps what they wrote

```gherkin
Given a developer's repository has a hand-written root agent instruction file and no instrukt configuration
When the developer initializes instrukt in it
Then the project's master states the purpose with everything the file said
And the developer is told that the instruction file is generated from now on and how to stop tracking it
And the file itself is left as it was until the first publication renders it
```

#### UC-IN2: Initializing again completes an adoption without touching the developer's files

```gherkin
Given a developer's repository that already has its own master, project index and ignore file
And instrukt's hooks are missing
When the developer initializes instrukt in it
Then the developer's master and ignore lines are unchanged, and the project index is unchanged apart from its schema modeline
And the missing hooks are installed
When the developer initializes instrukt once more
Then nothing in the repository changes
```

#### UC-IN3: A developer adopting a docs repository sees what it will claim before it is published

```gherkin
Given a developer's computer where another project already owns one domain
And a repository holding always-on doctrine, a free domain and that owned domain
When the developer initializes instrukt in the repository
Then they are told it will claim the always-on doctrine and the free domain
And that the owned domain belongs to the other project and its publication would be refused
And whether publication will write the global agent instructions of the agent CLIs on the computer
And nothing has been published
```

#### UC-IN4: A developer's new project starts with the shape of the computer's sdlc doctrine

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

- docs/local/spec/domain/repository.md — the repository context.
- docs/local/spec/feature/repository/hook-sections.md — the hook sections init installs.
- docs/local/spec/feature/publication/editor-schemas.md — the editor schemas and modelines init writes.
