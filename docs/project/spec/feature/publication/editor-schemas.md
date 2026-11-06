---
description: "Editor schemas — the binary generates JSON Schemas from its own types for each position's `index.yaml`, for `schema.yaml` and for `instrukt.yaml` into the repository's git-ignored `.instrukt/schemas/`, and `init` and sync keep a relative `yaml-language-server` modeline at the top of each of those files, so the editor and the validator read the same rules."
delivered_by: [instrukt-first-build]
---
# Editor Schemas — Spec

## What it is

A developer editing instrukt's YAML files in a repository gets completion and
validation from their editor, from the same rules the installed instrukt
validates with. The binary generates a JSON Schema from its own document
types for every YAML file a developer authors for instrukt, writes them into
the repository's `.instrukt/schemas/`, and keeps a modeline at the top of each
such file pointing at its schema. The schemas are generated, git-ignored and
never authored; `instrukt init` and `instrukt sync` regenerate them. No file
names a home path.

## Canonical fields

### Schemas

| File | Schema in `.instrukt/schemas/` | Generated from |
| --- | --- | --- |
| `docs/general/index.yaml` | `general-index.schema.json` | the `general` position's index type: `expanded`, `listed`, `agents`, where `agents` admits only the supported agent CLI names (`claude`, `codex`, `gemini`, `deepseek`) |
| `docs/domain/<x>/index.yaml` | `domain-index.schema.json` | the domain position's index type: `description`, `expanded`, `listed` |
| `docs/project/index.yaml` | `project-index.schema.json` | the project position's index type: `expanded`, `listed` |
| `schema.yaml` at the root of any position | `snippet-schema.schema.json` | the snippet-schema type |
| `instrukt.yaml` (the project layer) | `instrukt.schema.json` | the configuration schema at the current schema version |

- Each schema is generated from the same type the validator reads that file
  with — the position index and snippet schema types of the docs context
  (`project/instrukt/spec/feature/docs/position-index`,
  `project/instrukt/spec/feature/docs/snippet-schema`) and the
  configuration schema
  (`project/instrukt/spec/domain/configuration`) — so the editor and
  `instrukt docs validate` or `instrukt config validate` never disagree about
  which keys a file may hold. A key the schema does not declare is reported by
  the editor as it is by the validator.
- The configuration type includes direct project `webhooks.<domain>`
  declarations with required current and optional previous secret references,
  native/GitHub/generic HMAC verification and exclusive event-name sources.
  Project editor schemas reject global-only Redis and ingress keys; source
  domains, verification/name-source constraints and reference shapes match configuration validation. Runtime ingress
  secret resolution remains a validation operation, not secret data in a schema.
- The schemas are a pure function of the installed binary: the same instrukt
  version writes the same bytes on every computer.
- `.instrukt/` is ignored through the line `instrukt init` adds to
  `.gitignore` (`project/instrukt/spec/feature/repository/project-initialization`).

### Modelines

- Each file in the table that exists in the repository's primary worktree
  starts with `# yaml-language-server: $schema=<path>`.
- `<path>` is relative, from the file's own directory to its schema inside the
  repository's `.instrukt/schemas/`, because an editor resolves a modeline's
  relative path against the file that carries it: `instrukt.yaml` names
  `.instrukt/schemas/instrukt.schema.json`, `docs/domain/sdlc/index.yaml`
  names `../../../.instrukt/schemas/domain-index.schema.json`. A modeline
  never holds a home path or an absolute path.
- A missing modeline is inserted as the first line; a modeline naming another
  path is replaced. Every other byte of the file is kept.
- The modeline is a comment: it never changes what a file means, and
  validation reads the file the same with or without it.
- Every write instrukt makes to one of these files keeps the modeline at the
  top.
- The modeline is part of the file as committed: a modeline the refresh adds
  appears as a working-tree change for the developer to commit, and the
  refresh reports each file it changed.

### When they are refreshed

- `instrukt init` writes the schemas and modelines as part of adoption,
  through the operation publication supplies to it
  (`project/instrukt/spec/feature/repository/project-initialization`).
- Every `instrukt sync` that publishes regenerates the schemas and refreshes
  the modelines in the project's primary worktree. After an upgrade of
  instrukt, the next sync that publishes brings them to the installed
  version; `instrukt sync --force` does so at once.
- A schema file already holding the generated content is left untouched.
- A path the refresh writes that is a symbolic link is never written through:
  `init` reports it, and `sync` refuses as for any location publication writes
  (`project/instrukt/spec/feature/publication/project-publication`).
- A refresh that cannot write a schema or a modeline for another reason fails
  the operation, naming the file: `init` reports the failure and exits
  non-zero, and a sync fails as any failed sync does — it records no
  published commit, so the next sync tries again
  (`project/instrukt/spec/feature/publication/project-publication`).

## Use cases

### UC-ES1: A developer's editor checks instrukt's files as they type

```gherkin
Given a developer has initialized instrukt in their repository
When their editor opens the project's configuration, a position's index or a snippet schema
Then each of those files names, in its first line, the schema instrukt generated for it, by a path within the repository
And that schema admits exactly the keys validation admits for that file, so the editor can offer and mark them as validation would
And no file in the repository names a path in the developer's home
```

### UC-ES2: A developer's editor follows the instrukt they have installed

```gherkin
Given a developer's repository whose editor schemas were written by an older instrukt
When they upgrade instrukt and publication runs for the project
Then the editor schemas describe the rules of the installed instrukt
And every file they describe still points at its schema
```

## Known caveats

- The global configuration layer `~/.instrukt/instrukt.yaml` lives outside any
  repository and carries no modeline.
- Sync refreshes the primary worktree only. A linked worktree carries the
  committed modelines but not the git-ignored schemas; its editor resolves
  them once `instrukt init` runs in it.

## See Also

- [project/instrukt/spec/domain/publication](../../domain/publication.md) — the publication context.
- [project/instrukt/spec/feature/docs/position-index](../docs/position-index.md) — the `index.yaml` model the index schemas describe.
- [project/instrukt/spec/feature/docs/snippet-schema](../docs/snippet-schema.md) — the `schema.yaml` model its schema describes.
- [project/instrukt/spec/feature/repository/project-initialization](../repository/project-initialization.md) — the adoption that first writes them.
- [project/instrukt/spec/feature/publication/project-publication](project-publication.md) — the sync that refreshes them.
