---
description: Creating, reading and changing `instrukt.yaml` through `instrukt config init|get|patch` — revision checks, whole-document validation and atomic writes.
binding_reservations:
  UC-CE3: instrukt-first-build
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

Configuration migration precedes refusal or skip on every verb, not only init.
Owned-file evolution and protected secret commands are
`project/instrukt/spec/feature/configuration/owned-files`.

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

### Webhook and transport edits

The same revision-checked patch validates raw source references before
interpolation and checks the full effective event switch. Invalid source shapes,
domain keys, verification schemes, conflicting name sources, layer placement or a source with events off refuse atomically.
Ingress requires its signing references to have nonblank values in the machine's
owner-only `~/.instrukt/secrets.env`; non-ingress source validation may carry
those reference names without needing their signing values. Every path reads
its required instrukt secrets through that same protected machine-local file. Secret
resolution errors name project and reference, never value. `get`, `get --json`,
revision reports and patch diagnostics retain or mask secret references;
expanded signing/Redis credentials never enter their output. Reference text is
stored unexpanded. No config read or patch installs services. The next received
request observes a valid source change without receiver restart. Patches address
`webhooks.<domain>`; that domain selects `/hooks/inbound/project/<slug>/<domain>`
and the event type prefix. Source fields are `secret`, `previous_secret`, `verify`,
`header`, `name_from` and `name` under the inbound framework contract.

### Module infrastructure workflow

Enabling a module is a desired-state change, not evidence that its dependencies
are installed. The configuration agent reads settings and their revision, patches
the intended switches, validates, then invokes `instrukt install` and
`instrukt install --check`. It inspects `instrukt ticks status` and, for events,
`instrukt events status`; dead letters are available through
`instrukt events dead-letters`. These commands are defined by
`project/instrukt/spec/feature/runner/module-installation`.

Rerunning `instrukt install` repairs missing or stale owned resources and reuses
working infrastructure. It evaluates all adopted projects before shared withdrawal.
`--check` compares without changing configuration, services, data or reader
positions. Failed setup reports unmet prerequisites separately from the committed
configuration; it never quietly changes the module switch or reports installation
success. Ordinary config reads, patch and publication install no resources.

A disable patch removes any conflicting webhook declarations atomically. It stops
new module admissions; the agent's subsequent installation convergence withdraws
only unneeded instrukt-owned units. Other projects' dependencies, all durable
state and foreign services remain. A13's configuration skill selects participating
computers and local agents; these CLI commands act only on their own computer.

### Revision

A revision is an opaque digest of the layer document's content. Two reads of
an unchanged document give the same revision; any change gives another.

## Use cases

### UC-CE1: An agent changes a setting for a developer without losing anyone else's change

```gherkin
Given an agent acting for a developer has read the project's configuration
When it changes a setting based on what it read
Then the configuration holds the new setting and every other setting unchanged
But when the configuration changed after the agent read it
Then the change is refused, nothing is written, and the agent is told to read again
But when the change would leave the configuration invalid
Then the change is refused, nothing is written, and the agent is told what is wrong
```

### UC-CE2: A developer sets up configuration for a machine or a project

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

### UC-CE3: Know whether a module's infrastructure is ready

```gherkin
Given an agent has recorded a developer's valid module enablement
When it installs and checks the module's infrastructure
Then the developer sees the installed or reused dependencies and any unmet prerequisite
And configuration success is distinguishable from installation success
And a repeated setup repairs only differing owned resources
```

## Known caveats

- `instrukt config get --revision` prints the addressed layer's current revision.
- `get --json` renders JSON, including when no path is supplied.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](layered-configuration.md)
- [project/instrukt/spec/feature/runner/module-installation](../runner/module-installation.md)
