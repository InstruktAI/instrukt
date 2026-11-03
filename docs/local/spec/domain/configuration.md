---
description: The configuration bounded context — `instrukt.yaml` in a global and a project layer, per-key layer rules, schema versions and migration, `instrukt config get|patch|init|validate`, interpolation and runtime selection.
delivered_by: [instrukt-first-build]
---

# Configuration — Spec

## What it is

The configuration context owns `instrukt.yaml`: one schema, read in two layers
on git's model of a user file and a repository file. The global layer
(`~/.instrukt/instrukt.yaml`) holds a machine's defaults and the settings only
a machine may decide; the project layer (`instrukt.yaml` at a repository's
root) holds what a project declares and its overrides. The context decides
which key may live in which layer, validates documents, migrates older schema
versions, edits documents safely under concurrent change, and answers values
with interpolation — including which configured runtime the caller is running
in. It owns no behavior the keys configure: each key's meaning belongs to the
context or module that reads it.

## Canonical fields

Global `agents.order` supplies the default ordered agent preferences with thinking mode. Runtime entries may declare an `available` check template. Dispatch names validate against agents the runtime mapping can start, never a fixed set. `get --revision` prints the layer revision; `get --json` renders JSON.

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

**Runtime mapping**
: The ordered `runtimes` list in the global layer: each entry says how one
runtime identifies its sessions (`session_id_env`) and the commands it offers.

**Active runtime**
: The first runtime-mapping entry whose `session_id_env` is set in the
caller's environment; addressed as `runtime`.

**Project slug**
: The `project` key of a project layer; the identity of the project's local
documentation and its index entries.

### Ownership boundary

- **Owns** — the `instrukt.yaml` schema; layer rules; validation of a layer
  document; schema versions and migration; the effective configuration;
  revisions and safe patching; interpolation; selection of the active runtime.
- **Does not own** — what any key does: publication reads `publishes`;
  docs reads `project`; repository decides when publication runs and its commit
  composition reads the runtime mapping and `agents.<name>.git_author` and decides the
  commit author and trailers; blueprint, work, workflow and runner read the
  keys and module switches governing their capabilities; the flavor's
  procedures call `actions` by name. When migration runs during publication, publication decides when;
  configuration decides how.
- **Published contracts** — `instrukt config get [<path>] [--global]`,
  `instrukt config patch --revision <rev> [--global]`,
  `instrukt config init [--global]`, `instrukt config validate [--global]`;
  the migration operation that `init` and publication invoke; the active-runtime
  selection rule, which commit composition applies to agent-CLI entries.
- **Allowed dependencies** — none. Configuration depends on no other context.
- **Implementation roots** — the configuration module of the `instrukt-core`
  crate; its module layout is set by the architecture design.

### Aggregates

#### Layer document (aggregate)

- **Root:** Layer document (global or project).
- **Contains:** `schema_version`; settings; the runtime mapping (global only).
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

- **layered-configuration** — `docs/local/spec/feature/configuration/layered-configuration.md` — a machine's settings and a project's settings combine, a cloned project cannot decide what runs on the machine, and older documents keep working.
- **configuration-editing** — `docs/local/spec/feature/configuration/configuration-editing.md` — settings are created, read and changed without losing a concurrent change or writing an invalid document.
- **interpolation** — `docs/local/spec/feature/configuration/interpolation.md` — values resolve across keys, the environment and the caller's own runtime session when read.

### Governing designs and decisions (index)

None yet beyond the feature specs above.

### Context map position

- **Docs** — customer / supplier: configuration supplies read-time
  interpolation through the CLI.
- **Publication** — customer / supplier: configuration supplies published
  project slug, published domains with triggers and runtime mapping. Publication
  also invokes configuration’s migration operation; it is a callable contract,
  not a configuration key.
- **Repository** — customer / supplier: configuration supplies the project
  slug, runtime mapping and agent author settings.
- **Specs** — customer / supplier: configuration supplies the project slug
  identifying the inventory and bindings.
- **Blueprint** — customer / supplier: configuration supplies its module
  switch and engine selection.
- **Work, workflow, runner** — customer / supplier: configuration supplies
  board, approval gates, runtime mapping and runner settings.

These are context relationships; module switches determine enablement, not
semantic ownership. Configuration is the supplier in each relationship.

## Known caveats

None beyond those in the feature specs; key names, layers and defaults are
`local/projects/instrukt/spec/feature/configuration/layered-configuration`.
