---
description: "Project initialization — `instrukt init` adopts a git repository in one idempotent step: project layer with its slug, docs folders, a master, the docs reference filter with its attribute patterns, and instrukt's hook sections, never overwriting what exists."
delivered_by: [instrukt-first-build]
---
# Project Initialization — Spec

## What it is

A developer adopts instrukt in an existing git repository with one command.
`instrukt init` writes what instrukt needs and nothing else, never overwrites
a file that exists, and can be run again at any time to complete a partial
adoption. It commits nothing: the developer reviews and commits the result,
and that first commit to main starts publication through the hook sections.

## Canonical fields

When a hand-written `AGENTS.md` exists without a master, init seeds no master and reports that its content must first move into a `docs/local` snippet; the next run seeds the thin master.

### What init establishes

| Part | Where | Written when |
| --- | --- | --- |
| Project layer | `instrukt.yaml` at the repository root, holding `schema_version` and `project` (the slug, from the primary worktree's folder name, normalized) with defaults | absent; an existing one is brought to the current schema version instead (`local/projects/instrukt/spec/domain/configuration`) |
| Docs folders | `docs/global/` and `docs/local/` | absent |
| Master | `AGENTS.master.md` at the repository root | absent |
| Docs reference filter | the repository's own git configuration: filter `instrukt-docs`, expanding `@~/.instrukt` to the absolute home path on checkout and restoring `@~/.instrukt` on commit, marked required | not defined, or defined differently |
| Filter patterns | `.gitattributes`: `docs/**/*.md filter=instrukt-docs` | the line is missing; it is appended, every other line kept |
| Hook sections | the hooks directory | as `instrukt hooks install` (`local/projects/instrukt/spec/feature/repository/hook-sections`) |

- Refused outside a git repository's working tree.
- Reports each part as created, completed or already present.
- Run again on an adopted repository, it changes nothing.

### Use cases

#### UC-IN1: A developer adopts instrukt in their repository in one step

```gherkin
Given a developer's existing git repository with no instrukt configuration
When the developer initializes instrukt in it
Then the repository has a project configuration named after its primary checkout's folder, documentation folders and a master instruction file
And documentation references to the home are expanded on checkout and restored on commit
And instrukt's hooks are installed beside any hooks the repository had
And nothing has been committed
```

#### UC-IN2: Initializing again completes an adoption without touching the developer's files

```gherkin
Given a developer's repository that already has its own master instruction file and attribute file
And instrukt's hooks are missing
When the developer initializes instrukt in it
Then the developer's master instruction file and attribute lines are unchanged
And the missing hooks are installed
When the developer initializes instrukt once more
Then nothing in the repository changes
```

## Known caveats

- A seeded master contains only `## Purpose` with the project name and `## Required reads` naming the project's local baseline.
- The slug comes from the primary worktree's folder name, normalized to a slug. `instrukt init --slug <slug>` supplies an explicit usable slug instead of the folder-derived value. An empty result without that override is refused and names `--slug <slug>`.
- `init` adds an ignore for generated `AGENTS.md` files.

## See Also

- docs/local/spec/domain/repository.md — the repository context.
- docs/local/spec/feature/repository/hook-sections.md — the hook sections init installs.
