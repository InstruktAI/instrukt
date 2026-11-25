---
description: "Editor schemas — the binary generates JSON Schemas from its own types for each position's `index.yaml`, for `schema.yaml` and for `instrukt.yaml` into the repository's git-ignored `.instrukt/schemas/`, and `init` and the pre-commit section keep a relative `yaml-language-server` modeline at the top of each of those files, so the editor and the validator read the same rules."
---
# Editor Schemas — Spec

## What it is

A developer editing instrukt's YAML files in a repository gets completion and
validation from their editor, from the same rules the installed instrukt
validates with. The binary generates a JSON Schema from its own document
types for the repository YAML files below, writes them into
the repository's `.instrukt/schemas/`, and keeps a modeline at the top of each
such file pointing at its schema. The schemas are generated, git-ignored and
never authored; `instrukt init` and `instrukt sync` regenerate them. No file
names a home path.

## Canonical fields

### Schemas

| File | Schema in `.instrukt/schemas/` | Generated from |
| --- | --- | --- |
| `docs/general/index.yaml` | `general-index.schema.json` | the `general` position's index type: `expanded`, `listed`, `agents`, where `agents` admits open provider keys constrained to 1–64 ASCII characters, a leading letter and then letters, digits, underscores or hyphens |
| `docs/domain/<x>/index.yaml` | `domain-index.schema.json` | the domain position's index type: `description`, which the schema requires and admits only as a text, `expanded`, `listed` |
| `docs/project/index.yaml` | `project-index.schema.json` | the project position's index type: `expanded`, `listed` |
| `schema.yaml` at the root of any position | `snippet-schema.schema.json` | the snippet-schema type |
| `instrukt.yaml` (the project layer) | `instrukt.schema.json` | the configuration schema at the current schema version |

The table lists the files that carry a modeline, each with the schema it
names. The generated set holds one schema more: `company-index.schema.json`,
generated from the type publication reads a company home's `docs/index.yaml`
with (`project/instrukt/spec/feature/publication/agent-entrypoints`:
`expanded` and `listed`, each a list of paths ending in `.md`, no other key,
and a document holding nothing admitted). It is written wherever the others
are, and no file carries a modeline naming it: in a project a file at
`docs/index.yaml` is a misplaced index, and in a company home nothing writes
`.instrukt/schemas/`.

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

`instrukt config schema --json` prints the configuration schema of the
addressed document, a company home's document included, which has no editor
schema file of its own
(`project/instrukt/spec/feature/configuration/configuration-editing`).

The separate machine-local `agents.yaml` has a hand-maintained schema in
instrukt's repository, with open model names and known-name editor suggestions
under `project/instrukt/spec/feature/configuration/agent-selection`. It is not
one of these generated repository schemas.

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
- The modeline is part of the file as committed, so it changes only in a
  commit: `init` writes it during adoption, and instrukt's pre-commit section
  inserts or corrects it in each of these files being committed and stages it
  (`project/instrukt/spec/feature/repository/hook-sections`). Each reports the
  files it changed.

### When they are refreshed

- `instrukt init` writes the schemas and modelines as part of adoption,
  through the operation publication supplies to it
  (`project/instrukt/spec/feature/repository/project-initialization`).
- Every `instrukt sync` that publishes regenerates the schemas in the
  project's primary worktree and writes no committed file. After an upgrade of
  instrukt, the next sync brings the schemas to the installed version, whether
  or not main has moved.
- A schema file already holding the generated content is left untouched.
- Nothing is written through a symbolic link. A schema path that is a link
  is replaced by the schema file, by `init` and by `sync`, as for any of
  publication's own destinations
  (`project/instrukt/spec/feature/publication/project-publication`). A file
  that would take a modeline and is a link is the developer's: it is left as
  it is and named, and the operation goes on.
- A refresh that cannot write a schema, or `init` a modeline, for another
  reason fails the operation, naming the file: `init` reports the failure and exits
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
