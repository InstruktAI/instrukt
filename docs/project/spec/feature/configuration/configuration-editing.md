---
description: Creating, reading and changing `instrukt.yaml` through `instrukt config init|get|patch` — revision checks, whole-document validation and atomic writes.
delivered_by: [instrukt-first-build]
---

# Configuration Editing — Spec

## What it is

Configuration is changed through the CLI, never by hand-editing a file that
another writer may be changing. A patch is made against the revision the
caller read; a patch against a changed document is refused instead of
overwriting it. The whole resulting document is validated before anything is
written, and the write is atomic. Every verb addresses the project layer by
default and the global layer with `--global`.

## Canonical fields

### Verbs

| Verb | Does |
| --- | --- |
| `init` | creates the layer document with the current `schema_version` and defaults when it is absent; the project layer's `project` is the primary worktree's folder name normalized to a slug, including when invoked in a linked worktree. An empty normalized slug is refused, directing the caller to `instrukt init --slug <slug>`. An existing document is not overwritten; it is migrated when older (`project/instrukt/spec/feature/configuration/layered-configuration`). |
| `get [<path>]` | answers the value at a dotted path, or the whole document without a path, from the effective configuration (the global layer with `--global`), interpolated (`project/instrukt/spec/feature/configuration/interpolation`). A scalar value prints bare. A path that does not exist fails non-zero, naming the missing segment. |
| `patch --revision <rev>` | applies a merge patch to the addressed layer (below). |
| `validate` | reports the addressed layer's findings (`project/instrukt/spec/feature/configuration/layered-configuration`). |

### Patch

- **Input:** a YAML mapping, given inline with `--yaml`, from a file with
  `--from-file`, or on standard input. Anything other than a mapping is
  refused.
- **Merge:** mappings merge key by key; a null removes the key; any other
  value replaces.
- **Revision:** `--revision` is required. When the addressed document's
  current revision differs, the patch is refused with an error naming the
  expected and the current revision, and nothing is written. The revision
  check and the write form one step with respect to other patches: of two
  patches made from the same revision, one succeeds and the other is refused.
- **Validation:** the full resulting document is validated, including layer
  rules; any finding refuses the patch, names the finding, and writes nothing.
- **Write:** atomic — the document is written to a temporary file and renamed
  over the original, so a reader sees the old or the new document, never a
  mix.
- **Stored text:** the patch is stored as given; references are stored
  unexpanded.
- **Missing layer:** a patch to a layer that does not exist is refused,
  directing the caller to `init`. A project-layer patch outside a project is
  refused.

### Revision

A revision is an opaque digest of the layer document's content. Two reads of
an unchanged document give the same revision; any change gives another.

### Use cases

#### UC-CE1: An agent changes a setting for a developer without losing anyone else's change

```gherkin
Given an agent acting for a developer has read the project's configuration
When it changes a setting based on what it read
Then the configuration holds the new setting and every other setting unchanged
But when the configuration changed after the agent read it
Then the change is refused, nothing is written, and the agent is told to read again
But when the change would leave the configuration invalid
Then the change is refused, nothing is written, and the agent is told what is wrong
```

#### UC-CE2: A developer sets up configuration for a machine or a project

```gherkin
Given a developer starts using instrukt on a machine or in a project without configuration
When they initialize the configuration
Then a valid configuration at the current schema with the current defaults exists
And machine configuration needs no project identity
And project configuration names the project by the primary worktree's folder normalized to a slug
But when that normalization yields an empty project identity
Then initialization is refused and the developer is directed to `instrukt init --slug <slug>`
But when configuration already exists at the current schema
Then it is kept as it was
But when existing configuration has an older schema
Then it is migrated to the current schema with every setting preserved
But when existing configuration has a newer schema
Then initialization is refused and the developer is directed to upgrade instrukt
```

## Known caveats

- `instrukt config get --revision` prints the addressed layer's current revision.
- `get --json` renders JSON, including when no path is supplied.

## See Also

- project/instrukt/spec/domain/configuration
- project/instrukt/spec/feature/configuration/layered-configuration
