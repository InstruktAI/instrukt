---
description: The configuration bounded context — `instrukt.yaml` in a global and a project layer, per-key layer rules, schema versions and migration, `instrukt config get|patch|init|validate`, interpolation and runtime selection.
delivered_by: [instrukt-first-build]
---

# Configuration — Spec

## What it is

The configuration context owns `instrukt.yaml`: one schema, read in two layers
on git's model of a user file and a repository file. The global layer
(`~/.instrukt/instrukt.yaml`) holds a machine's defaults and the settings only a
machine may decide; the project layer (`instrukt.yaml` at a repository's root)
holds what a project declares and its overrides. The context decides which key
may live in which layer, validates documents, migrates older schema versions,
edits documents safely under concurrent change, and answers values with
interpolation — including the caller identity recognised by the explicitly
selected runtime adapter. It owns no behavior the keys configure: each key's
meaning belongs to the context or module that reads it.

## Canonical fields

Global `agents.order` supplies the default ordered agent preferences with
thinking mode. Dispatch preferences use the selected adapter's capabilities and
availability metadata; there are no executable runtime templates in
configuration. `get --revision` prints the layer revision; `get --json` renders
JSON.

### Ubiquitous language

**Layer**
: One `instrukt.yaml` document: the **global layer** of the machine or the
**project layer** of a repository.

**Effective configuration**
: The global layer with the project layer applied over it, for a caller
standing in a project; the global layer alone elsewhere.

**Layer rule**
: Which layer a key may live in: global-only, project-only, or both (the
project overriding).

**Schema version**
: The `schema_version` every layer document carries, naming the schema it was
written against.

**Migration**
: Rewriting a document of an older schema version into the current one,
preserving its settings.

**Revision**
: An opaque digest of a layer document's content, used to refuse a patch made
against a document that has changed since it was read.

**Merge patch**
: A partial document applied to a layer: mappings merge key by key, a null
removes a key, any other value replaces.

**Interpolation**
: Expanding a value's references when it is read: `{other.key}`, `${VAR}`, and
the derived `runtime.session_id`.

**Selected runtime**
: The compiled adapter explicitly chosen by global `runner.runtime`, initially
`t3`. Absence means no runtime, not a request for detection or a default.

**Caller identity**
: The selected adapter's recognition of the current native session. It may
be absent outside that runtime and does not change runtime selection.

**Project slug**
: The `project` key of a project layer; the identity of the project's
`docs/project/` documentation (ids `project/<slug>/…`) and of its index entries.

### Ownership boundary

- **Owns** — the `instrukt.yaml` schema; layer rules; validation of a layer
  document; schema versions and migration; the effective configuration;
  revisions and safe patching; interpolation; validation of the explicit runtime choice and derived caller identity.
- **Does not own** — what any key does: docs and publication read
  `project`; repository decides when publication runs and its commit
  composition reads adapter caller identity and `agents.<name>.git_author` and decides the
  commit author and trailers; blueprint, work, workflow and runner read the
  keys and module switches governing their capabilities; the flavor's
  procedures call `actions` by name. When migration runs during publication, publication decides when;
  configuration decides how.
- **Published contracts** — `instrukt config get [<path>] [--global]`,
  `instrukt config patch --revision <rev> [--global]`,
  `instrukt config init [--global]`, `instrukt config validate [--global]`;
  the migration operation that `init` and publication invoke; the
  configuration schema type the editor schema of `instrukt.yaml` is generated
  from (`project/instrukt/spec/feature/publication/editor-schemas`); the explicit runtime selection and adapter caller-identity lookup used by
  commit composition.
- **Allowed dependencies** — none. Configuration depends on no other context.
  It declares the caller-identity lookup interface; CLI wiring supplies the
  selected adapter's environment-only recognition without importing runner.
- **Implementation roots** — the configuration module of the `instrukt-core`
  crate; its module layout is set by the architecture design.

### Aggregates

#### Layer document (aggregate)

- **Root:** Layer document (global or project).
- **Contains:** `schema_version`; settings; explicit runtime choice (global only).
- **Invariants:**
  - Every key it holds is permitted in its layer; a key of the other layer is
    a finding, and a write that would introduce one is refused.
  - It carries a `schema_version`.
  - A written document is valid as a whole; no write leaves an invalid or
    partly written document behind.
  - A patch applies only against the revision it was made from.
- **Identifier:** its layer — the machine for the global layer, the
  repository root for a project layer.
- **Lifecycle:** created by `instrukt config init` (or `instrukt init`, which
  writes the project layer with the same defaults); changed by `patch`;
  migrated when `init` or publication finds an older schema version; never
  deleted by instrukt.

### Domain events emitted

None. instrukt runs no daemon and emits no events from this context.

### Domain events consumed

None.

### Use cases (index of feature specs)

- **layered-configuration** — `project/instrukt/spec/feature/configuration/layered-configuration` — a machine's settings and a project's settings combine, a cloned project cannot decide what runs on the machine, and older documents keep working.
- **configuration-editing** — `project/instrukt/spec/feature/configuration/configuration-editing` — settings are created, read and changed without losing a concurrent change or writing an invalid document.
- **configuration-skill** — [project/instrukt/spec/feature/configuration/configuration-skill](../feature/configuration/configuration-skill.md) — one skill coordinates local configuration and infrastructure verification across participating computers through the selected runtime, or locally without a route.
- **interpolation** — `project/instrukt/spec/feature/configuration/interpolation` — values resolve across keys, the environment and the caller's own runtime session when read.
- **logging** — `project/instrukt/spec/feature/configuration/logging` — every command's diagnostics are recorded where the InstruktAI logging contract puts them, readable afterwards with `instrukt-ai-logs instrukt`.

### Governing designs and decisions (index)

None yet beyond the feature specs above.

### Context map position

- **Docs** — customer / supplier: configuration supplies read-time
  interpolation through the CLI.
- **Publication** — customer / supplier: configuration supplies the project
  slug, runtime choice and the configuration schema type the editor schema of
  `instrukt.yaml` is generated from. Publication also invokes configuration’s
  migration operation; it is a callable contract, not a configuration key. What a
  project publishes follows from its `docs/general/` and `docs/domain/<x>/`
  positions, never from a configuration key.
- **Repository** — customer / supplier: configuration supplies the project
  slug, runtime choice and agent author settings.
- **Specs** — customer / supplier: configuration supplies the project slug
  identifying the inventory and bindings.
- **Blueprint** — customer / supplier: configuration supplies its module
  switch and engine selection.
- **Work, workflow, runner** — customer / supplier: configuration supplies
  board, approval gates, runtime choice and runner settings.

These are context relationships; module switches determine enablement, not
semantic ownership. Configuration is the supplier in each relationship.

## Known caveats

None beyond those in the feature specs; key names, layers and defaults are
`project/instrukt/spec/feature/configuration/layered-configuration`.
